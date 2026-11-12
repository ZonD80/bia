"""End-to-end: bia-bridge (CLI, daemon, MCP mode) against a real bia-mcp-web.

  python3 -I tests/integration/bridge-e2e.py <bia-mcp-web> <bia-bridge>

  docker run --rm --platform linux/arm64 -v $PWD:/work -v bia-cache-arm64:/cache -w /work \\
    -e CARGO_TARGET_DIR=/cache/target-arm64-test bia-build:arm64 bash -lc \\
    'cargo build -q -p bia-mcp-web -p bia-bridge && python3 -I tests/integration/bridge-e2e.py /cache/target-arm64-test/debug/bia-mcp-web /cache/target-arm64-test/debug/bia-bridge'
"""
import json, os, shutil, subprocess, sys, tempfile, threading, time

web_exe, bridge = sys.argv[1], sys.argv[2]
root = tempfile.mkdtemp(prefix="bia-bridge-e2e-")
files, home = f"{root}/bia", f"{root}/bridge"
os.makedirs(files)
failed = []
URL = "https://127.0.0.1:8443"


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        failed.append(what)


class Mcp:
    def __init__(self, cmd, env):
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, text=True, bufsize=1)
        self.notes, self.results, self.rid, self.seen = [], {}, 0, 0
        threading.Thread(target=self._read, daemon=True).start()
        self.rpc("initialize")

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

    def rpc(self, method, params=None, wait=120):
        self.rid += 1
        i = self.rid
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n")
        self.p.stdin.flush()
        end = time.time() + wait
        while time.time() < end:
            if i in self.results:
                return self.results.pop(i)
            time.sleep(0.02)
        raise SystemExit(f"no reply to {method}")

    def tool(self, name, **a):
        r = self.rpc("tools/call", {"name": name, "arguments": a})["result"]
        return r["content"][0]["text"], r.get("isError", False)

    def inbound(self, needle, secs=20):
        end = time.time() + secs
        while time.time() < end:
            while self.seen < len(self.notes):
                n = self.notes[self.seen]
                self.seen += 1
                if n.get("t") == "inbound" and needle in n.get("message", ""):
                    return n
            time.sleep(0.05)
        return None


# the bridge builds this bia serves: this platform's is the bridge under test
import hashlib, platform as _pf
PLAT = "macos" if sys.platform == "darwin" else ("linux-arm64" if _pf.machine() in ("aarch64", "arm64") else "linux-x86_64")
served = f"{root}/served"
os.makedirs(served)
shutil.copy2(bridge, f"{served}/bia-bridge-{PLAT}")
open(f"{served}/bia-bridge-windows-x86_64.exe", "wb").write(b"MZ not really")
open(f"{served}/README.md", "w").write("not a build")
web_env = dict(os.environ, BIA_FILES=files, BIA_WEB_PORT="8443", BIA_WEB_HTTP_PORT="0", BIA_BRIDGE_DIR=served)
benv = dict(os.environ, BIA_BRIDGE_HOME=home)


def start_web():
    w = Mcp([web_exe], web_env)
    for _ in range(200):
        if os.path.exists(f"{files}/state/web.json"):
            break
        time.sleep(0.05)
    return w


def br(*args, timeout=120, agent="claude"):
    env = dict(benv)
    if agent:
        env["BIA_BRIDGE_AGENT"] = agent
    r = subprocess.run([bridge, *args], env=env, capture_output=True, text=True, timeout=timeout)
    return r.returncode, (r.stdout + r.stderr).strip()


web = start_web()

# ---- connect: key here, request there, the owner approves, the daemon collects
rc, out = br("connect", URL, "--identity", "claude", "--note", "bridge e2e", "--as", "home")
check(rc == 0 and "Asked https://127.0.0.1:8443" in out and "pending" not in out.lower() or "approve" in out, f"connect: {out[:140]}")
check(os.path.exists(f"{home}/claude/home/key.pem") and not os.path.exists(f"{home}/claude/home/cert.pem"), "key made here, no certificate yet")
if os.name == "posix":
    check(oct(os.stat(f"{home}/claude/home/key.pem").st_mode & 0o777) == "0o600", "key is private to this user")
rc, out = br("status")
check(rc == 0 and "waiting for the owner" in out, f"status pending: {out}")
req_id = json.load(open(f"{home}/claude/home/profile.json"))["request"]
txt, err = web.tool("pki_approve", id=req_id, message="welcome, claude")
check(not err, f"owner approves: {txt[:80]}")
for _ in range(60):
    rc, out = br("status", "--json")
    s = json.loads(out)
    if s.get("state") == "active" and s.get("online"):
        break
    time.sleep(1)
