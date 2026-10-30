"""End-to-end: bia-mcp-web over stdio + HTTPS with a client certificate.

Starts the MCP like the agent does (JSON-RPC on stdin/stdout, BIA_FILES in a temp dir, HTTPS on 8443),
takes the root certificate like a first visitor, then drives the web bus protocol (webbus.py): both
directions of the one web chat, resent actions applied once, files both ways inside the socket, store
and forward with catch-up from rx, one live session per identity, bans, the identity rules (never
bia, identity required), the removed HTTP chat API and root's console mirror.

  docker run --rm --platform linux/arm64 -v $PWD:/work -v bia-cache-arm64:/cache -w /work \
    -e CARGO_TARGET_DIR=/cache/target-arm64-test bia-build:arm64 bash -lc \
    'cargo build -q -p bia-mcp-web && python3 -I tests/integration/web-mcp-e2e.py /cache/target-arm64-test/debug/bia-mcp-web'
"""
import json, os, ssl, subprocess, sys, tempfile, threading, time, http.client, base64, hashlib
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from webbus import Session

exe, files = sys.argv[1], tempfile.mkdtemp(prefix="bia-web-e2e-")
# files belong to user bia while the MCP runs as root: what it writes there must stay readable for bia's backups
BIA_UID = 1000 if os.geteuid() == 0 else os.geteuid()
if os.geteuid() == 0:
    os.chown(files, BIA_UID, BIA_UID)
env = dict(os.environ, BIA_FILES=files, BIA_WEB_PORT="8443", BIA_WEB_HTTP_PORT="0")
p = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, text=True, bufsize=1)
notes, results, rid = [], {}, [0]
def reader():
    for line in p.stdout:
        v = json.loads(line)
        if v.get("method") == "notifications/message":
            notes.append(v["params"]["data"])
        elif "id" in v:
            results[v["id"]] = v
threading.Thread(target=reader, daemon=True).start()
def rpc(method, params=None):
    rid[0] += 1; i = rid[0]
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n"); p.stdin.flush()
    for _ in range(300):
        if i in results: return results.pop(i)
        time.sleep(0.02)
    raise SystemExit(f"no reply to {method}")
def tool(name, **args):
    r = rpc("tools/call", {"name": name, "arguments": args})["result"]
    return r["content"][0]["text"], r.get("isError", False)
def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what); 
    if not cond: check.failed = True
check.failed = False

rpc("initialize")
names = [t["name"] for t in rpc("tools/list")["result"]["tools"]]
check({"send", "react", "history", "call_start", "pki_issue", "status"} <= set(names), f"tools listed ({len(names)})")
for _ in range(100):
    if os.path.exists(f"{files}/state/web.json"): break
    time.sleep(0.05)
st = json.load(open(f"{files}/state/web.json"))
check(st.get("port") == 8443, f"web.json published, port {st.get('port')}")
txt, err = tool("status")
check(not err and "CA fingerprint" in txt, "web_status")

# first visit: CA + root certificate (bootstrap)
noverify = ssl.create_default_context(); noverify.check_hostname = False; noverify.verify_mode = ssl.CERT_NONE
def req(method, path, body=None, ctx=noverify):
    c = http.client.HTTPSConnection("127.0.0.1", 8443, context=ctx, timeout=10)
    c.request(method, path, body=json.dumps(body) if isinstance(body, dict) else body, headers={"Content-Type": "application/json"} if body is not None else {})
    try:
        r = c.getresponse(); return r.status, r.read()
    except ConnectionResetError:
        return 0, b""  # refused during the TLS handshake (no client certificate)
