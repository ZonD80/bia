#!/usr/bin/env python3
"""Checks of tests/e2e/status-qemu.sh on what came off the stick (dir from argv[1]): the fake
bus's logs (status-e2e/*.jsonl) and the agent's conversation.json, with a real model.

What the agent must do whatever the model chooses:
- for_ids never reaches the MCP; every slow job is tagged with whose message it serves;
- a job result names whose it is;
- a chat is never asked for a status twice within its interval (reminders and job results
  together): 20 s in the group, 10 s in the call;
- in the call nothing is asked over bia's own words: no reminder and no "heard nothing" while
  it speaks, and the spoken fallback never cuts it;
- the people waiting hear something while the slow jobs run.
What the model did (statuses, cuts it made itself) is reported too.
"""
import datetime, json, os, re, sys

d = sys.argv[1]
failed = []


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        failed.append(what)


def info(what):
    print("info " + what, flush=True)


def load(name):
    p = os.path.join(d, "status-e2e", name)
    return [json.loads(l) for l in open(p)] if os.path.exists(p) else []


M, S, I = load("mcp.jsonl"), load("speech.jsonl"), load("inbound.jsonl")
conv = json.load(open(os.path.join(d, "conversation.json")))
hist = conv.get("history") or conv.get("messages") or []


def ts(s):
    return datetime.datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


ENV = re.compile(r"^([^|\n]*)\|([^|\n]*)\|([^|\n]*)\|([^|\n]*)\|([^|\n]*)\|", re.M)
envs = []  # (delivered time, identity, channel, text) of every envelope the model got
for m in hist:
    if m.get("role") != "user":
        continue
    c = m.get("content", "")
    for e in ENV.finditer(c):
        start = e.end()
        nxt = ENV.search(c, start)
        envs.append((ts(e.group(4)), e.group(1), e.group(2), c[start: nxt.start() if nxt else len(c)]))
tools_out = [m.get("content", "") for m in hist if m.get("role") == "tool"]
print(f"     {len(hist)} messages, {len(envs)} envelopes, {len(M)} MCP calls, {sum('start' in x for x in S)} utterances")

slow = [m for m in M if m["name"] in ("search_flights", "book_table")]
check(len(slow) >= 3, f"the model ran the slow tools ({[(m['name'], m['args'].get('to', m['args'].get('restaurant'))) for m in slow]})")
check(all("for_ids" not in m["args"] for m in slow), "for_ids never reached the MCP")
stubs = [t for t in tools_out if "accepted job:" in t and ("search_flights" in t or "book_table" in t or " for " in t)]
tagged = [t for t in tools_out if re.search(r"accepted job:\w+ for \S+ \(", t)]
check(len(tagged) >= len(slow), f"every slow job was tagged with whose message it serves ({len(tagged)} tagged, {len(slow)} slow)")
for t in tagged:
    info("tag: " + re.search(r"accepted job:\w+ for [^;]*", t).group(0))
results = [e for e in envs if e[2].startswith("job:") and ("Cheapest" in e[3] or "Booked" in e[3])]
check(len(results) >= 3 and all("[for " in e[3] for e in results), f"each slow job's result names whose it is ({sum('[for ' in e[3] for e in results)} of {len(results)})")

# asks: reminders listing the chat and job results asking for a status there
def asks(ch):
    out = [e[0] for e in envs if e[3].lstrip().startswith("unanswered:") and f"channel={ch} " in e[3]]
    out += [e[0] for e in envs if f"{ch} has heard nothing" in e[3]]
    return sorted(out)

for ch, gap in (("telegram:-100", 19), ("telegram:7", 9)):
    a = asks(ch)
    close = [round(b - x, 1) for x, b in zip(a, a[1:]) if b - x < gap]
    check(not close, f"{ch}: never asked twice within {gap + 1}s ({len(a)} asks{', too close: ' + str(close) if close else ''})")

