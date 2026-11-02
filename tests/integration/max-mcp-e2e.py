"""End-to-end without a MAX account: bia-mcp-max over stdio, the real bia_max.py and PyMax behind
it, and a fake MAX server speaking PyMax's own TCP framing on localhost.

This checks bia's side against PyMax. It says nothing about the real MAX servers: the fake only
knows what PyMax expects of them.

  python3 -I tests/integration/max-mcp-e2e.py <bia-mcp-max> <bia_max.py>   (BIA_MAX_SITE: PyMax's packages)
"""
import asyncio, base64, http.server, json, math, struct, os, signal, socketserver, subprocess, sys, tempfile, threading, time

binary, script = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
sys.path.insert(0, os.environ["BIA_MAX_SITE"])
import websockets.asyncio.server  # noqa: E402
sys.path.insert(0, os.path.dirname(script))
from bia_max_call import CallAudio  # noqa: E402
from pymax.protocol.enums import Command, Opcode  # noqa: E402
from pymax.protocol.tcp.framing import TcpPacketFramer  # noqa: E402
from pymax.protocol.tcp.payload import MsgpackPayloadCodec, TcpPayloadDecoder  # noqa: E402

ME, ANN, GROUP = 1000, 2000, -555
DIALOG = ME ^ ANN  # how MAX numbers dialogs
framer, codec = TcpPacketFramer(), MsgpackPayloadCodec()
decoder = TcpPayloadDecoder(serializer=codec)


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        check.failed = True


check.failed = False


def user(uid, name):
    return {"id": uid, "names": [{"name": name, "firstName": name, "type": "ONEME"}], "updateTime": 1}


def now_ms():
    return int(time.time() * 1000)


def chat(cid, kind, title=None, owner=ME, at=None):
    participants = {str(ME): 1, str(ANN): 1}
    return {"id": cid, "type": kind, "status": "ACTIVE", "owner": owner, "participants": participants, "title": title,
            "lastEventTime": at or 1000, "cid": cid, "admins": [ME] if kind != "DIALOG" else []}