s, ca = req("GET", "/v1/pki/ca"); check(s == 200 and b"BEGIN CERTIFICATE" in ca, "CA download")
s, b = req("POST", "/v1/pki/bootstrap/root"); check(s == 200 and len(b) > 500, "root PKCS#12 issued")
d = tempfile.mkdtemp()
open(f"{d}/root.p12", "wb").write(b); open(f"{d}/ca.pem", "wb").write(ca)
subprocess.run(["openssl", "pkcs12", "-in", f"{d}/root.p12", "-passin", "pass:12345678", "-nodes", "-out", f"{d}/root.pem"], check=True, capture_output=True)
ctx = ssl.create_default_context(cafile=f"{d}/ca.pem"); ctx.check_hostname = False
ctx.load_cert_chain(f"{d}/root.pem")
s, b = req("GET", "/v1/whoami", ctx=ctx); check(s == 200 and json.loads(b)["identity"] == "root", "mTLS as root")
s, b = req("GET", "/md.js"); check(s == 200 and b"const BiaMd" in b, f"markdown renderer served before sign-in ({s})")

def inbound(pred, secs=10):
    end = time.time() + secs
    while time.time() < end:
        for n in list(notes):
            if isinstance(n, dict) and n.get("t") == "inbound" and pred(n):
                notes.remove(n)
                return n
        time.sleep(0.05)
    return None

# the web bus: hello, then a person writes -> the agent gets an inbound on channel web
s1 = Session(8443, ctx, rx=None, instance="tab-1")
check(s1.hello is not None and s1.hello.get("proto") == 2 and s1.hello["me"]["identity"] == "root" and s1.hello["me"]["root"] is True, f"hello (protocol 2): {s1.hello}")
n = inbound(lambda n: n["identity"] == "system" and n["message"].startswith("[web open] root connected"))
check(n is not None, "the model hears root connected")
a = s1.act("send", id="w-hello-1", text="hello bia", cid="c-1")
check(a and a["ok"] and a["r"]["id"] == "w-hello-1", f"send acked with the client's id: {a}")
ib = inbound(lambda n: n["identity"] == "root")
check(ib is not None and ib["channel"] == "web" and ib["message"] == "hello bia" and ib["id"] == "w-hello-1", f"inbound to agent: {ib}")
check(s1.act("send", id="w-hello-1", text="hello bia", cid="c-1")["r"].get("dup") is True, "the same action again is applied once (cid)")
s1.act("send", id="w-hello-1", text="hello bia", cid="c-1b")
check(inbound(lambda n: n["identity"] == "root", 1.5) is None, "the same message id again reaches the model once")
f = s1.msg("hello bia")
check(f is not None and f["seq"] > 0 and f["m"]["from"] == "root", "the client sees its own message with a revision")

# the agent answers with web_send; it arrives as an m frame
txt, err = tool("send", identity="root", channel="web", text="hi root")
check(not err and "sent to root@web" in txt and "waits" not in txt, f"web_send: {txt}")
f = s1.msg("hi root")
check(f is not None and f["m"]["from"] == "bia", "reply arrives live")
check(os.path.exists(f"{files}/state/web/root/main.json") and not os.path.exists(f"{files}/state/web/bia"), "stored in web/root, nothing in web/bia")
txt, err = tool("send", identity="bia", channel="web", text="x"); check(err and "this machine itself" in txt, "identity bia refused")
txt, err = tool("send", channel="web", text="x"); check(err and "identity is required" in txt, "identity required")
txt, err = tool("read", identity="root", channel="web", message_id="w-hello-1"); check(not err, "read mark")
f = s1.wait(lambda f: f.get("t") == "m" and f["m"]["id"] == "w-hello-1" and f["m"].get("read"))
check(f is not None, "the read mark reaches the client")

# a file from the client: listed in send, pulled by the server over the socket
data = os.urandom(300_000) + b"end"
a = s1.act("send", id="w-file-1", text="a file", files=[s1.offer(data, "../../etc/notes.bin")])
check(a and a["ok"], f"send with a file: {a}")
ib = inbound(lambda n: n["identity"] == "root" and "a file" in n["message"], 20)
got = ib and [l[2:].split(" (")[0] for l in ib["message"].splitlines() if l.startswith("- ")]
check(bool(got) and open(f"{files}/{got[0]}", "rb").read() == data and got[0].startswith("inbox/web/root/notes.bin"), f"file pulled into the inbox, intact: {got}")
f = s1.wait(lambda f: f.get("t") == "m" and f["m"]["id"] == "w-file-1" and f["m"]["files"][0].get("path"))
check(f is not None and not f["m"]["files"][0].get("pending"), "the message shows the file arrived")

