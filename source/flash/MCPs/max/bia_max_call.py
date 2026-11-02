# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mikhail Nevskiy (Haltia Messenger)
"""One-to-one MAX voice calls for bia_max.py: signalling to MAX's call service and WebRTC media on
GStreamer's webrtcbin, with bia's speech on both ends instead of a microphone and speakers.

Adapted from Haltia Messenger (https://github.com/haltia-de/messenger), whose
docs/calls-protocol.md describes the protocol: call_params, call_endpoint, CallAudio and VoiceCall
come from there, without its video, ringing and screens. It works as a web client, so calls need a
QR login. No reconnect and no retry: a call that fails has ended.
"""
import asyncio
import base64
import json
import logging
import os
import urllib.parse
from concurrent.futures import ThreadPoolExecutor

log = logging.getLogger("bia-max.call")
MEDIA_WORKER = ThreadPoolExecutor(1, thread_name_prefix="call-media")
FRAME_BYTES = 1920  # 60 ms of mono s16 at 16 kHz, what bia's listener takes


def call_params(vcp):
    """The raw LZ4 block in an incoming invitation: <uncompressed length>:<base64 block>."""
    size, encoded = vcp.split(":", 1)
    size = int(size)
    if not 0 < size <= 65536 or len(encoded) > 131072:
        raise ValueError("call invitation too large")
    data = base64.b64decode(encoded, validate=True)
    out, pos = bytearray(), 0

    def length(n):
        nonlocal pos
        if n == 15:
            while True:
                more = data[pos]
                pos += 1
                n += more
                if more != 255:
                    break
        return n

    while pos < len(data):
        token = data[pos]
        pos += 1
        n = length(token >> 4)
        if pos + n > len(data) or len(out) + n > size:
            raise ValueError("bad call literals")
        out.extend(data[pos:pos + n])
        pos += n
        if pos == len(data):
            break
        offset = data[pos] | data[pos + 1] << 8
        pos += 2
        n = length(token & 15) + 4
        if not 0 < offset <= len(out) or len(out) + n > size:
            raise ValueError("bad call match")
        for _ in range(n):
            out.append(out[-offset])
    if len(out) != size:
        raise ValueError("bad call length")
    return json.loads(out)


def call_endpoint(params, conversation, incoming):
    """Only the call service MAX names, never an arbitrary websocket (BIA_MAX_CALL_TEST_HOST: tests)."""
    endpoint = params.get("wse") if incoming else params.get("endpoint")
    url = urllib.parse.urlsplit(endpoint or "")
    test_host = os.environ.get("BIA_MAX_CALL_TEST_HOST")
    if test_host and url.hostname == test_host and url.scheme == "ws":
        pass
    elif url.scheme != "wss" or not url.hostname or not \
            (url.hostname == "okcdn.ru" or url.hostname.endswith(".okcdn.ru")) or \
            url.path != "/ws2" or url.username or url.password or url.fragment:
        raise ValueError("invalid call endpoint")
    query = dict(urllib.parse.parse_qsl(url.query))
    if incoming:
        query.update(userId=str(params["trnu"].rsplit(":", 1)[1]), entityType="USER", conversationId=conversation, token=params["tkn"])
    query.update(platform="WEB", appVersion="1.1", version="5", device="browser", capabilities="2A03F", clientType="ONE_ME", tgt="accept" if incoming else "start")
    return urllib.parse.urlunsplit(url._replace(query=urllib.parse.urlencode(query)))


def call_media():
    """The optional media libraries; ImportError or ValueError when they are not installed."""
    import gi
    for namespace in ("Gst", "GstWebRTC", "GstSdp"):
        gi.require_version(namespace, "1.0")
    from gi.repository import Gst, GstSdp, GstWebRTC
    return Gst, GstWebRTC, GstSdp