class Fake:
    """The state of the fake server, and what it was asked."""
    def __init__(self):
        self.requests = []
        self.conn = None
        self.next_id = 5000
        self.backlog = []
        self.moved_at = None
        self.token_ok = True
        self.qr_scanned = False
        self.dials = []

    def seen(self, op):
        return [p for o, p in self.requests if o == op]

    def login(self):
        chats = [chat(DIALOG, "DIALOG", at=self.moved_at), chat(GROUP, "CHAT", "Team")]
        return {"profile": {"contact": user(ME, "Bia Owner")}, "chats": chats, "contacts": [user(ANN, "Ann Lee")], "messages": {}, "time": now_ms(), "token": "tok-1"}

    def answer(self, op, p):
        if op == Opcode.GET_QR:
            return {"expiresAt": now_ms() + 60000, "pollingInterval": 100, "qrLink": "https://max.ru/:auth/fake", "trackId": "q1", "ttl": 60000}
        if op == Opcode.GET_QR_STATUS:
            return {"status": {"expiresAt": now_ms() + 60000, "loginAvailable": self.qr_scanned}}
        if op == Opcode.LOGIN_BY_QR:
            return {"tokenAttrs": {"LOGIN": {"token": "tok-qr"}}}
        if op == 78:
            self.dials.append(p)
            call_cfg = {"endpoint": f"ws://127.0.0.1:{ws_port[0]}/ws2?userId=1&entityType=USER&conversationId={p['conversationId']}&token=t",
                        "id": {"internal": 1, "external": str(ME)}, "turn": {"urls": []}, "stun": {"urls": []}}
            return {"conversationId": p["conversationId"], "rejectedParticipants": [], "internalCallerParams": json.dumps(call_cfg)}
        if op == Opcode.SESSION_INIT:
            return {"callsSeed": 12345}
        if op == Opcode.AUTH_REQUEST:
            return {"token": "start-1", "codeLength": 6, "requestMaxDuration": 60000, "requestCountLeft": 5, "altActionDuration": 30000}
        if op == Opcode.AUTH:
            code = p.get("verifyCode")
            if code == "111111":
                return {"tokenAttrs": {"LOGIN": {"token": "tok-1"}}}
            if code == "222222":
                return {"passwordChallenge": {"trackId": "tr1", "hint": "pet"}}
            return Command.ERROR, {"error": "error.code", "message": "wrong code", "localizedMessage": "Неверный код", "title": "Wrong"}
        if op == Opcode.AUTH_LOGIN_CHECK_PASSWORD:
            if p.get("password") == "secret":
                return {"tokenAttrs": {"LOGIN": {"token": "tok-1"}}}
            return {"error": "wrong password"}
        if op == Opcode.LOGIN:
            if not self.token_ok:
                return Command.ERROR, {"error": "FAIL_LOGIN_TOKEN", "message": "FAIL_LOGIN_TOKEN", "localizedMessage": "Сессия завершена", "title": "x"}
            return self.login()
        if op == Opcode.MSG_SEND:
            self.next_id += 1
            m = p.get("message") or {}
            return {"chatId": p.get("chatId"), "message": {"id": self.next_id, "sender": ME, "text": m.get("text", ""), "time": now_ms(), "type": "USER", "cid": m.get("cid"), "attaches": []}}
        if op == Opcode.MSG_EDIT:
            return {"chatId": p.get("chatId"), "message": {"id": p.get("messageId"), "sender": ME, "text": p.get("text", ""), "time": now_ms(), "type": "USER", "status": "EDITED"}}
        if op == Opcode.CHAT_HISTORY:
            if self.backlog:
                return {"messages": self.backlog}
            return {"messages": [{"id": 41, "sender": ANN, "text": "old message", "time": 1_600_000_000_000, "type": "USER"}]}
        if op == Opcode.CHAT_MARK:
            return {"unread": 0, "mark": now_ms()}
        if op == Opcode.CHAT_INFO:
            return {"chats": [chat(c, "DIALOG" if c == DIALOG else "CHAT", None if c == DIALOG else "Team") for c in p.get("chatIds", [])]}
        if op == Opcode.CONTACT_INFO:
            return {"contacts": [user(u, "Ann Lee" if u == ANN else "Bia Owner") for u in p.get("contactIds", [])]}
        if op == Opcode.CONTACT_INFO_BY_PHONE:
            return {"contact": user(ANN, "Ann Lee")}
        if op == Opcode.PHOTO_UPLOAD:
            return {"url": f"{media_url()}/upload/photo?photos=p1", "photoIds": ["p1"]}
        if op == Opcode.FILE_UPLOAD:
            return {"info": [{"url": f"{media_url()}/upload/file?id=71", "fileId": 71, "token": "ft"}]}
        if op == Opcode.VIDEO_UPLOAD:
            return {"info": [{"url": f"{media_url()}/upload/voice?id=72", "videoId": 72, "token": "vt"}]}
        if op == Opcode.FILE_DOWNLOAD:
            return {"unsafe": False, "url": f"{media_url()}/media/report.pdf"}
        if op == Opcode.MSG_REACTION:
            return {"reactionInfo": {"totalCount": 1, "counters": [{"count": 1, "reaction": "👍"}], "yourReaction": "👍"}}
        return {}

    def push(self, op, payload):
        data = framer.pack(ver=10, cmd=Command.REQUEST, seq=0, opcode=op, flags=0, payload_bytes=codec.encode(payload))
        if self.qr_scanned:  # a QR login's connection is the websocket
            on_phone(self.web.send(data))
        else:
            self.conn.sendall(data)


fake = Fake()


class Conn(socketserver.BaseRequestHandler):
    def handle(self):
        fake.conn = self.request
        buf = b""
        while True:
            try:
                got = self.request.recv(1 << 16)
            except OSError:
                return
            if not got:
                return
            buf += got
            while True:
                packet = framer.unpack(buf)
                if packet is None:
                    break
                buf = buf[framer.HEADER_SIZE + packet.header.payload_len:]
                h = packet.header
                p = decoder.decode(packet.payload_bytes, h.flags)
                fake.requests.append((h.opcode, p))
                reply = fake.answer(h.opcode, p)
                cmd = Command.RESPONSE
                if isinstance(reply, tuple):
                    cmd, reply = reply
                self.request.sendall(framer.pack(ver=h.ver, cmd=cmd, seq=h.seq, opcode=h.opcode, flags=0, payload_bytes=codec.encode(reply)))


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


