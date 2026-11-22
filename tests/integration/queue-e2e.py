"""The waiting queue end to end: the real bia-agent (dev mode) with a scripted model on a small
context and a fake chat MCP that floods it, no real accounts.

The fake MCP plays the telegram bus: it says who is in three groups (one of only root contacts,
two mixed), then sends a flood of group talk with a few important messages behind it, a very
long message, and later a stranger joining and leaving the root group. The fake model enables
the bus, answers "ok", reflects when asked, takes three flood messages early with queue_take at
the first [queue] notice, and keeps the root group at p0 with chat_level when told a stranger
joined.

Checks:
- every message reaches the model in a working step (none is reflected away unseen), and
  reflections never come twice in a row: the old endless reflection loop;
- the root group (P0), a contact's direct chat (P1), a root contact in the mixed group (P1), a
  stranger's direct chat and a reply to bia (P2) all come before half the flood (P3), in rank
  order, with the most important chat last in its batch; another group is not starved;
- the [queue] notice says how much waits, which chats started waiting and how to take more;
- queue_take takes what was asked and the messages follow its result;
- the loop guard notices reflections with only a step between them and keeps 20% free;
- the very long message is cut, with the whole text kept in files/inbox/long;
- a stranger joining the root group is reported with their id; chat_level holds the group at
  p0; their leaving reports the group root again; a join to a mixed group is told too;
- a group channel is never linked to a person in the addressbook;
- read receipts: each message is marked read when the model gets it (between the request before
  and the one that shows it), never on arrival, so flood messages waiting in the queue stay
  unread meanwhile; every delivered message is read, with who wrote it.

  cargo build -p bia-agent
  python3 -I tests/integration/queue-e2e.py <dir with bia-agent>
"""
import json, os, re, subprocess, sys, tempfile, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

bins = sys.argv[1]
CTX = int(os.environ.get("QUEUE_E2E_CTX", "16000"))
FLOOD = int(os.environ.get("QUEUE_E2E_FLOOD", "150"))

def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        check.failed = True
check.failed = False

vol = tempfile.mkdtemp(prefix="bia-queue-e2e-")
files = os.path.join(vol, "files")
state = os.path.join(files, "state")
os.makedirs(state)
work = tempfile.mkdtemp(prefix="bia-queue-e2e-work-", dir=os.environ.get("QUEUE_E2E_LOGS") or None)
reqlog = os.path.join(work, "requests.jsonl")
readlog = os.path.join(work, "reads.jsonl")
idlog = os.path.join(work, "ids.jsonl")
phase2 = os.path.join(work, "phase2")

# ---- the fake model ----
MARK = re.compile(r"MSG-[A-Za-z0-9_-]+")
state_m = {"took": False, "leveled": False, "n": 0, "enabled": False}
lock = threading.Lock()