def calls_missing():
    """Why calls cannot work here, or an empty string."""
    try:
        Gst, _rtc, _sdp = call_media()
        Gst.init(None)
    except (ImportError, ValueError) as e:
        return f"GStreamer is not available ({e})"
    gone = [n for n in ("webrtcbin", "nicesrc", "opusenc", "opusdec", "appsrc", "appsink") if Gst.ElementFactory.find(n) is None]
    return f"GStreamer elements are missing: {', '.join(gone)}" if gone else ""


class CallAudio:
    """One WebRTC peer with Opus. What is heard leaves through `heard` as 60 ms frames; what bia says
    comes in through feed(). Gst callbacks cross into the loop; nothing else runs on their threads."""

    def __init__(self, loop, candidate, status, params, heard):
        Gst, GstWebRTC, GstSdp = call_media()
        self.Gst, self.RTC, self.Sdp = Gst, GstWebRTC, GstSdp
        Gst.init(None)
        self.loop, self.candidate, self.status, self.heard = loop, candidate, status, heard
        self.closed = False
        self.remote_set = False
        self.pending = []
        self.pipeline = None
        self.rtc = None
        self.bus = None
        self.signals = []
        self.partial = bytearray()
        try:
            # RFC 7587 section 7 requires /2 in Opus SDP even for mono; without it the peer never answers.
            self.pipeline = Gst.parse_launch(
                "webrtcbin name=rtc bundle-policy=max-bundle "
                "appsrc name=speech is-live=true format=time do-timestamp=true max-bytes=64000 "
                "caps=audio/x-raw,format=S16LE,rate=16000,channels=1,layout=interleaved "
                "! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=1 "
                "! opusenc bitrate=32000 audio-type=voice inband-fec=true packet-loss-percentage=10 complexity=5 "
                "! rtpopuspay pt=111 "
                "! application/x-rtp,media=audio,encoding-name=OPUS,payload=111,clock-rate=48000,encoding-params=(string)2 ! rtc.")
            self.rtc = self.pipeline.get_by_name("rtc")
            self.speech = self.pipeline.get_by_name("speech")
            for name, callback in (("on-ice-candidate", self._candidate), ("pad-added", self._received), ("notify::connection-state", self._state)):
                self.signals.append(self.rtc.connect(name, callback))
            # Every MAX client keeps a video section in the SDP, even in a voice call.
            caps = Gst.Caps.from_string("application/x-rtp,media=video,encoding-name=VP8,payload=96,clock-rate=90000")
            self.rtc.emit("add-transceiver", GstWebRTC.WebRTCRTPTransceiverDirection.INACTIVE, caps)
            turn, stun = params.get("turn") or {}, params.get("stun") or {}
            stuns = stun.get("urls") or params.get("stne", "").split(",")
            turns = turn.get("urls") or params.get("trne", "").split(",")
            stuns = [stuns] if isinstance(stuns, str) else stuns
            turns = [turns] if isinstance(turns, str) else turns
            if stuns and stuns[0]:
                self.rtc.set_property("stun-server", stuns[0].replace("stun:", "stun://", 1))
            for server in filter(None, turns):
                scheme, address = server.split(":", 1)
                if scheme not in ("turn", "turns"):
                    raise ValueError("invalid TURN scheme")
                user = urllib.parse.quote(turn.get("username", params.get("trnu", "")), safe="")
                password = urllib.parse.quote(turn.get("credential", params.get("trnp", "")), safe="")
                if not self.rtc.emit("add-turn-server", f"{scheme}://{user}:{password}@{address.removeprefix('//')}"):
                    raise ValueError("invalid TURN server")
            if params.get("p2p_forbidden"):
                self.rtc.set_property("ice-transport-policy", GstWebRTC.WebRTCICETransportPolicy.RELAY)
            self.bus = self.pipeline.get_bus()
            self.bus.set_sync_handler(self._bus, None)
        except BaseException:
            self.close()
            raise

    async def start(self):
        """PLAYING waits for streaming threads, so not on the loop."""
        pipeline = self.pipeline
        if self.closed or pipeline is None:
            raise RuntimeError("audio pipeline failed")
        result = await asyncio.wrap_future(MEDIA_WORKER.submit(pipeline.set_state, self.Gst.State.PLAYING))
        if not self.closed and result == self.Gst.StateChangeReturn.FAILURE:
            self.close()
            raise RuntimeError("audio pipeline failed")

    def _deliver(self, callback, value):
        if not self.closed and callback:
            callback(value)

    def _candidate(self, _rtc, index, candidate):
        self.loop.call_soon_threadsafe(self._deliver, self.candidate, {"candidate": candidate, "sdpMLineIndex": index})

    def _state(self, rtc, _spec):
        self.loop.call_soon_threadsafe(self._deliver, self.status, rtc.get_property("connection-state").value_nick)

    def _bus(self, _bus, message, _data):
        if message.type == self.Gst.MessageType.ERROR:
            # Logged by element and code only: the debug text can hold TURN credentials.
            err, _debug = message.parse_error()
            factory = message.src.get_factory() if hasattr(message.src, "get_factory") else None
            log.warning("WRN gst error %s %s:%d", factory.get_name() if factory else "bin", err.domain, err.code)
            self.loop.call_soon_threadsafe(self._deliver, self.status, "failed")
        return self.Gst.BusSyncReply.DROP

    def _received(self, _rtc, pad):
        pipeline = self.pipeline
        if pad.direction != self.Gst.PadDirection.SRC or self.closed or pipeline is None:
            return
        caps = pad.get_current_caps() or pad.query_caps(None)
        media = caps.get_structure(0).get_string("media")
        if media == "audio":
            description = ("queue ! rtpopusdepay ! opusdec plc=true use-inband-fec=true ! audioconvert ! audioresample "
                           "! audio/x-raw,format=S16LE,rate=16000,channels=1 ! appsink name=heard emit-signals=true sync=false")
        else:
            description = "fakesink sync=false async=false"
        received = self.Gst.parse_bin_from_description(description, True)
        if media == "audio":
            received.get_by_name("heard").connect("new-sample", self._sample)
        pipeline.add(received)
        pad.link(received.get_static_pad("sink"))
        received.sync_state_with_parent()

    def _sample(self, sink):
        sample = sink.emit("pull-sample")
        if sample is not None:
            buffer = sample.get_buffer()
            ok, info = buffer.map(self.Gst.MapFlags.READ)
            if ok:
                data = bytes(info.data)
                buffer.unmap(info)
                self.loop.call_soon_threadsafe(self._heard, data)
        return self.Gst.FlowReturn.OK

    def _heard(self, data):
        if self.closed:
            return
        self.partial.extend(data)
        while len(self.partial) >= FRAME_BYTES:
            frame = bytes(self.partial[:FRAME_BYTES])
            del self.partial[:FRAME_BYTES]
            self.heard(frame)

    def feed(self, pcm):
        """What bia says, as it is paced by the MCP."""
        if not self.closed and self.speech is not None:
            self.speech.emit("push-buffer", self.Gst.Buffer.new_wrapped(pcm))

    async def _promise(self, command, *args):
        future = self.loop.create_future()

        def complete(promise, *_unused):
            answered = promise.wait() == self.Gst.PromiseResult.REPLIED
            reply = promise.get_reply() if answered else None

            def deliver():
                if not future.done():
                    if not answered or (reply is not None and reply.has_field("error")):
                        future.set_exception(RuntimeError("WebRTC negotiation failed"))
                    else:
                        future.set_result(reply)
            self.loop.call_soon_threadsafe(deliver)
        promise = self.Gst.Promise.new_with_change_func(complete, None, None)
        self.rtc.emit(command, *args, promise)
        return await asyncio.wait_for(future, 10)

    async def remote(self, description):
        kind = {"offer": self.RTC.WebRTCSDPType.OFFER, "answer": self.RTC.WebRTCSDPType.ANSWER}[description["type"]]
        if self.closed:
            return
        result, sdp = self.Sdp.SDPMessage.new_from_text(description["sdp"])
        if result != self.Sdp.SDPResult.OK:
            raise ValueError("invalid remote SDP")
        await self._promise("set-remote-description", self.RTC.WebRTCSessionDescription.new(kind, sdp))
        self.remote_set = True
        for candidate in self.pending:
            self.add_candidate(candidate)
        self.pending.clear()

    async def description(self, kind):
        reply = await self._promise("create-" + kind, None)
        desc = reply.get_value(kind)
        await self._promise("set-local-description", desc)
        return {"type": kind, "sdp": desc.sdp.as_text()}

    def add_candidate(self, value):
        if not self.remote_set:
            if len(self.pending) >= 256:
                raise ValueError("too many ICE candidates")
            self.pending.append(value)
        else:
            self.rtc.emit("add-ice-candidate", int(value.get("sdpMLineIndex", 0)), value["candidate"])

    def close(self):
        self.closed = True
        pipeline, self.pipeline = self.pipeline, None
        rtc, self.rtc = self.rtc, None
        bus, self.bus = self.bus, None
        self.speech = None
        if bus is not None:
            bus.set_sync_handler(None, None)
        if rtc is not None:
            for handler in self.signals:
                rtc.disconnect(handler)
        self.signals.clear()
        if pipeline is not None:
            MEDIA_WORKER.submit(pipeline.set_state, self.Gst.State.NULL)
        self.pending.clear()
        self.candidate = self.status = None


