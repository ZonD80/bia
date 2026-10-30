"""A small client of the web bus protocol (protocol 2, `GET /ws`) for the integration tests:
WebSocket framing over mutual TLS, hello, actions with acks, the server's frames in a queue,
files offered here (served on `fget`) and files pulled from the server (`fget` / chunks / `fack`).
See ARCHITECTURE.md, "The web bus protocol".
"""
import base64, hashlib, json, os, socket, struct, threading, time


class Ws:
    """A client WebSocket on a TLS socket (frames masked, pings answered)."""

    def __init__(self, port, ctx, path="/ws", host="127.0.0.1"):
        raw = socket.create_connection((host, port), timeout=10)
        self.s = ctx.wrap_socket(raw, server_hostname=host)
        key = base64.b64encode(os.urandom(16)).decode()
        self.s.sendall(
            f"GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n".encode()
        )
        head = b""
        while b"\r\n\r\n" not in head:
            c = self.s.recv(1)
            if not c:
                break
            head += c
        self.status = int(head.split(b" ")[1]) if head.startswith(b"HTTP/") else 0
        self.lock = threading.Lock()
        self.s.settimeout(None)

    def send(self, op, payload):
        mask = os.urandom(4)
        n = len(payload)
        h = bytes([0x80 | op])
        if n < 126:
            h += bytes([0x80 | n])
        elif n < 65536:
            h += bytes([0x80 | 126]) + struct.pack(">H", n)
        else:
            h += bytes([0x80 | 127]) + struct.pack(">Q", n)
        body = bytes(b ^ mask[i % 4] for i, b in enumerate(payload)) if n < 4096 else _xor(payload, mask)
        with self.lock:
            self.s.sendall(h + mask + body)

    def send_json(self, v):
        self.send(1, json.dumps(v).encode())

    def _read(self, n):
        b = b""
        while len(b) < n:
            c = self.s.recv(n - len(b))
            if not c:
                raise EOFError
            b += c
        return b

    def recv(self):
        """(opcode, payload) of the next data frame; pings are answered."""
        while True:
            h = self._read(2)
            op, n = h[0] & 0x0F, h[1] & 0x7F
            if n == 126:
                n = struct.unpack(">H", self._read(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self._read(8))[0]
            mask = self._read(4) if h[1] & 0x80 else None
            p = self._read(n)
            if mask:
                p = _xor(p, mask)
            if op == 9:
                self.send(10, p)
                continue
            if op == 10:
                continue
            return op, p

    def close(self):
        try:
            self.s.close()
        except OSError:
            pass


def _xor(p, mask):
    m = (mask * (len(p) // 4 + 1))[: len(p)]
    return (int.from_bytes(p, "big") ^ int.from_bytes(m, "big")).to_bytes(len(p), "big") if p else b""


class Session:
    """One live session: hello, then the server's frames arrive in `self.frames`."""

    def __init__(self, port, ctx, rx=None, instance=None, name=""):
        self.ws = Ws(port, ctx)
        self.frames, self.acks, self.offers, self.pulls = [], {}, {}, {}
        self.cv = threading.Condition()
        self.hello = self.bye = None
        self.closed = False
        self.seen = 0
        self.instance = instance or os.urandom(6).hex()
        if self.ws.status != 101:
            self.closed = True
            return
        self.ws.send_json({"t": "hello", "proto": 2, "name": name, "ver": "test", "instance": self.instance, "rx": rx})
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=self._ping, daemon=True).start()
        with self.cv:
            self.cv.wait_for(lambda: self.hello is not None or self.bye is not None or self.closed, 15)

    def _ping(self):
        """A ping every 5 s: the server closes a session silent for 15 s."""
        while not self.closed:
            time.sleep(5)
            try:
                self.ws.send_json({"t": "ping"})
            except OSError:
                return

    def _read(self):
        try:
            while True:
                op, p = self.ws.recv()
                if op == 8:
                    break
                if op == 2:
                    self._chunk(p)
                    continue
                v = json.loads(p)
                t = v.get("t")
                with self.cv:
                    if t == "hello":
                        self.hello = v
                    elif t == "bye":
                        self.bye = v.get("reason")
                    elif t == "ack" and v.get("cid") is not None:
                        self.acks[v["cid"]] = v
                    if t == "fget":
                        threading.Thread(target=self._serve, args=(v["fid"], v.get("off", 0)), daemon=True).start()
                    elif t not in ("fack", "fdone", "ping"):
                        self.frames.append(v)
                    self.cv.notify_all()
        except (EOFError, OSError, ValueError):
            pass
        with self.cv:
            self.closed = True
            self.cv.notify_all()

    # ---- actions ----

    def act(self, t, wait=15, **kw):
        cid = kw.pop("cid", None) or "c" + os.urandom(6).hex()
        with self.cv:
            self.acks.pop(cid, None)
        self.ws.send_json(dict(kw, t=t, cid=cid))
        with self.cv:
            self.cv.wait_for(lambda: cid in self.acks or self.closed, wait)
            return self.acks.get(cid)

    def wait(self, pred, secs=15):
        """The next frame from the server (after the last one taken) that matches."""
        end = time.time() + secs
        with self.cv:
            while True:
                while self.seen < len(self.frames):
                    f = self.frames[self.seen]
                    self.seen += 1
                    if pred(f):
                        return f
                left = end - time.time()
                if left <= 0 or (self.closed and self.seen >= len(self.frames)):
                    return None
                self.cv.wait(min(left, 0.5))

    def msg(self, needle, secs=15):
        """An `m` frame whose message text contains `needle`."""
        return self.wait(lambda f: f.get("t") == "m" and needle in f["m"].get("text", ""), secs)

    # ---- files ----

    def offer(self, data, name):
        """A file this side sends: listed in a send, served when the server pulls it."""
        fid = hashlib.sha256(data).hexdigest()
        self.offers[fid] = data
        return {"fid": fid, "name": name, "size": len(data)}

    def _serve(self, fid, off):
        data = self.offers.get(fid)
        if data is None:
            self.ws.send_json({"t": "frefuse", "fid": fid, "reason": "not here"})
            return
        raw = bytes.fromhex(fid)
        for o in range(off, len(data), 32 * 1024):
            self.ws.send(2, raw + struct.pack(">Q", o) + data[o : o + 32 * 1024])

    def pull(self, fid, size, secs=30):
        """Pull a file from the server; returns its bytes (or None)."""
        ev = threading.Event()
        self.pulls[fid] = [bytearray(), size, ev]
        self.ws.send_json({"t": "fget", "fid": fid, "off": 0})
        ev.wait(secs)
        b = bytes(self.pulls[fid][0])
        return b if len(b) == size else None

    def _chunk(self, p):
        fid, off, data = p[:32].hex(), struct.unpack(">Q", p[32:40])[0], p[40:]
        st = self.pulls.get(fid)
        if not st or off != len(st[0]):
            return
        st[0].extend(data)
        self.ws.send_json({"t": "fack", "fid": fid, "off": len(st[0])})
        if len(st[0]) >= st[1]:
            self.ws.send_json({"t": "fdone", "fid": fid})
            st[2].set()

    def close(self, reason="closed"):
        try:
            self.ws.send_json({"t": "bye", "reason": reason})
            time.sleep(0.2)
        except OSError:
            pass
        self.ws.close()


def ctx_for(ca, cert_pem, key_pem=None):
    import ssl

    c = ssl.create_default_context(cafile=ca)
    c.check_hostname = False
    c.load_cert_chain(cert_pem, key_pem)
    return c
