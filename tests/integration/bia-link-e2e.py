"""End-to-end: two bias on the web bus (real bia-mcp-web processes, mutual TLS on localhost).

alex (8443) and fred (8444) set up trust with one approval, then the link opens by itself and
each is an ordinary identity on the other's web chat. Checks: who dials (the lower name),
messages both ways with the same ids, replies, edits and reactions, files both ways (and one
resumed from a partial download), store and forward while the other is down or disconnected,
the roles swapped (fred dials), bans (the model's and root's), bye on SIGTERM and the restarted
side coming back, a copy of a bia with the same key refused, and removing a peer.

  docker run --rm --platform linux/arm64 -v $PWD:/work -v bia-cache-arm64:/cache -w /work \\
    -e CARGO_TARGET_DIR=/cache/target-arm64-test bia-build:arm64 bash -lc \\
    'cargo build -q -p bia-mcp-web && python3 -I tests/integration/bia-link-e2e.py /cache/target-arm64-test/debug/bia-mcp-web'
"""
import hashlib, json, os, shutil, signal, subprocess, sys, tempfile, threading, time

exe = sys.argv[1]
root = tempfile.mkdtemp(prefix="bia-link-e2e-")
# as root (docker) the files folders belong to user bia, as on a device
BIA_UID = 1000 if os.geteuid() == 0 else os.geteuid()
failed = []


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        failed.append(what)


class Bia:
    def __init__(self, name, port, files=None):
        self.name, self.port = name, port
        self.files = files or os.path.join(root, name)
        os.makedirs(f"{self.files}/state/pki", exist_ok=True)
        if not os.path.exists(f"{self.files}/state/pki/agent.json"):
            json.dump({"name": name}, open(f"{self.files}/state/pki/agent.json", "w"))
        if os.geteuid() == 0:
            for dp, dirs, fs in os.walk(self.files):
                for x in [dp] + [os.path.join(dp, f) for f in fs]:
                    os.chown(x, BIA_UID, BIA_UID)
        self.start()

    def start(self):
        env = dict(os.environ, BIA_FILES=self.files, BIA_WEB_PORT=str(self.port), BIA_WEB_HTTP_PORT="0")
        self.p = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, text=True, bufsize=1)
        self.notes, self.results, self.rid, self.seen = [], {}, 0, 0
        threading.Thread(target=self._read, daemon=True).start()
        self.rpc("initialize")
        for _ in range(200):
            if os.path.exists(f"{self.files}/state/web.json"):
                break
            time.sleep(0.05)

    def _read(self):
        for line in self.p.stdout:
            try:
                v = json.loads(line)
            except ValueError:
                continue
            if v.get("method") == "notifications/message":
                d = v["params"]["data"]
                if isinstance(d, dict) and d.get("t") == "inbound":
                    self.notes.append(d)
            elif "id" in v:
                self.results[v["id"]] = v

    def rpc(self, method, params=None, wait=60):
        self.rid += 1
        i = self.rid
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n")
        self.p.stdin.flush()
        end = time.time() + wait
        while time.time() < end:
            if i in self.results:
                return self.results.pop(i)
            time.sleep(0.02)
        raise SystemExit(f"{self.name}: no reply to {method}")

    def tool(self, name, **args):
        r = self.rpc("tools/call", {"name": name, "arguments": args})["result"]
        return r["content"][0]["text"], r.get("isError", False)

    def say(self, to, text, **kw):
        return self.tool("send", identity=to, channel="web", text=text, **kw)

    def wait(self, pred, secs=30):
        """The next inbound (after the last one taken) that matches."""
        end = time.time() + secs
        while time.time() < end:
            while self.seen < len(self.notes):
                n = self.notes[self.seen]
                self.seen += 1
                if pred(n):
                    return n
            time.sleep(0.05)
        return None

    def notice(self, needle, secs=30):
        return self.wait(lambda n: n["identity"] == "system" and n["channel"] == "web" and needle in n["message"], secs)

    def msg(self, peer, needle, secs=30):
        return self.wait(lambda n: n["identity"] == peer and n["channel"] == "web" and needle in n["message"], secs)

    def link(self, peer):
        txt, _ = self.tool("peer_list")
        line = next((l for l in txt.splitlines() if l.strip().startswith("link:") and f'identity=\\"{peer}\\"' in l or f'identity="{peer}"' in l), "")
        return line

    def stop(self, sig=signal.SIGTERM):
        self.p.send_signal(sig)
        try:
            self.p.wait(10)
        except subprocess.TimeoutExpired:
            self.p.kill()


def files_of(n):
    return n and [l[2:].split(" (")[0] for l in n["message"].splitlines() if l.startswith("- ")]


alex = Bia("alex", 8443)
fred = Bia("fred", 8444)