# a file from bia: listed by fid, pulled by the client
src = f"{files}/out.pdf"; blob = os.urandom(200_000); open(src, "wb").write(blob)
txt, err = tool("send", identity="root", channel="web", text="here it is", files=[src])
f = s1.msg("here it is")
att = f and f["m"]["files"][0]
check(att is not None and att["fid"] == hashlib.sha256(blob).hexdigest() and att["size"] == len(blob), f"bia's file listed with its sha256: {att}")
check(att is not None and s1.pull(att["fid"], att["size"]) == blob, "pulled by the client over the socket, intact")

# history and search over the socket
a = s1.act("hist", limit=3)
check(a and a["ok"] and [m["text"] for m in a["r"]["messages"]][-1] == "here it is" and a["r"]["more_before"], f"hist: {a and [m['text'] for m in a['r']['messages']]}")
a = s1.act("search", q="hello")
check(a and a["ok"] and any(r["m"]["id"] == "w-hello-1" for r in a["r"]["results"]), "search")

# store and forward: what bia does while the client is away arrives on reconnect
rx = max(f2["seq"] for f2 in s1.frames if f2.get("seq"))
s1.close()
time.sleep(0.5)
txt, err = tool("send", identity="root", channel="web", text="while you were away")
check(not err and "waits there" in txt, f"send while away: {txt}")
mid = txt.split("message id ")[1].split(")")[0]
tool("edit", identity="root", channel="web", message_id=mid, text="while you were away (edited)")
tool("delete_messages", identity="root", channel="web", message_ids=["w-file-1"])
s2 = Session(8443, ctx, rx=rx, instance="tab-1")
f = s2.msg("while you were away (edited)")
check(f is not None, "catch-up brings what was said while away, edited")
f = s2.wait(lambda f: f.get("t") == "gone" and "w-file-1" in f["ids"])
check(f is not None, "catch-up brings the deletion")

# one live session per identity: a second tab is refused while the first answers
s3 = Session(8443, ctx, rx=None, instance="tab-2")
check(s3.bye == "duplicate_key" and s3.hello is None, f"a second live session is refused: bye {s3.bye}")
a = s2.act("send", text="still here")
check(a and a["ok"], "the first session keeps working")
# a reload of the same tab replaces its own stale session
s4 = Session(8443, ctx, rx=None, instance="tab-1")
check(s4.hello is not None, "a reload (same instance) takes over")
check(s2.wait(lambda f: False, 3) is None and s2.bye == "duplicate", f"the old one is told: {s2.bye}")

# the HTTP chat API and the old peer link are gone
s, b = req("POST", "/v1/chats/action", {"t": "send", "text": "x"}, ctx=ctx)
check(s == 404, f"POST /v1/chats/action is gone ({s})")
c = http.client.HTTPSConnection("127.0.0.1", 8443, context=ctx, timeout=10)
c.request("GET", "/ws/peer", headers={"Upgrade": "websocket", "Connection": "Upgrade", "Sec-WebSocket-Key": base64.b64encode(os.urandom(16)).decode(), "Sec-WebSocket-Version": "13"})
r = c.getresponse(); check(r.status == 410, f"/ws/peer answers 410 ({r.status})")
c = http.client.HTTPSConnection("127.0.0.1", 8443, context=ctx, timeout=10)
k = base64.b64encode(os.urandom(16)).decode()
c.request("GET", "/ws/term", headers={"Upgrade": "websocket", "Connection": "Upgrade", "Sec-WebSocket-Key": k, "Sec-WebSocket-Version": "13"})
r = c.getresponse(); check(r.status == 101, f"/ws/term is root's console mirror ({r.status})")

# Web Push: worker + manifest before sign-in, VAPID key, subscriptions (no real push is sent)
for path, what in (("/sw.js", b"pushsubscriptionchange"), ("/manifest.webmanifest", b'"standalone"'), ("/icon-192.png", b"\x89PNG")):
    s, b = req("GET", path); check(s == 200 and what in b, f"{path} served before sign-in ({s})")