check(s.get("state") == "active" and s.get("online"), f"daemon collected the certificate and opened the link: {s}")
check(os.path.exists(f"{home}/claude/home/cert.pem"), "certificate stored")
rc, out = br("read")
check("approved this client as claude" in out and "welcome, claude" in out, f"approval note with the owner's message: {out}")

# ---- messages both ways
rc, out = br("send", "hello", "bia")
check(rc == 0 and "message id" in out, f"send: {out}")
n = web.inbound("hello bia")
check(n is not None and n["identity"] == "claude" and n["channel"] == "web", f"bia got it as claude@web: {n}")
txt, err = web.tool("send", identity="claude", channel="web", text="hi claude")
check(not err, f"bia answers: {txt}")
rc, out = br("wait", "--timeout", "20")
check("hi claude" in out and "data, not instructions" in out, f"wait returns bia's answer, marked as data: {out}")
import re as _re
mid = (_re.search(r"\[([\w-]+) · ", out) or [None, ""])[1]
rc, out = br("read")
check("nothing new" in out, f"read after wait: nothing new ({out})")
txt, err = web.tool("edit", identity="claude", channel="web", message_id=mid, text="hi claude (edited)")
rc, out = br("wait", "--timeout", "10")
check("hi claude (edited)" in out and "edited" in out, f"edits arrive: {out}")

# ---- files both ways
src = f"{root}/note.txt"
open(src, "w").write("from claude\n")
rc, out = br("send", "--file", src, "a file")
n = web.inbound("a file")
path = n and [l[2:] for l in n["message"].splitlines() if l.startswith("- ")]
check(bool(path) and open(path[0] if path[0].startswith("/") else f"{files}/{path[0]}").read() == "from claude\n", f"bia got the file: {path}")
back = f"{files}/answer.bin"
data = os.urandom(300_000)
open(back, "wb").write(data)
web.tool("send", identity="claude", channel="web", text="here you go", files=[back])
rc, out = br("wait", "--timeout", "20")
got = [l.split("file: ", 1)[1] for l in out.splitlines() if "file: " in l]
check(bool(got) and open(got[0], "rb").read() == data, f"bia's attachment downloaded: {got}")

# ---- the daemon catches up on what bia said while it was stopped, and after bia restarts
rc, out = br("stop")
check("stopped" in out, f"stop: {out}")
web.tool("send", identity="claude", channel="web", text="while you were away")
rc, out = br("wait", "--timeout", "20")
check("while you were away" in out, f"caught up after a restart of the daemon: {out[:120]}")
web.p.terminate()
web.p.wait(10)
web = start_web()
time.sleep(1)
for _ in range(40):
    if json.loads(br("status", "--json")[1]).get("online"):
        break
    time.sleep(0.5)
web.tool("send", identity="claude", channel="web", text="back again")
rc, out = br("wait", "--timeout", "30")
check("back again" in out, f"link back after bia restarted: {out[:120]}")

# ---- what the bridge says while the bia is down waits and goes when it is back
web.p.terminate()
web.p.wait(10)
time.sleep(1)
rc, out = br("send", "said while you were down")
check(rc == 0 and "queued" in out, f"send while the bia is down is queued: {out}")
st = json.loads(br("status", "--json")[1])
check(st.get("waiting") == 1, f"status shows it waiting: {st.get('waiting')}")
web = start_web()
n = web.inbound("said while you were down", 60)
check(n is not None and n["identity"] == "claude", f"delivered once the bia is back: {n}")

# ---- MCP mode (Claude Code & co.)
m = Mcp([bridge, "mcp", "--agent", "claude"], benv)
names = [t["name"] for t in m.rpc("tools/list")["result"]["tools"]]
check({"connect", "import", "status", "send", "wait", "read", "history"} <= set(names) and "chats" not in names, f"MCP tools: {names}")
txt, err = m.tool("status")
check(not err and "connected" in txt, f"mcp status: {txt}")
txt, err = m.tool("send", text="from the mcp")
check(not err and web.inbound("from the mcp") is not None, f"mcp send reaches bia: {txt}")
web.tool("send", identity="claude", channel="web", text="answer for the mcp")
txt, err = m.tool("wait", timeout_secs=20)
check(not err and "answer for the mcp" in txt, f"mcp wait: {txt[:120]}")
txt, err = m.tool("history", limit=5)
check(not err and "answer for the mcp" in txt and "you (claude)" in txt, f"mcp history: {txt[:200]}")
m.p.terminate()

