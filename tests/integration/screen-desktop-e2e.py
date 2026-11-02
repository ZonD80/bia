"""The screen MCP's desktop tools on a real X session: Xvfb, Openbox, bia's session bus, AT-SPI and a
real GTK app (mousepad), as in bia's image. Runs as root in a Debian trixie container, like the agent.

  python3 -I tests/integration/screen-desktop-e2e.py <bia-mcp-screen> <bia_desktop.py>
"""
import json, os, re, subprocess, sys, tempfile, threading, time

binary, script = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        check.failed = True


check.failed = False


def sh(*a, **k):
    return subprocess.run(a, check=True, **k)


if subprocess.run(["id", "bia"], capture_output=True).returncode:
    sh("useradd", "-m", "bia")
uid = subprocess.check_output(["id", "-u", "bia"], text=True).strip()
run = f"/run/user/{uid}"
sh("install", "-d", "-o", "bia", "-m", "700", run)
os.makedirs("/tmp/.X11-unix", exist_ok=True)
os.chmod("/tmp/.X11-unix", 0o1777)
os.environ.update(DISPLAY=":0", BIA_DESKTOP_SCRIPT=script)
xauth = "/home/bia/.Xauthority"
open(xauth, "a").close()
sh("chown", "bia:bia", xauth)
xvfb = subprocess.Popen(["Xvfb", ":0", "-screen", "0", "1280x800x24", "-ac"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1)
wm = subprocess.Popen(["openbox"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1)
# as bia-session does
sh("runuser", "-u", "bia", "--", "env", f"XDG_RUNTIME_DIR={run}", "HOME=/home/bia", "DISPLAY=:0", "dbus-daemon", "--session", "--fork", f"--address=unix:path={run}/bus", stdout=subprocess.DEVNULL)

files = tempfile.mkdtemp(prefix="bia-desktop-e2e-")
p = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(os.path.join(files, "mcp.log"), "w"), env=dict(os.environ, BIA_FILES=files), text=True, bufsize=1)
results, rid = {}, [0]


def reader():
    for line in p.stdout:
        v = json.loads(line)
        if "id" in v:
            results[v["id"]] = v


threading.Thread(target=reader, daemon=True).start()


def rpc(method, params=None, wait=150):
    rid[0] += 1
    i = rid[0]
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n")
    p.stdin.flush()
    end = time.time() + wait
    while time.time() < end:
        if i in results:
            return results.pop(i)
        time.sleep(0.02)
    raise SystemExit(f"no reply to {method} {params}")


def tool(name, **args):
    r = rpc("tools/call", {"name": name, "arguments": args})["result"]
    text = "".join(c.get("text", "") for c in r.get("content", []))
    return ("error: " + text) if r.get("isError") else text


def ref_of(snapshot, pattern):
    for line in snapshot.splitlines():
        if re.search(pattern, line):
            m = re.search(r"\b([0-9a-f]+:e\d+)\b", line)
            if m:
                return m.group(1)
    return None


rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})
names = {t["name"] for t in rpc("tools/list")["result"]["tools"]}
for t in ("apps", "snapshot", "find", "get", "act", "launch", "close_app", "windows", "window", "clipboard", "wait", "shot", "click", "key"):
    check(t in names, f"tool {t}")

out = tool("apps")
check("no app is on the accessibility bus" in out, f"an empty bus says what to do: {out[:90]}")
out = tool("snapshot", app="nothing")
check(out.startswith("error:") and "no app" in out, f"an unknown app is refused: {out[:90]}")
out = tool("act", ref="deadbeef:e1", action="click")
check("unknown or too old" in out, f"an unknown ref is refused: {out[:80]}")
out = tool("launch", command="no-such-program")
check("no program" in out, "a missing program is said")

out = tool("launch", command=["mousepad"], wait=25)
check("started mousepad" in out, f"launch: {out}")
check("mousepad" in tool("apps"), "the app is on the bus")
out = tool("windows")
check("Mousepad" in out or "Untitled" in out, f"its window is listed: {out[:120]}")
win = out.split()[0]