# ---- trust with one approval; the link opens by itself, alex (the lower name) dials
txt, err = alex.tool("peer_add", url="https://127.0.0.1:8444", note="alex here")
check(not err and "Asked fred" in txt, f"peer_add: {txt[:100]}")
reqs = json.load(open(f"{fred.files}/state/pki/requests/" + os.listdir(f"{fred.files}/state/pki/requests")[0]))
txt, err = fred.tool("pki_approve", id=reqs["id"])
check(not err and 'identity=\\"alex\\"' in txt or 'identity="alex"' in txt, f"approve: {txt[:200]}")
check(fred.notice("[web open] alex connected", 60) is not None, "fred: alex connected")
check(alex.notice("[web open] fred connected", 60) is not None, "alex: fred connected")
# fred knows alex first (it approved), so it dials before alex has collected its certificate
for _ in range(120):
    txt, _ = alex.tool("peer_list")
    if "fred  active" in txt:
        break
    time.sleep(0.5)
check("fred  active" in txt and "online since" in txt, f"alex has fred active and online: {txt}")


def dialer():
    """(client, server) of the link right now."""
    txt, _ = alex.tool("peer_list")
    return (alex, fred) if "dialed to" in txt else (fred, alex)

# ---- messages both ways, like with anyone on web
txt, err = alex.say("fred", "hello fred")
check(not err and "message id b" in txt, f"alex → fred: {txt}")
mid = txt.split("message id ")[1].split(")")[0]
n = fred.msg("alex", "hello fred")
check(n is not None and n["message"] == "hello fred" and n["id"] == mid, f"fred got it as a plain message, same id: {n}")
txt, err = fred.say("alex", "hi alex", reply_to=mid)
check(not err, f"fred replies: {txt}")
n = alex.msg("fred", "hi alex")
check(n is not None and n["message"].startswith(f"[reply to {mid} from you"), f"alex sees the reply: {n and n['message']!r}")
txt, err = alex.tool("history", identity="fred", limit=5)
check(not err and "hello fred" in txt and "hi alex" in txt, f"alex's history: {txt}")
txt, err = fred.tool("history", identity="alex", limit=5)
check(not err and "hello fred" in txt and "hi alex" in txt, f"fred's history: {txt}")
alex.tool("edit", identity="fred", message_id=mid, text="hello fred (edited)")
check(fred.msg("alex", f"[edited {mid}]") is not None, "alex's edit arrives")
alex.tool("react", identity="fred", message_id=mid, emoji="👍")
check(fred.msg("alex", "[reaction] 👍") is not None, "alex's reaction arrives")
fid_reply = alex.notes[-1]["id"] if alex.notes else ""
fred.tool("react", identity="alex", message_id=mid, emoji="❤️")
check(alex.msg("fred", "[reaction] ❤️") is not None, "fred's reaction arrives")
fred.tool("read", identity="alex", message_id=mid)

# ---- files both ways, and one resumed from a partial download
data = os.urandom(3 * 1024 * 1024 + 123)
src = f"{alex.files}/out.bin"
open(src, "wb").write(data)
txt, err = alex.say("fred", "a file", files=[src])
n = fred.msg("alex", "a file", 60)
got = files_of(n)
check(bool(got) and open(f"{fred.files}/{got[0]}", "rb").read() == data, f"alex → fred file arrived intact: {got}")
back = os.urandom(1024 * 1024)
open(f"{fred.files}/back.bin", "wb").write(back)
fred.say("alex", "one back", files=[f"{fred.files}/back.bin"])
n = alex.msg("fred", "one back", 60)
got = files_of(n)
check(bool(got) and open(f"{alex.files}/{got[0]}", "rb").read() == back, f"fred → alex file arrived intact: {got}")
big = os.urandom(2 * 1024 * 1024)
fid = hashlib.sha256(big).hexdigest()
os.makedirs(f"{fred.files}/inbox/web/alex/.part", exist_ok=True)
open(f"{fred.files}/inbox/web/alex/.part/{fid}", "wb").write(big[:700_000])
open(f"{alex.files}/half.bin", "wb").write(big)
alex.say("fred", "resumed", files=[f"{alex.files}/half.bin"])
n = fred.msg("alex", "resumed", 60)
got = files_of(n)
check(bool(got) and open(f"{fred.files}/{got[0]}", "rb").read() == big, "partial download resumed and verified")
check(bool(got) and os.stat(f"{fred.files}/{got[0]}").st_uid == BIA_UID and os.stat(f"{fred.files}/state/links/alex.json").st_uid == BIA_UID,
      "received files and link state belong to the files owner")