# ---- certificates the agent already has: a pickup .p12, PEM files
import re, ssl, urllib.request
txt, err = web.tool("pki_issue", identity="alice", role="agent")
m = re.search(r"/v1/pki/pickup/[A-Za-z0-9_-]+", txt)
check(not err and m is not None, f"owner issues a .p12 for alice: {txt[:100]}")
nov = ssl.create_default_context(); nov.check_hostname = False; nov.verify_mode = ssl.CERT_NONE
p12 = f"{root}/alice.p12"
open(p12, "wb").write(urllib.request.urlopen(URL + m.group(0), context=nov).read())
rc, out = br("import", URL, "--p12", p12, "--as", "stolen")
check(rc != 0 and "identity on a bia is claude, not alice" in out, f"another agent's certificate is refused: {out[:120]}")
rc, out = br("import", URL, "--p12", p12, "--password", "nope", "--as", "alice", agent="alice")
check(rc != 0 and "wrong .p12 password" in out, f"wrong password refused: {out}")
rc, out = br("import", URL, "--p12", p12, "--as", "alice", agent="alice")
check(rc == 0 and "Imported: alice" in out and "as alice" in out, f"import .p12: {out}")
rc, out = br("send", "--bia", "alice", "hi from alice", agent="alice")
n = web.inbound("hi from alice")
check(n is not None and n["identity"] == "alice", f"imported identity talks: {n}")
rc, out = br("import", URL, "--cert", f"{home}/claude/home/cert.pem", "--key", f"{home}/claude/home/key.pem", "--ca", f"{home}/claude/home/ca.pem", "--as", "pemcopy")
check(rc == 0 and "as claude" in out, f"import PEM files: {out}")
time.sleep(3)
rc, out = br("status", "--bia", "pemcopy")
check("approved" in out and "another bia-bridge with this certificate" in out, f"a copy of the same certificate is refused while the first is connected: {out}")
rc, out = br("status")
check(rc != 0 and "several bias" in out, f"two of its own need --bia: {out}")
rc, out = br("forget", "--bia", "pemcopy")
br("stop", "--bia", "alice", agent="alice")

# ---- another agent on this machine is another person
rc, out = br("status", agent=None)
check(rc != 0 and "which agent is this" in out, f"no agent named: {out[:100]}")
rc, out = br("status", agent="grok")
check(rc != 0 and "grok has no bia" in out, f"claude's only profile is not grok's: {out[:120]}")
rc, out = br("send", "--bia", "home", "pretending", agent="grok")
check(rc != 0 and web.inbound("pretending", 2) is None, f"nor by name: {out[:120]}")
rc, out = br("import", URL, "--cert", f"{home}/claude/home/cert.pem", "--key", f"{home}/claude/home/key.pem", "--ca", f"{home}/claude/home/ca.pem", "--as", "home", agent="grok")
check(rc != 0 and "not claude" in out, f"nor by importing its certificate: {out[:120]}")
rc, out = br("connect", URL, "--identity", "claude", "--as", "x", agent="grok")
check(rc != 0, f"nor by asking under its name: {out[:120]}")
rc, out = br("connect", URL, "--identity", "grok", "--as", "home", agent="grok")
check(rc == 0, f"its own request, for the same bia and name: {out[:100]}")
req3 = json.load(open(f"{home}/grok/home/profile.json"))["request"]
web.tool("pki_approve", id=req3, message="welcome, grok")
for _ in range(60):
    if json.loads(br("status", "--json", agent="grok")[1]).get("online"):
        break
    time.sleep(1)
br("send", "from grok", agent="grok")
n = web.inbound("from grok")
check(n is not None and n["identity"] == "grok", f"grok speaks as grok: {n}")
br("send", "from claude too")
n = web.inbound("from claude too")
check(n is not None and n["identity"] == "claude", f"claude still speaks as claude, both linked: {n}")
txt = br("profiles", agent=None)[1]
check("agent grok" in txt and "agent claude" in txt, f"profiles name the agent: {txt}")
# --owner: the same agent run for another person is its own identity, certificate and chat
oenv = dict(benv, BIA_BRIDGE_AGENT="claude", BIA_BRIDGE_OWNER="ann")
def bro(*args):
    r = subprocess.run([bridge, *args], env=oenv, capture_output=True, text=True, timeout=120)
    return r.returncode, (r.stdout + r.stderr).strip()