snap = tool("snapshot", app="mousepad")
print(snap[:1200])
check(":e1 " in snap, "a snapshot has refs")
edit = ref_of(snap, r"\b(text|paragraph|entry)\b.*editable")
check(edit is not None, "the editor is found as an editable element")
out = tool("act", ref=edit, action="type", value="hello from bia")
check("hello from bia" in out, f"type goes in through the app: {out}")
detail = json.loads(tool("get", ref=edit))
check(detail["text"].startswith("hello from bia"), f"and reads back: {detail.get('text')!r}")
out = tool("act", ref=edit, action="set", value="replaced")
check("replaced" in out, f"set replaces: {out}")
out = tool("act", ref=edit, action="clear")
check("cleared" in out, f"clear: {out}")
tool("act", ref=edit, action="set", value="words to find")
check("words to find" in tool("find", app="mousepad", text="words to find"), "find by text")
check(":e" in tool("find", app="mousepad", role="menu item") or ":e" in tool("find", app="mousepad", role="menu"), "find by role")

check("there" in tool("wait", window="Mousepad", timeout=5) or "there" in tool("wait", app="mousepad", text="words to find", timeout=5), "wait sees what is there")
out = tool("wait", window="no such title", timeout=1)
check(out.startswith("error:") and "not there" in out, f"wait gives up: {out}")

# a menu through the app itself, then the dialog it opens
snap = tool("snapshot", app="mousepad")
menu = ref_of(snap, r"\bmenu\b.*\"(File|Datei)\"") or ref_of(snap, r"\bmenu\b")
if menu:
    out = tool("act", ref=menu, action="click")
    check(out.startswith("clicked"), f"a menu opens without the mouse: {out}")
    tool("key", key="Escape")

# the mouse, twice to one spot: xdotool's mousemove --sync never returns when nothing moves
def timed(*calls):
    start = time.time()
    outs = [tool(n, **a) for n, a in calls]
    return outs, time.time() - start


for what, name, args in (("a click", "click", {"x": 700, "y": 500}), ("a move", "click", {"x": 710, "y": 510, "button": 0}),
                         ("a scroll", "scroll", {"x": 720, "y": 520, "dy": 1}), ("a human move", "cursor_move_human", {"x": 730, "y": 530, "duration_ms": 200}),
                         ("a drag", "drag", {"x": 740, "y": 540, "x2": 740, "y2": 540})):
    outs, took = timed((name, args), (name, args), (name, args))
    check(took < 15 and not any(o.startswith("error:") for o in outs), f"{what} repeated on one spot returns: {took:.1f}s {outs[-1][:50]}")

# windows
check("maximize" in tool("window", window=win, action="maximize"), "maximize")
time.sleep(0.5)
size = next(l for l in tool("windows").splitlines() if l.startswith(win))
check(" 12" in size and "x7" in size, f"it filled the screen: {size}")
tool("window", window=win, action="restore")
tool("window", window=win, action="resize", w=600, h=400)
tool("window", window=win, action="move", x=50, y=60)
time.sleep(0.5)
size = next(l for l in tool("windows").splitlines() if l.startswith(win))
check("600x400" in size, f"resize: {size}")
check("focus" in tool("window", window=win, action="focus"), "focus")
out = tool("window", window="0x1", action="focus")
check("no window" in out, f"an unknown window is refused: {out}")

# the clipboard, across processes
check("clipboard set" in tool("clipboard", action="set", text="clip text"), "clipboard set")
check(tool("clipboard") == "clip text", "clipboard get")
check("cleared" in tool("clipboard", action="clear"), "clipboard clear")

# a ref that no longer means the same refuses, and never lands elsewhere
snap = tool("snapshot", app="mousepad")
any_ref = ref_of(snap, r"\bmenu\b") or edit
check("close" in tool("window", window=win, action="close"), "close")
time.sleep(1)
# mousepad asks about the unsaved text: a dialog, readable like the rest
snap = tool("snapshot", app="mousepad")
check("save the changes" in snap, "a dialog's question is in the snapshot")
button = ref_of(snap, r"\bbutton \"Don.t Save")
if button:
    check(tool("act", ref=button, action="click").startswith("clicked"), "a dialog's button is pressed by its ref")
    check("gone" in tool("wait", window="Mousepad", gone=True, timeout=10), "the app has closed")
    out = tool("act", ref=any_ref, action="click")
    check(out.startswith("error:") and ("stale" in out or "gone away" in out), f"a ref of a gone app is refused: {out}")
else:
    check(False, f"the save dialog can be answered: {snap[-500:]}")

for proc in (p, wm, xvfb):
    proc.terminate()
if check.failed:
    print(open(os.path.join(files, "mcp.log")).read()[-1500:])
    sys.exit(1)
print("all ok")