class Media(http.server.BaseHTTPRequestHandler):
    """MAX's file servers: downloads, and uploads that are confirmed on the connection."""
    def log_message(self, *a):
        pass

    def reply(self, body, kind="application/octet-stream"):
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self.reply(b"%PDF-1.4 fake" if self.path.endswith(".pdf") else b"\xff\xd8\xff fake", "application/pdf" if self.path.endswith(".pdf") else "image/jpeg")

    def do_POST(self):
        size = int(self.headers.get("Content-Length") or 0)
        uploads.append((self.path, self.rfile.read(size)))
        self.reply(json.dumps({"photos": {"p1": {"token": "pt"}}}).encode(), "application/json")
        if self.path.startswith("/upload/file"):
            threading.Timer(0.2, fake.push, (Opcode.NOTIF_ATTACH, {"fileId": 71})).start()
        if self.path.startswith("/upload/voice"):
            threading.Timer(0.2, fake.push, (Opcode.NOTIF_ATTACH, {"videoId": 72})).start()

    do_PUT = do_POST


uploads = []
media = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Media)
threading.Thread(target=media.serve_forever, daemon=True).start()


def media_url():
    return f"http://127.0.0.1:{media.server_address[1]}"


class Phone:
    """The other person's phone on MAX's call service: a real WebRTC peer behind a fake ws2."""
    def __init__(self):
        self.reset()

    def reset(self):
        self.commands, self.frames, self.ws, self.audio, self.sound, self.incoming, self.pongs = [], 0, None, None, None, True, 0
        self.peak, self.clip = 0, None

    async def ping(self):
        while True:
            await asyncio.sleep(0.5)
            await self.ws.send("ping")

    async def tell(self, name, **fields):
        await self.ws.send(json.dumps({"type": "notification", "notification": name, "stamp": now_ms(), **fields}))

    async def transmitted(self, data):
        await self.tell("transmitted-data", participantId=77, data=data)

    async def start_media(self):
        loop = asyncio.get_running_loop()
        cand = lambda c: asyncio.ensure_future(self.transmitted({"candidate": c}))  # noqa: E731
        self.audio = CallAudio(loop, cand, lambda state: None, {}, self.heard)
        await self.audio.start()
        self.sound = asyncio.ensure_future(self.speak())

    def heard(self, frame):
        self.frames += 1
        self.peak = max(self.peak, max(abs(x) for x in struct.unpack(f"<{len(frame) // 2}h", frame)))

    async def play(self, path):
        """Say what a raw 16 kHz recording says, then silence."""
        self.clip = open(path, "rb").read()

    async def speak(self):
        n = 0
        while True:
            pcm = b"".join(struct.pack("<h", int(12000 * math.sin(2 * math.pi * 440 * (n + i) / 16000))) for i in range(960))
            if self.clip is not None:
                pcm, self.clip = (self.clip[:1920].ljust(1920, b"\0"), self.clip[1920:]) if self.clip else (b"\0" * 1920, self.clip)
            n += 960
            if self.audio:
                self.audio.feed(pcm)
            await asyncio.sleep(0.06)

    async def stop(self):
        if self.sound:
            self.sound.cancel()
        if self.audio:
            self.audio.close()
            self.audio = None

    async def hang_up(self):
        await self.tell("hungup", participantId=77, reason="HUNGUP")
        await self.tell("closed-conversation", reason="HUNGUP")
        await self.stop()

    async def serve(self, ws):
        self.ws = ws
        await self.tell("connection", peerId={"id": 1}, conversationParams={}, conversation={"id": "c", "state": "ACTIVE", "topology": "DIRECT",
                        "participants": [{"id": 77, "externalId": {"type": "ONE_ME", "id": str(ANN)}, "state": "ACCEPTED"}]})
        if self.incoming:
            await self.start_media()
            await self.transmitted({"sdp": await self.audio.description("offer")})
        else:
            await self.tell("registered-peer", participantId=77, participantType="USER", peerId={"id": 2})
            asyncio.get_running_loop().call_later(0.3, lambda: asyncio.ensure_future(self.tell("accepted-call", participantId=77, peerId={"id": 2})))
        self.pinger = asyncio.ensure_future(self.ping())
        async for raw in ws:
            if raw == "pong":
                self.pongs += 1
                continue
            cmd = json.loads(raw)
            self.commands.append(cmd)
            if cmd["command"] == "hangup":
                await self.stop()
                continue
            await ws.send(json.dumps({"type": "response", "response": cmd["command"], "sequence": cmd["sequence"]}))
            data = cmd.get("data") or {}
            if "sdp" in data:
                if data["sdp"]["type"] == "offer":
                    await self.start_media()
                await self.audio.remote(data["sdp"])
                if data["sdp"]["type"] == "offer":
                    await self.transmitted({"sdp": await self.audio.description("answer")})
            if "candidate" in data and self.audio:
                self.audio.add_candidate(data["candidate"])


