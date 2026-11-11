"""Voice in a web call end to end: the real chrome MCP, real Chromium with the real extension, and a
page on localhost that behaves like a meeting: it takes the microphone, sends it over WebRTC and
plays what comes back. The "meeting" repeats the microphone a few seconds later as one participant
and adds a second one (a tone). The container has no sound device, like a server.

With the speech models (BIA_SPEECH_DIR) bia also speaks and must hear its own words come back as a
voice of the call; without them only the audio path is checked.

  python3 -I tests/integration/chrome-call-e2e.py <bia-mcp-chrome> <chrome MCP folder>
"""
import http.server, json, os, re, shutil, subprocess, sys, tempfile, threading, time

binary, mcp_src = [os.path.abspath(x) for x in sys.argv[1:3]]
speech = bool(os.environ.get("BIA_SPEECH_DIR"))
failed = []


def check(cond, what):
    print(time.strftime("%H:%M:%S ") + ("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        failed.append(what)


def sh(*a, **k):
    return subprocess.run(a, check=True, capture_output=True, **k)


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

files = "/home/bia/files"
os.makedirs(files + "/state", exist_ok=True)
subprocess.run(["chown", "-R", "bia:bia", "/home/bia"])
work = tempfile.mkdtemp(prefix="bia-call-e2e-")
os.chmod(work, 0o755)  # the native host runs as bia from in here

PAGE = b"""<!doctype html><title>meeting</title><p>meeting</p><script>
window.r = {};
window.join = async () => {
  try {
    const mic = await navigator.mediaDevices.getUserMedia({audio: true});
    r.mic = mic.getAudioTracks()[0].label + '|' + mic.getAudioTracks()[0].getSettings().deviceId;
    r.devices = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === 'audioinput').map((d) => d.label).join(',');
    r.perm = (await navigator.permissions.query({name: 'microphone'})).state;
    // the meeting: what this page sends comes back as participant one; participant two hums
    const up = new RTCPeerConnection(), down = new RTCPeerConnection();
    up.onicecandidate = (e) => e.candidate && down.addIceCandidate(e.candidate);
    down.onicecandidate = (e) => e.candidate && up.addIceCandidate(e.candidate);
    r.tracks = 0;
    down.ontrack = (e) => {
      r.tracks++;
      const el = new Audio();
      el.srcObject = new MediaStream([e.track]);
      el.play().catch((err) => { r.play = String(err); });
    };
    let ctx;
    try { ctx = new AudioContext({sinkId: {type: 'none'}}); } catch (e) { ctx = new AudioContext(); }
    // participant one repeats what the microphone said 8 s later (live it would talk over bia)
    const late = ctx.createDelay(10), back = ctx.createMediaStreamDestination();
    late.delayTime.value = 8;
    ctx.createMediaStreamSource(mic).connect(late).connect(back);
    up.addTrack(back.stream.getAudioTracks()[0], back.stream);
    const osc = ctx.createOscillator(), gain = ctx.createGain(), out = ctx.createMediaStreamDestination();
    gain.gain.value = 0.2;
    osc.connect(gain).connect(out);
    osc.start();
    up.addTrack(out.stream.getAudioTracks()[0], out.stream);
    const offer = await up.createOffer();
    await up.setLocalDescription(offer);
    await down.setRemoteDescription(offer);
    const answer = await down.createAnswer();
    await down.setLocalDescription(answer);
    await up.setRemoteDescription(answer);
    r.tone = ctx.state;
    r.joined = 'yes';
  } catch (e) {
    r.joined = 'error: ' + e.name + ': ' + e.message;
  }
};
// a meeting that decodes the audio itself and plays it through Web Audio (Google Meet does):
// nothing arrives on a WebRTC track or a media element
window.joinAudio = async () => {
  try {
    const mic = await navigator.mediaDevices.getUserMedia({audio: true});
    const ctx = new AudioContext();
    const late = ctx.createDelay(10);
    late.delayTime.value = 8;
    ctx.createMediaStreamSource(mic).connect(late);
    late.connect(ctx.destination);
    const osc = ctx.createOscillator(), gain = ctx.createGain();
    gain.gain.value = 0.2;
    osc.connect(gain);
    gain.connect(ctx.destination);
    osc.start();
    await new Promise((done) => setTimeout(done, 500));
    r.joined = ctx.state === 'running' ? 'yes' : 'audio ' + ctx.state;
  } catch (e) {
    r.joined = 'error: ' + e.name + ': ' + e.message;
  }
};
window.plain = async () => {
  try {
    await navigator.mediaDevices.getUserMedia({audio: true});
    r.plain = 'got a microphone';
  } catch (e) {
    r.plain = e.name;
  }
};
</script>"""


class Page(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(PAGE)))
        self.end_headers()
        self.wfile.write(PAGE)


srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Page)
threading.Thread(target=srv.serve_forever, daemon=True).start()
origin = f"http://localhost:{srv.server_address[1]}"