class VoiceCall:
    """One 1:1 call, from the invitation or dial until it ends. Commands have deadlines."""

    DISCONNECT_GRACE = 7  # seconds; a brief network drop is not a failed call
    RING_SECONDS = 90
    SETTINGS = {"isAudioEnabled": True, "isVideoEnabled": False, "isScreenSharingEnabled": False,
                "isFastScreenSharingEnabled": False, "isAudioSharingEnabled": False}
    FATAL_ERRORS = frozenset({"conversation-ended", "invalid-token", "service-unavailable", "gen.obsoleteClient"})

    def __init__(self, conversation, peer, incoming, changed, heard, agent=None):
        self.id, self.peer, self.incoming = conversation, peer, incoming
        self.changed, self.heard, self.agent = changed, heard, agent
        self.state = "ringing" if incoming else "dialling"
        self.closed = False
        self.end_reason = None
        self.reported = None
        self.accepted = False
        self.sequence = 0
        self.pending = {}
        self.socket = None
        self.audio = None
        self.participant = None
        self.device = 0
        self.offer = None
        self.candidates = []
        self.local_candidates = []
        self.local_sent = False
        self.jobs = set()
        self.ready = asyncio.Event()
        self.offer_ready = asyncio.Event()
        self.params = {}
        self.task = None
        self.disconnect_timer = None
        self.frames_in = 0
        self.peak_in = 0
        self.frames_out = 0

    def hangup_reason(self):
        return "HUNGUP" if self.accepted else "REJECTED" if self.incoming else "CANCELED"

    def show(self, state):
        if not self.closed:
            log.info("call: %s", state)
            self.state = state
            self.changed(self, state)

    def job(self, coro):
        if self.closed:
            coro.close()
            return
        task = asyncio.create_task(coro)
        self.jobs.add(task)

        def done(future):
            self.jobs.discard(future)
            if not future.cancelled() and future.exception() is not None and not self.closed:
                log.warning("WRN call job failed: %s", type(future.exception()).__name__)
                self.job(self.end("FAILED", "failed"))
        task.add_done_callback(done)

    async def command(self, name, **body):
        if self.closed or self.socket is None:
            raise ConnectionError("the call has ended")
        self.sequence += 1
        sequence = self.sequence
        future = asyncio.get_running_loop().create_future()
        self.pending[sequence] = future
        try:
            await self.socket.send(json.dumps(dict(command=name, sequence=sequence, **body)))
            return await asyncio.wait_for(future, 10)
        finally:
            self.pending.pop(sequence, None)
            if future.done() and not future.cancelled():
                future.exception()

    def start(self, params):
        self.task = asyncio.create_task(self.run(params))

    async def send_hangup(self):
        if self.end_reason and self.socket:
            self.sequence += 1
            try:
                await asyncio.wait_for(self.socket.send(json.dumps({"command": "hangup", "sequence": self.sequence, "reason": self.end_reason})), 2)
            except Exception:
                pass

    async def run(self, params):
        self.params = params
        try:
            from websockets.asyncio.client import connect
            async with connect(call_endpoint(params, self.id, self.incoming), origin="https://web.max.ru", user_agent_header=self.agent,
                               open_timeout=10, close_timeout=2, max_size=262144) as socket:
                self.socket = socket
                if self.closed:
                    await self.send_hangup()
                    return
                self.job(self.initialize())
                self.job(self.ring_deadline())
                while not self.closed:
                    raw = await asyncio.wait_for(socket.recv(), 20)
                    if raw == "ping":
                        await socket.send("pong")
                        continue
                    event = json.loads(raw)
                    if event.get("type") in ("response", "error"):
                        future = self.pending.get(event.get("sequence"))
                        code = event.get("error") or event.get("message")
                        if future is not None and not future.done():
                            if event["type"] == "error":
                                future.set_exception(RuntimeError("call command refused"))
                                if code in self.FATAL_ERRORS:
                                    self.job(self.end("FAILED", "failed"))
                            else:
                                future.set_result(event)
                        elif event["type"] == "error" and code in self.FATAL_ERRORS:
                            self.job(self.end("FAILED", "failed"))
                    elif event.get("type") == "notification":
                        await self.event(event)
        except asyncio.CancelledError:
            raise
        except Exception as e:
            # The exception's text can hold the endpoint's token: its type only.
            if not self.closed:
                log.warning("WRN call signalling failed: %s", type(e).__name__)
            await self.end(None, "failed")
        finally:
            await self.end(None, "ended")

    async def initialize(self):
        await asyncio.wait_for(self.ready.wait(), 10)
        await self.command("change-media-settings", mediaSettings=dict(self.SETTINGS, isAudioEnabled=False))
        await self.command("change-participant-state", participantState={"state": {}})

    async def event(self, event):
        name = event.get("notification")
        if name == "connection":
            conversation = event.get("conversation") or {}
            if conversation.get("topology", "DIRECT") != "DIRECT":
                self.job(self.end("FAILED", "unsupported"))
                return
            self.params.update(event.get("conversationParams") or {})
            for person in conversation.get("participants", []):
                if str((person.get("externalId") or {}).get("id")) == str(self.peer):
                    self.participant = person["id"]
                    self.device = person.get("deviceIdx", 0)
            self.ready.set()
        elif name in ("registered-peer", "accepted-call", "transmitted-data"):
            if event.get("participantId") is None:
                return
            if self.participant is not None and str(self.participant) != str(event["participantId"]):
                return
            self.participant = event["participantId"]
            self.device = event.get("deviceIdx", 0)
            if name == "accepted-call" and not self.incoming and not self.accepted:
                self.accepted = True
                self.job(self.negotiate())
            elif name == "transmitted-data":
                data = event.get("data") or {}
                if "sdp" in data:
                    if data["sdp"]["type"] == "offer" and self.incoming:
                        self.offer = data["sdp"]
                        self.offer_ready.set()
                    elif data["sdp"]["type"] == "answer" and self.audio and not self.incoming:
                        self.job(self.audio.remote(data["sdp"]))
                if "candidate" in data:
                    candidate = data["candidate"]
                    if not isinstance(candidate, dict) or not isinstance(candidate.get("candidate"), str):
                        log.info("call: invalid candidate skipped")
                    elif self.audio:
                        self.audio.add_candidate(candidate)
                    elif len(self.candidates) < 256:
                        self.candidates.append(candidate)
        elif name in ("hungup", "closed-conversation"):
            await self.end(remote=True, remote_reason=event.get("reason"))
        elif name == "topology-changed" and event.get("topology") != "DIRECT":
            self.job(self.end("FAILED", "unsupported"))

    async def answer(self):
        if self.closed or not self.incoming or self.accepted:
            return
        self.accepted = True
        await asyncio.wait_for(self.ready.wait(), 10)
        await self.command("accept-call", mediaSettings=self.SETTINGS)
        self.job(self.negotiate())

    async def transmit(self, data):
        target = {"participantId": self.participant, "participantType": "USER", "data": data}
        if self.device:
            target["deviceIdx"] = self.device
        await self.command("transmit-data", **target)

    async def best_effort(self, command, *args):
        try:
            await command(*args)
        except asyncio.CancelledError:
            raise
        except Exception:
            log.info("call: candidate not acknowledged")

    def candidate(self, value):
        if self.local_sent:
            self.job(self.best_effort(self.transmit, {"candidate": value}))
        else:
            self.local_candidates.append(value)

    def audio_state(self, state):
        if self.closed:
            return
        if state == "connected":
            if self.disconnect_timer is not None:
                self.disconnect_timer.cancel()
                self.disconnect_timer = None
            self.show("active")
        elif state == "failed":
            self.job(self.end("NETWORK_ERROR", "failed"))
        elif state == "disconnected" and self.disconnect_timer is None:
            def expired():
                self.disconnect_timer = None
                if not self.closed:
                    self.job(self.end("NETWORK_ERROR", "failed"))
            self.disconnect_timer = asyncio.get_running_loop().call_later(self.DISCONNECT_GRACE, expired)

    def got_audio(self, frame):
        self.frames_in += 1
        self.peak_in = max(self.peak_in, max(abs(int.from_bytes(frame[i:i + 2], "little", signed=True)) for i in range(0, len(frame), 2)))
        self.heard(frame)

    def feed(self, pcm):
        if self.audio is not None:
            self.frames_out += 1
            self.audio.feed(pcm)

    async def negotiate(self):
        self.show("connecting")
        if self.incoming:
            await asyncio.wait_for(self.offer_ready.wait(), 15)
        if self.closed:
            return
        self.audio = CallAudio(asyncio.get_running_loop(), self.candidate, self.audio_state, self.params, self.got_audio)
        await self.audio.start()
        if self.closed:
            return
        for candidate in self.candidates:
            self.audio.add_candidate(candidate)
        self.candidates.clear()
        if self.incoming:
            await self.audio.remote(self.offer)
        description = await self.audio.description("answer" if self.incoming else "offer")
        if self.incoming and self.offer.get("p2pRelay"):
            description["p2pRelay"] = True
        await self.transmit({"sdp": description})
        self.local_sent = True
        for candidate in self.local_candidates:
            self.job(self.best_effort(self.transmit, {"candidate": candidate}))
        self.local_candidates.clear()
        await self.command("change-media-settings", mediaSettings=self.SETTINGS)
        self.job(self.deadline())

    async def ring_deadline(self):
        await asyncio.sleep(self.RING_SECONDS)
        if not self.closed and not self.accepted:
            await self.end("MISSED" if self.incoming else "CANCELED", "ended")

    async def deadline(self):
        await asyncio.sleep(30)
        if not self.closed and self.state != "active":
            await self.end("TIMEOUT", "failed")

    async def end(self, reason=None, state="ended", *, remote=False, remote_reason=None):
        if self.closed:
            return
        self.closed = True
        self.end_reason = reason
        self.reported = remote_reason if remote else reason
        if self.disconnect_timer is not None:
            self.disconnect_timer.cancel()
            self.disconnect_timer = None
        self.state = state
        current = asyncio.current_task()
        try:
            if self.audio:
                self.audio.close()
        except Exception:
            log.warning("WRN call media teardown failed")
        for task in list(self.jobs):
            if task is not current:
                task.cancel()
        for future in list(self.pending.values()):
            if not future.done():
                future.cancel()
        self.changed(self, state)
        if self.socket:
            await self.send_hangup()
            try:
                await self.socket.close()
            except Exception:
                pass
        if self.socket and self.task and self.task is not current:
            self.task.cancel()
