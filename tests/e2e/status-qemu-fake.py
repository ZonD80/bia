#!/usr/bin/env python3
"""Fake telegram MCP for tests/e2e/status-qemu.sh: runs inside the bia VM in place of the real
one (a throwaway copy of the image), with a real cloud model deciding what to do.

Timeline (after the agent restores the bus):
  1. group "Trip" (telegram:-100): Anna asks for flights, Ben for a table; the tools take
     SLOW seconds (search_flights, book_table), so statuses fall due while they run;
  2. once both jobs ended and the chat settled: a voice call with Carl in Spanish
     ([call ready], a transcribed phrase); his search takes CALL_SLOW seconds. bia's speech
     into the call plays for as long as it would be spoken (~14 chars/s), then
     [you finished speaking]; a new one cuts the old ([voice cancelled]);
  3. [call ended], then a system line STATUS-E2E-DONE on the console for the driver.
Logs (JSON lines) in /home/bia/files/status-e2e/: mcp (every tools/call), speech, inbound.
"""
import json, os, sys, threading, time

D = "/home/bia/files/status-e2e"
os.makedirs(D, exist_ok=True)
SLOW, CALL_SLOW = 45, 60
LOG = open(f"{D}/mcp.jsonl", "a", buffering=1)
SPEECH = open(f"{D}/speech.jsonl", "a", buffering=1)
IN = open(f"{D}/inbound.jsonl", "a", buffering=1)
lock = threading.Lock()
call_on = {"7": False}
jobs = {"running": 0, "done": []}
speak_gen, playing = [0], [False]
started = [False]


def out(v):
    with lock:
        sys.stdout.write(json.dumps(v) + "\n")
        sys.stdout.flush()


def note(d):
    out({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": d}})


seq = [0]


def msg(chat, sender, name, text, group=False, mid=None):
    seq[0] += 1
    d = {"t": "inbound", "identity": f"{name} ({sender.split(':')[1]})" if sender else name, "channel": chat,
         "id": str(1000 + seq[0]) if mid is None else mid, "message": text, "issued": int(time.time()), "sender": sender}
    if group:
        d["group"] = True
    IN.write(json.dumps({"t": time.time(), "chat": chat, "text": text}) + "\n")
    note(d)


def system(chat, text):
    msg(chat, "", "system", text, mid="")


def speak(chat, text):
    with lock:
        if playing[0]:
            SPEECH.write(json.dumps({"cut": time.time(), "gen": speak_gen[0], "by": text}) + "\n")
        speak_gen[0] += 1
        g = speak_gen[0]
        playing[0] = True
    secs = max(1.0, len(text) / 14)
    SPEECH.write(json.dumps({"start": time.time(), "secs": secs, "gen": g, "text": text}) + "\n")

    def done():
        time.sleep(secs)
        if speak_gen[0] == g:
            playing[0] = False
            SPEECH.write(json.dumps({"end": time.time(), "gen": g}) + "\n")
            system("telegram:" + chat, "[you finished speaking]")
    threading.Thread(target=done, daemon=True).start()


def wait_jobs(n, settle, cap):
    """Until n slow jobs ended (then `settle` s more for the answers), at most `cap` s."""
    t = time.time()
    while time.time() - t < cap and len(jobs["done"]) < n:
        time.sleep(1)
    time.sleep(settle)


def script():
    time.sleep(8)
    msg("telegram:-100", "telegram:11", "Anna", "Milo, can you find me the cheapest flight from Barcelona to Berlin this Friday?", True)
    time.sleep(6)
    msg("telegram:-100", "telegram:12", "Ben", "Milo, and please book a table for two at Trattoria Sole on Friday at 8 pm, under Ben.", True)
    wait_jobs(2, 40, 300)
    call_on["7"] = True
    msg("telegram:7", "telegram:7", "Carl", "[call ready]")
    time.sleep(2)
    msg("telegram:7", "telegram:7", "Carl", "[speech es] Hola Milo, ¿me buscas un vuelo de Barcelona a Madrid para mañana por la mañana?", mid="")
    wait_jobs(3, 40, 300)
    call_on["7"] = False
    msg("telegram:7", "telegram:7", "Carl", "[call ended]")
    time.sleep(5)
    system("console", "STATUS-E2E-DONE")


def slow(name, args, secs):
    jobs["running"] += 1
    time.sleep(secs)
    jobs["running"] -= 1
    jobs["done"].append(name)


def handle(req):
    m, rid = req.get("method"), req.get("id")
    p = req.get("params") or {}
    if m == "initialize":
        out({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                                                     "serverInfo": {"name": "fake-telegram", "version": "0"}}})
    elif m == "tools/list":
        obj = lambda props, req=(): {"type": "object", "properties": props, "required": list(req)}
        s = {"type": "string"}
        out({"jsonrpc": "2.0", "id": rid, "result": {"tools": [
            {"name": "restore", "description": "Start the bus.", "inputSchema": obj({})},
            {"name": "read", "description": "Read receipts.", "inputSchema": obj({})},
            {"name": "typing", "description": "Typing indicator.", "inputSchema": obj({})},
            {"name": "send", "description": "Send a message.", "inputSchema": obj({"chat_id": s, "text": s})},
            {"name": "call_say", "description": "Speak into the active call.", "inputSchema": obj({"chat_id": s, "text": s, "lang": s})},
            {"name": "search_flights", "description": "Search flights and prices between two cities on a date (live airline data; slow: about 45-60 seconds).",
             "inputSchema": obj({"from": s, "to": s, "date": s}, ("from", "to", "date"))},
            {"name": "book_table", "description": "Book a restaurant table (calls the restaurant; slow: about 45 seconds).",
             "inputSchema": obj({"restaurant": s, "people": {"type": "integer"}, "when": s, "name": s}, ("restaurant", "people", "when"))},
        ]}})
    elif m == "tools/call":
        name, args = p.get("name"), p.get("arguments") or {}
        LOG.write(json.dumps({"t": time.time(), "name": name, "args": args}) + "\n")
        err = False
        if name == "search_flights":
            to = str(args.get("to", ""))
            if "madrid" in to.lower():
                slow(name, args, CALL_SLOW)
                text = "Cheapest tomorrow morning: Vueling VY1003 Barcelona 07:15 → Madrid 08:30, 54 EUR; Iberia IB2711 09:00 → 10:15, 71 EUR."
            else:
                slow(name, args, SLOW)
                text = "Cheapest on Friday: Vueling VY1890 Barcelona 07:40 → Berlin 10:25, 89 EUR; Eurowings EW8031 13:10 → 15:55, 112 EUR."
        elif name == "book_table":
            slow(name, args, SLOW)
            text = "Booked: Trattoria Sole, 2 people, Friday 20:00, under Ben. Confirmation TS-4471."
        elif name == "call_say":
            on = call_on.get(str(args.get("chat_id")), False)
            if on:
                speak(str(args.get("chat_id")), args.get("text", ""))
            text, err = ("speaking" if on else "no active call"), not on
        elif name == "send":
            text = json.dumps({"id": str(int(time.time() * 1000))})
        else:
            text = "ok"
        out({"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": text}], "isError": err}})
        if name == "restore" and not started[0]:
            started[0] = True
            threading.Thread(target=script, daemon=True).start()
    elif rid is not None:
        out({"jsonrpc": "2.0", "id": rid, "result": {}})


for line in sys.stdin:
    req = json.loads(line)
    if req.get("id") is None:
        continue
    threading.Thread(target=handle, args=(req,), daemon=True).start()