mcp_dir = work + "/mcp"
shutil.copytree(mcp_src, mcp_dir, symlinks=True)
os.makedirs("/opt/cr", exist_ok=True)
wrapper = "/opt/cr/chromium"  # a container has no sandbox for it
open(wrapper, "w").write('#!/bin/sh\nexec /usr/lib/chromium/chromium --no-sandbox "$@"\n')
os.chmod(wrapper, 0o755)
env = dict(os.environ, BIA_FILES=files, BIA_CHROME_BIN=wrapper)
p = subprocess.Popen([binary], cwd=mcp_dir, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(work + "/mcp.log", "w"), env=env, text=True, bufsize=1)
results, rid, inbound = {}, [0], []


def reader():
    for line in p.stdout:
        try:
            v = json.loads(line)
        except ValueError:
            continue
        d = (v.get("params") or {}).get("data")
        if isinstance(d, dict) and d.get("t") == "inbound":
            inbound.append(d)
            print(time.strftime("%H:%M:%S ") + f"     {d.get('identity')}|{d.get('channel')}|{d.get('message')}", flush=True)
        elif "id" in v:
            results[v["id"]] = v


threading.Thread(target=reader, daemon=True).start()


def rpc(method, params=None, wait=120):
    rid[0] += 1
    i = rid[0]
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n")
    p.stdin.flush()
    end = time.time() + wait
    while time.time() < end:
        if i in results:
            return results.pop(i)
        time.sleep(0.02)
    return {"result": {"content": [{"text": f"no reply to {params}"}], "isError": True}}


def tool(tool_name, /, **args):
    r = rpc("tools/call", {"name": tool_name, "arguments": args})["result"]
    text = "".join(c.get("text", "") for c in r.get("content", []))
    return ("error: " + text) if r.get("isError") else text


def js(code):
    out = tool("evaluate_script", function=f"() => {{ return {code}; }}")
    m = re.search(r"```json\s*(.*?)\s*```", out, re.S)
    if not m:
        return out
    try:
        return json.loads(m.group(1))
    except ValueError:
        return None  # a script that returns undefined


def line(start, wait=30, who=None, since=0):
    """The first line from the MCP starting with `start` (from `who`, after the first `since`), or None."""
    end = time.time() + wait
    while time.time() < end:
        for d in inbound[since:]:
            if d.get("message", "").startswith(start) and (who is None or d.get("identity") == who):
                return d
        time.sleep(0.1)
    return None


def status():
    out = tool("call_status")
    try:
        return json.loads(out)
    except ValueError:
        return {"error": out}


def until(cond, wait=30):
    end = time.time() + wait
    while time.time() < end:
        if cond():
            return True
        time.sleep(0.5)
    return False


def select(url):
    """Make the tab showing `url` the one page tools act on."""
    out = tool("list_pages")
    for m in re.finditer(r"^\s*(\d+): .*\((\S+)\)", out, re.M):
        if m.group(2) == url:
            return tool("select_page", pageId=int(m.group(1)))
    return "error: no page " + url + " in " + out


rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})
names = {t["name"] for t in rpc("tools/list")["result"]["tools"]}
check({"call_open", "call_say", "call_name", "call_status", "call_close"} <= names, "the call tools are listed")
out = tool("install")
print(out[:300])
connected = json.loads(out).get("connected") if out.startswith("{") else False
if not connected:
    connected = any(json.loads(tool("status")).get("connected") or time.sleep(2) for _ in range(30))
check(connected, "the extension connected")
if not connected:
    print(tool("status"))
    print(open(work + "/mcp.log").read()[-2000:])
    sys.exit(1)

# an ordinary tab is left alone: its page gets what Chrome gives (no microphone here)
tool("navigate_page", url=origin + "/", type="url")
time.sleep(1)
js("plain()")
check(until(lambda: js("r.plain || ''") != "", 10) and js("r.plain") != "got a microphone", f"a tab not opened for a call keeps Chrome's own answer: {js('r.plain')}")
check(status() == {"call": False}, "no call is open")
check("no call is open" in tool("call_say", text="hello", lang="en"), "nothing to speak into yet")

# the call tab
out = tool("call_open", url=origin + "/?room=1")
check(not out.startswith("error") and "chrome_call_say" in out, f"call_open: {out[:120]}")
check("already open" in tool("call_open", url=origin + "/?room=2"), "one call at a time")
out = select(origin + "/?room=1")
check(not out.startswith("error"), f"the call's tab is selected: {out[:160]}")
check(until(lambda: js("document.title") == "meeting", 15), "the meeting page loaded in it")
check("microphone" in tool("call_say", text="hello", lang="en"), "not joined: speaking is refused, nobody would hear it")

