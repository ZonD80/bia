#!/usr/bin/env python3
"""Statuses while bia works, end to end: the real bia-agent (dev mode) with a scripted model
and a fake telegram MCP that has a slow tool, no real accounts.

Scenes:
1. A group: Anna asks, the model starts a slow job without for_ids (it works for Anna's open
   messages by default); then Ben asks, the model starts one with for_ids=[Ben's id].
   - for_ids never reaches the MCP; each call's result names who it works for;
   - ~20 s later one unanswered notice lists both messages with the jobs working for them,
     one status line for the group and a close line per person; the model says one status;
   - each job result names its person ([for …]); the answers close both.
2. A voice call with Carl in Spanish: [call ready], then a transcribed phrase; the model starts
   a slow job. At the first notice it says a long status (15 s of speech), then stays silent.
   - the first notice comes 10 s after the phrase and says it is a voice call;
   - nothing is asked and nothing is said over bia's own words: no notice, no fallback while it
     speaks; the next notice comes 10 s after [you finished speaking];
   - 10 s after that, ignored, bia itself speaks the fallback in Spanish (lang es);
   - no utterance cuts another; the job result names Carl; nothing is left open.
Everywhere: a chat is never asked for a status twice within its interval.

  cargo build -p bia-agent
  python3 -I tests/integration/status-e2e.py <dir with bia-agent>
"""
import json, os, re, subprocess, sys, tempfile, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

bins = sys.argv[1]
SLOW = 32  # seconds a slow job runs
CALL_SLOW = 60  # the call's job: long enough for a status, a reminder and the fallback

def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        check.failed = True
check.failed = False

vol = tempfile.mkdtemp(prefix="bia-status-e2e-")
files = os.path.join(vol, "files")
state = os.path.join(files, "state")
os.makedirs(state)
work = tempfile.mkdtemp(prefix="bia-status-e2e-work-")
reqlog = os.path.join(work, "requests.jsonl")
mcplog = os.path.join(work, "mcp.jsonl")
inlog = os.path.join(work, "inbound.jsonl")
speechlog = os.path.join(work, "speech.jsonl")

# ---- the scripted model ----
ENV = re.compile(r"^(?P<who>[^|\n]+)\|(?P<ch>[^|\n]+)\|[^|\n]*\|[^|\n]*\|(?P<id>[^|\n]*)\|(?P<msg>.*)$", re.M)
st = {"n": 0, "enabled": False, "fired": set(), "status_said": False, "answered": set(), "ids": {}}
lock = threading.Lock()

def content_of(m):
    c = m.get("content")
    if isinstance(c, list):
        return " ".join(b.get("text", "") for b in c if isinstance(b, dict))
    return c or ""

def tool(name, args):
    return {"call": name, "arguments": args}