s, b = req("GET", "/v1/push/key", ctx=ctx)
key = base64.urlsafe_b64decode(json.loads(b)["key"] + "==") if s == 200 else b""
check(len(key) == 65 and key[0] == 4, f"VAPID key is an uncompressed P-256 point ({s}, {len(key)} bytes)")
for key in ("ca.key", "server.key"):
    st_p = os.stat(f"{files}/state/pki/{key}")
    check(oct(st_p.st_mode & 0o777) == "0o600" and st_p.st_uid == BIA_UID, f"pki/{key} is 0600 and belongs to the files owner ({oct(st_p.st_mode & 0o777)}, uid {st_p.st_uid})")
st_k = os.stat(f"{files}/state/web/push/vapid.pk8")
check(oct(st_k.st_mode & 0o777) == "0o600" and st_k.st_uid == BIA_UID, f"VAPID private key is 0600 and belongs to the files owner ({oct(st_k.st_mode & 0o777)}, uid {st_k.st_uid})")
subprocess.run(["openssl", "ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", f"{d}/ua.pem"], check=True, capture_output=True)
der = subprocess.run(["openssl", "ec", "-in", f"{d}/ua.pem", "-pubout", "-outform", "DER"], check=True, capture_output=True).stdout
b64u = lambda x: base64.urlsafe_b64encode(x).rstrip(b"=").decode()
keys = {"p256dh": b64u(der[-65:]), "auth": b64u(os.urandom(16))}
ep = "https://fcm.googleapis.com/fcm/send/bia-e2e"
for bad in ("http://fcm.googleapis.com/x", "https://192.168.1.10/x", "https://localhost/x"):
    s, b = req("POST", "/v1/push/subscribe", {"endpoint": bad, "keys": keys}, ctx=ctx); check(s == 400, f"subscribe refuses {bad} ({s})")
s, b = req("POST", "/v1/push/subscribe", {"endpoint": ep, "keys": {"p256dh": keys["p256dh"], "auth": "short"}}, ctx=ctx); check(s == 400, f"subscribe refuses a bad auth secret ({s})")
s, b = req("POST", "/v1/push/subscribe", {"endpoint": ep, "keys": keys}, ctx=ctx); check(s == 200, f"subscribe ({s} {b[:80]!r})")
subs = json.load(open(f"{files}/state/web/push/subs.json"))
check([(x["identity"], x["endpoint"]) for x in subs] == [("root", ep)], f"subscription stored for root: {subs}")
st_s = os.stat(f"{files}/state/web/push/subs.json")
check(oct(st_s.st_mode & 0o777) == "0o600" and st_s.st_uid == BIA_UID, f"subs.json is 0600 and belongs to the files owner ({oct(st_s.st_mode & 0o777)}, uid {st_s.st_uid})")
s, b = req("POST", "/v1/push/subscribe", {"endpoint": ep, "keys": keys}); check(s in (0, 401, 403), f"subscribe needs a client cert ({s})")
def req_ua(method, path, body, ua):
    c = http.client.HTTPSConnection("127.0.0.1", 8443, context=ctx, timeout=10)
    c.request(method, path, body=json.dumps(body), headers={"Content-Type": "application/json", "User-Agent": ua})
    r = c.getresponse(); return r.status, r.read()
ep2 = "https://web.push.apple.com/bia-e2e"
s, b = req_ua("POST", "/v1/push/subscribe", {"endpoint": ep2, "keys": keys, "expirationTime": 1000}, "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1")
id2 = json.loads(b).get("id", "") if s == 200 else ""
check(s == 200 and len(id2) == 12, f"second browser subscribed ({s} {b!r})")
s, b = req("GET", "/v1/push/subs", ctx=ctx); rows = json.loads(b)["subs"] if s == 200 else []
row2 = next((r for r in rows if r["id"] == id2), {})
check(len(rows) == 2 and row2.get("device") == "Safari on iPhone" and row2.get("service") == "Apple" and row2.get("expired"), f"both browsers listed, iPhone one expired: {rows}")
s, b = req("POST", "/v1/push/remove", {"ids": [id2]}, ctx=ctx)
check(s == 200 and json.loads(b)["removed"] == 1, f"remove a browser by id ({s} {b!r})")
s, b = req("GET", "/v1/push/subs", ctx=ctx); check([r["service"] for r in json.loads(b)["subs"]] == ["Google (FCM)"], f"one browser left: {b[:200]!r}")
s, b = req("POST", "/v1/push/unsubscribe", {"endpoint": ep}, ctx=ctx)
check(s == 200 and json.loads(b)["removed"] and json.load(open(f"{files}/state/web/push/subs.json")) == [], f"unsubscribe ({s} {b!r})")

