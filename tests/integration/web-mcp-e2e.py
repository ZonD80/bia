"""End-to-end: bia-mcp-web over stdio + HTTPS with a client certificate.

Starts the MCP like the agent does (JSON-RPC on stdin/stdout, BIA_FILES in a temp dir, HTTPS on 8443),
takes the root certificate like a first visitor, then checks both directions of a web chat, the
identity rules (never bia, identity required), the removed stop action and root's console mirror.

  docker run --rm --platform linux/arm64 -v $PWD:/work -v bia-cache-arm64:/cache -w /work \
    -e CARGO_TARGET_DIR=/cache/target-arm64-test bia-build:arm64 bash -lc \
    'cargo build -q -p bia-mcp-web && python3 -I tests/integration/web-mcp-e2e.py /cache/target-arm64-test/debug/bia-mcp-web'
"""
import json, os, ssl, subprocess, sys, tempfile, threading, time, http.client, base64

exe, files = sys.argv[1], tempfile.mkdtemp(prefix="bia-web-e2e-")
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
    r = c.getresponse(); return r.status, r.read()
s, ca = req("GET", "/v1/pki/ca"); check(s == 200 and b"BEGIN CERTIFICATE" in ca, "CA download")
s, b = req("POST", "/v1/pki/bootstrap/root"); check(s == 200 and len(b) > 500, "root PKCS#12 issued")
d = tempfile.mkdtemp()
open(f"{d}/root.p12", "wb").write(b); open(f"{d}/ca.pem", "wb").write(ca)
subprocess.run(["openssl", "pkcs12", "-in", f"{d}/root.p12", "-passin", "pass:12345678", "-nodes", "-out", f"{d}/root.pem"], check=True, capture_output=True)
ctx = ssl.create_default_context(cafile=f"{d}/ca.pem"); ctx.check_hostname = False
ctx.load_cert_chain(f"{d}/root.pem")
s, b = req("GET", "/v1/whoami", ctx=ctx); check(s == 200 and json.loads(b)["identity"] == "root", "mTLS as root")
s, b = req("GET", "/md.js"); check(s == 200 and b"const BiaMd" in b, f"markdown renderer served before sign-in ({s})")

# a person writes on web -> the agent gets an inbound notification on channel web
s, b = req("POST", "/v1/chats/action", {"t": "send", "chat": "main", "text": "hello bia"}, ctx=ctx)
check(s == 200, f"browser send ({s})")
for _ in range(200):
    ib = [n for n in notes if isinstance(n, dict) and n.get("t") == "inbound"]
    if ib: break
    time.sleep(0.05)
check(bool(ib) and ib[0]["identity"] == "root" and ib[0]["channel"] == "web" and "hello bia" in ib[0]["message"], f"inbound to agent: {ib[:1]}")

# the agent answers with web_send; it lands in root's own chat
txt, err = tool("send", identity="root", channel="web", text="hi root")
check(not err and "sent to root@web" in txt, f"web_send: {txt}")
s, b = req("GET", "/v1/chats/main/messages?limit=5", ctx=ctx)
msgs = json.loads(b)["messages"]
check(any(m["text"] == "hi root" and m["from"] == "bia" for m in msgs), "reply visible in the browser chat")
check(os.path.exists(f"{files}/state/web/root/main.json") and not os.path.exists(f"{files}/state/web/bia"), "stored in web/root, nothing in web/bia")
txt, err = tool("send", identity="bia", channel="web", text="x"); check(err and "this machine itself" in txt, "identity bia refused")
txt, err = tool("send", channel="web", text="x"); check(err and "identity is required" in txt, "identity required")
txt, err = tool("read", identity="root", channel="web", message_id=ib[0]["id"]); check(not err, "read mark")

# the stop action is gone; root's console mirror (/ws/term) is back since 1.1.3
s, b = req("POST", "/v1/chats/action", {"t": "stop", "chat": "main"}, ctx=ctx)
check(s != 200 or b"unknown action" in b, f"stop action removed ({s} {b[:60]!r})")
c = http.client.HTTPSConnection("127.0.0.1", 8443, context=ctx, timeout=10)
k = base64.b64encode(os.urandom(16)).decode()
c.request("GET", "/ws/term", headers={"Upgrade": "websocket", "Connection": "Upgrade", "Sec-WebSocket-Key": k, "Sec-WebSocket-Version": "13"})
r = c.getresponse(); check(r.status == 101, f"/ws/term is root's console mirror again since 1.1.3 ({r.status})")

p.stdin.close(); p.wait(timeout=10)
print("RESULT", "FAILED" if check.failed else "PASSED")