class LLM(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def sse(self, events):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for data in events:
            self.wfile.write(f"data: {json.dumps(data)}\n\n".encode())
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(n) or b"{}")
        msgs = body.get("messages", [])
        k = max([i for i, m in enumerate(msgs) if m.get("role") == "assistant"], default=-1)
        new_msgs = msgs[k + 1:]
        new = "\n".join(content_of(m) for m in new_msgs)
        all_text = "\n".join(content_of(m) for m in msgs)
        usage = {"prompt_tokens": len(all_text) // 4 + 2000, "completion_tokens": 5}
        calls = []
        with lock:
            st["n"] += 1
            for e in ENV.finditer(new):
                for mark in ("MSG-A", "MSG-B", "MSG-C"):
                    if mark in e["msg"]:
                        st["ids"][mark] = (e["id"], e["who"], e["ch"])
            open(reqlog, "a").write(json.dumps({"n": st["n"], "t": time.time(), "new": new,
                                                "tool_results": [content_of(m) for m in new_msgs if m.get("role") == "tool"]}) + "\n")
            if not st["enabled"] and "booted" in all_text:
                st["enabled"] = True
                calls.append({"path": "mcp/telegram", "enable": True})
            else:
                # start each task once; A: default for_ids, B: explicit, C: default (call)
                if "MSG-A" in st["ids"] and "A" not in st["fired"]:
                    st["fired"].add("A")
                    calls.append(tool("telegram_slow", {"what": "flights", "secs": SLOW}))
                if "MSG-B" in st["ids"] and "B" not in st["fired"]:
                    st["fired"].add("B")
                    calls.append(tool("telegram_slow", {"what": "table", "secs": SLOW, "for_ids": [st["ids"]["MSG-B"][0]]}))
                if "MSG-C" in st["ids"] and "C" not in st["fired"]:
                    st["fired"].add("C")
                    calls.append(tool("telegram_slow", {"what": "vuelo", "secs": CALL_SLOW}))
                # the call's first reminder: one long spoken status, then silence on purpose
                if "unanswered:" in new and "telegram:7" in new and not st.get("call_status"):
                    st["call_status"] = True
                    c = st["ids"]["MSG-C"]
                    calls.append(tool("say", {"identity": c[1], "channel": c[2], "message": "LONG estoy buscando vuelos a Berlín, comparando precios y horarios", "lang": "es"}))
                    st["second_status"] = True
                elif st.get("second_status") and "spoken in call" in new:
                    # as a real model did: a second status right after the first; the agent must
                    # not let it cut the first
                    st["second_status"] = False
                    c = st["ids"]["MSG-C"]
                    calls.append(tool("say", {"identity": c[1], "channel": c[2], "message": "sigo buscando", "lang": "es"}))
                # the group's first nudge: one status for everyone there (the call is left silent)
                if "unanswered:" in new and "telegram:-100" in new and not st["status_said"]:
                    st["status_said"] = True
                    a = st["ids"]["MSG-A"]
                    calls.append(tool("say", {"identity": a[1], "channel": a[2], "message": "@Anna @Ben still on it"}))
                # results: answer and close
                for what, mark in (("flights", "MSG-A"), ("table", "MSG-B"), ("vuelo", "MSG-C")):
                    if f"done {what}" in new and mark not in st["answered"]:
                        st["answered"].add(mark)
                        i, who, ch = st["ids"][mark]
                        calls.append(tool("say", {"identity": who, "channel": ch, "message": f"answer {what}", "answered": [i]}))
        time.sleep(0.05)
        if calls:
            evs = [{"choices": [{"delta": {"tool_calls": [{"index": j, "id": f"c{st['n']}_{j}", "function": {"name": "discover", "arguments": json.dumps(c)}}]}, "finish_reason": None}]}
                   for j, c in enumerate(calls)]
            evs.append({"choices": [{"delta": {}, "finish_reason": "tool_calls"}], "usage": usage})
            self.sse(evs)
        else:
            self.sse([{"choices": [{"delta": {"content": "ok"}, "finish_reason": None}]},
                      {"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": usage}])

srv = ThreadingHTTPServer(("127.0.0.1", 0), LLM)
threading.Thread(target=srv.serve_forever, daemon=True).start()
port = srv.server_address[1]

# ---- the fake telegram MCP ----
FAKE = r'''#!/usr/bin/env python3
import json, os, sys, threading, time
LOG = open(os.environ["MCPLOG"], "a", buffering=1); IN = open(os.environ["INLOG"], "a", buffering=1)
lock = threading.Lock()
call_on = {"7": False}
SPEECH = open(os.environ["SPEECHLOG"], "a", buffering=1)
speak_gen = [0]
playing = [False]
def speak(chat, text):
    # a say into the call plays for a while (LONG = 15 s), then [you finished speaking]; a new
    # one cuts the old ([voice cancelled])
    with lock:
        if playing[0]:
            SPEECH.write(json.dumps({"cut": time.time(), "gen": speak_gen[0], "by": text}) + "\n")
            note({"t": "inbound", "identity": "system", "channel": "telegram:" + chat, "id": "", "message": "[voice cancelled]", "issued": int(time.time()), "sender": ""})
        speak_gen[0] += 1
        g = speak_gen[0]
        playing[0] = True
    secs = 15 if text.startswith("LONG") else 2
    SPEECH.write(json.dumps({"start": time.time(), "secs": secs, "gen": g, "text": text}) + "\n")
    def done():
        time.sleep(secs)
        if speak_gen[0] == g:
            playing[0] = False
            SPEECH.write(json.dumps({"end": time.time(), "gen": g}) + "\n")
            note({"t": "inbound", "identity": "system", "channel": "telegram:" + chat, "id": "", "message": "[you finished speaking]", "issued": int(time.time()), "sender": ""})
    threading.Thread(target=done, daemon=True).start()
def out(v):
    with lock:
        sys.stdout.write(json.dumps(v) + "\n"); sys.stdout.flush()
def note(d):
    out({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": d}})
seq = [0]
def msg(chat, sender, name, text, group=False, mid=None):
    seq[0] += 1
    d = {"t": "inbound", "identity": f"{name} ({sender.split(':')[1]})", "channel": chat,
         "id": str(1000 + seq[0]) if mid is None else mid, "message": text, "issued": int(time.time()), "sender": sender}
    if group: d["group"] = True
    IN.write(json.dumps({"t": time.time(), "text": text}) + "\n")
    note(d)
def script():
    time.sleep(3)
    msg("telegram:-100", "telegram:11", "Anna", "Milo, MSG-A find me flights to Berlin", True)
    time.sleep(4)
    msg("telegram:-100", "telegram:12", "Ben", "Milo, MSG-B book a table for Friday", True)
    # the call starts once the group's answers are out
    while not os.path.exists(os.environ["CALLGO"]):
        time.sleep(0.5)
    call_on["7"] = True
    msg("telegram:7", "telegram:7", "Carl", "[call ready]")
    time.sleep(1)
    msg("telegram:7", "telegram:7", "Carl", "[speech es] MSG-C búscame un vuelo a Berlín", mid="")
def handle(req):
    m, rid = req.get("method"), req.get("id")
    p = req.get("params") or {}
    if m == "initialize":
        out({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake-telegram", "version": "0"}}})
    elif m == "tools/list":
        obj = lambda props: {"type": "object", "properties": props}
        out({"jsonrpc": "2.0", "id": rid, "result": {"tools": [
            {"name": "restore", "description": "start", "inputSchema": obj({})},
            {"name": "read", "description": "read receipts", "inputSchema": obj({})},
            {"name": "send", "description": "send", "inputSchema": obj({"chat_id": {"type": "string"}, "text": {"type": "string"}})},
            {"name": "call_say", "description": "speak", "inputSchema": obj({"chat_id": {"type": "string"}, "text": {"type": "string"}, "lang": {"type": "string"}})},
            {"name": "slow", "description": "A slow task.", "inputSchema": obj({"what": {"type": "string"}, "secs": {"type": "integer"}})}]}})
    elif m == "tools/call":
        name, args = p.get("name"), p.get("arguments") or {}
        LOG.write(json.dumps({"t": time.time(), "name": name, "args": args}) + "\n")
        if name == "slow":
            time.sleep(int(args.get("secs", 1)))
            text, err = f"done {args.get('what')}", False
        elif name == "call_say":
            on = call_on.get(str(args.get("chat_id")), False)
            if on:
                speak(str(args.get("chat_id")), args.get("text", ""))
            text, err = ("speaking" if on else "no active call"), not on
        elif name == "send":
            text, err = json.dumps({"id": str(int(time.time() * 1000))}), False
        else:
            text, err = "ok", False
        out({"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": text}], "isError": err}})
        if name == "restore":
            threading.Thread(target=script, daemon=True).start()
    elif rid is not None:
        out({"jsonrpc": "2.0", "id": rid, "result": {}})
for line in sys.stdin:
    req = json.loads(line)
    if req.get("id") is None:
        continue
    threading.Thread(target=handle, args=(req,), daemon=True).start()
'''
callgo = os.path.join(work, "callgo")
mcp_dir = os.path.join(vol, "MCPs", "telegram")
os.makedirs(mcp_dir)
open(os.path.join(mcp_dir, "fake.py"), "w").write(FAKE)
os.chmod(os.path.join(mcp_dir, "fake.py"), 0o755)
json.dump({"name": "telegram", "version": "0", "description": "fake telegram for the status test", "command": "fake.py",
           "boot_tool": "restore", "env": {"MCPLOG": mcplog, "INLOG": inlog, "CALLGO": callgo, "SPEECHLOG": speechlog}},
          open(os.path.join(mcp_dir, "mcp.json"), "w"))
json.dump({"provider": "openai", "api_key": "mock-key", "model": "mock", "base_url": f"http://127.0.0.1:{port}/v1",
           "enabled": True, "ctx": 64000}, open(os.path.join(state, "fmm.json"), "w"))
json.dump({"name": "Milo", "gender": "male"}, open(os.path.join(state, "self.json"), "w"))

agent_log = os.path.join(work, "agent.log")
agent = subprocess.Popen([os.path.join(bins, "bia-agent"), "--volume", vol], stdin=subprocess.PIPE,
                         stdout=open(agent_log, "w"), stderr=subprocess.STDOUT, cwd=vol,
                         env=dict(os.environ, NO_COLOR="1"))

def load(path):
    try:
        return [json.loads(l) for l in open(path)]
    except FileNotFoundError:
        return []

def wait(cond, secs, what):
    t = time.time()
    while time.time() - t < secs and agent.poll() is None:
        if cond():
            return True
        time.sleep(0.5)
    return False

# scene 1: the group
ok = wait(lambda: {"MSG-A", "MSG-B"} <= st["answered"], 150, "group answers")
check(ok, "both group requests were answered")
open(callgo, "w").write("go")
# scene 2: the call
ok = wait(lambda: "MSG-C" in st["answered"], 150, "call answer")
check(ok, "the call request was answered")
time.sleep(3)
agent.terminate()
try:
    agent.wait(10)
except subprocess.TimeoutExpired:
    agent.kill()

R = load(reqlog)
M = load(mcplog)
I = load(inlog)
t_in = {next(m for m in ("MSG-A", "MSG-B", "MSG-C") if m in x["text"]): x["t"] for x in I if "MSG-" in x["text"]}
ida, idb, idc = (st["ids"].get(m, ("?",))[0] for m in ("MSG-A", "MSG-B", "MSG-C"))

slow = [m for m in M if m["name"] == "slow"]
check(len(slow) == 3, f"three slow jobs ran ({len(slow)})")
check(all("for_ids" not in m["args"] for m in slow), f"for_ids never reached the MCP ({[m['args'] for m in slow]})")

results = [r for q in R for r in q["tool_results"]]
# a job's result arrives as a system|job:<id> envelope (not as the call's tool result)
job_texts = [q["new"] for q in R if "|job:" in q["new"]]
stub_a = next((r for r in results if "accepted job:" in r and f"for {ida} (Anna" in r), "")
check(bool(stub_a), f"Anna's job, called without for_ids, works for her message {ida} by default")
check(not any("accepted job:" in r and f"for {ida} (Anna" in r and "Ben" in r for r in results),
      "Ben's message (sent after) is not taken into Anna's job")
stub_b = next((r for r in results if "accepted job:" in r and f"for {idb} (Ben" in r), "")
check(bool(stub_b), f"Ben's job works for the id it was given ({idb})")

notices = [q for q in R if "unanswered:" in q["new"] and "telegram:-100" in q["new"]]
first = notices[0] if notices else {"new": "", "t": 0}
n1 = first["new"][first["new"].find("unanswered:"):]
lag = first["t"] - t_in.get("MSG-A", 0)
check(bool(notices) and 19 <= lag <= 23, f"the group's first notice came ~20 s after Anna asked ({lag:.1f}s)")
check(n1.count("→ still working: say(") == 1, "it has one status line for the group")
check("@Name each" in n1, "the status is asked for everyone waiting there")
check(n1.count("→ done: say(") == 2, "and a close line for each person")
check(re.search(rf"id={re.escape(ida)} from Anna[^\n]*\n    working for it: job:\S+ telegram_slow", n1) is not None, "Anna's line shows the job working for her")
check(re.search(rf"id={re.escape(idb)} from Ben[^\n]*\n    working for it: job:\S+ telegram_slow", n1) is not None, "Ben's line shows the job working for him")
group_sends = [m for m in M if m["name"] == "send" and str(m["args"].get("chat_id")) == "-100"]
check(any("still on it" in m["args"].get("text", "") for m in group_sends), "the model's one status reached the group")
check(not any("working on it" in m["args"].get("text", "").lower() for m in group_sends), "no automatic fallback in the group (the model answered its nudge)")
res_a = next((r[r.find("done flights"):] for r in job_texts if "done flights" in r), "")
check(f"[for {ida} (Anna (11) on telegram:-100)]" in res_a, f"Anna's job result names her ({res_a[-120:]!r})")
res_b = next((r[r.find("done table"):] for r in job_texts if "done table" in r), "")
check(f"[for {idb} (Ben (12) on telegram:-100)]" in res_b, "Ben's job result names him")

# the call
S = load(speechlog)
speeches = []  # (start, end) of everything bia said into the call
for x in S:
    if "start" in x:
        end = next((y.get("end") or y.get("cut") for y in S if y.get("gen") == x["gen"] and ("end" in y or "cut" in y)), x["start"] + x["secs"])
        speeches.append((x["start"], end, x["text"]))
def during_speech(t):
    return any(a + 0.5 < t < b - 0.5 for a, b, _ in speeches)
call_notices = [q for q in R if "unanswered:" in q["new"] and "telegram:7" in q["new"]]
cn = call_notices[0] if call_notices else {"new": "", "t": 0}
lag = cn["t"] - t_in.get("MSG-C", 0)
check(bool(call_notices) and 9 <= lag <= 16, f"in the call the first notice came ~10 s after the phrase ({lag:.1f}s)")
check("in a voice call" in cn["new"] and "spoken sentence" in cn["new"], "it says it is a voice call")
long_say = next((sp for sp in speeches if sp[2].startswith("LONG")), None)
check(long_say is not None and long_say[1] - long_say[0] >= 14, "the model's long status played for 15 s")
check(not any(during_speech(q["t"]) for q in call_notices), f"no call notice came while bia was speaking ({[round(q['t'] - t_in.get('MSG-C', 0), 1) for q in call_notices]})")
if long_say:
    after = [q for q in call_notices if q["t"] > long_say[1]]
    gap = after[0]["t"] - long_say[1] if after else -1
    check(bool(after) and 9 <= gap <= 13, f"the next notice came ~10 s after bia finished speaking ({gap:.1f}s)")
spoken = [m for m in M if m["name"] == "call_say" and str(m["args"].get("chat_id")) == "7" and m["args"].get("text") == "Sigo trabajando en ello."]
check(bool(spoken), f"the fallback was spoken into the call in Spanish ({[m['args'] for m in M if m['name'] == 'call_say'][-3:]})")
if spoken and long_say:
    check(spoken[0]["args"].get("lang") == "es", "with lang es")
    check(not during_speech(spoken[0]["t"]), "the fallback did not cut bia's own words")
    lag = spoken[0]["t"] - long_say[1]
    check(18 <= lag <= 24, f"~20 s after the speech ended: one ignored notice, then the fallback ({lag:.1f}s)")
check(any("not said: you are still speaking into the call on telegram:7" in r for r in results), "a second status while bia spoke was not said (it would have cut the first)")
cuts = [x for x in S if "cut" in x]
check(not cuts, f"no utterance cut another ({cuts})")
res_c = next((r[r.find("done vuelo"):] for r in job_texts if "done vuelo" in r), "")
check(f"[for {idc} (Carl (7) on telegram:7)]" in res_c, f"the call's job result names Carl ({res_c[-160:]!r})")

# a chat is asked for a status at most once per interval (notices and job results together)
for ch, gap_min in (("telegram:-100", 19), ("telegram:7", 9)):
    asks = sorted([q["t"] for q in R if "unanswered:" in q["new"] and f"channel={ch} " in q["new"]] +
                  [q["t"] for q in R if f"{ch} has heard nothing" in q["new"]])
    close = [round(b - a, 1) for a, b in zip(asks, asks[1:]) if b - a < gap_min]
    check(not close, f"{ch}: never asked twice within its interval ({len(asks)} asks, too close: {close})")

left = json.load(open(os.path.join(state, "unanswered.json"))).get("open", [])
check(not left, f"nothing is left open ({left})")

if check.failed:
    print("agent log tail:")
    print("".join(open(agent_log, errors="replace").readlines()[-60:]))
    print(f"logs: {work}")
    sys.exit(1)
print("all status checks passed")
