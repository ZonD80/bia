"""A small client of the web bus protocol (protocol 3, `GET /ws`) for the integration tests:
WebSocket framing over TLS, hello and the signed sign-in (like a browser: Python's ssl cannot
read the TLS exporter, so the signature is not bound to the connection), actions with acks,
the server's frames in a queue, files offered here (served on `fget`) and files pulled from the
server (`fget` / chunks / `fack`). `Cred` also signs in over HTTP for the `bia_s` cookie.
See ARCHITECTURE.md, "Sign-in" and "The web bus protocol".
"""
import base64, hashlib, http.client, json, os, socket, ssl, struct, threading, time

PROTO = 3


def noverify():
    """TLS that takes the server's certificate as it is (self-signed here)."""
    c = ssl.create_default_context()
    c.check_hostname = False
    c.verify_mode = ssl.CERT_NONE
    return c


def b64u(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


class Cred:
    """A client certificate chain and its key (PEM, one file or two), signing like the web app."""

    def __init__(self, cert_pem, key_pem=None, port=8443, host="127.0.0.1"):
        from cryptography import x509
        from cryptography.hazmat.primitives import serialization

        data = open(cert_pem, "rb").read()
        self.chain = [c.public_bytes(serialization.Encoding.DER) for c in x509.load_pem_x509_certificates(data)]
        self.key = serialization.load_pem_private_key(open(key_pem or cert_pem, "rb").read(), None)
        self.port, self.host = port, host
        self.cookie = None
        self.ca_sha = None

    def sign(self, msg):
        from cryptography.hazmat.primitives import hashes
        from cryptography.hazmat.primitives.asymmetric import ec, ed25519, padding, rsa
        from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature

        k = self.key
        if isinstance(k, ec.EllipticCurvePrivateKey):
            h = hashes.SHA384() if k.curve.key_size == 384 else hashes.SHA256()
            r, s = decode_dss_signature(k.sign(msg, ec.ECDSA(h)))
            n = (k.curve.key_size + 7) // 8
            return r.to_bytes(n, "big") + s.to_bytes(n, "big")
        if isinstance(k, ed25519.Ed25519PrivateKey):
            return k.sign(msg)
        if isinstance(k, rsa.RSAPrivateKey):
            return k.sign(msg, padding.PKCS1v15(), hashes.SHA256())
        raise ValueError("key type")

    def auth(self, nonce, ca_sha):
        ts = int(time.time())
        msg = f"bia-auth-v1\n{nonce}\n{ca_sha}\n{ts}\n".encode()
        return {"t": "auth", "chain": [b64u(d) for d in self.chain], "ts": ts, "sig": b64u(self.sign(msg)), "nonce": nonce}

    def _ca(self):
        if self.ca_sha is None:
            c = http.client.HTTPSConnection(self.host, self.port, context=noverify(), timeout=10)
            c.request("GET", "/v1/auth/challenge")
            self.ca_sha = json.loads(c.getresponse().read())["ca_sha256"]
        return self.ca_sha

    def signin(self):
        """HTTP sign-in: challenge, signed auth, the `bia_s` cookie. Returns (status, body)."""
        c = http.client.HTTPSConnection(self.host, self.port, context=noverify(), timeout=10)
        c.request("GET", "/v1/auth/challenge")
        ch = json.loads(c.getresponse().read())
        self.ca_sha = ch["ca_sha256"]
        body = json.dumps(self.auth(ch["nonce"], self.ca_sha))
        c = http.client.HTTPSConnection(self.host, self.port, context=noverify(), timeout=10)
        c.request("POST", "/v1/auth/signin", body=body, headers={"Content-Type": "application/json", "Origin": self.origin()})
        r = c.getresponse()
        b = r.read()
        for k, v in r.getheaders():
            if k.lower() == "set-cookie" and v.startswith("bia_s="):
                self.cookie = v.split(";")[0].split("=", 1)[1]
        return r.status, b

    def origin(self):
        return f"https://{self.host}:{self.port}"

    def headers(self):
        """Headers a signed-in page sends (cookie + its own Origin)."""
        if self.cookie is None:
            self.signin()
        return {"Cookie": f"bia_s={self.cookie}", "Origin": self.origin()}


def request(port, method, path, body=None, cred=None, headers=None, host="127.0.0.1"):
    """One HTTPS request, as `cred` (cookie) or anonymous. Returns (status, body)."""
    h = dict(headers or {})
    if body is not None:
        h.setdefault("Content-Type", "application/json")
    if cred is not None:
        h.update(cred.headers())
    c = http.client.HTTPSConnection(host, port, context=noverify(), timeout=10)
    c.request(method, path, body=json.dumps(body) if isinstance(body, dict) else body, headers=h)
    try:
        r = c.getresponse()
        return r.status, r.read()
    except ConnectionResetError:
        return 0, b""


class Ws:
    """A client WebSocket on a TLS socket (frames masked, pings answered)."""

    def __init__(self, port, cred=None, path="/ws", host="127.0.0.1"):
        raw = socket.create_connection((host, port), timeout=10)
        self.s = noverify().wrap_socket(raw, server_hostname=host)
        key = base64.b64encode(os.urandom(16)).decode()
        extra = ""
        if cred is not None and path != "/ws":
            # the other websockets go by the session cookie and this page's origin
            extra = "".join(f"{k}: {v}\r\n" for k, v in cred.headers().items())
        self.s.sendall(
            f"GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n{extra}\r\n".encode()
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

    def __init__(self, port, cred, rx=None, instance=None, name=""):
        self.cred = cred
        self.ws = Ws(port)
        self.frames, self.acks, self.offers, self.pulls = [], {}, {}, {}
        self.cv = threading.Condition()
        self.hello = self.bye = None
        self.bye_port = None  # a bye that says where the web app answers next
        self.closed = False
        self.seen = 0
        self.instance = instance or os.urandom(6).hex()
        if self.ws.status != 101:
            self.closed = True
            return
        self.ws.send_json({"t": "hello", "proto": PROTO, "name": name, "ver": "test", "instance": self.instance, "rx": rx})
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
                if t == "challenge":
                    self.ws.send_json(self.cred.auth(v["nonce"], self.cred._ca()))
                    continue
                with self.cv:
                    if t == "hello":
                        self.hello = v
                    elif t == "bye":
                        self.bye = v.get("reason")
                        self.bye_port = v.get("port")
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


def ctx_for(ca, cert_pem, key_pem=None, port=8443):
    """The credentials a client signs in with (`ca` is unused: the server's CA comes from it)."""
    return Cred(cert_pem, key_pem, port=port)