phone = Phone()


def invitation(conversation):
    """The real format: a raw LZ4 block, here only literals."""
    body = json.dumps({"tkn": "t", "wse": f"ws://127.0.0.1:{ws_port[0]}/ws2", "trnu": "9:1", "stne": "", "trne": "", "et": 0, "iv": False}).encode()
    n = len(body) - 15
    block = bytes([0xF0]) + b"\xff" * (n // 255) + bytes([n % 255]) + body
    return {"callerId": ANN, "chatId": 0, "conversationId": conversation, "type": "AUDIO", "vcp": f"{len(body)}:{base64.b64encode(block).decode()}"}


def on_phone(coro):
    return asyncio.run_coroutine_threadsafe(coro, ws_loop[0])


async def web_conn(ws):
    """The web client: the same frames, over a websocket. /ws2 is the call service."""
    if ws.request.path.startswith("/ws2"):
        await phone.serve(ws)
        return
    fake.web = ws
    async for data in ws:
        packet = framer.unpack(data)
        h = packet.header
        p = decoder.decode(packet.payload_bytes, h.flags)
        fake.requests.append((h.opcode, p))
        reply = fake.answer(h.opcode, p)
        cmd = Command.RESPONSE
        if isinstance(reply, tuple):
            cmd, reply = reply
        await ws.send(framer.pack(ver=h.ver, cmd=cmd, seq=h.seq, opcode=h.opcode, flags=0, payload_bytes=codec.encode(reply)))


def web_server():
    async def main():
        ws_loop.append(asyncio.get_running_loop())
        async with websockets.asyncio.server.serve(web_conn, "127.0.0.1", 0) as server:
            ws_port.append(server.sockets[0].getsockname()[1])
            await asyncio.Future()
    asyncio.run(main())


ws_port, ws_loop = [], []
threading.Thread(target=web_server, daemon=True).start()
while not ws_port:
    time.sleep(0.01)

srv = Server(("127.0.0.1", 0), Conn)
threading.Thread(target=srv.serve_forever, daemon=True).start()
files = tempfile.mkdtemp(prefix="bia-max-e2e-")


class Mcp:
    def __init__(self):
        env = dict(os.environ, BIA_FILES=files, BIA_MAX_SCRIPT=script, BIA_MAX_HOST="127.0.0.1", BIA_MAX_PORT=str(srv.server_address[1]), BIA_MAX_TLS="0", BIA_MAX_WS_URL=f"ws://127.0.0.1:{ws_port[0]}", BIA_MAX_CALL_TEST_HOST="127.0.0.1")
        self.log = open(os.path.join(files, "mcp.log"), "a")
        self.p = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, env=env, text=True, bufsize=1)
        self.results, self.inbound, self.rid = {}, [], 0
        threading.Thread(target=self.reader, daemon=True).start()
        self.rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})

    def reader(self):
        for line in self.p.stdout:
            v = json.loads(line)
            d = (v.get("params") or {}).get("data")
            if isinstance(d, dict) and d.get("t") == "inbound":
                self.inbound.append(d)
            elif "id" in v:
                self.results[v["id"]] = v

    def rpc(self, method, params=None, wait=90):
        self.rid += 1
        i = self.rid
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n")
        self.p.stdin.flush()
        end = time.time() + wait
        while time.time() < end:
            if i in self.results:
                return self.results.pop(i)
            time.sleep(0.02)
        raise SystemExit(f"no reply to {method} {params}; see {files}/mcp.log:\n" + open(os.path.join(files, "mcp.log")).read()[-3000:])

    def tool(self, tool, /, **args):
        r = self.rpc("tools/call", {"name": tool, "arguments": args})["result"]
        text = "".join(c.get("text", "") for c in r.get("content", []))
        return ("error: " + text) if r.get("isError") else text

    def wait_inbound(self, needle, wait=20):
        end = time.time() + wait
        while time.time() < end:
            for d in self.inbound:
                if needle in json.dumps(d, ensure_ascii=False):
                    return d
            time.sleep(0.05)
        return None

    def stop(self):
        self.p.stdin.close()
        self.p.send_signal(signal.SIGTERM)
        self.p.wait(10)