# the call: bia's speech intervals
speeches = []
for x in S:
    if "start" in x:
        end = next((y.get("end") or y.get("cut") for y in S if y.get("gen") == x["gen"] and ("end" in y or "cut" in y)), None)
        if end is None:  # cut by the next one
            nxt = next((y for y in S if y.get("gen") == x["gen"] + 1 and "start" in y), None)
            end = nxt["start"] if nxt else x["start"] + x["secs"]
        speeches.append((x["start"], end, x["text"]))
for a, b, t in speeches:
    info(f"spoke {b - a:4.1f}s: {t[:90]}")


def during(t, slack=1.5):
    # envelope times are whole seconds: allow for that
    return any(a + slack < t < b - slack for a, b, _ in speeches)

call_asks = asks("telegram:7")
over = [round(t, 1) for t in call_asks if during(t)]
check(not over, f"no reminder or status request came while bia spoke into the call ({len(call_asks)} asks{', over its words: ' + str(over) if over else ''})")
fb = [m for m in M if m["name"] == "call_say" and m["args"].get("text") == "Sigo trabajando en ello."]
check(not any(during(m["t"], 0.3) for m in fb), f"the spoken fallback never cut bia's own words ({len(fb)} spoken)")
blank = [m for m in M if m["name"] == "send" and not str(m["args"].get("text", "")).strip() and not m["args"].get("files")]
check(not blank, f"no empty message was sent ({len(blank)})")
held = [t for t in tools_out if t.startswith("not said: you are still speaking")]
info(f"statuses the agent held back while bia spoke (they would have cut it): {len(held)}")
cuts = [x for x in S if "cut" in x]
by_fb = [x for x in cuts if x["by"] == "Sigo trabajando en ello."]
check(not by_fb, f"no utterance was cut by bia's automatic fallback ({len(by_fb)})")
answers = [x for x in cuts if "answered" in x.get("by", "")]
info(f"utterances cut by a later say: {len(cuts) - len(by_fb)} (an answer may interrupt on purpose)")

# people heard something while the slow jobs ran
t_anna = next((x["t"] for x in I if "Anna" in x.get("text", "") or "flight from Barcelona to Berlin" in x.get("text", "")), None)
t_carl = next((x["t"] for x in I if x.get("text", "").startswith("[speech es]")), None)
group_sends = [m for m in M if m["name"] == "send" and str(m["args"].get("chat_id")) == "-100"]
call_says = [m for m in M if m["name"] == "call_say" and str(m["args"].get("chat_id")) == "7"]
for m in group_sends:
    info(f"group +{m['t'] - (t_anna or m['t']):5.1f}s: {m['args'].get('text', '')[:110]}")
for m in call_says:
    info(f"call  +{m['t'] - (t_carl or m['t']):5.1f}s: {m['args'].get('text', '')[:110]}")
g_res = [e[0] for e in results if "Berlin" in e[3] or "Booked" in e[3]]
c_res = [e[0] for e in results if "Madrid" in e[3]]
if t_anna and g_res:
    during_wait = [m for m in group_sends if t_anna < m["t"] < max(g_res)]
    check(bool(during_wait), f"the group heard something while its jobs ran ({len(during_wait)} messages)")
if t_carl and c_res:
    during_wait = [m for m in call_says if t_carl < m["t"] < min(c_res)]
    check(bool(during_wait), f"Carl heard something while his search ran ({len(during_wait)} utterances)")
    silences = []
    marks = sorted([t_carl] + [b for a, b, _ in speeches if t_carl < a < min(c_res)])
    starts = sorted([a for a, b, _ in speeches if t_carl < a < min(c_res)])
    for s in starts:
        prev = max([m for m in marks if m <= s], default=t_carl)
        silences.append(round(s - prev, 1))
    info(f"silences before each utterance in the call (s): {silences}")

left = json.load(open(os.path.join(d, "unanswered.json"))).get("open", []) if os.path.exists(os.path.join(d, "unanswered.json")) else []
info(f"left open at the end: {[(e['channel'], e['identity']) for e in left]}")

if failed:
    print(f"FAILED: {len(failed)} check(s)")
    sys.exit(1)
print("all status checks passed (real model in QEMU)")
