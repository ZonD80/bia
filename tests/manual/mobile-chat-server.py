"""A real bia-mcp-web for trying the mobile apps' chat, without a deployed bia. Runs in the builder:

  docker run --rm -p 8443:8443 -v $PWD:/work:ro -v /tmp/chat:/out -v bia-cache-arm64:/cache bia-build:arm64 \
    python3 -I /work/tests/manual/mobile-chat-server.py /cache/target-arm64-test/debug/bia-mcp-web /out

Writes /out/bia-root.p12 and /out/ca.crt, as the apps keep them per bia. Copy them to the app's
<app dir>/servers/localhost:8443/ and open it (debug builds: launch arguments `-chat localhost:8443`
and optionally `-chatjs <script>` on iOS, intent extra chat on Android, after adb reverse tcp:8443 tcp:8443). Prints what the model would hear, so "[web open] root connected" proves the websocket came
up with the certificate. A line in /out/say is sent to root. JSON in /out/make-biaconfig calls
pki_biaconfig with those arguments and prints what it says.
"""
import json, os, shutil, ssl, subprocess, sys, tempfile, threading, time, urllib.request

exe, out = sys.argv[1], sys.argv[2]
files = tempfile.mkdtemp(prefix="bia-chat-")
os.chmod(files, 0o755)
env = dict(os.environ, BIA_FILES=files, BIA_WEB_PORT="8443", BIA_WEB_HTTP_PORT="0")
p = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=sys.stderr, env=env, text=True, bufsize=1)
rid, results = [0], {}


def reader():
    for line in p.stdout:
        v = json.loads(line)
        d = (v.get("params") or {}).get("data")
        if isinstance(d, dict) and d.get("t") == "inbound":
            print("HEARD", d.get("identity"), d.get("channel"), repr(d.get("message"))[:300], flush=True)
        elif "id" in v:
            results[v["id"]] = v


threading.Thread(target=reader, daemon=True).start()


def rpc(method, params=None):
    rid[0] += 1
    i = rid[0]
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n")
    p.stdin.flush()
    for _ in range(3000):
        if i in results:
            return results.pop(i)
        time.sleep(0.02)
    raise SystemExit(f"no reply to {method}")


rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "chat", "version": "0"}})
ctx = ssl.create_default_context()
ctx.check_hostname = False
ctx.verify_mode = ssl.CERT_NONE


def get(path, method="GET"):
    return urllib.request.urlopen(urllib.request.Request(f"https://127.0.0.1:8443{path}", method=method), context=ctx, timeout=10).read()


for _ in range(100):
    try:
        ca = get("/v1/pki/ca")
        break
    except OSError:
        time.sleep(0.2)
open(f"{out}/ca.crt", "wb").write(ca)
open(f"{out}/bia-root.p12", "wb").write(get("/v1/pki/bootstrap/root", "POST"))
print("READY", flush=True)
while True:
    time.sleep(0.5)
    make = f"{out}/make-biaconfig"
    if os.path.exists(make):
        args = json.load(open(make))
        os.remove(make)
        made = rpc("tools/call", {"name": "pki_biaconfig", "arguments": args})["result"]
        text = made["content"][0]["text"]
        print("BIACONFIG", json.dumps({"error": made.get("isError", False), "text": text}), flush=True)
        for word in text.split():
            if word.rstrip(",").endswith(".biaconfig"):
                shutil.copy(word.rstrip(","), f"{out}/made.biaconfig")
    say = f"{out}/say"
    if os.path.exists(say):
        text = open(say).read().strip()
        os.remove(say)
        print("SENT", rpc("tools/call", {"name": "send", "arguments": {"identity": "root", "channel": "web", "text": text}})["result"]["content"][0]["text"][:200], flush=True)
