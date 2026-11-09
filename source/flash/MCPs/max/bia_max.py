#!/usr/bin/env python3
"""bia-max: the protocol side of the MAX MCP, on PyMax (MaxApiTeam/PyMax, MIT).

Speaks the same JSON lines as bia-meta on stdin/stdout: requests {"id", "method", "params"},
answers {"id", "result"} or {"id", "error"}, events {"event", ...}. stdout carries nothing else;
logs go to stderr and from there to the MCP's log.

State in --dir: session.db (PyMax's login token, 0600) and account.json (how it logged in).
"""
import argparse
import asyncio
import base64
import json
import logging
import mimetypes
import os
import platform
import re
import sys
import time
import uuid
from collections import OrderedDict
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, os.environ.get("BIA_MAX_SITE") or str(HERE / "py" / platform.machine()))
sys.path.insert(0, str(HERE))  # bia_max_call

import aiohttp  # noqa: E402
from pymax import ApiError, Client, ExtraConfig, File, Photo, QrAuthFlow, SmsAuthFlow, Video, Voice, WebClient  # noqa: E402
from pymax.api.chats.enums import ChannelPermissions  # noqa: E402
from pymax.protocol.enums import Opcode  # noqa: E402
from pymax.types.domain.enums import ChatType, LinkType  # noqa: E402
from bia_max_call import VoiceCall, call_endpoint, call_params, calls_missing  # noqa: E402

PHOTO_EXT = {".jpg", ".jpeg", ".png", ".webp", ".gif", ".heic", ".bmp"}
VIDEO_EXT = {".mp4", ".mov", ".m4v", ".webm", ".mkv", ".avi"}
INVALID_TOKEN = ("FAIL_LOGIN_TOKEN", "FAIL_LOGOUT_ALL")
# the calls' opcodes, some of them unknown to PyMax's enum (docs in bia_max_call.py)
OP_CALL_START, OP_CALL_INVITE, OP_CALL_ANSWERED, OP_CALL_OVER = 78, 137, 120, 121
log = logging.getLogger("bia-max")


def emit(obj):
    sys.stdout.write(json.dumps(obj, ensure_ascii=False, default=str) + "\n")
    sys.stdout.flush()


def event(kind, /, **fields):
    emit({"event": kind, **fields})


def explain(e):
    """An error in words the model can act on."""
    if isinstance(e, ApiError):
        return e.localized_message or e.message or e.title or e.error or f"MAX refused the request (opcode {e.opcode})"
    if isinstance(e, (ConnectionError, TimeoutError, OSError)):
        return f"cannot reach MAX: {e or type(e).__name__}; it retries by itself, try again in a minute"
    return str(e) or type(e).__name__


def full_name(user):
    for n in getattr(user, "names", None) or []:
        name = n.name or " ".join(x for x in (n.first_name, n.last_name) if x)
        if name:
            return name
    return ""


def as_int(value, what="id"):
    try:
        return int(str(value).strip())
    except ValueError:
        raise ValueError(f"{value!r} is not a MAX {what} (they are numbers)") from None


def enum_value(v):
    return getattr(v, "value", v)


def secs(ms):
    return int((ms or 0) / 1000)


class Lru(OrderedDict):
    def __init__(self, cap):
        super().__init__()
        self.cap = cap

    def put(self, key, value):
        self[key] = value
        self.move_to_end(key)
        while len(self) > self.cap:
            self.popitem(last=False)


