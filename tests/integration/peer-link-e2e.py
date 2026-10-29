"""End-to-end: live peer links between real bia-mcp-web processes (mutual TLS on localhost).

Two bias, alex (8443) and fred (8444), set up trust with one approval, then the link opens
by itself. Checks: connect notices, messages both ways with the origin chain and replies,
"asked again", a file (and one resumed from a partial download), offline sends refused,
disconnect / reconnect, ban (the model's and root's), bye on SIGTERM with the reason, the
restarted side coming back, a killed peer reported lost, a copy of a bia with the same key
refused, and removing a peer.

  docker run --rm --platform linux/arm64 -v $PWD:/work -v bia-cache-arm64:/cache -w /work \\
    -e CARGO_TARGET_DIR=/cache/target-arm64-test bia-build:arm64 bash -lc \\
    'cargo build -q -p bia-mcp-web && python3 -I tests/integration/peer-link-e2e.py /cache/target-arm64-test/debug/bia-mcp-web'
"""
import hashlib, http.client, json, os, shutil, signal, ssl, subprocess, sys, tempfile, threading, time

exe = sys.argv[1]
root = tempfile.mkdtemp(prefix="bia-peer-e2e-")
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

    def wait(self, pred, secs=30, what=""):
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

    def notice(self, peer, needle, secs=30):
        return self.wait(lambda n: n["identity"] == "system" and n["channel"] == f"peer:{peer}" and needle in n["message"], secs)

    def msg(self, peer, needle, secs=30):
        return self.wait(lambda n: n["identity"] == peer and n["channel"] == f"peer:{peer}" and needle in n["message"], secs)

    def stop(self, sig=signal.SIGTERM):
        self.p.send_signal(sig)
        try:
            self.p.wait(10)
        except subprocess.TimeoutExpired:
            self.p.kill()


alex = Bia("alex", 8443)
fred = Bia("fred", 8444)

# ---- trust with one approval; the link opens by itself
txt, err = alex.tool("peer_add", url="https://127.0.0.1:8444", note="alex here")
check(not err and "Asked fred" in txt, f"peer_add: {txt[:100]}")
reqs = json.load(open(f"{fred.files}/state/pki/requests/" + os.listdir(f"{fred.files}/state/pki/requests")[0]))
txt, err = fred.tool("pki_approve", id=reqs["id"])
check(not err and "peer:alex" in txt, f"approve: {txt[:160]}")
n = fred.notice("alex", "Connected", 60)
check(n is not None, "fred: [peer alex] Connected")
n = alex.notice("fred", "Connected", 60)
check(n is not None, "alex: [peer fred] Connected")
txt, _ = alex.tool("peer_list")
check("online since" in txt and "peer:fred" in txt, f"peer_list shows the link: {txt}")

# ---- messages, origin chain, replies
txt, err = alex.tool("send", channel="peer:fred", text="hello fred", origin={"new": "b1", "from": "root"})
check(not err and "message id p" in txt, f"alex → fred: {txt}")
mid = txt.split("message id ")[1].split(")")[0]
n = fred.msg("alex", "hello fred")
check(n is not None and "[request alex/b1: root → alex → you]" in n["message"], f"fred got it with the origin: {n and n['message']!r}")
check(n is not None and n["id"] == mid, "same message id on both sides")
txt, err = fred.tool("send", channel="peer:alex", text="hi alex", reply_to=mid, origin={"id": "alex/b1", "path": ["root", "alex"]})
check(not err, f"fred replies: {txt}")
n = alex.msg("fred", "hi alex")
check(n is not None and f"[reply to {mid} from you" in n["message"], f"alex sees the reply: {n and n['message']!r}")
check(n is not None and "[asked again: this request started with you" in n["message"], "alex knows its own request came back")
check(n is not None and "root → alex → fred → you" in n["message"], "path grows at each bia")
txt, err = alex.tool("history", channel="peer:fred", limit=5)
check(not err and "hello fred" in txt and "hi alex" in txt, f"history: {txt}")
txt, err = alex.tool("edit", channel="peer:fred", message_id=mid, text="hello fred (edited)")
n = fred.msg("alex", f"[edited {mid}]")
check(not err and n is not None, "edit arrives")
alex.tool("react", channel="peer:fred", message_id=mid, emoji="👍")
check(fred.msg("alex", "[reaction] 👍") is not None, "reaction arrives")

# ---- a file, and one resumed from a partial download
data = os.urandom(3 * 1024 * 1024 + 123)
src = f"{alex.files}/out.bin"
open(src, "wb").write(data)
txt, err = alex.tool("send", channel="peer:fred", text="a file", files=[src])
check(not err and "pulls 1 file" in txt, f"send file: {txt}")
n = fred.msg("alex", "a file", 60)
got = n and [l[2:] for l in n["message"].splitlines() if l.startswith("- ")]
check(bool(got) and open(got[0], "rb").read() == data, f"file arrived intact: {got}")
big = os.urandom(2 * 1024 * 1024)
fid = hashlib.sha256(big).hexdigest()
os.makedirs(f"{fred.files}/inbox/peer/alex/.part", exist_ok=True)
open(f"{fred.files}/inbox/peer/alex/.part/{fid}", "wb").write(big[:700_000])
open(f"{alex.files}/half.bin", "wb").write(big)
alex.tool("send", channel="peer:fred", text="resumed", files=[f"{alex.files}/half.bin"])
n = fred.msg("alex", "resumed", 60)
got = n and [l[2:] for l in n["message"].splitlines() if l.startswith("- ")]
check(bool(got) and open(got[0], "rb").read() == big, "partial download resumed and verified")