def wait_for(cond, wait=15):
    end = time.time() + wait
    while time.time() < end:
        if cond():
            return True
        time.sleep(0.05)
    return False


def incoming(mid, text, chat_id=DIALOG, sender=ANN, **kw):
    return {"chatId": chat_id, "message": {"id": mid, "sender": sender, "text": text, "time": now_ms(), "type": "USER", **kw}}


m = Mcp()
names = {t["name"] for t in m.rpc("tools/list")["result"]["tools"]}
for t in ("login", "submit_code", "submit_password", "status", "send", "send_voice", "edit", "react", "delete_messages", "history", "chats", "search", "resolve",
          "typing", "create_group", "group_info", "set_group_info", "add_members", "remove_member", "join_group", "leave_group", "group_invite_link", "group_permissions",
          "join_requests", "answer_join_request", "set_admin", "ban", "unban", "block", "pin", "set_profile", "send_contact", "send_location", "delete_chat", "restore", "logout"):
    check(t in names, f"tool {t}")
check("accept_request" not in names and "mute" not in names and "login_qr" not in names, "no Meta-only tools, and login is the one way in")
login_doc = next(t for t in m.rpc("tools/list")["result"]["tools"] if t["name"] == "login")["description"]
check("by QR (the default)" in login_doc and "its own phone number" in login_doc, "login tells the model about QR and bia's own account")
check("logged_out" in m.tool("status"), "logged out at first")
check("not logged in" in m.tool("send", chat_id=str(DIALOG), text="hi"), "send asks for a login")
check("country code" in m.tool("login", phone="123"), "a phone without country code is refused")

# a wrong code ends the attempt, and says so
out = m.tool("login", phone="+79991234567")
check("waiting_code" in out, f"login asks for the code: {out}")
out = m.tool("submit_code", code="000000")
check("Неверный код" in out and "max_login" in out, f"a wrong code says what to do: {out}")

# two-step password: wrong, then right
check("waiting_code" in m.tool("login", phone="+79991234567"), "second attempt")
out = m.tool("submit_code", code="222222")
check("waiting_password" in out and "pet" in out, f"asks for the password with its hint: {out}")
out = m.tool("submit_password", password="nope")
check("waiting_password" in out and "wrong" in out, f"a wrong password is said: {out}")
out = m.tool("submit_password", password="secret")
check("logged in to MAX as Bia Owner" in out, f"logged in: {out}")

session = os.path.join(files, "max", "session.db")
check(os.path.exists(session), "session on disk (the agent starts the MCP at boot from it)")
check(os.stat(session).st_mode & 0o077 == 0, "session.db is private")
out = m.tool("status")
check("auth=ready" in out and "Bia Owner" in out, f"status: {out}")

rows = m.tool("chats")
check(f"max:{GROUP}  Team  [group]" in rows, f"group in chats: {rows}")
check(f"max:{DIALOG}  Ann Lee" in rows, f"a dialog is named after the person: {rows}")
check("Ann Lee" in m.tool("search", query="ann"), "search by name")
check(f"max:" in m.tool("search", query="+7 999 000 11 22") and fake.seen(Opcode.CONTACT_INFO_BY_PHONE), "search by phone")
check(str(DIALOG) in m.tool("resolve", query=str(DIALOG)), "resolve a chat id")

