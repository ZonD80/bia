"""End-to-end without a Meta account: bia-mcp-messenger / bia-mcp-instagram over stdio with
the real bia-meta behind them.

Starts each MCP like the agent does (JSON-RPC on stdin/stdout, BIA_FILES in a temp dir),
with a fake Chrome DevTools endpoint (BIA_CHROME_DEBUG_URL) whose cookies hold no Facebook or
Instagram session, and checks the tools that work logged out, the helper round trip, and
that login explains what to do when Chrome is not signed in or not running.

  docker run --rm --platform linux/arm64 -v $PWD:/work -v bia-cache-arm64:/cache -w /work \\
    -e CARGO_TARGET_DIR=/cache/target-arm64-test bia-build:arm64 bash -lc \\
    'cargo build -q -p bia-mcp-meta && python3 -I tests/integration/meta-mcp-e2e.py $CARGO_TARGET_DIR/debug dist/arm64/bia-meta'
"""
import base64, hashlib, http.server, json, os, socket, socketserver, subprocess, sys, tempfile, threading, time

bins, meta = sys.argv[1], os.path.abspath(sys.argv[2])

def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what)
    if not cond:
        check.failed = True
check.failed = False

# --- a fake Chrome: /json/version + one websocket answering Storage.getCookies ---
COOKIES = [{"name": "datr", "value": "x", "domain": ".facebook.com"}, {"name": "csrftoken", "value": "y", "domain": ".instagram.com"}]

class Chrome(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass
    def do_GET(self):
        if self.path == "/json/version":
            body = json.dumps({"webSocketDebuggerUrl": f"ws://127.0.0.1:{PORT}/devtools/browser/x"}).encode()
            self.send_response(200); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
            return
        key = self.headers.get("Sec-WebSocket-Key", "")
        accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        self.send_response(101); self.send_header("Upgrade", "websocket"); self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept); self.end_headers(); self.wfile.flush()
        hdr = self.rfile.read(2)
        n = hdr[1] & 0x7F
        if n == 126:
            n = int.from_bytes(self.rfile.read(2), "big")
        mask = self.rfile.read(4)
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(self.rfile.read(n)))
        req = json.loads(payload)
        out = json.dumps({"id": req["id"], "result": {"cookies": COOKIES}}).encode()
        frame = bytes([0x81]) + (bytes([len(out)]) if len(out) < 126 else bytes([126]) + len(out).to_bytes(2, "big")) + out
        self.wfile.write(frame); self.wfile.flush()
        time.sleep(0.2)

srv = socketserver.ThreadingTCPServer(("127.0.0.1", 0), Chrome)
PORT = srv.server_address[1]
threading.Thread(target=srv.serve_forever, daemon=True).start()

def run_mcp(platform):
    files = tempfile.mkdtemp(prefix=f"bia-{platform}-e2e-")
    env = dict(os.environ, BIA_FILES=files, BIA_META_BIN=meta, BIA_CHROME_DEBUG_URL=f"http://127.0.0.1:{PORT}")
    p = subprocess.Popen([os.path.join(bins, f"bia-mcp-{platform}")], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, env=env, text=True, bufsize=1)
    results, rid = {}, [0]
    def reader():
        for line in p.stdout:
            v = json.loads(line)
            if "id" in v:
                results[v["id"]] = v
    threading.Thread(target=reader, daemon=True).start()
    def rpc(method, params=None):
        rid[0] += 1; i = rid[0]
        p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n"); p.stdin.flush()
        for _ in range(3000):
            if i in results:
                return results.pop(i)
            time.sleep(0.02)
        raise SystemExit(f"no reply to {method}")
    def tool(name, **args):
        r = rpc("tools/call", {"name": name, "arguments": args})["result"]
        return r["content"][0]["text"], r.get("isError", False)
    return p, files, rpc, tool

