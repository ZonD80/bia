"""End-to-end: bia-mcp-web's plain-HTTP port (port 80 in production; a high port here).

Another program holds the port at start: the web app still starts on HTTPS and bia gets a [web] notice.
Once the port is free, web_http(retry=true) takes it: http:// redirects to bia's own https address,
ACME challenge paths are never redirected, and web_http(redirect=…) sends browsers elsewhere (with or
without the path) and back.

  docker run --rm --platform linux/arm64 -v $PWD:/work -v bia-cache-arm64:/cache -w /work \
    -e CARGO_TARGET_DIR=/cache/target-arm64-test bia-build:arm64 bash -lc \
    'cargo build -q -p bia-mcp-web && python3 -I tests/integration/web-http80-e2e.py /cache/target-arm64-test/debug/bia-mcp-web'
"""
import http.client, json, os, socket, subprocess, sys, tempfile, threading, time

exe, files = sys.argv[1], tempfile.mkdtemp(prefix="bia-http80-e2e-")
HTTPS, HTTP = 8443, 18080

blocker = socket.socket()
blocker.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
blocker.bind(("0.0.0.0", HTTP))
blocker.listen(1)

env = dict(os.environ, BIA_FILES=files, BIA_WEB_PORT=str(HTTPS), BIA_WEB_HTTP_PORT=str(HTTP))
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
    rid[0] += 1
    i = rid[0]
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n")
    p.stdin.flush()
    for _ in range(300):
        if i in results:
            return results.pop(i)
        time.sleep(0.02)
    raise SystemExit(f"no reply to {method}")


def tool(name, **args):
    r = rpc("tools/call", {"name": name, "arguments": args})["result"]
    return r["content"][0]["text"], r.get("isError", False)


failed = []


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what)
    if not cond:
        failed.append(what)


def inbound(pred, secs=10):
    end = time.time() + secs
    while time.time() < end:
        for n in list(notes):
            if isinstance(n, dict) and n.get("t") == "inbound" and pred(n):
                notes.remove(n)
                return n
        time.sleep(0.05)
    return None


def get(path):
    c = http.client.HTTPConnection("127.0.0.1", HTTP, timeout=5)
    c.request("GET", path, headers={"Host": "10.0.0.5"})
    r = c.getresponse()
    r.read()
    return r.status, r.getheader("Location")


try:
    rpc("initialize")
    for _ in range(100):
        if os.path.exists(f"{files}/state/web.json"):
            break
        time.sleep(0.05)
    st = json.load(open(f"{files}/state/web.json"))
    check(st.get("port") == HTTPS, f"HTTPS runs although the plain-HTTP port is taken (port {st.get('port')})")
    n = inbound(lambda n: n["identity"] == "system" and n["message"].startswith("[web] Plain HTTP"))
    check(n is not None and f"port {HTTP} could not be taken" in n["message"] and "web_http(retry=true)" in n["message"],
          f"bia is told the port is taken: {n and n['message'][:160]}")
    txt, err = tool("status")
    check(not err and "NOT held" in txt and "pki/ca" not in txt.split("\n")[0], "web_status says the port is not held; no CA in the web line")
    txt, err = tool("http", retry=True)
    check("Still not" in txt, "retry while still taken")

    blocker.close()
    txt, err = tool("http", retry=True)
    check(not err and "is held now" in txt and "bia's own" in txt, f"retry takes the port: {txt}")
    check(get("/anything") == (301, f"https://10.0.0.5:{HTTPS}/"), f"http:// → bia's own https address: {get('/anything')}")
    check(get("/.well-known/acme-challenge/nope")[0] == 404, "ACME challenge paths are answered, not redirected")

    txt, err = tool("http", redirect="https://example.com/")
    check(not err and "example.com" in txt, "redirect saved")
    check(get("/blog?a=1") == (301, "https://example.com/blog?a=1"), "redirect keeps the path")
    check(get("/.well-known/acme-challenge/nope")[0] == 404, "ACME still answered with a redirect elsewhere")
    tool("http", redirect="http://127.0.0.1:3000/app", keep_path=False)
    check(get("/blog") == (301, "http://127.0.0.1:3000/app"), "redirect without the path")
    check(json.load(open(f"{files}/state/web/http.json"))["redirect"] == "http://127.0.0.1:3000/app", "kept in state/web/http.json")
    txt, err = tool("http", redirect="not a url")
    check(err, "a bad URL is refused")
    tool("http", redirect="")
    check(get("/x") == (301, f"https://10.0.0.5:{HTTPS}/") and not os.path.exists(f"{files}/state/web/http.json"),
          "redirect=\"\" back to bia's own address")
finally:
    p.stdin.close()
    p.wait(timeout=10)

print("FAILED: " + "; ".join(failed) if failed else "all ok")
sys.exit(1 if failed else 0)