# out
out = m.tool("send", chat_id=f"max:{DIALOG}", text="**hello**")
check("sent message id" in out and "Ann Lee" in out, f"send: {out}")
sent = fake.seen(Opcode.MSG_SEND)[-1]
check(sent["message"]["text"] == "hello" and sent["message"]["elements"], f"markdown becomes MAX formatting: {sent['message']}")
check("error" not in m.tool("edit", chat_id=str(DIALOG), message_id="5001", text="changed"), "edit")
check("reacted" in m.tool("react", chat_id=str(DIALOG), message_id="42", emoji="👍"), "react")
check("deleted 2" in m.tool("delete_messages", chat_id=str(DIALOG), message_ids=["1", "2"], revoke=False), "delete")
check(fake.seen(Opcode.MSG_DELETE)[-1].get("forMe") is True, "revoke=false deletes only here")
out = m.tool("history", chat_id=str(DIALOG))
check("old message" in out and "Ann Lee" in out, f"history: {out}")
out = m.tool("send_location", chat_id=str(DIALOG), latitude=55.75, longitude=37.61, name="Red Square")
check("map link" in out and "openstreetmap" in fake.seen(Opcode.MSG_SEND)[-1]["message"]["text"], f"location as a link: {out}")
m.tool("send_contact", chat_id=str(DIALOG), user=str(ANN))
check(fake.seen(Opcode.MSG_SEND)[-1]["message"]["attaches"][0]["_type"] == "CONTACT", "contact card")
os.makedirs(os.path.join(files, "out"), exist_ok=True)
for name, body in (("pic.jpg", b"\xff\xd8\xff fake"), ("notes.txt", b"file body")):
    open(os.path.join(files, "out", name), "wb").write(body)
out = m.tool("send", chat_id=str(DIALOG), text="вот", files=["out/pic.jpg", "out/notes.txt"])
check("sent 2 file(s)" in out, f"files go out: {out}")
kinds = [a.get("_type") for a in fake.seen(Opcode.MSG_SEND)[-1]["message"]["attaches"]]
check(kinds == ["PHOTO", "FILE"], f"as a photo and a file: {kinds}")
check(any(b"file body" in body for _, body in uploads), "the file's bytes were uploaded")
m.tool("typing", chat_id=str(DIALOG))
out = m.tool("call_start", chat_id=str(DIALOG))
check("QR login" in out and not fake.dials, f"calls refuse an SMS login, before MAX hears of it: {out}")
check(wait_for(lambda: fake.seen(Opcode.MSG_TYPING)), "typing")

