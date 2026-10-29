"""Ignored chats end to end, without a Meta account: bia-mcp-messenger over stdio with a fake
bia-meta that sends messages in and records which ones the MCP marks read.

Checks that a chat in files/state/ignore.json gets its messages flagged "ignored" and left
unread, that a wake pattern, a reply to bia and a wake word spoken in a voice note (matched
on the transcript) deliver them flagged "woke" and read, and that other chats are untouched.

  cargo build -p bia-mcp-meta
  python3 -I tests/integration/ignore-e2e.py <dir with bia-mcp-messenger> [<voice note .ogg> <word spoken in it>]

The voice note needs bia-speech's models (BIA_SPEECH_DIR, BIA_SPEECH_STORE, LD_LIBRARY_PATH as
for the speech examples); without it those checks are left out.
"""
import json, os, subprocess, sys, tempfile, threading, time

bins = sys.argv[1]
voice, word = (os.path.abspath(sys.argv[2]), sys.argv[3]) if len(sys.argv) > 3 else (None, "zzz-no-voice")

def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what)
    if not cond:
        check.failed = True
check.failed = False

# the fake bia-meta: answers calls, copies the voice note on "download", and after
# "restore" sends the messages in FAKE_EVENTS
FAKE = r'''#!/usr/bin/env python3
import json, os, shutil, sys
log = open(os.environ["FAKE_LOG"], "a", buffering=1)
def out(v):
    sys.stdout.write(json.dumps(v) + "\n"); sys.stdout.flush()
for line in sys.stdin:
    req = json.loads(line)
    m, p = req.get("method"), req.get("params") or {}
    log.write(json.dumps({"method": m, "params": p}) + "\n")
    res = {}
    if m == "download":
        os.makedirs(os.path.dirname(p["path"]), exist_ok=True)
        shutil.copy(os.environ["FAKE_VOICE"], p["path"]); res = {"mime": "audio/ogg"}
    out({"id": req["id"], "result": res})
    if m == "restore":
        for ev in json.load(open(os.environ["FAKE_EVENTS"])):
            out(ev)
'''

files = tempfile.mkdtemp(prefix="bia-ignore-e2e-")
work = tempfile.mkdtemp(prefix="bia-ignore-fake-")
fake = os.path.join(work, "bia-meta")
open(fake, "w").write(FAKE)
os.chmod(fake, 0o755)
calls = os.path.join(work, "calls.jsonl")
os.makedirs(os.path.join(files, "state"))
# logged in already (restore then starts the helper)
json.dump({"logged_in": True, "last_seen": 1}, open(os.path.join(files, "state", "messenger.json"), "w"))
json.dump({"chats": {"messenger:t200": {"since": 1, "wake": [r"\bfred\b", word]}, "messenger:t300": {"since": 1, "wake": ["zebra"]}}}, open(os.path.join(files, "state", "ignore.json"), "w"))

now = int(time.time())
def msg(chat, mid, text, **kw):
    return {"event": "message", "chat": chat, "id": mid, "sender": "42", "sender_name": "Anna", "text": text, "ts": now, "group": chat.startswith("t"), **kw}
events = [
    msg("100", "m1", "hello there"),
    msg("t200", "m2", "the price is up again"),
    msg("t200", "m3", "what does Fred think?"),
    msg("t200", "m4", "really?", reply={"id": "m0", "me": True, "text": "ok"}),
]
if voice:
    events += [
        msg("t200", "m5", "", attachments=[{"kind": "voice", "ref": "v1", "mime": "audio/ogg"}]),
        # the same voice note in a chat whose wake word it does not say: transcribed, still ignored
        msg("t300", "m6", "", attachments=[{"kind": "voice", "ref": "v2", "mime": "audio/ogg"}]),
    ]
last = "m6" if voice else "m4"
events_path = os.path.join(work, "events.json")
json.dump(events, open(events_path, "w"))

env = dict(os.environ, BIA_FILES=files, BIA_META_BIN=fake, FAKE_LOG=calls, FAKE_VOICE=voice or "", FAKE_EVENTS=events_path)
p = subprocess.Popen([os.path.join(bins, "bia-mcp-messenger")], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=open(os.path.join(work, "mcp.log"), "w"), env=env, text=True, bufsize=1)
inbound, results, rid = [], {}, [0]

def reader():
    for line in p.stdout:
        v = json.loads(line)
        d = (v.get("params") or {}).get("data")
        if isinstance(d, dict) and d.get("t") == "inbound":
            inbound.append(d)
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

def by_id(mid):
    return next((d for d in inbound if d.get("id") == mid), None)

# voice notes are transcribed: wait for the last one (and its read mark)
for _ in range(1200):
    if by_id(last):
        break
    time.sleep(0.1)
time.sleep(1)
p.kill()
reads = [json.loads(l)["params"].get("id") for l in open(calls) if json.loads(l)["method"] == "read"]

m = {k: by_id(k) for k in ["m1", "m2", "m3", "m4", "m5", "m6"]}
check(m["m1"] and "ignored" not in m["m1"] and "woke" not in m["m1"] and "m1" in reads, "another chat: delivered as before and read")
check(m["m2"] and m["m2"].get("ignored") is True and "m2" not in reads, "ignored chat: flagged ignored, left unread")
check(m["m3"] and m["m3"].get("woke", "").startswith("wake pattern") and "m3" in reads, f"wake pattern: woke ({m['m3'] and m['m3'].get('woke')}) and read")
check(m["m4"] and m["m4"].get("woke") == "a reply to you" and "m4" in reads, "reply to bia: woke and read")
if not voice:
    sys.exit(1 if check.failed else 0)
v5 = m["m5"] or {}
check(word.lower() in v5.get("message", "").lower() and "[speech" in v5.get("message", ""), f"voice note transcribed: {v5.get('message', '')[:120]!r}")
check(v5.get("woke") == f"wake pattern {word}" and "m5" in reads, "wake word spoken in a voice note: woke and read")
v6 = m["m6"] or {}
check(v6.get("ignored") is True and "[speech" in v6.get("message", "") and "m6" not in reads, "voice note without the wake word: transcribed, ignored, unread")
print("transcript:", v5.get("message", "").replace("\n", " ")[:300])
sys.exit(1 if check.failed else 0)
