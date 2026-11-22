"""The unanswered ticker end to end: the real bia-agent (dev mode) with a scripted model and a
fake telegram bus, no real accounts.

A contact asks for something in a direct chat; the model starts a long bash job and says
nothing. Meanwhile a mixed group talks without pause (one message mentions bia by name). The
model sends a status at the second nudge and the answer when the job is done.

Checks:
- the first nudge comes about 15s after the model got the request, though the group never
  stops talking, and it lists the running job; the second follows about 30s later;
- a status the model sends reaches the chat; the answer closes the request (no nudge after);
- group talk is no request: none of it is ever nudged, a root contact's talk in the mixed group
  neither, nothing is sent to the group; a message naming bia there is nudged.

  cargo build -p bia-agent
  python3 -I tests/integration/ticker-e2e.py <dir with bia-agent>
"""
import json, os, re, subprocess, sys, tempfile, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

bins = sys.argv[1]
JOB_SECS = int(os.environ.get("TICKER_E2E_JOB", "75"))

def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        check.failed = True
check.failed = False

vol = tempfile.mkdtemp(prefix="bia-ticker-e2e-")
files = os.path.join(vol, "files")
state = os.path.join(files, "state")
os.makedirs(state)
work = tempfile.mkdtemp(prefix="bia-ticker-e2e-work-", dir=os.environ.get("TICKER_E2E_LOGS") or None)
reqlog = os.path.join(work, "requests.jsonl")
idlog = os.path.join(work, "ids.jsonl")
sendlog = os.path.join(work, "sends.jsonl")

# ---- the fake model ----
st = {"n": 0, "enabled": False, "fired": False, "nudges": 0, "done": False, "task_id": ""}
lock = threading.Lock()

def content_of(m):
    c = m.get("content")
    if isinstance(c, list):
        return " ".join(b.get("text", "") for b in c if isinstance(b, dict))
    return c or ""

def say(channel, identity, message, answered=None):
    a = {"identity": identity, "channel": channel, "message": message}
    if answered:
        a["answered"] = answered
    return {"call": "say", "arguments": a}