rc, out = bro("status")
check(rc != 0 and "claude@ann has no bia" in out, f"claude for ann is not claude: {out[:120]}")
rc, out = bro("connect", URL, "--identity", "claude", "--as", "home")
check(rc == 0, f"connect with an owner asks as claude@ann: {out[:120]}")
req4 = json.load(open(f"{home}/claude@ann/home/profile.json"))["request"]
web.tool("pki_approve", id=req4, message="welcome, ann's claude")
for _ in range(60):
    if json.loads(bro("status", "--json")[1]).get("online"):
        break
    time.sleep(1)
bro("send", "from ann's claude")
n = web.inbound("from ann's claude")
check(n is not None and n["identity"] == "claude@ann", f"it speaks as claude@ann: {n}")
br("send", "from plain claude")
n = web.inbound("from plain claude")
check(n is not None and n["identity"] == "claude", f"claude without an owner is still claude: {n}")
rc, out = subprocess.run([bridge, "status", "--agent", "claude@bob", "--owner", "ann"], env=benv, capture_output=True, text=True).returncode, ""
check(rc != 0, "an agent naming another owner is refused")
m = Mcp([bridge, "mcp", "--owner", "ann"], benv)
m.rpc("initialize", {"clientInfo": {"name": "Some Client"}})
txt, err = m.tool("status")
check(err and "Some-Client@ann has no bia" in txt, f"MCP: the client's name with the owner: {txt[:100]}")
m.p.terminate()
m = Mcp([bridge, "mcp"], benv)
txt, err = m.tool("status")
check(err and "which agent is this" in txt, f"MCP with no name at all: {txt[:100]}")
m.rpc("initialize", {"clientInfo": {"name": "Some Client"}})
txt, err = m.tool("status")
check(err and "Some-Client has no bia" in txt, f"MCP with no --agent is its client, which has no profile: {txt[:100]}")
m.p.terminate()

# ---- give the turn back: exit after N seconds of silence
web.tool("send", identity="grok", channel="web", text="first part")
t0 = time.time()
threading.Timer(2, lambda: web.tool("send", identity="grok", channel="web", text="second part")).start()
rc, out = br("wait", "--silence", "4", agent="grok")
took = time.time() - t0
check(rc == 0 and "first part" in out and "second part" in out, f"--silence collects until bia is quiet: {out[:160]}")
check(5 <= took <= 12, f"and exits about 4 s after the last word: {took:.1f} s")
t0 = time.time()
rc, out = br("wait", "--silence", "2", agent="grok")
check(rc == 0 and time.time() - t0 < 6 and "0 message" in out, f"nothing said: it exits too: {out}")
log = f"{root}/answers.txt"
web.tool("send", identity="grok", channel="web", text="into the file")
r = subprocess.run([bridge, "wait", "--silence", "2", "--out", log], env=dict(benv, BIA_BRIDGE_AGENT="grok"), capture_output=True, text=True, timeout=60)
check(r.returncode == 0 and r.stdout == "" and "into the file" in open(log).read(), f"--out goes to the file, not stdout: {r.stdout!r}")
t0 = time.time()
rc, out = br("wait", "--silence", "30", "--timeout", "2", agent="grok")
check(time.time() - t0 < 8, "--timeout limits the whole wait")
rc, out = br("wait", "--silence", "2", "--out", "/nonexistent/dir/x", agent="grok")
check(rc != 0 and "/nonexistent" in out, f"a bad file fails at once: {out[:80]}")
m = Mcp([bridge, "mcp", "--agent", "grok"], benv)
web.tool("send", identity="grok", channel="web", text="to the mcp silently")
txt, err = m.tool("wait", silence_secs=2)
check(not err and "to the mcp silently" in txt, f"mcp wait silence_secs: {txt[:100]}")
m.p.terminate()
br("stop", agent="grok")

# ---- the bia serves the bridge, from the same build
import urllib.error
nov2 = ssl.create_default_context(); nov2.check_hostname = False; nov2.verify_mode = ssl.CERT_NONE
lst = json.loads(urllib.request.urlopen(URL + "/v1/bridge", context=nov2).read())
mine = next((f for f in lst["files"] if f["platform"] == PLAT), None)
check(mine is not None and mine["sha256"] == hashlib.sha256(open(bridge, "rb").read()).hexdigest() and {f["platform"] for f in lst["files"]} == {PLAT, "windows-x86_64"},
      f"GET /v1/bridge lists the builds with sha256 (no certificate needed): {[f['file'] for f in lst['files']]}")