js("join()")
check(until(lambda: js("r.joined || ''") != "", 20) and js("r.joined") == "yes", f"the page joined with a microphone: {js('r.joined')}")
check(js("r.mic") == "bia voice|bia-voice", f"its microphone is bia's voice: {js('r.mic')}")
check(js("r.devices") == "bia voice" and js("r.perm") == "granted", f"the page sees one microphone, allowed: {js('r.devices')} / {js('r.perm')}")
check(line("[call microphone]", 10, "system") is not None, "the model is told the page took the microphone")
check(line("[call audio]", 20, "system") is not None, "the model is told bia hears the call")
check(all(d["channel"] == "chrome:call" for d in inbound if d["message"].startswith("[call ")), "on chrome:call")


def tone():
    return [v for v in status().get("voices", []) if v["seconds_received"] >= 3]


ok = until(lambda: len(tone()) >= 1, 40)
st = status()
print(json.dumps(st, indent=1)[:2500])
check(ok, "audio of a participant arrives (the tone)")
page = (st.get("page") or [{}])[0]
check(page.get("armed") is True and len(page.get("taps", [])) == 2 and page.get("audio") == "running", f"the page reports two tapped tracks and running audio: {json.dumps(page)[:300]}")
check(st.get("microphone_taken_by_page") is True and st.get("hearing") is True, "status: microphone taken, hearing")
check(page.get("from_tracks") is True and page.get("uses", {}).get("outputs") == 0, "heard from the WebRTC tracks")

if speech:
    # bia speaks; the meeting sends it back as participant one; bia must hear its own words
    out = tool("call_say", text="Good morning. This is a test of the meeting voice.", lang="en")
    check(not out.startswith("error"), f"call_say: {out[:120]}")
    check(line("[you finished speaking]", 60, "system") is not None, "the page played it and said so")
    heard = line("[speech en", 60)
    check(heard is not None and heard["identity"].startswith("voice:s") and "morning" in heard["message"].lower(), f"bia's words came back as a voice of the call: {heard}")
    if heard:
        s = heard["identity"].split(":", 1)[1]
        out = tool("call_name", voice=s, name="Echo")
        check("Echo" in out, f"call_name: {out}")
        n = len(inbound)
        tool("call_say", text="Second sentence, about the budget for next year.", lang="en")
        named = until(lambda: any(d["identity"] == "voice:Echo" and "budget" in d["message"].lower() for d in inbound[n:]), 90)
        check(named, "after naming, the voice arrives under its name")
    # a goodbye plays out before the tab closes
    until(lambda: status().get("speaking") is False, 30)
    tool("call_say", text="Goodbye everyone.", lang="en")
    out = tool("call_close")
    check("as soon as" in out, f"closing while speaking waits: {out}")
else:
    print("no speech models (BIA_SPEECH_DIR): speaking and transcription are not checked")
    out = tool("call_close")
    check("Left the call" in out, f"call_close: {out}")
end = line("[call ended]", 40, "system")
check(end is not None, f"the call ended: {end and end['message']}")
if speech and end:
    m = re.search(r"is in (\S+)\.$", end["message"])
    text = open(m.group(1)).read() if m else ""
    check("bia: Good morning" in text and "Echo:" in text, f"what was said is kept: {text[:300]!r}")
check(until(lambda: origin + "/?room=1" not in tool("list_pages"), 10), "its tab is closed")
check(status() == {"call": False}, "no call is open any more")

# a meeting that plays through Web Audio only: what it sends to its audio output is the call
n = len(inbound)
out = tool("call_open", url=origin + "/?room=2")
check(not out.startswith("error"), f"a second call opens: {out[:80]}")
select(origin + "/?room=2")
check(until(lambda: js("document.title") == "meeting", 15), "its page loaded")
js("joinAudio()")
check(until(lambda: js("r.joined || ''") != "", 20) and js("r.joined") == "yes", f"the page plays through Web Audio: {js('r.joined')}")
check(line("[call audio]", 20, "system", n) is not None, "bia hears it")
ok = until(lambda: len([v for v in status().get("voices", []) if v["source"].startswith("a") and v["seconds_received"] >= 2]) >= 1, 40)
st = status()
page = (st.get("page") or [{}])[0]
print(json.dumps(page)[:1500])
check(ok, "audio of a participant arrives from the page's output (the tone)")
check(page.get("from_tracks") is False and page.get("uses", {}).get("outputs") == 2, "two things are connected to the output: two voices")
if speech:
    tool("call_say", text="The weather is nice in the mountains today.", lang="en")
    heard = line("[speech en", 60, None, n)
    check(heard is not None and heard["identity"].startswith("voice:s") and "weather" in heard["message"].lower(), f"bia's words came back from the page's output: {heard}")
out = tool("call_close")
check(line("[call ended]", 40, "system", n) is not None, f"the second call ended ({out[:40]})")

if failed:
    print(open(work + "/mcp.log").read()[-3000:])
    print(subprocess.run(f"tail -40 {files}/state/chrome/chrome.log; tail -30 {files}/speech/logs/speech.log", shell=True, capture_output=True, text=True).stdout)
p.terminate()
print("\nFAILED:\n  " + "\n  ".join(failed) if failed else "\nall passed")
sys.exit(1 if failed else 0)