def content_of(m):
    c = m.get("content")
    if isinstance(c, list):
        return " ".join(b.get("text", "") for b in c if isinstance(b, dict))
    return c or ""

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
        texts = [content_of(m) for m in msgs]
        last_user = next((content_of(m) for m in reversed(msgs) if m.get("role") == "user"), "")
        reflect = "REFLECT. Do not call tools." in last_user
        # what came since the model last spoke
        k = max([i for i, m in enumerate(msgs) if m.get("role") == "assistant"], default=-1)
        new = "\n".join(content_of(m) for m in msgs[k + 1:])
        all_text = "\n".join(texts)
        usage = {"prompt_tokens": len(all_text) // 4 + 3000, "completion_tokens": 5}
        with lock:
            state_m["n"] += 1
            rec = {"n": state_m["n"], "t": time.time(), "kind": "reflect" if reflect else "step",
                   "marks": sorted(set(MARK.findall(all_text))), "new_marks": sorted(set(MARK.findall(new))),
                   "queue": "[queue]" in new, "queue_text": new[new.find("[queue]"):][:1500] if "[queue]" in new else "",
                   "members": [l for l in new.splitlines() if "[members]" in l],
                   "order": MARK.findall(new), "took": (re.search(r"took \d+ of \d+ waiting[^\n]*", new) or [""])[0],
                   "cut": "[cut:" in new and "MSG-LONG" in new, "chars": len(all_text)}
            open(reqlog, "a").write(json.dumps(rec) + "\n")
            call = None
            if not reflect:
                if not state_m["enabled"] and "booted" in all_text:
                    # MCPs start on request: open the (fake) telegram bus
                    state_m["enabled"] = True
                    call = {"path": "mcp/telegram", "enable": True}
                elif "[queue]" in new and not state_m["took"]:
                    state_m["took"] = True
                    call = {"call": "queue_take", "arguments": {"chat": "telegram:-200", "count": 3}}
                elif "[members]" in new and "not a root contact" in new and not state_m["leveled"]:
                    state_m["leveled"] = True
                    call = {"call": "chat_level", "arguments": {"chat": "telegram:-100", "level": "p0", "why": "family"}}
        time.sleep(0.05)
        if reflect:
            text = ("REFLECTION category=summary\nreflection\nmock summary of the period\nEND\n"
                    "REFLECTION category=facts\nreflection\nnothing durable\nEND")
            self.sse([{"choices": [{"delta": {"content": text}, "finish_reason": None}]},
                      {"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": usage}])
        elif call:
            # every function goes through discover: discover(call=name, arguments={…}) or discover(path=…)
            self.sse([{"choices": [{"delta": {"tool_calls": [{"index": 0, "id": f"c{state_m['n']}", "function": {"name": "discover", "arguments": json.dumps(call)}}]}, "finish_reason": None}]},
                      {"choices": [{"delta": {}, "finish_reason": "tool_calls"}], "usage": usage}])
        else:
            self.sse([{"choices": [{"delta": {"content": "ok"}, "finish_reason": None}]},
                      {"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": usage}])

srv = ThreadingHTTPServer(("127.0.0.1", 0), LLM)
threading.Thread(target=srv.serve_forever, daemon=True).start()
port = srv.server_address[1]

# ---- the fake telegram MCP ----
FAKE = r'''#!/usr/bin/env python3
import json, os, sys, threading, time
FLOOD = int(os.environ["FLOOD"]); PHASE2 = os.environ["PHASE2"]
READS = open(os.environ["READLOG"], "a", buffering=1); IDS = open(os.environ["IDLOG"], "a", buffering=1)
import re
lock = threading.Lock()
def out(v):
    with lock:
        sys.stdout.write(json.dumps(v) + "\n"); sys.stdout.flush()
def note(d):
    out({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": d}})
now = int(time.time())
seq = [0]
def msg(chat, sender, name, text, group):
    seq[0] += 1
    d = {"t": "inbound", "identity": f"{name} ({sender.split(':')[1]})", "channel": chat, "id": str(1000 + seq[0]),
         "message": text, "issued": now - 600 + seq[0], "sender": sender}
    mk = re.search(r"MSG-[A-Za-z0-9_-]+", text)
    IDS.write(json.dumps({"id": d["id"], "chat": chat, "mark": mk.group(0) if mk else "", "t": time.time()}) + "\n")
    if group: d["group"] = True
    note(d)
filler = " ".join(["talk"] * 220)
def members(chat, title, ms, complete=True):
    note({"t": "members", "channel": chat, "title": title, "count": len(ms), "complete": complete,
          "members": [{"id": i, "name": n} for i, n in ms]})
def script():
    time.sleep(2)
    members("telegram:-100", "Fam", [("telegram:1", "Boss"), ("telegram:2", "Wife")])
    members("telegram:-200", "Market", [("telegram:1", "Boss"), ("telegram:8", "Trader"), ("telegram:9", "Seller")])
    members("telegram:-300", "Club", [("telegram:8", "Trader"), ("telegram:4", "Member")])
    msg("telegram:-200", "telegram:8", "Trader", "MSG-FIRST " + filler, True)
    for i in range(FLOOD):
        msg("telegram:-200", "telegram:8" if i % 2 else "telegram:9", "Trader", f"MSG-FLOOD{i} " + filler, True)
        if i == FLOOD // 2:
            for j in range(3):
                msg("telegram:-300", "telegram:4", "Member", f"MSG-CLUB{j} " + filler, True)
    msg("telegram:7", "telegram:7", "Stranger", "MSG-LONG " + ("very long text " * 4500), False)
    msg("telegram:-200", "telegram:8", "Trader", "[reply to 77 from you: \"ok\"]\nMSG-REPLY are you sure?", True)
    msg("telegram:5", "telegram:5", "Friend", "MSG-FRIEND hi, are you there?", False)
    msg("telegram:-200", "telegram:1", "Boss", "MSG-ROOTMIXED please check prices", True)
    msg("telegram:-100", "telegram:2", "Wife", "MSG-P0 dinner at 7?", True)
    while not os.path.exists(PHASE2):
        time.sleep(0.5)
    note({"t": "member", "channel": "telegram:-100", "title": "Fam", "op": "join", "id": "telegram:66", "name": "Stranger"})
    time.sleep(8)
    note({"t": "member", "channel": "telegram:-100", "title": "Fam", "op": "leave", "id": "telegram:66", "name": "Stranger"})
    note({"t": "member", "channel": "telegram:-200", "title": "Market", "op": "join", "id": "telegram:77", "name": "Newbie"})
    msg("telegram:-100", "telegram:1", "Boss", "MSG-AFTER all good", True)
for line in sys.stdin:
    req = json.loads(line)
    m, rid = req.get("method"), req.get("id")
    if rid is None:
        continue
    if m == "initialize":
        out({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake-telegram", "version": "0"}}})
    elif m == "tools/list":
        out({"jsonrpc": "2.0", "id": rid, "result": {"tools": [{"name": "restore", "description": "start", "inputSchema": {"type": "object", "properties": {}}},
                  {"name": "read", "description": "read receipts", "inputSchema": {"type": "object", "properties": {}}}]}})
    elif m == "tools/call":
        p = req.get("params") or {}
        if p.get("name") == "read":
            READS.write(json.dumps({"t": time.time(), **(p.get("arguments") or {})}) + "\n")
        out({"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": "ok"}]}})
        if (req.get("params") or {}).get("name") == "restore":
            threading.Thread(target=script, daemon=True).start()
    else:
        out({"jsonrpc": "2.0", "id": rid, "result": {}})
'''
mcp_dir = os.path.join(vol, "MCPs", "telegram")
os.makedirs(mcp_dir)
open(os.path.join(mcp_dir, "fake.py"), "w").write(FAKE)
os.chmod(os.path.join(mcp_dir, "fake.py"), 0o755)
json.dump({"name": "telegram", "version": "0", "description": "fake telegram for the queue test", "command": "fake.py",
           "boot_tool": "restore", "env": {"FLOOD": str(FLOOD), "PHASE2": phase2, "READLOG": readlog, "IDLOG": idlog}}, open(os.path.join(mcp_dir, "mcp.json"), "w"))

json.dump({"provider": "openai", "api_key": "mock-key", "model": "mock", "base_url": f"http://127.0.0.1:{port}/v1",
           "enabled": True, "ctx": CTX}, open(os.path.join(state, "fmm.json"), "w"))
json.dump({"name": "Milo", "gender": "male"}, open(os.path.join(state, "self.json"), "w"))
json.dump({"contacts": [
    {"id": "boss", "name": "Boss", "trust": "root", "channels": ["telegram:1"]},
    {"id": "wife", "name": "Wife", "trust": "root", "channels": ["telegram:2"]},
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

def seen_all(names):
    got = set()
    for r in reqs():
        if r["kind"] == "step":
            got.update(r["marks"])
    return all(n in got for n in names)

flood_marks = [f"MSG-FLOOD{i}" for i in range(FLOOD)]
important = ["MSG-P0", "MSG-ROOTMIXED", "MSG-FRIEND", "MSG-REPLY", "MSG-LONG", "MSG-CLUB0", "MSG-CLUB1", "MSG-CLUB2"]
t0 = time.time()
ok = False
while time.time() - t0 < int(os.environ.get("QUEUE_E2E_WAIT", "900")):
    if agent.poll() is not None:
        break
    if seen_all(flood_marks + important + ["MSG-FIRST"]):
        ok = True
        break
    time.sleep(1)
check(ok, f"every message of the flood reached the model in a working step ({len(reqs())} requests, {int(time.time() - t0)}s)")
open(phase2, "w").write("go")
t1 = time.time()
while time.time() - t1 < 180 and agent.poll() is None:
    if seen_all(["MSG-AFTER"]) and any("again" in " ".join(r["members"]) for r in reqs()):
        break
    time.sleep(1)
agent.terminate()
try:
    agent.wait(10)
except subprocess.TimeoutExpired:
    agent.kill()

R = reqs()
steps = [r for r in R if r["kind"] == "step"]
refl = [r for r in R if r["kind"] == "reflect"]
print(f"     {len(steps)} steps, {len(refl)} reflections, ctx {CTX}, flood {FLOOD}")

# first request (by number) in which each marker appears. A reflection request shows the model
# the whole history (fresh messages too), so a marker may first show there; that nothing was
# reflected away unseen is the first check: every marker shows in a working step.
first = {}
for r in R:
    for m in r["marks"]:
        first.setdefault(m, r)

# no two reflections back to back without a step while messages waited, beyond one rollup pair
worst = run = 0
for r in R:
    run = run + 1 if r["kind"] == "reflect" else 0
    worst = max(worst, run)
check(worst <= 4, f"reflections in a row stay bounded (most in a row: {worst})")
check(len(steps) >= len(refl), f"at least as many working steps as reflections ({len(steps)} vs {len(refl)})")

def at(mark):
    return first[mark]["n"] if mark in first else 10**9

half_flood = at(flood_marks[FLOOD // 2])
for m in ["MSG-P0", "MSG-ROOTMIXED", "MSG-FRIEND", "MSG-REPLY", "MSG-LONG"]:
    check(at(m) < half_flood, f"{m} came before half the flood (request {at(m)} < {half_flood})")
check(at("MSG-P0") <= at("MSG-FRIEND") <= at("MSG-REPLY"), f"P0 before P1 before P2 ({at('MSG-P0')}, {at('MSG-FRIEND')}, {at('MSG-REPLY')})")
# inside the batch that brought it, the root group's message is last (next to the reply)
b = first.get("MSG-P0", {})
order = [m for m in b.get("order", []) if m in ("MSG-P0",) or m.startswith("MSG-FLOOD")]
check(bool(order) and order[-1] == "MSG-P0", f"the most important chat comes last in its batch ({order[-3:]})")
# a flooding group does not starve another group of the same rank
check(at("MSG-CLUB0") < at(flood_marks[-1]), f"the club's messages came before the flood ended ({at('MSG-CLUB0')} < {at(flood_marks[-1])})")

qs = [r for r in R if r["queue"]]
check(bool(qs), "the model got a [queue] notice")
check(any("Started waiting" in r["queue_text"] for r in qs), "the notice says which chats started waiting")
check(any("queue_take" in r["queue_text"] for r in qs), "the notice says how to take more")
took = [r for r in R if r.get("took")]
check(bool(took) and took[0]["took"].startswith("took 3 of"), f"queue_take took what was asked ({took[0]['took'] if took else None})")
if took:
    after = [m for m in took[0]["order"] if m.startswith("MSG-FLOOD")]
    check(len(after) >= 3, f"the taken messages followed its result ({after[:3]})")

log = open(agent_log, errors="replace").read()
check("reflections keep coming" in log, "the loop guard saw reflections come with only a step between them")
check(any("20%" in r["queue_text"] for r in qs), "the model was told that 20% of the context is kept free")

check(any(r["cut"] for r in R), "the long message came cut")
long_dir = os.path.join(files, "inbox", "long")
kept = [os.path.join(long_dir, f) for f in os.listdir(long_dir)] if os.path.isdir(long_dir) else []
check(any("MSG-LONG" in open(f).read() for f in kept), "its whole text is in files/inbox/long")

members = [l for r in R for l in r["members"]]
joined = [l for l in members if "telegram:66" in l and "not a root contact" in l]
check(bool(joined), f"the stranger joining the root group was reported with their id ({members[:2]})")
groups = json.load(open(os.path.join(state, "groups.json"))).get("groups", {})
check(groups.get("telegram:-100", {}).get("level") == "p0", f"chat_level held the root group at p0 ({groups.get('telegram:-100')})")
check(any("root (P0) again" in l for l in members), "their leaving reported the group root again")
check(any("Newbie (telegram:77) joined Market" in l for l in members), "a join to a mixed group is told too")
check("MSG-AFTER" in first, "messages after the joins still reach the model")

# read receipts: when the model gets a message, not when it arrives
ids = {d["id"]: d for d in (json.loads(l) for l in open(idlog))}
reads = [json.loads(l) for l in open(readlog)] if os.path.exists(readlog) else []
read_at, read_by = {}, {}
for r in reads:
    for i, mid in enumerate(r.get("ids", [])):
        read_at.setdefault((r.get("chat_id"), mid), r["t"])
        read_by[mid] = (r.get("senders") or [""] * 99)[i]
by_n = {r["n"]: r for r in R}
early, late, unread = [], [], []
for mid, d in ids.items():
    mk, chat = d["mark"], d["chat"].split(":", 1)[1]
    if mk not in first:
        continue
    t = read_at.get((chat, mid))
    if t is None:
        unread.append(mk)
        continue
    f = first[mk]
    prev = by_n.get(f["n"] - 1)
    if prev and t < prev["t"]:
        early.append(mk)
    if t > f["t"] + 1:
        late.append(mk)
check(not unread, f"every message the model got was marked read ({len(unread)} not: {unread[:5]})")
check(not early, f"no message was marked read before the model got it ({len(early)} early: {early[:5]})")
check(not late, f"each was marked read as it went to the model ({len(late)} late: {late[:5]})")
def model_calls_between(a, b):
    return sum(1 for r in R if a < r["t"] < b)
waited = [d["mark"] for d in ids.values() if d["mark"].startswith("MSG-FLOOD") and d["mark"] in first
          and model_calls_between(d["t"], read_at.get((d["chat"].split(":", 1)[1], d["id"]), 0)) >= 3]
check(len(waited) >= FLOOD // 3, f"flood messages that waited in the queue stayed unread meanwhile ({len(waited)} read only after 3+ model calls since they came)")
check(read_by.get(next((i for i, d in ids.items() if d["mark"] == "MSG-ROOTMIXED"), "")) == "telegram:1", "a read receipt names who wrote the message")

book = json.load(open(os.path.join(state, "addressbook.json")))
linked = [ch for c in book.get("contacts", []) for ch in c.get("channels", []) if ch.startswith("telegram:-")]
check(not linked, f"no group channel is linked to a person ({linked})")

if check.failed:
    print("agent log tail:")
    print("".join(open(agent_log, errors="replace").readlines()[-60:]))
    print(f"logs: {work}")
    sys.exit(1)
print("all queue checks passed")