got = urllib.request.urlopen(URL + mine["url"], context=nov2).read()
check(hashlib.sha256(got).hexdigest() == mine["sha256"], "GET /bridge/<file> serves it")
for bad in ("/bridge/README.md", "/bridge/..%2Fbia-bridge-macos", "/bridge/bia-bridge-nothere"):
    try:
        urllib.request.urlopen(URL + bad, context=nov2)
        check(False, f"{bad} refused")
    except urllib.error.HTTPError as e:
        check(e.code in (401, 404), f"{bad} refused ({e.code})")
disc = json.loads(urllib.request.urlopen(URL + "/.well-known/bia.json", context=nov2).read())
check(disc["bridge"]["list"] == "/v1/bridge" and any(f["platform"] == PLAT for f in disc["bridge"]["files"]), "discovery points to the bia's own builds")
mybin = f"{root}/bin/bia-bridge"
os.makedirs(f"{root}/bin")
shutil.copy2(bridge, mybin)
r = subprocess.run([mybin, "update", "--agent", "claude", "--bia", "home"], env=benv, capture_output=True, text=True, timeout=60)
check(r.returncode == 0 and "already the bia's build" in r.stdout, f"update: already current ({r.stdout.strip()}{r.stderr.strip()})")
st = json.loads(br("status", "--bia", "home", "--json")[1])
check(st.get("bia_version") == st.get("bridge_version") and st.get("bia_version"), f"status knows the bia's version: {st.get('bia_version')} / {st.get('bridge_version')}")
if PLAT.startswith("linux"):
    # a different build on the bia (one byte more: an ELF still runs) replaces this one
    open(f"{served}/bia-bridge-{PLAT}", "ab").write(b"\0")
    want = hashlib.sha256(open(f"{served}/bia-bridge-{PLAT}", "rb").read()).hexdigest()
    r = subprocess.run([mybin, "update", "--agent", "claude", "--bia", "home"], env=benv, capture_output=True, text=True, timeout=60)
    check(r.returncode == 0 and "updated" in r.stdout and hashlib.sha256(open(mybin, "rb").read()).hexdigest() == want, f"update replaced the program: {r.stdout.strip()}{r.stderr.strip()}")
    r = subprocess.run([mybin, "version"], capture_output=True, text=True)
    check(r.returncode == 0, "the updated program runs")
    shutil.copy2(bridge, f"{served}/bia-bridge-{PLAT}")

# ---- a denied request
rc, out = br("connect", URL, "--identity", "eve", "--as", "other", agent="eve")
check(rc == 0, f"second profile: {out[:80]}")
req2 = json.load(open(f"{home}/eve/other/profile.json"))["request"]
web.tool("pki_deny", id=req2, reason="not today")
for _ in range(60):
    s = json.loads(br("status", "--bia", "other", "--json", agent="eve")[1])
    if s.get("state") != "pending":
        break
    time.sleep(1)
check(s.get("state") == "denied" and "not today" in s.get("message", ""), f"denial seen with its reason: {s}")
rc, out = br("profiles", agent=None)
check("home" in out and "other" in out, f"profiles: {out}")

# ---- the bia moves its web app to another port: the connected bridge follows it there
import socket
with socket.socket() as sk:
    sk.bind(("0.0.0.0", 0)); new_port = sk.getsockname()[1]
for _ in range(40):
    if json.loads(br("status", "--json")[1]).get("online"):
        break
    time.sleep(0.5)
txt = web.tool("port", port=new_port)
check(f":{new_port}/" in str(txt), f"web_port: {str(txt)[:80]}")
web.p.terminate()
web.p.wait(10)
web = start_web()
for _ in range(120):
    if json.loads(br("status", "--json")[1]).get("online"):
        break
    time.sleep(0.5)
url = json.load(open(f"{home}/claude/home/profile.json"))["url"]
check(url.endswith(f":{new_port}"), f"the profile follows the bia to its new port: {url}")
web.tool("send", identity="claude", channel="web", text="hello from the new port")
rc, out = br("wait", "--timeout", "30")
check("hello from the new port" in out and "moved its web app to" in out, f"told about the move, and the link is back there: {out[:200]}")

for who, prof in (("claude", "home"), ("eve", "other"), ("alice", "alice"), ("grok", "home")):
    br("stop", "--bia", prof, agent=who)
web.p.terminate()
shutil.rmtree(root, ignore_errors=True)
print(f"\n{'FAILED: ' + str(len(failed)) if failed else 'all passed'}")
sys.exit(1 if failed else 0)