# in
fake.push(Opcode.NOTIF_MESSAGE, incoming(100, "привет"))
d = m.wait_inbound("привет")
check(d is not None and d.get("channel") == f"max:{DIALOG}" and "Ann Lee" in d.get("identity", ""), f"message reaches the model: {d}")
check(wait_for(lambda: fake.seen(Opcode.CHAT_MARK)), "and is marked read")
fake.push(Opcode.NOTIF_MESSAGE, incoming(101, "моё", sender=ME))
fake.push(Opcode.NOTIF_MESSAGE, incoming(102, "дальше"))
check(m.wait_inbound("дальше") is not None and m.wait_inbound("моё", 1) is None, "this account's own messages stay out")
quoted = {"type": "REPLY", "chatId": DIALOG, "message": {"id": 90, "sender": ME, "text": "quoted text", "time": now_ms(), "type": "USER"}}
fake.push(Opcode.NOTIF_MESSAGE, incoming(103, "ответ", link=quoted))
d = m.wait_inbound("ответ")
check(d is not None and "reply to 90 from you" in d["message"] and "quoted text" in d["message"], f"a reply shows what it answers: {d}")
fake.push(Opcode.NOTIF_MESSAGE, incoming(104, "", attaches=[{"_type": "CONTACT", "contactId": 3, "name": "Bo"}]))
check(m.wait_inbound("[contact] Bo") is not None, "contact line")
fake.push(Opcode.NOTIF_MESSAGE, incoming(105, "", attaches=[{"_type": "CALL", "callType": "AUDIO", "hangupType": "MISSED", "conversationId": "c", "contactIds": []}]))
check(m.wait_inbound("[call] audio call: missed") is not None, "a call shows up afterwards")
fake.push(Opcode.NOTIF_MESSAGE, incoming(106, "", attaches=[{"_type": "LOCATION", "latitude": 55.5, "longitude": 37.5}]))
check(m.wait_inbound("[location]") is not None, "location line")
fake.push(Opcode.NOTIF_MESSAGE, incoming(100, "было", status="EDITED"))
d = m.wait_inbound("было")
check(d is not None and "привет" in d["message"] and "было" in d["message"], f"an edit knows the old text: {d}")
fake.push(Opcode.NOTIF_MESSAGE, incoming(108, "смотри", attaches=[{"_type": "PHOTO", "baseUrl": media_url() + "/media/p.jpg", "height": 1, "width": 1, "photoId": 9, "photoToken": "t"}]))
d = m.wait_inbound("смотри")
got = (d or {}).get("message", "")
path = os.path.join(files, got.split("file: ")[-1].strip()) if "file: " in got else ""
check(path.endswith(".jpg") and os.path.isfile(path), f"a photo is saved under inbox/max: {got}")
fake.push(Opcode.NOTIF_MESSAGE, incoming(109, "", attaches=[{"_type": "FILE", "fileId": 8, "name": "report.pdf", "size": 13, "token": "t"}]))
d = m.wait_inbound(f"file: inbox/max/{DIALOG}", 20) and m.wait_inbound(".pdf", 20)
got = (d or {}).get("message", "")
path = os.path.join(files, got.split("file: ")[-1].strip()) if "file: " in got else ""
check(path.endswith(".pdf") and open(path, "rb").read() == b"%PDF-1.4 fake", f"a file comes through MAX's download address: {got}")
fake.push(Opcode.NOTIF_MSG_DELETE, {"chatId": DIALOG, "messageIds": [102]})
check(m.wait_inbound("[deleted] 102") is not None, "delete notice")
fake.push(Opcode.NOTIF_MSG_REACTIONS_CHANGED, {"chatId": DIALOG, "messageId": 100, "reactionInfo": {"totalCount": 1, "counters": [{"count": 1, "reaction": "🔥"}]}})
check(m.wait_inbound("[reaction] 🔥") is not None, "reaction line")
m.tool("ban", chat_id=str(DIALOG))
fake.push(Opcode.NOTIF_MESSAGE, incoming(107, "забаненный"))
check(wait_for(lambda: any("banned" in (p.get("message") or {}).get("text", "") for p in fake.seen(Opcode.MSG_SEND))), "a banned chat is told so")
check(m.wait_inbound("забаненный", 1) is None, "and does not reach the model")
m.tool("unban", chat_id=str(DIALOG))

# a restart: the login comes back, and what came meanwhile is delivered once
m.stop()
fresh = now_ms() + 5000
fake.moved_at = fresh
fake.backlog = [{"id": 300, "sender": ANN, "text": "пока тебя не было", "time": fresh, "type": "USER"}]
m = Mcp()
out = m.tool("restore")
check("connected as Bia Owner" in out, f"restore: {out}")
check(m.wait_inbound("пока тебя не было") is not None, "catch-up after a restart")
fake.backlog = []

# the server revoked the session
m.stop()
fake.token_ok = False
m = Mcp()
out = m.tool("restore")
check("session ended" in out or "Сессия завершена" in out, f"a revoked session says what to do: {out}")
check(not os.path.exists(session), "and is forgotten")
check("logged_out" in m.tool("status"), "logged out")

# by QR: the picture appears for the window and the web app, and the login finishes by itself
fake.token_ok = True
out = m.tool("login")
qr = os.path.join(files, "max", "qr.png")
check("waiting_qr" in out, f"QR login waits for the scan: {out}")
check(wait_for(lambda: os.path.isfile(qr) and open(qr, "rb").read(4) == b"\x89PNG"), "the QR is a PNG where the web app looks")
fake.qr_scanned = True
check(m.wait_inbound("logged in as Bia Owner") is not None, "told when the scan is done")
check("auth=ready" in m.tool("status"), "connected by QR")
check(not os.path.exists(qr), "the QR is gone after the login")
check(json.load(open(os.path.join(files, "state", "max.json")))["logged_in"] is True, "and the login is remembered")
check("sent message id" in m.tool("send", chat_id=str(DIALOG), text="по QR"), "sends after a QR login")