class LLM(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def sse(self, events):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for e in events:
            self.wfile.write(b"data: " + json.dumps(e).encode() + b"\n\n")
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(n) or b"{}")
        msgs = body.get("messages", [])
        all_text = "\n".join(content_of(m) for m in msgs)
        last_user = next((content_of(m) for m in reversed(msgs) if m.get("role") == "user"), "")
        reflect = "REFLECT. Do not call tools." in last_user
        k = max([i for i, m in enumerate(msgs) if m.get("role") == "assistant"], default=-1)
        new = "\n".join(content_of(m) for m in msgs[k + 1:])
        usage = {"prompt_tokens": len(all_text) // 4 + 3000, "completion_tokens": 5}
        call = None
        with lock:
            st["n"] += 1
            nudge = new[new.find("unanswered:"):] if "unanswered:" in new and not reflect else ""
            open(reqlog, "a").write(json.dumps({"n": st["n"], "t": time.time(), "kind": "reflect" if reflect else "step",
                                                "task": "MSG-TASK" in new, "nudge": nudge,
                                                "job_done": "finished-job" in new}) + "\n")
            if not reflect:
                m = re.search(r"\|telegram:5\|[^|]*\|[^|]*\|([^|]*)\|MSG-TASK", new)
                if m:
                    st["task_id"] = m.group(1)
                if not st["enabled"] and "booted" in all_text:
                    st["enabled"] = True
                    call = {"path": "mcp/telegram", "enable": True}
                elif "MSG-TASK" in new and not st["fired"]:
                    st["fired"] = True
                    call = {"call": "bash", "arguments": {"command": f"sleep {JOB_SECS}; echo finished-job"}}
                elif "finished-job" in new and not st["done"]:
                    st["done"] = True
                    call = say("telegram:5", "Friend (5)", "MSG-DONE here it is", [st["task_id"]])
                elif nudge and "telegram:5" in nudge:
                    st["nudges"] += 1
                    if st["nudges"] == 2:
                        call = say("telegram:5", "Friend (5)", "MSG-STATUS still on it")
        time.sleep(0.05)
        if reflect:
            text = ("REFLECTION category=summary\nreflection\nmock summary\nEND\n"
                    "REFLECTION category=facts\nreflection\nnothing durable\nEND")
            self.sse([{"choices": [{"delta": {"content": text}, "finish_reason": None}]},
                      {"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": usage}])
        elif call:
            self.sse([{"choices": [{"delta": {"tool_calls": [{"index": 0, "id": f"c{st['n']}", "function": {"name": "discover", "arguments": json.dumps(call)}}]}, "finish_reason": None}]},
                      {"choices": [{"delta": {}, "finish_reason": "tool_calls"}], "usage": usage}])
        else:
            self.sse([{"choices": [{"delta": {"content": "ok"}, "finish_reason": None}]},
                      {"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": usage}])

srv = ThreadingHTTPServer(("127.0.0.1", 0), LLM)
threading.Thread(target=srv.serve_forever, daemon=True).start()
port = srv.server_address[1]

# ---- the fake telegram MCP ----
FAKE = r'''#!/usr/bin/env python3
import json, os, re, sys, threading, time
IDS = open(os.environ["IDLOG"], "a", buffering=1); SENDS = open(os.environ["SENDLOG"], "a", buffering=1)
TALK_SECS = int(os.environ["TALK_SECS"])
lock = threading.Lock()
def out(v):
    with lock:
        sys.stdout.write(json.dumps(v) + "\n"); sys.stdout.flush()
def note(d):
    out({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": d}})
seq = [0]
def msg(chat, sender, name, text, group):
    seq[0] += 1
    d = {"t": "inbound", "identity": f"{name} ({sender.split(':')[1]})", "channel": chat, "id": str(1000 + seq[0]),
         "message": text, "issued": int(time.time()), "sender": sender}
    if group: d["group"] = True
    mk = re.search(r"MSG-[A-Za-z0-9_-]+", text)
    IDS.write(json.dumps({"id": d["id"], "chat": chat, "mark": mk.group(0) if mk else "", "t": time.time()}) + "\n")
    note(d)
def script():
    time.sleep(2)
    note({"t": "members", "channel": "telegram:-200", "title": "Market", "count": 2, "complete": True,
          "members": [{"id": "telegram:1", "name": "Boss"}, {"id": "telegram:8", "name": "Trader"}]})
    msg("telegram:5", "telegram:5", "Friend", "MSG-TASK please build the report, it takes a while", False)
    end, i = time.time() + TALK_SECS, 0
    while time.time() < end:
        i += 1
        if i == 10:
            msg("telegram:-200", "telegram:1", "Boss", f"MSG-BOSSTALK{i} prices are up", True)
        elif i == 20:
            msg("telegram:-200", "telegram:8", "Trader", f"Milo, MSG-MENTION{i} can you check the rate?", True)
        else:
            msg("telegram:-200", "telegram:8", "Trader", f"MSG-TALK{i} chatting", True)
        time.sleep(0.4)
sent = [5000]
for line in sys.stdin:
    req = json.loads(line)
    m, rid = req.get("method"), req.get("id")
    if rid is None:
        continue
    if m == "initialize":
        out({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake-telegram", "version": "0"}}})
    elif m == "tools/list":
        tools = [{"name": n, "description": n, "inputSchema": {"type": "object", "properties": {}}} for n in ("restore", "send", "read", "typing")]
        out({"jsonrpc": "2.0", "id": rid, "result": {"tools": tools}})
    elif m == "tools/call":
        p = req.get("params") or {}
        text = "ok"
        if p.get("name") == "send":
            sent[0] += 1
            SENDS.write(json.dumps({"t": time.time(), **(p.get("arguments") or {})}) + "\n")
            text = f"sent message_id={sent[0]}"
        out({"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": text}]}})
        if p.get("name") == "restore":
            threading.Thread(target=script, daemon=True).start()
    else:
        out({"jsonrpc": "2.0", "id": rid, "result": {}})
'''
mcp_dir = os.path.join(vol, "MCPs", "telegram")
os.makedirs(mcp_dir)
open(os.path.join(mcp_dir, "fake.py"), "w").write(FAKE)
os.chmod(os.path.join(mcp_dir, "fake.py"), 0o755)
json.dump({"name": "telegram", "version": "0", "description": "fake telegram for the ticker test", "command": "fake.py",
           "boot_tool": "restore", "env": {"IDLOG": idlog, "SENDLOG": sendlog, "TALK_SECS": "60"}},
          open(os.path.join(mcp_dir, "mcp.json"), "w"))

json.dump({"provider": "openai", "api_key": "mock-key", "model": "mock", "base_url": f"http://127.0.0.1:{port}/v1",
           "enabled": True, "ctx": 64000}, open(os.path.join(state, "fmm.json"), "w"))
json.dump({"name": "Milo", "gender": "male"}, open(os.path.join(state, "self.json"), "w"))
json.dump({"contacts": [
    {"id": "boss", "name": "Boss", "trust": "root", "channels": ["telegram:1"]},
    {"id": "friend", "name": "Friend", "trust": "normal", "channels": ["telegram:5"]},
]}, open(os.path.join(state, "addressbook.json"), "w"))

agent_log = os.path.join(work, "agent.log")
agent = subprocess.Popen([os.path.join(bins, "bia-agent"), "--volume", vol], stdin=subprocess.PIPE,
                         stdout=open(agent_log, "w"), stderr=subprocess.STDOUT, cwd=vol,
                         env=dict(os.environ, NO_COLOR="1"))

def reqs():
    try:
        return [json.loads(l) for l in open(reqlog)]
    except FileNotFoundError:
        return []

t0 = time.time()
done_at = None
while time.time() - t0 < JOB_SECS + 120 and agent.poll() is None:
    if st["done"] and done_at is None:
        done_at = time.time()
    if done_at and time.time() - done_at > 25:
        break
    time.sleep(1)
agent.terminate()
try:
    agent.wait(10)
except subprocess.TimeoutExpired:
    agent.kill()

R = reqs()
ids = [json.loads(l) for l in open(idlog)] if os.path.exists(idlog) else []
sends = [json.loads(l) for l in open(sendlog)] if os.path.exists(sendlog) else []
task = next((r for r in R if r["task"]), None)
nudges = [r for r in R if "channel=telegram:5" in r["nudge"]]
print(f"     {len(R)} requests, {len(nudges)} nudges for the request, {len(ids)} messages in")
check(task is not None, "the request reached the model")
if task and nudges:
    d1 = nudges[0]["t"] - task["t"]
    check(12 <= d1 <= 30, f"the first nudge came about 15s after the request, with the group talking all along ({d1:.0f}s)")
    check("running jobs: none" not in nudges[0]["nudge"] and "bash" in nudges[0]["nudge"], "it lists the running job")
else:
    check(False, "a nudge came for the open request")
if len(nudges) >= 2:
    d2 = nudges[1]["t"] - nudges[0]["t"]
    check(25 <= d2 <= 50, f"the second came about 30s after the first ({d2:.0f}s)")
else:
    check(False, f"a second nudge came ({len(nudges)})")
check(any(s.get("chat_id") == "5" and "MSG-STATUS" in str(s.get("text", "")) for s in sends), "the status reached the chat")
check(any(s.get("chat_id") == "5" and "MSG-DONE" in str(s.get("text", "")) for s in sends), "the answer reached the chat")
done_req = next((r for r in R if r["job_done"]), None)
late = [r for r in nudges if done_req and r["t"] > done_req["t"] + 2]
check(done_req is not None and not late, f"the answer closed the request: no nudge for it after ({len(late)})")

all_nudges = "\n".join(r["nudge"] for r in R)
talk_ids = [d["id"] for d in ids if d["mark"].startswith("MSG-TALK")]
nudged_talk = [i for i in talk_ids if f"channel=telegram:-200 id={i} " in all_nudges]
check(len(talk_ids) > 50 and not nudged_talk, f"group talk is never nudged ({len(nudged_talk)} of {len(talk_ids)})")
boss = next((d["id"] for d in ids if d["mark"].startswith("MSG-BOSSTALK")), "x")
check(f"id={boss} " not in all_nudges, "a root contact's talk in the mixed group is not nudged")
mention = next((d["id"] for d in ids if d["mark"].startswith("MSG-MENTION")), "x")
check(f"channel=telegram:-200 id={mention} " in all_nudges, "a message naming bia in the group is nudged")
check(not [s for s in sends if s.get("chat_id") == "-200"], f"nothing was sent to the group ({[s.get('text') for s in sends if s.get('chat_id') == '-200']})")

if check.failed:
    print("agent log tail:")
    print("".join(open(agent_log, errors="replace").readlines()[-60:]))
    print(f"logs: {work}")
    sys.exit(1)
print("all ticker checks passed")
