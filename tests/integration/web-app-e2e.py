"""End-to-end: the web app (web/app.js + web/link.js) in a real headless Chrome against a real
bia-mcp-web, driven over the DevTools protocol.

A browser without a person cannot pick a client certificate, so a small local proxy holds root's
certificate: Chrome opens http://127.0.0.1:<proxy> (a secure context, like any localhost) and the
proxy speaks mutual TLS to the web MCP on 8443. Checks: the app connects with protocol 2 and loads
the chat, a message both ways, a file from the page (hashed in the page, pulled by bia) and a photo
from bia (pulled by the page, shown as a blob), a second tab refused while the first is open and
taking over once it closes, and a message written while bia is down that survives a reload and
reaches bia once it is back.

  python3 -I tests/integration/web-app-e2e.py target/debug/bia-mcp-web [chrome]
"""
import base64, hashlib, json, os, shutil, signal, socket, ssl, struct, subprocess, sys, tempfile, threading, time, urllib.request
import http.client

exe = sys.argv[1]
chrome = sys.argv[2] if len(sys.argv) > 2 else next((c for c in (
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/chromium", "/usr/bin/google-chrome") if os.path.exists(c)), None)
if not chrome:
    print("SKIP: no Chrome or Chromium here")
    sys.exit(0)
root = tempfile.mkdtemp(prefix="bia-webapp-e2e-")
files = f"{root}/bia"
os.makedirs(files)
PROXY, CDP = 9461, 9462
failed = []


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        failed.append(what)


# ---- the web MCP
class Mcp:
    def __init__(self):
        env = dict(os.environ, BIA_FILES=files, BIA_WEB_PORT="8443", BIA_WEB_HTTP_PORT="0")
        self.p = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, text=True, bufsize=1)
        self.notes, self.results, self.rid = [], {}, 0
        threading.Thread(target=self._read, daemon=True).start()
        self.rpc("initialize")
        for _ in range(200):
            if os.path.exists(f"{files}/state/web.json"):
                break
            time.sleep(0.05)

    def _read(self):
        for line in self.p.stdout:
            try:
                v = json.loads(line)
            except ValueError:
                continue
            if v.get("method") == "notifications/message" and isinstance(v["params"]["data"], dict):
                self.notes.append(v["params"]["data"])
            elif "id" in v:
                self.results[v["id"]] = v

    def rpc(self, method, params=None):
        self.rid += 1
        i = self.rid
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n")
        self.p.stdin.flush()
        for _ in range(3000):
            if i in self.results:
                return self.results.pop(i)
            time.sleep(0.02)
        raise SystemExit(f"no reply to {method}")

    def tool(self, name, **a):
        r = self.rpc("tools/call", {"name": name, "arguments": a})["result"]
        return r["content"][0]["text"], r.get("isError", False)

    def inbound(self, pred, secs=20):
        end = time.time() + secs
        while time.time() < end:
            for n in list(self.notes):
                if n.get("t") == "inbound" and pred(n):
                    self.notes.remove(n)
                    return n
            time.sleep(0.05)
        return None

    def stop(self):
        self.p.send_signal(signal.SIGTERM)
        try:
            self.p.wait(10)
        except subprocess.TimeoutExpired:
            self.p.kill()


web = Mcp()
nov = ssl.create_default_context(); nov.check_hostname = False; nov.verify_mode = ssl.CERT_NONE
c = http.client.HTTPSConnection("127.0.0.1", 8443, context=nov, timeout=10)
c.request("POST", "/v1/pki/bootstrap/root")
p12 = c.getresponse().read()
open(f"{root}/root.p12", "wb").write(p12)
subprocess.run(["openssl", "pkcs12", "-in", f"{root}/root.p12", "-passin", "pass:12345678", "-nodes", "-out", f"{root}/root.pem"], check=True, capture_output=True)
mtls = ssl.create_default_context(); mtls.check_hostname = False; mtls.verify_mode = ssl.CERT_NONE
mtls.load_cert_chain(f"{root}/root.pem")


# ---- the proxy: plain on 127.0.0.1:PROXY, mutual TLS to the web MCP
def pipe(a, b):
    try:
        while True:
            d = a.recv(65536)
            if not d:
                break
            b.sendall(d)
    except OSError:
        pass
    for s in (a, b):
        try:
            s.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass


def proxy():
    ls = socket.socket()
    ls.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    ls.bind(("127.0.0.1", PROXY))
    ls.listen(64)
    while True:
        cl, _ = ls.accept()
        try:
            up = mtls.wrap_socket(socket.create_connection(("127.0.0.1", 8443), timeout=10), server_hostname="127.0.0.1")
            up.settimeout(None)
        except OSError:
            cl.close()
            continue
        threading.Thread(target=pipe, args=(cl, up), daemon=True).start()
        threading.Thread(target=pipe, args=(up, cl), daemon=True).start()


threading.Thread(target=proxy, daemon=True).start()

# ---- Chrome over the DevTools protocol
prof = f"{root}/chrome"
cr = subprocess.Popen([chrome, "--headless=new", f"--remote-debugging-port={CDP}", f"--user-data-dir={prof}", "--no-first-run",
                       "--no-default-browser-check", "--disable-gpu", "about:blank"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
for _ in range(100):
    try:
        ver = json.loads(urllib.request.urlopen(f"http://127.0.0.1:{CDP}/json/version").read())
        break
    except OSError:
        time.sleep(0.1)


class Cdp:
    """The browser endpoint: plain WebSocket, JSON commands, flat sessions."""

    def __init__(self, url):
        host, path = url.split("://", 1)[1].split("/", 1)
        h, port = host.split(":")
        self.s = socket.create_connection((h, int(port)))
        key = base64.b64encode(os.urandom(16)).decode()
        self.s.sendall(f"GET /{path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n".encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            buf += self.s.recv(1)
        self.id, self.res, self.lock = 0, {}, threading.Lock()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        def rd(n):
            b = b""
            while len(b) < n:
                c = self.s.recv(n - len(b))
                if not c:
                    raise EOFError
                b += c
            return b
        try:
            while True:
                h = rd(2)
                n = h[1] & 0x7F
                if n == 126:
                    n = struct.unpack(">H", rd(2))[0]
                elif n == 127:
                    n = struct.unpack(">Q", rd(8))[0]
                v = json.loads(rd(n))
                if "id" in v:
                    self.res[v["id"]] = v
        except (EOFError, OSError):
            pass

    def call(self, method, params=None, session=None, wait=60):
        with self.lock:
            self.id += 1
            i = self.id
            msg = {"id": i, "method": method, "params": params or {}}
            if session:
                msg["sessionId"] = session
            p = json.dumps(msg).encode()
            mask = os.urandom(4)
            n = len(p)
            hd = bytes([0x81]) + (bytes([0x80 | n]) if n < 126 else bytes([0x80 | 126]) + struct.pack(">H", n) if n < 65536 else bytes([0x80 | 127]) + struct.pack(">Q", n))
            self.s.sendall(hd + mask + bytes(b ^ mask[k % 4] for k, b in enumerate(p)))
        end = time.time() + wait
        while time.time() < end:
            if i in self.res:
                r = self.res.pop(i)
                if "error" in r:
                    raise RuntimeError(r["error"])
                return r["result"]
            time.sleep(0.01)
        raise TimeoutError(method)


cdp = Cdp(ver["webSocketDebuggerUrl"])
APP = f"http://127.0.0.1:{PROXY}/"


class Tab:
    def __init__(self):
        self.target = cdp.call("Target.createTarget", {"url": APP})["targetId"]
        self.sess = cdp.call("Target.attachToTarget", {"targetId": self.target, "flatten": True})["sessionId"]

    def js(self, expr, wait=60):
        r = cdp.call("Runtime.evaluate", {"expression": expr, "awaitPromise": True, "returnByValue": True}, self.sess, wait)
        if "exceptionDetails" in r:
            return {"error": r["exceptionDetails"].get("exception", {}).get("description", str(r["exceptionDetails"]))}
        return r["result"].get("value")

    def until(self, expr, secs=20):
        end = time.time() + secs
        while time.time() < end:
            try:
                v = self.js(expr, 3)
                if v and not (isinstance(v, dict) and "error" in v):
                    return v
            except (RuntimeError, TimeoutError):
                pass
            time.sleep(0.2)
        return None

    def reload(self):
        cdp.call("Page.reload", {}, self.sess)

    def close(self):
        # like a person closing the tab: the page's pagehide runs (Target.closeTarget would
        # kill the renderer without it)
        try:
            cdp.call("Page.close", {}, self.sess, 10)
        except (RuntimeError, TimeoutError):
            cdp.call("Target.closeTarget", {"targetId": self.target})


LOADED = "typeof S !== 'undefined' && S.wsOk && S.cache.get('main') && !S.cache.get('main').loading"

# ---- the app connects with protocol 2 and loads the chat
t1 = Tab()
check(t1.until(LOADED, 30) is not None, "the app connects (protocol 2) and loads the chat")
if os.environ.get("DEBUG"):
    print(t1.js("JSON.stringify({wsOk: S.wsOk, cur: S.cur, route: S.route, st: S.linkState, cache: [...S.cache.keys()], c: S.cache.get('main') && {loading: S.cache.get('main').loading, n: S.cache.get('main').list.length}, w: innerWidth, hash: location.hash, err: window.__err})"))
check(web.inbound(lambda n: n["identity"] == "system" and n["message"].startswith("[web open] root connected")) is not None, "bia hears root connected")
check(t1.js("S.link.hello.proto") == 2 and t1.js("document.querySelectorAll('#chatList .citem').length") == 1, "one chat in the list")
t1.js("go('chat/main')")
check(t1.until("!!document.querySelector('#msgsIn') && S.cur === 'main'", 10) is not None, "the chat opens")

# ---- a message both ways
t1.js("void sendMessage({text: 'hello from the app', files: []})")
n = web.inbound(lambda n: n["identity"] == "root" and n["message"] == "hello from the app")
check(n is not None and n["id"].startswith("w"), f"the page's message reaches bia with the page's id: {n}")
check(t1.until("[...document.querySelectorAll('#msgsIn .mrow.out')].some(r => r.textContent.includes('hello from the app')) && !(S.pending.get('main') || []).length", 10) is not None,
      "its bubble becomes the stored message")
web.tool("send", identity="root", channel="web", text="hi app")
check(t1.until("document.querySelector('#msgsIn').textContent.includes('hi app')", 10) is not None, "bia's answer shows up live")

# ---- a file from the page: hashed there, pulled by bia over the socket
size = 300_000
data = bytes((i * 7) % 251 for i in range(size))
t1.js(f"(() => {{ const u = new Uint8Array({size}); for (let i = 0; i < u.length; i++) u[i] = (i * 7) % 251; void sendMessage({{text: 'with a file', files: [new File([u], 'page.bin')]}}); }})()")
n = web.inbound(lambda n: n["identity"] == "root" and "with a file" in n["message"], 30)
got = n and [l[2:].split(" (")[0] for l in n["message"].splitlines() if l.startswith("- ")]
check(bool(got) and open(f"{files}/{got[0]}", "rb").read() == data, f"bia got the page's file intact: {got}")

# ---- a photo from bia: pulled by the page, shown from a blob
png = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFklEQVR4nGP8z8DAwMDAxMDAwMDAAAANBAEB3rFzxQAAAABJRU5ErkJggg==")
open(f"{files}/dot.png", "wb").write(png)
web.tool("send", identity="root", channel="web", text="a photo", files=[f"{files}/dot.png"])
src = t1.until("(() => { const i = [...document.querySelectorAll('#msgsIn .media img')].pop(); return i && i.complete && i.naturalWidth > 0 && i.src; })()", 20)
check(bool(src) and src.startswith("blob:"), f"the photo is shown from what the page pulled: {src}")
pulled = t1.js("(async () => { const i = [...document.querySelectorAll('#msgsIn .media img')].pop(); return (await (await fetch(i.src)).arrayBuffer()).byteLength; })()")
check(pulled == len(png), f"…intact ({pulled} bytes)")

# ---- one live session: a second tab waits while the first is open, then takes over
t2 = Tab()
if os.environ.get("DEBUG"):
    time.sleep(3)
    print("t2:", t2.js("JSON.stringify({href: location.href, ready: document.readyState, hasS: typeof S, body: document.body ? document.body.innerText.slice(0, 200) : null})"))
    print("t1:", t1.js("JSON.stringify({wsOk: S.wsOk, st: S.linkState})"))
check(t2.until("typeof S !== 'undefined' && S.linkState === 'blocked'", 20) is not None, "a second tab is refused while the first is open")
check("still open" in (t2.js("document.querySelector('#connState').textContent") or ""), "…and says the other session is still open")
check(t1.js("S.wsOk") is True, "the first tab keeps working")
t1.close()
if os.environ.get("DEBUG"):
    time.sleep(5)
    print("targets:", [t["url"] for t in json.loads(urllib.request.urlopen(f"http://127.0.0.1:{CDP}/json/list").read())])
    print("server after close:", web.tool("chats"))
check(t2.until(LOADED, 60) is not None, "once the first tab is closed, the second connects by itself")
if os.environ.get("DEBUG"):
    print(t2.js("JSON.stringify({wsOk: S.wsOk, st: S.linkState, hello: !!S.link.hello, bye: S.link.bye, tries: S.link.tries, cache: [...S.cache.keys()]})"))

# ---- written while bia is down, the tab closed: the next tab sends it once bia is back
web.stop()
check(t2.until("!S.wsOk", 20) is not None, "the page notices bia is gone")
t2.js("void sendMessage({text: 'written offline', files: []})")
check(t2.until("(S.pending.get('main') || []).some(p => p.text === 'written offline') && S.link.out.size === 1", 10) is not None, "it waits in the outbox")
time.sleep(1)
t2.close()
web = Mcp()
t3 = Tab()
n = web.inbound(lambda n: n["identity"] == "root" and n["message"] == "written offline", 60)
check(n is not None, "a new tab finds it in IndexedDB and it reaches bia once bia is back")
check(web.inbound(lambda n: n["identity"] == "root" and n["message"] == "written offline", 3) is None, "…once")
check(t3.until(LOADED, 30) is not None and t3.until("!(S.pending.get('main') || []).length && S.link.out.size === 0", 10) is not None, "the new tab is online with nothing waiting")
web.tool("send", identity="root", channel="web", text="welcome back")
t3.js("go('chat/main')")
check(t3.until("document.querySelector('#msgsIn') && document.querySelector('#msgsIn').textContent.includes('welcome back')", 15) is not None, "and gets bia's messages live")

for t in (t3,):
    t.close()
cr.terminate()
web.stop()
shutil.rmtree(root, ignore_errors=True)
print(f"\n{'FAILED: ' + str(len(failed)) if failed else 'all passed'}")
sys.exit(1 if failed else 0)