# calls, with real WebRTC audio between this login and a fake phone
out = m.tool("call_status")
check("no call" in out and "cannot work" not in out, f"calls can work here: {out}")

def heard_sound():
    out = m.tool("call_status")
    return ("frames" in out and "heard 0 frames" not in out and "peak 0" not in out, out)

m.inbound.clear()
phone.reset()
fake.push(137, invitation("call-in"))
check(m.wait_inbound("[incoming call] from Ann Lee") is not None, "an incoming call is told")
check(wait_for(lambda: phone.ws is not None), "and its signalling is open while it rings")
check("answering" in m.tool("call_accept", chat_id=str(DIALOG)), "accept")
check(m.wait_inbound("[call ready]", 30) is not None, "the media connect: [call ready]")
check(wait_for(lambda: heard_sound()[0], 15), f"bia hears the phone: {heard_sound()[1]}")
check(wait_for(lambda: phone.frames > 5, 15), f"the phone hears bia: {phone.frames} frames")
check(any(c["command"] == "accept-call" for c in phone.commands), "accept-call went to the service")
out = m.tool("call_say", chat_id="1", text="x", lang="en")
check("does not match" in out or "voice" in out.lower() or "speech" in out.lower(), f"call_say refuses another chat or a missing voice: {out}")
if os.environ.get("BIA_SPEECH_DIR"):
    # with bia's real voices and listener: speech goes out through the call, and comes back heard
    phone.peak = 0
    out = m.tool("call_say", chat_id=str(DIALOG), text="Hello, this is a test of the call.", lang="en")
    check("speaking" in out, f"call_say: {out}")
    check(wait_for(lambda: phone.peak > 1000, 25), f"the phone hears the voice, not silence: peak {phone.peak}")
    check(m.wait_inbound("[you finished speaking]", 40) is not None, "[you finished speaking]")
    words = os.environ.get("MAX_E2E_WORDS")
    if words:
        on_phone(phone.play(words)).result(5)
        check(m.wait_inbound("[speech", 60) is not None, "what the phone says arrives as [speech]")
check(phone.pongs > 0, "the signalling answers ping with pong, or the service would close it")
check("hung up" in m.tool("call_hangup"), "hang up")
check(m.wait_inbound("[call ended]") is not None, "[call ended]")
check(wait_for(lambda: any(c["command"] == "hangup" and c["reason"] == "HUNGUP" for c in phone.commands)), "the service is told HUNGUP")

m.inbound.clear()
on_phone(phone.stop()).result(5)
phone.reset()
phone.incoming = False
out = m.tool("call_start", chat_id=str(DIALOG))
check("calling" in out, f"call out: {out}")
check(wait_for(lambda: fake.dials) and fake.dials[-1]["calleeIds"] == [ANN], "MAX is asked to ring the person")
check(json.loads(fake.dials[-1]["internalParams"])["platform"] == "WEB", "as a web client")
check(m.wait_inbound("[call ready]", 30) is not None, "the other person answered: [call ready]")
check(wait_for(lambda: heard_sound()[0], 15), f"sound comes in on an outgoing call too: {heard_sound()[1]}")
on_phone(phone.hang_up()).result(5)
check(m.wait_inbound("[call ended]", 15) is not None, "they hang up: [call ended]")

m.inbound.clear()
phone.reset()
fake.push(137, invitation("call-no"))
check(m.wait_inbound("[incoming call]") is not None, "a second incoming call")
check(wait_for(lambda: phone.ws is not None), "signalling open")
check("rejected" in m.tool("call_reject"), "reject")
check(m.wait_inbound("[call rejected]") is not None, "[call rejected]")
check(wait_for(lambda: any(c["command"] == "hangup" and c["reason"] == "REJECTED" for c in phone.commands)), "the service is told REJECTED")
on_phone(phone.stop()).result(5)
fake.push(137, {"callerId": ANN, "conversationId": "bad", "vcp": "1:!!!"})
check("no call" in m.tool("call_status"), "a broken invitation is dropped")

check("logged out" in m.tool("logout"), "logout")
check(not os.path.exists(session), "logout forgets the session")
m.stop()

if check.failed:
    print(open(os.path.join(files, "mcp.log")).read()[-4000:])
    sys.exit(1)
print("all ok")