# root options go by trust: another identity with trust root gets them, a normal one does not
def ctx_for(identity, trust):
    s, b = req("POST", "/v1/pki/issue", {"identity": identity, "role": "user", "trust": trust}, ctx=ctx)
    path = json.loads(b)["path"]
    s, p12 = req("GET", path)
    open(f"{d}/{identity}.p12", "wb").write(p12)
    subprocess.run(["openssl", "pkcs12", "-in", f"{d}/{identity}.p12", "-passin", "pass:12345678", "-nodes", "-out", f"{d}/{identity}.pem"], check=True, capture_output=True)
    c = ssl.create_default_context(cafile=f"{d}/ca.pem"); c.check_hostname = False
    c.load_cert_chain(f"{d}/{identity}.pem")
    return c
alice, bob = ctx_for("alice", "root"), ctx_for("bob", "normal")
s, b = req("GET", "/v1/whoami", ctx=alice); check(s == 200 and json.loads(b).get("root") is True, f"alice (trust root) is root in the web app: {b[:120]!r}")
s, b = req("GET", "/v1/whoami", ctx=bob); check(s == 200 and json.loads(b).get("root") is False, f"bob (trust normal) is not: {b[:120]!r}")
for path in ("/v1/status", "/v1/pki/certs", "/v1/pki/requests"):
    sa, _ = req("GET", path, ctx=alice); sb, _ = req("GET", path, ctx=bob)
    check(sa == 200 and sb == 403, f"{path}: alice {sa}, bob {sb}")
for who, c, want in (("alice", alice, 101), ("bob", bob, 403)):
    cn = http.client.HTTPSConnection("127.0.0.1", 8443, context=c, timeout=10)
    cn.request("GET", "/ws/shell", headers={"Upgrade": "websocket", "Connection": "Upgrade", "Sec-WebSocket-Key": base64.b64encode(os.urandom(16)).decode(), "Sec-WebSocket-Version": "13"})
    r = cn.getresponse(); check(r.status == want, f"/ws/shell for {who}: {r.status}")
    cn.close()

# a ban refuses an identity's sessions until lifted
sb = Session(8443, bob, rx=None)
check(sb.hello is not None and sb.hello["me"]["identity"] == "bob", "bob has a session of his own")
txt, err = tool("peer_ban", peer="bob")
check(not err and "banned bob" in txt, f"ban: {txt}")
check(sb.wait(lambda f: False, 3) is None and sb.bye == "banned", f"bob's session is closed: {sb.bye}")
sb2 = Session(8443, bob, rx=None)
check(sb2.bye == "banned", f"bob is refused while banned: {sb2.bye}")
txt, err = tool("peer_unban", peer="bob")
check(not err and "unbanned bob" in txt, f"unban: {txt}")
sb3 = Session(8443, bob, rx=None)
check(sb3.hello is not None, "bob is back after the unban")
a = sb3.act("send", text="hi, bob here")
ib = inbound(lambda n: n["identity"] == "bob")
check(ib is not None and ib["message"] == "hi, bob here", f"bob's message reaches the model: {ib}")
check(os.path.exists(f"{files}/state/web/bob/main.json") and "hi, bob here" not in open(f"{files}/state/web/root/main.json").read(), "each identity has its own chat")

p.stdin.close(); p.wait(timeout=10)
print("RESULT", "FAILED" if check.failed else "PASSED")
