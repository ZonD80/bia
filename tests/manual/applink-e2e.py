"""App links end to end on this computer: a real bia-mcp-web and bia-mcp-system, and the phone
core's fetch_key against them (no phone needed).

  cargo build -p bia-mcp-web -p bia-mcp-system   (in source/)
  python3 -I tests/manual/applink-e2e.py target/debug [--serve]

--serve keeps the bia running after the checks and prints a fresh link for root, for the apps:
iOS simulator `-link '<link>'` (debug), Android `--es link '<link>'` after adb reverse tcp:18443 tcp:18443.

Checks: system_app_link makes a link with a key grant; the core pins the CA, takes the key once,
the link then finds it; a second take is refused; a contact not in the addressbook gets no link;
a contact gets a normal-trust key. Port 18443.
"""
import json, os, subprocess, sys, tempfile, threading, time, ssl, urllib.request

bins = os.path.abspath(sys.argv[1])
repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
files = tempfile.mkdtemp(prefix="bia-applink-")
env = dict(os.environ, BIA_FILES=files, BIA_WEB_PORT="18443", BIA_WEB_HTTP_PORT="0")


class Mcp:
    def __init__(self, exe):
        self.p = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, text=True, bufsize=1)
        self.rid, self.results, self.heard = 0, {}, []
        threading.Thread(target=self.read, daemon=True).start()
        self.rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}})

    def read(self):
        for line in self.p.stdout:
            v = json.loads(line)
            d = (v.get("params") or {}).get("data")
            if isinstance(d, dict) and d.get("t") == "inbound":
                self.heard.append(d.get("message"))
            elif isinstance(d, str):
                self.heard.append(d)
            elif "id" in v:
                self.results[v["id"]] = v

    def rpc(self, method, params=None):
        self.rid += 1
        i = self.rid
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n")
        self.p.stdin.flush()
        for _ in range(3000):
            if i in self.results:
                return self.results.pop(i)
            time.sleep(0.02)
        raise SystemExit(f"no reply to {method}")

    def call(self, name, args):
        r = self.rpc("tools/call", {"name": name, "arguments": args})
        res = r.get("result") or {}
        text = "".join(c.get("text", "") for c in res.get("content", []))
        return bool(res.get("isError")), text


web = Mcp(os.path.join(bins, "bia-mcp-web"))
ctx = ssl.create_default_context()
ctx.check_hostname, ctx.verify_mode = False, ssl.CERT_NONE
for _ in range(200):
    try:
        urllib.request.urlopen("https://127.0.0.1:18443/v1/pki/ca", context=ctx, timeout=2).read()
        break
    except Exception:
        time.sleep(0.1)
else:
    raise SystemExit("web MCP did not come up")
# the machine's addresses on a Mac are not 127.0.0.1: point the links at loopback
json.dump({"port": 0, "urls": ["https://127.0.0.1:18443"]}, open(os.path.join(files, "state", "web.json"), "w"))
sysm = Mcp(os.path.join(bins, "bia-mcp-system"))
fails = 0


def check(ok, what):
    global fails
    print(("PASS " if ok else "FAIL ") + what, flush=True)
    fails += 0 if ok else 1


tools = [t["name"] for t in sysm.rpc("tools/list")["result"]["tools"]]
check("app_link" in tools, "system MCP lists app_link")
err, text = sysm.call("app_link", {"path": "/"})
print(text)
link = next((w for w in text.split() if w.startswith("https://ownbia.com/open#")), "")
check(not err and "&t=" in link, "root's link carries a key grant")
# loopback is left out of links (it means the phone itself): put it back for this test
link = link.replace("&go=", "&at=127.0.0.1:18443&go=", 1) if "&at=" not in link else link
err, text = sysm.call("app_link", {"identity": "nobody"})
check(err and "addressbook" in text, "no link for someone not in the addressbook")


def core(link):
    home = tempfile.mkdtemp(prefix="bia-phone-")
    r = subprocess.run(["cargo", "test", "-q", "--lib", "fetch_key_from_a_real_bia", "--", "--ignored", "--nocapture"],
                       cwd=os.path.join(repo, "installers", "bia-deploy-mobile"), env=dict(os.environ, BIA_TEST_LINK=link, BIA_DEPLOY_HOME=home),
                       capture_output=True, text=True)
    print(r.stdout[-1500:], r.stderr[-1500:] if r.returncode else "")
    return r.returncode == 0


check(core(link), "phone core: CA pinned, key taken once, link opens with it, second take refused")
issued = web.call("pki_list", {})[1]
check("root" in issued and "trust=root" in issued, "root's link key has root trust")

# a contact: normal trust from the addressbook
os.makedirs(os.path.join(files, "state"), exist_ok=True)
json.dump({"contacts": [{"id": "ann", "name": "Ann", "trust": "normal", "channels": []}]}, open(os.path.join(files, "state", "addressbook.json"), "w"))
err, text = sysm.call("app_link", {"identity": "ann"})
link = next((w for w in text.split() if w.startswith("https://ownbia.com/open#")), "")
link = link.replace("&go=", "&at=127.0.0.1:18443&go=", 1) if "&at=" not in link else link
check(core(link), "contact's link works too")
issued = web.call("pki_list", {})[1]
print(issued)
check(any("ann" in l and "trust=normal" in l for l in issued.splitlines()), "contact's key has normal trust")
print("ALL PASS" if not fails else f"{fails} FAILED")
if "--serve" in sys.argv:
    json.dump({"port": 0, "urls": ["https://127.0.0.1:18443"]}, open(os.path.join(files, "state", "web.json"), "w"))
    text = sysm.call("app_link", {"path": "/"})[1]
    link = next((w for w in text.split() if w.startswith("https://ownbia.com/open#")), "")
    print("LINK", link.replace("&go=", "&at=127.0.0.1:18443&go=", 1) if "&at=" not in link else link, flush=True)
    while True:
        time.sleep(5)
        for h in web.heard[-5:]:
            pass
sys.exit(1 if fails else 0)