class Bridge:
    def __init__(self, directory):
        self.dir = Path(directory)
        self.dir.mkdir(parents=True, exist_ok=True)
        self.client = None
        self.run_task = None
        self.state = "logged_out"
        self.error = ""
        self.hint = ""
        self.qr_link = ""
        self.mode = ""
        self.code = None
        self.password = None
        self.changed = asyncio.Event()
        self.texts = Lru(5000)  # message id -> (text, sent seconds), for edit notices
        self.reacts = Lru(2000)  # message id -> {emoji: count}
        self.chats = {}  # chat id -> Chat
        self.lock = asyncio.Lock()
        self.call = None
        self.last_call = None
        self.declining = set()

    # --- state ---

    def set_state(self, state, error=None):
        self.state = state
        if error is not None:
            self.error = error
        self.changed.set()

    async def wait_state(self, wanted, timeout):
        end = time.monotonic() + timeout
        while self.state not in wanted:
            left = end - time.monotonic()
            if left <= 0:
                break
            self.changed.clear()
            if self.state in wanted:
                break
            try:
                await asyncio.wait_for(self.changed.wait(), left)
            except asyncio.TimeoutError:
                break
        return self.state

    def account_path(self):
        return self.dir / "account.json"

    def session_path(self):
        return self.dir / "session.db"

    def saved_account(self):
        try:
            return json.loads(self.account_path().read_text())
        except (OSError, ValueError):
            return {}

    def save_account(self, **fields):
        path = self.account_path()
        path.write_text(json.dumps({**self.saved_account(), **fields}))
        path.chmod(0o600)

    def own_state(self):
        """The agent runs as root: bia's own backups must still be able to read these."""
        try:
            owner = self.dir.parent.stat()
            for p in [self.account_path(), *self.dir.glob("session.db*")]:
                os.chown(p, owner.st_uid, owner.st_gid)
        except OSError as e:
            log.info("chown: %s", e)

    def forget_session(self):
        for p in self.dir.glob("session.db*"):
            p.unlink(missing_ok=True)
        self.account_path().unlink(missing_ok=True)

    def last_seen(self):
        try:
            p = self.dir.parent / "state" / "max.json"
            return int(json.loads(p.read_text()).get("last_seen", 0))
        except (OSError, ValueError):
            return 0

    def me_id(self):
        me = self.client.me if self.client else None
        return me.contact.id if me else 0

    def need_client(self):
        if not self.client or self.state != "ready":
            raise RuntimeError(f"MAX is not connected (state {self.state}); max_status says more, max_restore reconnects")
        return self.client

    # --- the connection ---

    def config(self):
        kw = dict(telemetry=False, relogin=False, reconnect=True, reconnect_delay=5.0, password_max_attempts=3, log_level="WARNING")
        if os.environ.get("BIA_MAX_HOST"):
            kw["host"] = os.environ["BIA_MAX_HOST"]
        if os.environ.get("BIA_MAX_PORT"):
            kw["port"] = int(os.environ["BIA_MAX_PORT"])
        if os.environ.get("BIA_MAX_TLS") == "0":
            kw["use_ssl"] = False
        if os.environ.get("BIA_MAX_WS_URL"):
            kw["url"] = os.environ["BIA_MAX_WS_URL"]
        return ExtraConfig(**kw)

    async def stop_client(self):
        if self.call is not None:
            await self.call.end(self.call.hangup_reason())
        client, task = self.client, self.run_task
        self.client, self.run_task = None, None
        if task and not task.done():
            task.cancel()
            try:
                await task
            except BaseException:
                pass
        if client:
            try:
                await client.close()
            except Exception:
                log.debug("close failed", exc_info=True)

    def build_client(self, mode, phone):
        if mode == "qr":
            flow = QrAuthFlow(self, self)
            client = WebClient(work_dir=str(self.dir), session_name="session.db", extra_config=self.config(), auth_flow=flow)
        else:
            flow = SmsAuthFlow(self, self)
            client = Client(phone=phone, work_dir=str(self.dir), session_name="session.db", extra_config=self.config(), auth_flow=flow)
        client.on_start()(self.on_started)
        client.on_message()(self.on_message)
        client.on_message_edit()(self.on_edit)
        client.on_message_delete()(self.on_delete)
        client.on_reaction_update()(self.on_reaction)
        client.on_chat_update()(self.on_chat)
        client.on_disconnect()(self.on_disconnect)
        client.on_raw()(self.on_raw)
        return client

    async def start_client(self, mode, phone=""):
        await self.stop_client()
        self.mode = mode
        self.error = ""
        self.hint = ""
        self.qr_link = ""
        self.set_state("connecting")
        self.client = self.build_client(mode, phone)
        self.run_task = asyncio.create_task(self.run(self.client))

    async def run(self, client):
        try:
            await client.start()
            if client is self.client and self.state != "logged_out":
                self.set_state("error", "the connection to MAX ended")
        except asyncio.CancelledError:
            raise
        except Exception as e:
            if client is not self.client:
                return
            if isinstance(e, ApiError) and (e.error in INVALID_TOKEN or e.message in INVALID_TOKEN):
                self.forget_session()
                self.set_state("logged_out", explain(e))
                event("logged_out", reason=explain(e))
                return
            why = self.login_error(e)
            log.warning("WRN stopped: %s", why)
            login_failed = not self.session_path().exists() or self.mode_waiting()
            self.set_state("error", why)
            event("status", state="error", error=why, login=login_failed)

    def mode_waiting(self):
        return self.state in ("waiting_code", "waiting_password", "waiting_qr", "authorizing", "connecting") and not self.saved_account().get("id")

    def login_error(self, e):
        text = explain(e)
        if isinstance(e, RuntimeError) and "RegistrationConfig" in text:
            return "this phone number has no MAX account yet: create one in the MAX app, then log in here"
        if isinstance(e, RuntimeError) and "QR authentication expired" in text:
            return "nobody scanned the QR in time; max_login_qr shows a new one"
        if type(e).__name__ == "PasswordAttemptsExceededError":
            return "the two-step password was wrong too many times; start again with max_login"
        return text

    # --- what PyMax asks for during a login (SmsCodeProvider, PasswordProvider, QrHandler) ---

    async def get_code(self, phone):
        self.code = asyncio.get_running_loop().create_future()
        self.set_state("waiting_code")
        return await asyncio.wait_for(self.code, 600)

    async def get_password(self, hint):
        self.hint = hint or ""
        self.password = asyncio.get_running_loop().create_future()
        self.set_state("waiting_password")
        return await asyncio.wait_for(self.password, 600)

    async def show_qr(self, link):
        self.qr_link = link
        self.set_state("waiting_qr")
        event("qr", link=link)

    # --- events ---

    async def on_started(self, client):
        me = client.me.contact
        self.save_account(mode=self.mode, id=str(me.id), name=full_name(me))
        self.own_state()
        for chat in client.chats or []:
            self.chats[chat.id] = chat
        was_login = self.state in ("waiting_code", "waiting_password", "waiting_qr", "authorizing") or bool(self.qr_link)
        self.qr_link = ""
        self.error = ""
        self.set_state("ready")
        if was_login:
            event("logged_in", id=str(me.id), name=full_name(me))
        await self.announce_chats()
        await self.catch_up()

    async def announce_chats(self):
        for chat in list(self.chats.values())[:500]:
            title = await self.chat_title(chat)
            if title:
                event("chat", chat=str(chat.id), title=title)

    async def catch_up(self):
        """What came while bia was off: newer messages of the chats that moved."""
        since = self.last_seen()
        rows = []
        if since:
            moved = [c for c in self.chats.values() if secs(c.last_event_time) > since or (c.last_message and secs(c.last_message.time) > since)]
            moved.sort(key=lambda c: c.last_event_time, reverse=True)
            for chat in moved[:30]:
                try:
                    for m in await self.client.fetch_history(chat.id, backward=50):
                        if secs(m.time) > since:
                            m.chat_id = m.chat_id or chat.id
                            rows.append(await self.row(m, chat.id))
                except Exception as e:
                    log.warning("WRN catch-up of %s: %s", chat.id, explain(e))
        event("backlog", messages=rows)

    async def on_message(self, message, client):
        chat = message.chat_id
        if chat is None:
            return
        event("message", **await self.row(message, chat))

    async def on_edit(self, message, client):
        chat = message.chat_id
        if chat is None:
            return
        old, sent = self.texts.get(str(message.id), ("", 0))  # row() below records the new text
        row = await self.row(message, chat)
        self.texts.put(row["id"], (message.text or "", sent))
        event("edit", chat=row["chat"], id=row["id"], sender=row["sender"], sender_name=row["sender_name"], me=row["me"],
              text=message.text or "", old=old, sent_ts=sent, ts=int(time.time()))

    async def on_delete(self, ev, client):
        for mid in ev.message_ids or []:
            event("delete", chat=str(ev.chat_id), id=str(mid))

    async def on_reaction(self, ev, client):
        mid = str(ev.message_id)
        now = {c.reaction: c.count for c in ev.counters or []}
        before = self.reacts.get(mid, {})
        self.reacts.put(mid, now)
        grown = [e for e, n in now.items() if n > before.get(e, 0)]
        if ev.your_reaction and grown == [ev.your_reaction]:
            return  # this account's own reaction
        emoji = grown[0] if grown else ""
        sender, name = "", ""
        chat = self.chats.get(ev.chat_id)
        peer = self.peer_of(chat)
        if peer:
            sender, name = str(peer), await self.name_of(peer)
        event("reaction", chat=str(ev.chat_id), id=mid, sender=sender, sender_name=name, emoji=emoji)

    async def on_chat(self, chat, client):
        self.chats[chat.id] = chat
        title = await self.chat_title(chat)
        if title:
            event("chat", chat=str(chat.id), title=title)

    async def on_disconnect(self, *args):
        if self.state == "ready":
            self.set_state("connecting")
        event("status", state="connecting")

    # --- calls ---

    async def on_raw(self, frame, client):
        opcode = int(frame.opcode)
        if opcode not in (OP_CALL_INVITE, OP_CALL_ANSWERED, OP_CALL_OVER):
            return
        payload = frame.payload if isinstance(frame.payload, dict) else {}
        try:
            if opcode == OP_CALL_INVITE:
                await self.call_invited(payload)
            elif self.call is not None and self.call.id == payload.get("conversationId"):
                if opcode == OP_CALL_OVER:
                    await self.call.end(remote=True, remote_reason=payload.get("reason"))
                elif self.call.incoming and not self.call.accepted:
                    self.call.accepted = True  # answered on another device
                    await self.call.end(remote=True, remote_reason="ANOTHER_DEVICE")
        except Exception as e:
            log.warning("WRN call frame %s: %s", opcode, type(e).__name__)

    def calls_problem(self):
        if self.saved_account().get("mode", self.mode) != "qr":
            return "MAX voice calls need a QR login: this one used an SMS code. max_login without a phone logs in by QR"
        return calls_missing()

    async def call_invited(self, payload):
        params = call_params(payload["vcp"])
        conversation, peer = payload["conversationId"], int(payload["callerId"])
        call_endpoint(params, conversation, True)  # checked before anything is told
        if self.calls_problem():
            log.warning("WRN incoming call not shown: %s", self.calls_problem())
            return  # the [call] line comes with the message MAX posts afterwards
        if conversation in self.declining:
            return
        if self.call is not None:
            if self.call.id != conversation:
                # a second caller gets busy on its own socket, and never a media pipeline
                self.declining.add(conversation)
                try:
                    busy = VoiceCall(conversation, peer, True, lambda *_: None, lambda _: None, self.agent())
                    await busy.end("BUSY")
                    await busy.run(params)
                finally:
                    self.declining.discard(conversation)
            return
        self.call = VoiceCall(conversation, peer, True, self.call_changed, self.call_heard, self.agent())
        self.call.start(params)
        await self.call_event(self.call, "ringing")

    def agent(self):
        try:
            return self.client._app.config.device.user_agent.header_user_agent
        except AttributeError:
            return None

    async def call_event(self, call, state):
        event("call", state=state, conversation=call.id, chat=str(self.dialog_with(call.peer)), peer=str(call.peer),
              name=await self.name_of(call.peer), incoming=call.incoming, reason=call.reported or "", accepted=call.accepted)

    def dialog_with(self, peer):
        return self.client._app.api.users.get_chat_id(self.me_id(), peer)

    def call_changed(self, call, state):
        if call is self.call and call.closed:
            self.call, self.last_call = None, call
        asyncio.ensure_future(self.call_event(call, state))

    def call_heard(self, frame):
        event("call_audio", pcm=base64.b64encode(frame).decode())

    def call_pcm(self, params):
        """A frame of what bia says; no answer, sixteen of these come every second."""
        if self.call is not None:
            try:
                self.call.feed(base64.b64decode(params["pcm"]))
            except Exception as e:
                log.info("call audio: %s", type(e).__name__)

    async def m_call_start(self, p):
        client = self.need_client()
        problem = self.calls_problem()
        if problem:
            raise RuntimeError(problem)
        if self.call is not None:
            raise RuntimeError(f"a call is already on with {self.call.peer}: end it first")
        peer = await self.call_peer(p["chat"])
        conversation = str(uuid.uuid4())
        internal = {"deviceId": client._app.session.device_id, "sdkVersion": "2.8.9", "clientAppKey": "CNHIJPLGDIHBABABA", "platform": "WEB",
                    "protocolVersion": 5, "domainId": "", "capabilities": "2A03F"}
        reply = await client._app.invoke(OP_CALL_START, {"conversationId": conversation, "calleeIds": [peer], "isVideo": False, "internalParams": json.dumps(internal)})
        params = json.loads(reply.payload["internalCallerParams"])
        call_endpoint(params, conversation, False)
        self.call = VoiceCall(conversation, peer, False, self.call_changed, self.call_heard, self.agent())
        self.call.start(params)
        return {"chat": str(self.dialog_with(peer)), "name": await self.name_of(peer)}

    async def call_peer(self, chat):
        q = str(chat).strip()
        chat_obj = await self.chat_of(as_int(q)) if re.fullmatch(r"-?\d+", q) else None
        if chat_obj is not None and self.is_group(chat_obj):
            raise ValueError("MAX group calls are not available here: only one person at a time")
        peer = self.peer_of(chat_obj)
        return peer or await self.user_id(q)

    def the_call(self, p):
        call = self.call
        if call is None:
            raise ValueError("no MAX call is on")
        if p.get("chat") and str(self.dialog_with(call.peer)) != str(p["chat"]).strip():
            raise ValueError(f"the call is with chat {self.dialog_with(call.peer)}, not {p['chat']}")
        return call

    async def m_call_accept(self, p):
        call = self.the_call(p)
        if not call.incoming or call.accepted:
            raise ValueError("no call is ringing here")
        await call.answer()
        return {"chat": str(self.dialog_with(call.peer))}

    async def m_call_hangup(self, p):
        call = self.the_call(p)
        chat = str(self.dialog_with(call.peer))
        reason = call.hangup_reason()
        await call.end(reason)
        return {"chat": chat, "reason": reason}

    async def m_call_status(self, p):
        call = self.call
        out = {"problem": self.calls_problem()}
        if call is not None:
            out.update(state=call.state, chat=str(self.dialog_with(call.peer)), peer=str(call.peer), name=await self.name_of(call.peer), incoming=call.incoming,
                       frames_in=call.frames_in, peak_in=call.peak_in, frames_out=call.frames_out)
        elif self.last_call is not None:
            last = self.last_call
            out.update(last_frames_in=last.frames_in, last_peak_in=last.peak_in, last_frames_out=last.frames_out)
        return out

    # --- people and chats ---

    def peer_of(self, chat):
        if not chat or enum_value(chat.type) != ChatType.DIALOG.value:
            return 0
        me = self.me_id()
        for uid in chat.participants or {}:
            if int(uid) != me:
                return int(uid)
        return 0

    async def name_of(self, uid):
        if not uid or not self.client:
            return ""
        try:
            user = self.client._app.users.get(uid) or await self.client.get_user(uid)
            return full_name(user) if user else ""
        except Exception:
            return ""

    async def chat_of(self, chat_id):
        chat = self.chats.get(chat_id)
        if chat is None and self.client:
            try:
                chat = await self.client.get_chat(chat_id)
                self.chats[chat_id] = chat
            except Exception:
                return None
        return chat

    async def chat_title(self, chat):
        if chat is None:
            return ""
        return chat.title or await self.name_of(self.peer_of(chat))

    def is_group(self, chat):
        return bool(chat) and enum_value(chat.type) != ChatType.DIALOG.value

    async def chat_row(self, chat):
        row = {"chat": str(chat.id), "title": await self.chat_title(chat), "group": self.is_group(chat)}
        peer = self.peer_of(chat)
        if peer:
            row["user"] = str(peer)
        if enum_value(chat.type) == ChatType.CHANNEL.value:
            row["about"] = "channel"
        return row

    async def user_row(self, user):
        chat_id = self.client._app.api.users.get_chat_id(self.me_id(), user.id)
        return {"chat": str(chat_id), "user": str(user.id), "title": full_name(user), "group": False}

    async def find(self, query, limit):
        q = query.strip()
        client = self.need_client()
        rows, seen = [], set()

        def add(row):
            if row["chat"] not in seen:
                seen.add(row["chat"])
                rows.append(row)

        digits = re.sub(r"[\s()+-]", "", q)
        if digits.isdigit() and len(digits) >= 7:
            try:
                add(await self.user_row(await client.search_by_phone(("+" if q.startswith("+") else "") + digits)))
            except Exception as e:
                log.info("phone search: %s", explain(e))
        low = q.casefold()
        for chat in self.chats.values():
            row = await self.chat_row(chat)
            if low in row["title"].casefold() or q == row["chat"]:
                add(row)
        for user in client.contacts or []:
            if user and (low in full_name(user).casefold() or q == str(user.id)):
                add(await self.user_row(user))
        if len(rows) < limit:
            # MAX's own search of public channels and groups, as its apps send it
            try:
                frame = await client._app.invoke(Opcode.PUBLIC_SEARCH, {"query": q, "count": limit, "type": "ALL"})
                for hit in (frame.payload or {}).get("result") or []:
                    found = hit.get("chat") or {}
                    if found.get("id"):
                        add({"chat": str(found["id"]), "title": found.get("title") or "", "group": True,
                             "about": "public; join it by its link: " + str(found.get("link") or "")})
            except Exception as e:
                log.info("public search: %s", explain(e))
        return rows[:limit]

    async def resolve(self, query):
        q = query.strip()
        client = self.need_client()
        if "max.ru/" in q or q.startswith("join/"):
            chat = await client.resolve_group_by_link(q)
            if chat:
                return await self.chat_row(chat)
            raise ValueError(f"no MAX chat behind {q}")
        if re.fullmatch(r"-?\d{1,15}", q):
            n = int(q)
            chat = await self.chat_of(n)
            if chat:
                return await self.chat_row(chat)
            if n > 0:
                user = await client.get_user(n)
                if user:
                    return await self.user_row(user)
        rows = await self.find(q, 1)
        if rows:
            return rows[0]
        raise ValueError(f"nobody on MAX matches {q!r}: try the phone number with its country code")

    async def user_id(self, who):
        """A person's id from an id, a phone number or a name."""
        q = str(who).strip()
        if re.fullmatch(r"\d{1,12}", q):
            return int(q)
        row = await self.resolve(q)
        if row.get("user"):
            return int(row["user"])
        raise ValueError(f"{who!r} is a chat, not a person")

    # --- messages as the MCP reads them ---

    def attachment_rows(self, m, chat):
        out, notes, location = [], [], None
        for a in m.attaches or []:
            kind = enum_value(getattr(a, "type", ""))
            ref = None
            if kind == "PHOTO":
                ref = {"kind": "image", "ref": {"k": "url", "url": a.base_url}}
            elif kind == "AUDIO":
                if a.url:
                    ref = {"kind": "voice", "ref": {"k": "url", "url": a.url}}
                else:
                    out.append({"kind": "voice", "note": "MAX sent no address for it"})
            elif kind == "STICKER":
                ref = {"kind": "sticker", "ref": {"k": "url", "url": a.url}}
            elif kind == "VIDEO":
                ref = {"kind": "video", "ref": {"k": "video", "chat": chat, "msg": m.id, "id": a.video_id}}
            elif kind == "FILE":
                ref = {"kind": "document", "name": a.name, "ref": {"k": "file", "chat": chat, "msg": m.id, "id": a.file_id}}
            elif kind == "CONTACT":
                name = a.name or " ".join(x for x in (a.first_name, a.last_name) if x) or "(no name)"
                who = f"{name} (MAX id {a.contact_id})" if a.contact_id else name
                notes.append(f"[contact] {who}")
            elif kind == "CALL":
                how = (enum_value(a.call_type) or "").lower()
                end = (enum_value(a.hangup_type) or "").lower()
                length = f", {a.duration // 1000}s" if a.duration else ""
                notes.append(f"[call] {how + ' ' if how else ''}call{': ' + end if end else ''}{length} (bia cannot take MAX calls; write back)")
            elif kind == "SHARE":
                if a.url or a.title:
                    notes.append("[shared] " + " — ".join(x for x in (a.title, a.url) if x))
            elif kind == "POLL":
                poll = getattr(a, "poll", None) or a
                options = [getattr(o, "text", "") for o in getattr(poll, "answers", None) or []]
                notes.append(f"[poll] {getattr(poll, 'question', '') or getattr(poll, 'title', '')}: " + " / ".join(x for x in options if x))
            elif kind == "CONTROL":
                notes.append(f"[group] {a.event}" + (f": {a.title}" if a.title else ""))
            elif kind == "INLINE_KEYBOARD":
                pass
            else:
                raw = a.model_dump() if hasattr(a, "model_dump") else {}
                lat, lon = raw.get("latitude", raw.get("lat")), raw.get("longitude", raw.get("lng", raw.get("lon")))
                if lat is not None and lon is not None:
                    location = {"lat": float(lat), "lon": float(lon), "name": raw.get("title") or ""}
                else:
                    notes.append(f"[{str(kind).lower() or 'attachment'} not shown here: open MAX to see it]")
            if ref:
                ref["ref"] = json.dumps(ref["ref"])
                out.append(ref)
        return out, notes, location

    async def row(self, m, chat_id):
        chat = await self.chat_of(chat_id)
        sender = m.sender or 0
        me = bool(sender) and sender == self.me_id()
        attachments, notes, location = self.attachment_rows(m, chat_id)
        text = m.text or ""
        row = {
            "chat": str(chat_id), "id": str(m.id), "ts": secs(m.time), "sender": str(sender) if sender else "",
            "sender_name": await self.name_of(sender), "me": me, "group": self.is_group(chat),
            "chat_title": (chat.title or "") if chat else "", "text": text, "attachments": attachments, "notes": notes,
        }
        if location:
            row["location"] = location
        link = m.link
        if link is not None and getattr(link, "message", None) is not None:
            quoted = link.message
            if enum_value(link.type) == LinkType.REPLY.value:
                row["reply"] = {"id": str(quoted.id), "sender": str(quoted.sender or ""), "sender_name": await self.name_of(quoted.sender),
                                "me": bool(quoted.sender) and quoted.sender == self.me_id(), "text": quoted.text or ""}
            else:
                # a forward: "comment" is what the sender wrote with it, "text" the forwarded text
                origin = getattr(link, "chat_name", None) or await self.name_of(quoted.sender)
                sub, sub_notes, _ = self.attachment_rows(quoted, link.chat_id or chat_id)
                row["notes"] = notes + sub_notes
                row["attachments"] = attachments + sub
                row["comment"] = text
                row["text"] = quoted.text or ""
                row["forward"] = {"from": origin or "", "me": bool(quoted.sender) and quoted.sender == self.me_id(),
                                  "ts": secs(quoted.time) if getattr(quoted, "time", None) else 0}
        self.texts.put(row["id"], (row["text"], row["ts"]))
        return row

    # --- requests ---

    async def m_status(self, p):
        acc = self.saved_account()
        me = self.client.me.contact if self.client and self.client.me else None
        out = {"state": self.state, "id": str(me.id) if me else acc.get("id", ""), "name": full_name(me) if me else acc.get("name", ""), "error": self.error}
        if self.hint:
            out["hint"] = self.hint
        return out

    async def m_login(self, p):
        phone = re.sub(r"[\s()-]", "", p.get("phone", ""))
        if not phone.startswith("+") or not phone[1:].isdigit():
            raise ValueError("phone needs its country code, for example +79991234567")
        await self.stop_client()
        self.forget_session()
        await self.start_client("sms", phone)
        state = await self.wait_state({"waiting_code", "ready", "error"}, 60)
        if state == "error":
            raise RuntimeError(self.error)
        return await self.m_status(p)

    async def m_submit_code(self, p):
        if self.state != "waiting_code" or not self.code:
            raise RuntimeError(f"no code is awaited (state {self.state}); start with max_login")
        self.set_state("authorizing")
        self.code.set_result(str(p.get("code", "")).strip())
        state = await self.wait_state({"waiting_password", "ready", "error"}, 60)
        if state == "error":
            raise RuntimeError(f"{self.error}; a wrong or expired code ends the attempt, so ask for a new one with max_login")
        return await self.m_status(p)

    async def m_submit_password(self, p):
        if self.state != "waiting_password" or not self.password:
            raise RuntimeError(f"no password is awaited (state {self.state})")
        self.set_state("authorizing")
        self.password.set_result(p.get("password", ""))
        state = await self.wait_state({"waiting_password", "ready", "error"}, 60)
        if state == "error":
            raise RuntimeError(self.error)
        out = await self.m_status(p)
        if state == "waiting_password":
            out["wrong"] = True
        return out

    async def m_login_qr(self, p):
        await self.stop_client()
        self.forget_session()
        await self.start_client("qr")
        state = await self.wait_state({"waiting_qr", "ready", "error"}, 60)
        if state == "error":
            raise RuntimeError(self.error)
        return {**await self.m_status(p), "link": self.qr_link}

    async def m_restore(self, p):
        if self.state == "ready":
            return await self.m_status(p)
        acc = self.saved_account()
        if not self.session_path().exists() or not acc.get("id"):
            self.set_state("logged_out")
            return await self.m_status(p)
        if not self.run_task or self.run_task.done():
            await self.start_client(acc.get("mode", "sms"), acc.get("phone", "") or "+0")
        await self.wait_state({"ready", "logged_out", "error"}, 60)
        return await self.m_status(p)

    async def m_logout(self, p):
        client = self.client
        if client and self.state == "ready":
            try:
                await client.logout()
            except Exception as e:
                log.warning("WRN logout: %s", explain(e))
        await self.stop_client()
        self.forget_session()
        self.set_state("logged_out", "")
        return {}

    def attachment(self, f):
        path = f["path"]
        ext = Path(path).suffix.lower()
        if f.get("kind") == "voice":
            return Voice(path=path, duration=max(1, int(f.get("seconds") or 1)))
        if ext in PHOTO_EXT:
            return Photo(path=path)
        if ext in VIDEO_EXT:
            return Video(path=path)
        return File(path=path)

    async def m_send(self, p):
        client = self.need_client()
        reply = p.get("reply_to")
        files = [self.attachment(f) for f in p.get("files") or []]
        m = await client.send_message(as_int(p["chat"], "chat id"), p.get("text") or None, reply_to=as_int(reply, "message id") if reply else None, attachments=files or None)
        self.texts.put(str(m.id), (m.text or "", secs(m.time)))
        return {"ids": [str(m.id)]}

    async def m_edit(self, p):
        await self.need_client().edit_message(as_int(p["chat"]), as_int(p["id"]), p.get("text"))
        return {}

    async def m_react(self, p):
        client = self.need_client()
        if p.get("emoji"):
            await client.add_reaction(as_int(p["chat"]), as_int(p["id"]), p["emoji"])
        else:
            await client.remove_reaction(as_int(p["chat"]), as_int(p["id"]))
        return {}

    async def m_delete(self, p):
        ids = [as_int(x) for x in p.get("ids") or []]
        await self.need_client().delete_message(as_int(p["chat"]), ids, for_me=not p.get("everyone", True))
        return {"deleted": [str(x) for x in ids], "failed": []}

    async def m_recent(self, p):
        return []  # the server keeps the whole history; there are no local-only chats

    async def m_history(self, p):
        client = self.need_client()
        chat = as_int(p["chat"])
        limit = int(p.get("limit") or 30)
        since = None
        if p.get("before"):
            found = await client.get_message(chat, as_int(p["before"]))
            if found is None:
                raise ValueError(f"no message {p['before']} in this chat")
            since = found.time - 1
        kw = {"from_time": since} if since else {}
        rows = [await self.row(m, chat) for m in await client.fetch_history(chat, backward=limit, **kw)]
        return rows

    async def m_chats(self, p):
        client = self.need_client()
        if p.get("refresh"):
            for chat in await client.fetch_chats():
                self.chats[chat.id] = chat
        chats = sorted(self.chats.values(), key=lambda c: c.last_event_time or 0, reverse=True)
        return [await self.chat_row(c) for c in chats[: int(p.get("limit") or 50)]]

    async def m_search(self, p):
        return await self.find(p.get("query", ""), int(p.get("limit") or 20))

    async def m_resolve(self, p):
        return await self.resolve(p.get("query", ""))

    async def m_group_info(self, p):
        client = self.need_client()
        chat = await client.get_chat(as_int(p["chat"]))
        self.chats[chat.id] = chat
        members, _ = await client.get_chat_members(chat.id, count=200)
        me = self.me_id()
        admins = set(chat.admins or []) | {int(k) for k in (chat.admin_participants or {})}
        rows = []
        for m in members:
            uid = m.contact.id
            role = "owner" if uid == chat.owner else "admin" if uid in admins else "member"
            rows.append({"id": str(uid), "name": full_name(m.contact), "role": role})
        settings = []
        options = chat.options if isinstance(chat.options, dict) else {}
        for key, label in (("ONLY_ADMIN_CAN_ADD_MEMBER", "only admins add people"), ("ONLY_OWNER_CAN_CHANGE_ICON_TITLE", "only the owner changes the name and photo"),
                           ("ALL_CAN_PIN_MESSAGE", "everyone may pin messages"), ("ONLY_ADMIN_CAN_CALL", "only admins start calls")):
            if key in options:
                settings.append(f"{label}: {'yes' if options[key] else 'no'}")
        if chat.link:
            settings.append(f"link: {chat.link}")
        return {"title": chat.title or "", "kind": "channel" if enum_value(chat.type) == ChatType.CHANNEL.value else "group", "description": chat.description or "",
                "settings": settings, "members": rows, "you_admin": me == chat.owner or me in admins}

    async def m_rename(self, p):
        await self.need_client().change_group_profile(as_int(p["chat"]), name=p["title"])
        return {}

    async def m_group_photo(self, p):
        await self.need_client().change_group_profile(as_int(p["chat"]), photo=Photo(path=p["path"]))
        return {}

    async def m_group_settings(self, p):
        client = self.need_client()
        chat = as_int(p["chat"])
        done = []
        if "description" in p:
            await client.change_group_profile(chat, description=p["description"])
            done.append("description set")
        names = {"only_admins_add": ("only_admin_can_add_member", "only admins add people"),
                 "only_admins_edit_info": ("only_owner_can_change_icon_title", "only the owner changes the name and photo"),
                 "all_can_pin": ("all_can_pin_message", "everyone may pin"),
                 "only_admins_call": ("only_admin_can_call", "only admins start calls")}
        options = {names[k][0]: p[k] for k in names if k in p}
        if options:
            await client.change_group_settings(chat, **options)
            done.extend(f"{names[k][1]}: {'yes' if p[k] else 'no'}" for k in names if k in p)
        if not done:
            raise ValueError("nothing to change: pass only_admins_add, only_admins_edit_info, all_can_pin or only_admins_call")
        return {"done": done}

    async def m_set_admin(self, p):
        if not p.get("admin", True):
            raise ValueError("MAX admin rights can only be taken back in the MAX app; this client has no call for it")
        await self.need_client().add_admin(as_int(p["chat"]), await self.user_id(p["member"]), list(ChannelPermissions))
        return {}

    async def m_join_requests(self, p):
        rows = await self.need_client().get_join_requests(as_int(p["chat"]))
        return [{"id": str(m.contact.id), "name": full_name(m.contact), "ts": 0} for m in rows]

    async def m_answer_join(self, p):
        client = self.need_client()
        chat, user = as_int(p["chat"]), await self.user_id(p["member"])
        if p.get("approve", True):
            await client.confirm_join_request(chat, user)
        else:
            await client.decline_join_request(chat, user)
        return {}

    async def m_create_group(self, p):
        users = [await self.user_id(x) for x in p.get("members") or []]
        made = await self.need_client().create_group(p["title"], users)
        if not made:
            raise RuntimeError("MAX did not create the group")
        chat = made[0]
        self.chats[chat.id] = chat
        return {"chat": str(chat.id)}

    async def m_add_members(self, p):
        users = [await self.user_id(x) for x in p.get("members") or []]
        await self.need_client().invite_users_to_group(as_int(p["chat"]), users)
        return {"failed": []}

    async def m_remove_member(self, p):
        await self.need_client().remove_users_from_group(as_int(p["chat"]), [await self.user_id(p["member"])], 0)
        return {}

    async def m_join(self, p):
        client = self.need_client()
        link = p["link"].strip()
        chat = await client.resolve_group_by_link(link)
        if chat is not None and enum_value(chat.type) == ChatType.CHANNEL.value:
            joined = await client.join_channel(link)
        else:
            joined = await client.join_group(link)
        self.chats[joined.id] = joined
        return await self.chat_row(joined)

    async def m_leave(self, p):
        client = self.need_client()
        chat_id = as_int(p["chat"])
        chat = await self.chat_of(chat_id)
        if chat is not None and enum_value(chat.type) == ChatType.CHANNEL.value:
            await client.leave_channel(chat_id)
        else:
            await client.leave_group(chat_id)
        self.chats.pop(chat_id, None)
        return {}

    async def m_delete_chat(self, p):
        chat_id = as_int(p["chat"])
        await self.need_client().delete_chat(chat_id, for_all=bool(p.get("everyone", False)))
        self.chats.pop(chat_id, None)
        return {}

    async def m_invite_link(self, p):
        client = self.need_client()
        chat_id = as_int(p["chat"])
        chat = await client.get_chat(chat_id)
        if not chat.link or p.get("new"):
            chat = await client.rework_invite_link(chat_id)
        if not chat.link:
            raise RuntimeError("MAX gave no invite link; only a group's admins can get one")
        return {"link": chat.link}

    async def m_pin(self, p):
        if not p.get("id"):
            raise ValueError("pass message_id: MAX pins messages, not chats")
        if not p.get("on", True):
            raise ValueError("unpinning is done in the MAX app; this client has no call for it")
        await self.need_client().pin_message(as_int(p["chat"]), as_int(p["id"]), False)
        return {}

    async def m_read(self, p):
        await self.need_client().read_message(as_int(p["id"]), as_int(p["chat"]))
        return {}

    async def m_typing(self, p):
        if p.get("on", True):
            await self.need_client()._app.invoke(Opcode.MSG_TYPING, {"chatId": as_int(p["chat"]), "type": "TEXT"})
        return {}

    async def m_set_profile(self, p):
        client = self.need_client()
        contact = client.me.contact
        first = contact.names[0] if contact.names else None
        first_name = p.get("first_name") or (first.first_name or first.name if first else "") or "bia"
        last_name = p["last_name"] if "last_name" in p else (first.last_name if first else None)
        description = p["description"] if "description" in p else contact.description
        photo = Photo(path=p["photo"]) if p.get("photo") else None
        await client.change_profile(first_name, last_name, description, photo)
        return {}

    async def m_block(self, p):
        action = "BLOCK" if p.get("on", True) else "UNBLOCK"
        uid = await self.user_id(p["user"])
        await self.need_client()._app.invoke(Opcode.CONTACT_UPDATE, {"contactId": uid, "action": action})
        return {}

    async def m_send_contact(self, p):
        uid = await self.user_id(p["user"])
        chat = as_int(p["chat"])
        payload = {"chatId": chat, "message": {"cid": int(time.time() * 1000), "elements": [], "attaches": [{"_type": "CONTACT", "contactId": uid}]}, "notify": True}
        frame = await self.need_client()._app.invoke(Opcode.MSG_SEND, payload)
        mid = ((frame.payload or {}).get("message") or {}).get("id", "")
        return {"ids": [str(mid)]}

    async def m_download(self, p):
        client = self.need_client()
        ref = json.loads(p["ref"])
        if ref["k"] == "file":
            info = await client.get_file_by_id(ref["chat"], ref["msg"], ref["id"])
            url = info.url if info else None
        elif ref["k"] == "video":
            info = await client.get_video_by_id(ref["chat"], ref["msg"], ref["id"])
            url = None
            if info:
                url = info.url or next((v for k, v in sorted((info.model_extra or {}).items(), reverse=True) if isinstance(v, str) and v.startswith("http")), None)
        else:
            url = ref.get("url")
        if not url:
            raise RuntimeError("MAX gave no address for this file")
        dest = Path(p["path"])
        dest.parent.mkdir(parents=True, exist_ok=True)
        tmp = dest.with_name(dest.name + ".part")
        timeout = aiohttp.ClientTimeout(total=15 * 60)
        async with aiohttp.ClientSession(timeout=timeout) as session, session.get(url) as r:
            r.raise_for_status()
            mime = r.headers.get("Content-Type", "").split(";")[0].strip()
            with tmp.open("wb") as f:
                async for chunk in r.content.iter_chunked(256 * 1024):
                    f.write(chunk)
        tmp.replace(dest)
        if not mime or mime == "application/octet-stream":
            mime = mimetypes.guess_type(dest.name)[0] or ""
        return {"mime": mime}

    async def handle(self, req):
        rid = req.get("id")
        fn = getattr(self, "m_" + str(req.get("method")), None)
        try:
            if fn is None:
                raise ValueError(f"unknown method {req.get('method')}")
            emit({"id": rid, "result": await fn(req.get("params") or {})})
        except asyncio.CancelledError:
            emit({"id": rid, "error": "cancelled"})
            raise
        except Exception as e:
            log.info("%s failed: %s", req.get("method"), explain(e))
            emit({"id": rid, "error": explain(e)})


async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--platform", default="max")
    ap.add_argument("--dir", required=True)
    args = ap.parse_args()
    os.umask(0o077)  # the login token lives here
    logging.basicConfig(stream=sys.stderr, level=logging.INFO, format="%(asctime)s %(name)s %(message)s")
    bridge = Bridge(args.dir)
    loop = asyncio.get_running_loop()
    reader = asyncio.StreamReader(limit=1 << 26)
    await loop.connect_read_pipe(lambda: asyncio.StreamReaderProtocol(reader), sys.stdin)
    tasks = set()
    while True:
        line = await reader.readline()
        if not line:
            break
        try:
            req = json.loads(line)
        except ValueError:
            continue
        if req.get("method") == "call_pcm":
            bridge.call_pcm(req.get("params") or {})
            continue
        task = asyncio.create_task(bridge.handle(req))
        tasks.add(task)
        task.add_done_callback(tasks.discard)
    await bridge.stop_client()


if __name__ == "__main__":
    asyncio.run(main())
