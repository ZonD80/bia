"""Who wrote a message and who is in a group, as the chat MCPs tell the agent, without real
accounts: bia-mcp-messenger over stdio with a fake bia-meta, and MAX's control-message parsing
from bia_max.py on fake attachments.

Checks that a group message carries the sender's id in channel form and the group flag, that a
direct message carries only the id; that the group's member list (bia left out) reaches the
agent before the group's first message and only once; that joins and leaves become member events
with ids in channel form, and bia's own are kept back; and that MAX turns add, remove, joins
by link and leaving into joins and leaves. Read receipts: nothing is marked read on arrival,
only the messages the agent names in `read` (when the model got them).

  cargo build -p bia-mcp-meta
  python3 -I tests/integration/members-e2e.py <dir with bia-mcp-messenger> <bia_max.py>
"""
import ast, json, os, subprocess, sys, tempfile, threading, time

bins, max_py = sys.argv[1], sys.argv[2]

def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        check.failed = True
check.failed = False

FAKE = r'''#!/usr/bin/env python3
import json, os, sys
log = open(os.environ["FAKE_LOG"], "a", buffering=1)
def out(v):
    sys.stdout.write(json.dumps(v) + "\n"); sys.stdout.flush()
for line in sys.stdin:
    req = json.loads(line)
    m, p = req.get("method"), req.get("params") or {}
    log.write(json.dumps({"method": m, "params": p}) + "\n")
    res = {}
    if m == "members":
        res = {"title": "Family", "members": [{"id": "42", "name": "Anna"}, {"id": "43", "name": "Ben"}, {"id": "900", "name": "bia"}]}
    out({"id": req["id"], "result": res})
    if m == "restore":
        for ev in json.load(open(os.environ["FAKE_EVENTS"])):
            out(ev)
'''

files = tempfile.mkdtemp(prefix="bia-members-e2e-")
work = tempfile.mkdtemp(prefix="bia-members-fake-")
fake = os.path.join(work, "bia-meta")
open(fake, "w").write(FAKE)
os.chmod(fake, 0o755)
calls = os.path.join(work, "calls.jsonl")
os.makedirs(os.path.join(files, "state"))
json.dump({"logged_in": True, "last_seen": 1, "id": "900"}, open(os.path.join(files, "state", "messenger.json"), "w"))

now = int(time.time())
def msg(chat, mid, sender, text, group):
    return {"event": "message", "chat": chat, "id": mid, "sender": sender, "sender_name": "Anna", "text": text, "ts": now, "group": group}
events = [
    msg("100", "m1", "42", "hello in private", False),
    msg("t200", "m2", "42", "hello in the group", True),
    msg("t200", "m3", "43", "and again", True),
    {"event": "member", "chat": "t200", "op": "join", "id": "44", "name": "Cleo", "me": False},
    {"event": "member", "chat": "t200", "op": "leave", "id": "43", "name": "Ben", "me": False},
    {"event": "member", "chat": "t300", "op": "join", "id": "900", "name": "bia", "me": True},
    msg("t200", "m4", "44", "hi all", True),
]
events_path = os.path.join(work, "events.json")
json.dump(events, open(events_path, "w"))

env = dict(os.environ, BIA_FILES=files, BIA_META_BIN=fake, FAKE_LOG=calls, FAKE_EVENTS=events_path)
p = subprocess.Popen([os.path.join(bins, "bia-mcp-messenger")], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=open(os.path.join(work, "mcp.log"), "w"), env=env, text=True, bufsize=1)
notes, results, rid = [], {}, [0]

def reader():
    for line in p.stdout:
        v = json.loads(line)
        d = (v.get("params") or {}).get("data")
        if isinstance(d, dict) and d.get("t") in ("inbound", "members", "member"):
            notes.append(d)
        elif "id" in v:
            results[v["id"]] = v
threading.Thread(target=reader, daemon=True).start()

def rpc(method, params=None):
    rid[0] += 1
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": rid[0], "method": method, "params": params or {}}) + "\n")
    p.stdin.flush()
    for _ in range(1200):
        if rid[0] in results:
            return results[rid[0]]
        time.sleep(0.05)
    raise SystemExit(f"no answer to {method}")

rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}})
rpc("tools/call", {"name": "restore", "arguments": {}})
for _ in range(400):
    if any(n.get("id") == "m4" for n in notes):
        break
    time.sleep(0.05)
time.sleep(0.5)
def helper_calls(method):
    return [json.loads(l)["params"] for l in open(calls) if json.loads(l)["method"] == method]
check(helper_calls("read") == [], f"nothing is marked read on arrival ({helper_calls('read')})")
r = rpc("tools/call", {"name": "read", "arguments": {"chat_id": "t200", "ids": ["m2", "m3"], "senders": ["messenger:42", "messenger:43"]}})
for _ in range(100):
    if len(helper_calls("read")) >= 2:
        break
    time.sleep(0.05)
got = [(c.get("chat"), c.get("id")) for c in helper_calls("read")]
check(got == [("t200", "m2"), ("t200", "m3")], f"the agent's read marks those messages read ({got})")
p.kill()

def inbound(mid):
    return next((n for n in notes if n.get("t") == "inbound" and n.get("id") == mid), {})

m1, m2, m3, m4 = inbound("m1"), inbound("m2"), inbound("m3"), inbound("m4")
check(m1.get("sender") == "messenger:42" and "group" not in m1, f"a direct message carries the sender's id only ({m1.get('sender')}, {m1.get('group')})")
check(m2.get("sender") == "messenger:42" and m2.get("group") is True, f"a group message carries the sender's id and the group flag ({m2.get('sender')}, {m2.get('group')})")
check(m3.get("sender") == "messenger:43", "each sender by their own id")
snaps = [n for n in notes if n.get("t") == "members"]
check(len(snaps) == 1 and snaps[0]["channel"] == "messenger:t200", f"one member list, for the group ({[s.get('channel') for s in snaps]})")
if snaps:
    ids = sorted(x["id"] for x in snaps[0]["members"])
    check(ids == ["messenger:42", "messenger:43"] and snaps[0]["complete"] is True, f"members in channel form, bia left out, complete ({ids})")
    check(notes.index(snaps[0]) < notes.index(m2), "the list comes before the group's first message")
ch = [n for n in notes if n.get("t") == "member"]
check([(c["op"], c["id"], c["channel"]) for c in ch] == [("join", "messenger:44", "messenger:t200"), ("leave", "messenger:43", "messenger:t200")],
      f"joins and leaves with ids in channel form, bia's own kept back ({[(c.get('op'), c.get('id')) for c in ch]})")
check(m4.get("sender") == "messenger:44", "the newcomer's message carries their id")
asked = [json.loads(l)["params"].get("chat") for l in open(calls) if json.loads(l)["method"] == "members"]
check(asked == ["t200"], f"the helper is asked for the member list once ({asked})")

# MAX: control_members from bia_max.py on fake attachments
src = open(max_py).read()
tree = ast.parse(src)
code = [n for n in tree.body if (isinstance(n, ast.FunctionDef) and n.name == "control_members") or
        (isinstance(n, ast.Assign) and any(getattr(t, "id", "") == "CONTROL_OPS" for t in n.targets))]
ns = {"enum_value": lambda v: getattr(v, "value", v)}
exec(compile(ast.Module(body=code, type_ignores=[]), "bia_max.py", "exec"), ns)

class A:
    def __init__(self, event, **extra):
        self.type, self.event, self.extra = "CONTROL", event, extra
    def model_dump(self, by_alias=False):
        return dict(self.extra, event=self.event)
class M:
    def __init__(self, sender, attaches):
        self.sender, self.attaches = sender, attaches
cm = ns["control_members"]
check(cm(M(5, [A("add", userIds=[7, 8])])) == [("join", 7), ("join", 8)], "MAX: add names the people added")
check(cm(M(5, [A("remove", userId=7)])) == [("leave", 7)], "MAX: remove names the person removed")
check(cm(M(9, [A("joinByLink")])) == [("join", 9)], "MAX: joining by link is the sender joining")
check(cm(M(9, [A("leave")])) == [("leave", 9)], "MAX: leaving is the sender leaving")
check(cm(M(9, [A("title", title="x")])) == [], "MAX: a new title is no member change")

sys.exit(1 if check.failed else 0)