for platform in ("messenger", "instagram"):
    p, files, rpc, tool = run_mcp(platform)
    rpc("initialize")
    names = {t["name"] for t in rpc("tools/list")["result"]["tools"]}
    want = {"login", "status", "restore", "send", "send_voice", "edit", "react", "delete_messages", "history", "chats",
            "search", "resolve", "typing", "group_info", "set_admin", "set_group_info", "create_group", "add_members", "remove_member", "leave_group",
            "ban", "unban", "block", "accept_request", "delete_chat", "mute"}
    check(want <= names, f"{platform}: tools listed ({len(names)})")
    check(("pin" in names) == (platform == "instagram"), f"{platform}: pin only on instagram")
    check(("set_profile" in names) == (platform == "messenger"), f"{platform}: set_profile (bio) only on messenger")
    m_only = {"group_permissions", "join_requests", "answer_join_request"}
    check((m_only <= names) == (platform == "messenger") and not (m_only & names and platform == "instagram"), f"{platform}: group settings and join requests only on messenger")
    check(not ({"members", "rename_group", "set_group_photo"} & names), f"{platform}: old group tools replaced by group_info / set_group_info")
    txt, err = tool("status")
    check(not err and txt.startswith("auth=logged_out"), f"{platform}: status logged out ({txt})")
    txt, err = tool("send", chat_id="123", text="hi")
    check(err and "not logged in" in txt, f"{platform}: send refused before login ({txt})")
    txt, err = tool("ban", chat_id=f"{platform}:555")
    check(not err and txt.startswith("banned"), f"{platform}: ban works logged out ({txt})")
    bans = json.load(open(f"{files}/state/{platform}-bans.json"))
    check(bans.get("chats") == ["555"], f"{platform}: ban saved ({bans})")
    txt, err = tool("unban", chat_id="555")
    check(not err and txt.startswith("unbanned"), f"{platform}: unban")
    txt, err = tool("typing", chat_id="1")
    check(not err and txt == "", f"{platform}: typing is silent when logged out")
    # login: Chrome answers, but it holds no session for the site
    txt, err = tool("login")
    site = "Facebook" if platform == "messenger" else "Instagram"
    check(err and f"Chrome is not signed in to {site}" in txt, f"{platform}: login explains Chrome has no session ({txt[:90]})")
    check(not json.load(open(f"{files}/state/{platform}.json")).get("logged_in", False) if os.path.exists(f"{files}/state/{platform}.json") else True,
          f"{platform}: failed login leaves it logged out")
    # the helper round trip: a stale "logged in" flag, restore asks bia-meta and clears it
    os.makedirs(f"{files}/state", exist_ok=True)
    json.dump({"logged_in": True, "id": "1", "name": "Old"}, open(f"{files}/state/{platform}.json", "w"))
    txt, err = tool("status")
    check(not err and "auth=logged_out" in txt, f"{platform}: status through bia-meta ({txt})")
    txt, err = tool("restore")
    check(not err and "session ended" in txt, f"{platform}: restore without a session ({txt})")
    check(not json.load(open(f"{files}/state/{platform}.json"))["logged_in"], f"{platform}: restore cleared the stale flag")
    check(os.path.isdir(f"{files}/{platform}") and os.path.exists(f"{files}/{platform}/meta.db"), f"{platform}: bia-meta store created")
    mode = os.stat(f"{files}/{platform}/meta.db").st_mode & 0o777
    check(mode == 0o600, f"{platform}: meta.db is private ({oct(mode)})")
    # bia thinks it is logged in but bia-meta has no session: the error says how to log in
    json.dump({"logged_in": True, "id": "1"}, open(f"{files}/state/{platform}.json", "w"))
    txt, err = tool("chats")
    site = "facebook.com" if platform == "messenger" else "instagram.com"
    check(err and f"{platform}_login" in txt and site in txt, f"{platform}: helper's not-logged-in tells what to do ({txt[:100]})")
    txt, err = tool("history", chat_id="123")
    check(not err and "(no messages)" in txt and f"{platform}_login" in txt, f"{platform}: history explains the missing server part ({txt[:120]})")
    json.dump({"logged_in": False}, open(f"{files}/state/{platform}.json", "w"))
    # pasted cookies without a session cookie
    txt, err = tool("login", cookies="foo=bar")
    check(err and "missing cookies" in txt, f"{platform}: pasted cookies are checked ({txt[:80]})")
    p.stdin.close(); p.wait(timeout=10)

# Chrome not running at all (the fake one is gone; its port is closed)
srv.shutdown()
srv.server_close()
p, files, rpc, tool = run_mcp("messenger")
rpc("initialize")
txt, err = tool("login")
check(err and ("Chrome is not running" in txt or "not signed in" in txt), f"login without Chrome ({txt[:90]})")
p.stdin.close(); p.wait(timeout=10)

print("RESULT", "FAILED" if check.failed else "PASSED")
sys.exit(1 if check.failed else 0)