check(os.stat(got[0]).st_uid == BIA_UID and os.stat(f"{fred.files}/state/peers/alex/link.json").st_uid == BIA_UID,
      "received files and link state belong to the files owner")

# ---- a peer certificate opens only the link
ctx = ssl.create_default_context(cafile=f"{alex.files}/state/pki/peers/fred/ca.pem")
ctx.check_hostname = False
ctx.load_cert_chain(f"{alex.files}/state/pki/peers/fred/cert.pem", f"{alex.files}/state/pki/agent.key")
def get(path):
    c = http.client.HTTPSConnection("127.0.0.1", 8444, context=ctx, timeout=10)
    c.request("GET", path)
    r = c.getresponse()
    return r.status, r.read()
check(get("/v1/whoami")[0] == 200, "peer certificate: whoami works")
check(get("/v1/chats")[0] == 403, "peer certificate: the chat API is refused (only /ws/peer)")

# ---- disconnect: told, offline sends refused, either side may reconnect
txt, err = fred.tool("peer_disconnect", peer="alex")
check(not err and "disconnected" in txt, f"fred disconnects: {txt}")
check(alex.notice("fred", "closed the link") is not None, "alex told fred closed the link")
time.sleep(0.5)
txt, err = alex.tool("send", channel="peer:fred", text="anyone?")
check(err and "offline" in txt and "live connection is required" in txt, f"offline send refused: {txt}")
txt, err = alex.tool("peer_connect", peer="fred")
check(not err and "connected" in txt, f"alex reconnects: {txt}")
check(fred.notice("alex", "Back online", 20) is not None, "fred told the link is back")

# ---- bans: the model's, then root's outranks it
txt, err = fred.tool("peer_ban", peer="alex")
check(not err and "banned alex" in txt, f"fred bans alex: {txt}")
check(alex.notice("fred", "banned this bia") is not None, "alex told it is banned")
txt, err = alex.tool("peer_connect", peer="fred")
check(err and "banned" in txt, f"banned alex cannot connect: {txt}")
txt, err = fred.tool("peer_unban", peer="alex")
check(not err and "unbanned" in txt, f"fred unbans: {txt}")
check(alex.notice("fred", "Back online", 30) is not None or alex.notice("fred", "Connected", 1) is not None, "link back after unban")
txt, err = fred.tool("peer_root", action="ban", peer="alex")
check(not err, f"root bans alex at fred: {txt}")
txt, err = fred.tool("peer_unban", peer="alex")
check(err and "only root" in txt, f"the model cannot lift root's ban: {txt}")
txt, err = fred.tool("peer_root", action="unban", peer="alex")
check(not err and "unbanned" in txt, f"root lifts it: {txt}")
check(alex.notice("fred", "Back online", 30) is not None, "link back after root's unban")

# ---- shutdown says bye with the reason; the restarted side comes back by itself
json.dump({"reason": "reboot", "at": int(time.time())}, open(f"{fred.files}/state/shutdown-reason", "w"))
t0 = time.time()
fred.stop(signal.SIGTERM)
n = alex.notice("fred", "is rebooting", 10)
check(n is not None and time.time() - t0 < 5, f"bye(reboot) reached alex at once ({time.time() - t0:.1f} s)")
os.remove(f"{fred.files}/state/shutdown-reason")
fred.start()
check(alex.notice("fred", "Back online", 40) is not None, "fred redialed after its restart")

# ---- a copy of fred (same key) is refused while fred is connected
clone = Bia("fred-copy", 8445, files=os.path.join(root, "fred-copy"))
clone.stop()
shutil.rmtree(clone.files)
shutil.copytree(fred.files, clone.files)
clone.start()
n = alex.notice("fred", "second machine", 40)
check(n is not None, f"alex refused the copy: {n and n['message'][:120]!r}")
check(clone.notice("alex", "another machine using this bia's key", 10) is not None, "the copy is told why")
clone.stop()
txt, err = alex.tool("send", channel="peer:fred", text="still you?")
check(not err and fred.msg("alex", "still you?") is not None, "the real fred kept its link")

# ---- a killed peer is reported lost after 30 s
fred.stop(signal.SIGKILL)
t0 = time.time()
n = alex.notice("fred", "Lost", 50)
check(n is not None and 25 <= time.time() - t0 <= 45, f"lost notice after {time.time() - t0:.0f} s")
fred.start()
check(alex.notice("fred", "Back online", 40) is not None, "back after the crash")

# ---- removing the peer closes the link at both ends
txt, err = alex.tool("peer_remove", peer="fred")
check(not err and "Removed fred" in txt, f"remove: {txt}")
check(fred.notice("alex", "no longer trusts this bia", 20) is not None, "fred told it is no longer trusted")
txt, err = alex.tool("send", channel="peer:fred", text="x")
check(err, f"no sends to a removed peer: {txt}")

for b in (alex, fred):
    b.stop()
shutil.rmtree(root, ignore_errors=True)
print(f"\n{'FAILED: ' + str(len(failed)) if failed else 'all passed'}")
sys.exit(1 if failed else 0)