# ---- store and forward: fred is down, alex's words wait and arrive when fred is back
fred.stop(signal.SIGTERM)
time.sleep(1)
txt, err = alex.say("fred", "while you were down")
check(not err and "waits there" in txt, f"send to a bia that is down waits: {txt}")
alex.tool("react", identity="fred", message_id=mid, emoji="🔥")
fred.start()
n = fred.msg("alex", "while you were down", 60)
check(n is not None, "fred gets it once it is back")
check(fred.msg("alex", "[reaction] 🔥", 10) is not None, "and the reaction alex made meanwhile")
# and the other way: alex is down, fred writes
alex.stop(signal.SIGTERM)
time.sleep(1)
txt, err = fred.say("alex", "while alex was down")
check(not err and "waits there" in txt, f"fred's send waits too: {txt}")
alex.start()
check(alex.msg("fred", "while alex was down", 60) is not None, "alex gets it once it is back")
check(alex.msg("fred", "hello fred", 3) is None and alex.msg("fred", "hi alex", 1) is None, "nothing old is delivered again after a restart")

# ---- the roles swapped: the client stops dialing, the other side dials
cl, sv = dialer()
txt, err = cl.tool("peer_disconnect", peer=sv.name)
check(not err and "disconnected" in txt, f"{cl.name} (the client) disconnects: {txt}")
time.sleep(1)
txt, err = cl.say(sv.name, "after disconnect")
check(not err and "waits there" in txt, f"a send after disconnect is kept: {txt}")
txt, err = sv.tool("peer_connect", peer=cl.name)
check(not err and "connected" in txt, f"{sv.name} dials {cl.name}: {txt}")
check(dialer()[0] is sv, f"{sv.name} is the client now")
check(sv.msg(cl.name, "after disconnect", 30) is not None, f"{cl.name}'s message waiting during the disconnect arrives ({cl.name} is the server now)")
sv.say(cl.name, "as client")
check(cl.msg(sv.name, "as client", 30) is not None, "the client's message arrives as an action")
src2 = f"{sv.files}/as-client.bin"
blob = os.urandom(500_000)
open(src2, "wb").write(blob)
sv.say(cl.name, "client file", files=[src2])
got = files_of(cl.msg(sv.name, "client file", 60))
check(bool(got) and open(f"{cl.files}/{got[0]}", "rb").read() == blob, "a file from the client side is pulled by the server side")
cl.tool("peer_connect", peer=sv.name)

# ---- bans: the model's, then root's outranks it
txt, err = fred.tool("peer_ban", peer="alex")
check(not err and "banned alex" in txt, f"fred bans alex: {txt}")
check(alex.notice("fred banned this bia", 30) is not None, "alex told it is banned")
txt, err = alex.tool("peer_connect", peer="fred")
check(err and "banned" in txt, f"banned alex cannot connect: {txt}")
txt, err = fred.tool("peer_unban", peer="alex")
check(not err and "unbanned" in txt, f"fred unbans: {txt}")
txt, err = fred.tool("peer_root", action="ban", peer="alex")
check(not err, f"root bans alex at fred: {txt}")
txt, err = fred.tool("peer_unban", peer="alex")
check(err and "only root" in txt, f"the model cannot lift root's ban: {txt}")
txt, err = fred.tool("peer_root", action="unban", peer="alex")
check(not err and "unbanned" in txt, f"root lifts it: {txt}")
alex.tool("peer_connect", peer="fred")
fred.say("alex", "after the bans")
check(alex.msg("fred", "after the bans", 40) is not None, "talking again after the unban")

# ---- shutdown says bye with the reason; the restarted side comes back by itself
json.dump({"reason": "reboot", "at": int(time.time())}, open(f"{fred.files}/state/shutdown-reason", "w"))
fred.stop(signal.SIGTERM)
os.remove(f"{fred.files}/state/shutdown-reason")
fred.start()
time.sleep(2)
alex.say("fred", "after the reboot")
check(fred.msg("alex", "after the reboot", 40) is not None, "the link is back after fred's restart")

# ---- a copy of fred (same key) is refused while fred is connected
clone = Bia("fred-copy", 8445, files=os.path.join(root, "fred-copy"))
clone.stop()
shutil.rmtree(clone.files)
shutil.copytree(fred.files, clone.files)
clone.start()
# a copy is refused at once; it is reported when its tries go on (a quick restart is not a copy)
n = alex.notice("second machine", 90)
check(n is not None, f"alex reports the copy: {n and n['message'][:120]!r}")
check(clone.notice("another machine using this bia's key", 60) is not None, "the copy is told why")
clone.stop()
alex.say("fred", "still you?")
check(fred.msg("alex", "still you?") is not None, "the real fred kept its link")

# ---- removing the peer closes the link
txt, err = alex.tool("peer_remove", peer="fred")
check(not err and "Removed fred" in txt, f"remove: {txt}")
n = fred.wait(lambda n: n["identity"] == "system" and ("no longer trusts this bia" in n["message"] or "refused this bia's certificate" in n["message"]), 60)
check(n is not None, f"fred is told it is no longer trusted: {n and n['message'][:100]!r}")

for b in (alex, fred):
    b.stop()
shutil.rmtree(root, ignore_errors=True)
print(f"\n{'FAILED: ' + str(len(failed)) if failed else 'all passed'}")
sys.exit(1 if failed else 0)
