"""A bia updates while Chrome already has the add-on: Chrome must move to the new add-on by itself, with
the profile untouched. Real chrome MCP, real Chromium, real force-install policy. Runs as root in a
Debian trixie container, like the agent. Before 2.0.7 the policy lacked override_update_url and Chrome
stayed on the old add-on forever. --control leaves that policy in place and expects exactly that.

  python3 -I tests/integration/chrome-update-e2e.py <bia-mcp-chrome> <chrome MCP folder> [--control]
"""
import glob, json, os, shutil, subprocess, sys, tempfile, threading, time

binary, mcp_src = [os.path.abspath(x) for x in sys.argv[1:3]]
control = "--control" in sys.argv
WAIT = int(os.environ.get("UPDATE_WAIT", "480"))
POLICY = "/etc/chromium/policies/managed/bia-chrome.json"
PROFILE = "/home/bia/.config/bia-chrome"
failed = []


def check(cond, what):
    print(time.strftime("%H:%M:%S ") + ("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        failed.append(what)


def sh(*a):
    return subprocess.run(a, check=True, capture_output=True)


if subprocess.run(["id", "bia"], capture_output=True).returncode:
    sh("useradd", "-m", "bia")
uid = subprocess.check_output(["id", "-u", "bia"], text=True).strip()
run = f"/run/user/{uid}"
sh("install", "-d", "-o", "bia", "-m", "700", run)
os.makedirs("/tmp/.X11-unix", exist_ok=True)
os.chmod("/tmp/.X11-unix", 0o1777)
os.environ["DISPLAY"] = ":0"
open("/home/bia/.Xauthority", "a").close()
sh("chown", "bia:bia", "/home/bia/.Xauthority")
xvfb = subprocess.Popen(["Xvfb", ":0", "-screen", "0", "1280x800x24", "-ac"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1)
wm = subprocess.Popen(["openbox"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1)
sh("runuser", "-u", "bia", "--", "env", f"XDG_RUNTIME_DIR={run}", "HOME=/home/bia", "DISPLAY=:0", "dbus-daemon", "--session", "--fork", f"--address=unix:path={run}/bus")
os.environ["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={run}/bus"

files = "/home/bia/files"
os.makedirs(files + "/state", exist_ok=True)
subprocess.run(["chown", "-R", "bia:bia", "/home/bia"])
work = tempfile.mkdtemp(prefix="bia-update-e2e-")
os.chmod(work, 0o755)  # the native host runs as bia from in here
mcp_dir = work + "/mcp"
shutil.copytree(mcp_src, mcp_dir, symlinks=True)
os.makedirs("/opt/cr", exist_ok=True)
wrapper = "/opt/cr/chromium"  # a container has no sandbox for it
open(wrapper, "w").write('#!/bin/sh\nexec /usr/lib/chromium/chromium --no-sandbox "$@"\n')
os.chmod(wrapper, 0o755)
env = dict(os.environ, BIA_FILES=files, BIA_CHROME_BIN=wrapper)
p = subprocess.Popen([binary], cwd=mcp_dir, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(work + "/mcp.log", "w"), env=env, text=True, bufsize=1)
results, rid = {}, [0]


def reader():
    for line in p.stdout:
        try:
            v = json.loads(line)
        except ValueError:
            continue
        if "id" in v:
            results[v["id"]] = v


threading.Thread(target=reader, daemon=True).start()


def rpc(method, params=None, wait=180):
    rid[0] += 1
    i = rid[0]
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n")
    p.stdin.flush()
    end = time.time() + wait
    while time.time() < end:
        if i in results:
            return results.pop(i)
        time.sleep(0.05)
    return {"result": {"content": [{"text": f"no reply to {params}"}], "isError": True}}


def tool(name, **args):
    r = rpc("tools/call", {"name": name, "arguments": args})["result"]
    text = "".join(c.get("text", "") for c in r.get("content", []))
    return ("error: " + text) if r.get("isError") else text


def num(v):
    return tuple(int(x) for x in v.split("."))


def on_disk(ext):
    """Versions of the add-on in the profile, oldest first."""
    dirs = [os.path.basename(d) for d in glob.glob(f"{PROFILE}/Default/Extensions/{ext}/*_*")]
    return sorted((d.rsplit("_", 1)[0] for d in dirs), key=num)


def connected(wait=60):
    end = time.time() + wait
    while time.time() < end:
        if json.loads(tool("status")).get("connected"):
            return True
        time.sleep(2)
    return False


def as_older_bia():
    """The policy as bia wrote it before 2.0.7."""
    v = json.load(open(POLICY))
    for s in v["ExtensionSettings"].values():
        s.pop("override_update_url", None)
    json.dump(v, open(POLICY, "w"))


rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})
out = tool("install")
print(out[:300], flush=True)
info = json.loads(out)
ext = info["extension_id"]
check(connected(), "the first add-on connected")
end = time.time() + 60
while not on_disk(ext) and time.time() < end:
    time.sleep(1)
old = on_disk(ext)
check(len(old) == 1, f"one add-on in the profile: {old}")
as_older_bia()
tool("restart")
check(connected(), "running as an older bia, on the policy it wrote")

# the update: a new extension/ folder from the image
with open(mcp_dir + "/extension/background.js", "a") as f:
    f.write("\n// the next release\n")
out = tool("restart") if control else tool("install")  # at boot the agent runs install (mcp.json boot_tool)
print(out[:300], flush=True)
new = json.load(open(mcp_dir + "/extension/manifest.json"))["version"]
check(num(new) > num(old[0]), f"the update packed a newer add-on: {old[0]} -> {new}")
started = time.time()
got = None
while time.time() - started < WAIT:
    if new in on_disk(ext):
        got = time.time() - started
        break
    time.sleep(5)
if control:
    check(got is None, f"without override_update_url Chrome stays on {on_disk(ext)} after {WAIT}s")
else:
    check(got is not None, f"Chrome fetched {new} by itself: {got and round(got)}s, on disk {on_disk(ext)}")
    print("status right after:", tool("status"), flush=True)
    # does it run the new one without a restart?
    end = time.time() + 120
    while time.time() < end and on_disk(ext) != [new]:
        time.sleep(5)
    print("on disk after waiting:", on_disk(ext), flush=True)
    prefs = json.load(open(f"{PROFILE}/Default/Preferences")).get("extensions", {}).get("settings", {}).get(ext, {})
    check(prefs.get("manifest", {}).get("version") == new, f"it runs {new} without a Chrome restart: {prefs.get('manifest', {}).get('version')}")
    check(connected(), "the add-on is connected after the update")
    tool("restart")
    check(connected(), "and after a Chrome restart")
    check(on_disk(ext) == [new], f"only {new} remains after a restart: {on_disk(ext)}")
    prefs = json.load(open(f"{PROFILE}/Default/Preferences")).get("extensions", {}).get("settings", {}).get(ext, {})
    check(prefs.get("manifest", {}).get("version") == new, f"Chrome's own record says {prefs.get('manifest', {}).get('version')}")
if not control:
    # the next update: the policy is already right, so only Chrome's own update check can notice
    with open(mcp_dir + "/extension/background.js", "a") as f:
        f.write("\n// the release after that\n")
    tool("install")
    third = json.load(open(mcp_dir + "/extension/manifest.json"))["version"]
    check(num(third) > num(new), f"a later update packed {third}")
    started = time.time()
    while time.time() - started < WAIT and third not in on_disk(ext):
        time.sleep(5)
    check(third in on_disk(ext), f"Chrome fetched {third} too, after {round(time.time() - started)}s, with the policy unchanged: {on_disk(ext)}")
    check(connected(), "the add-on is connected after that one too")
tool("stop")
p.stdin.close()
xvfb.terminate()
wm.terminate()
print("PASS" if not failed else f"FAILED: {failed}", flush=True)
sys.exit(1 if failed else 0)
