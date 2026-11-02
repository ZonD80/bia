"""bia's side of .biaconfig: the real web MCP makes the file (a new certificate, the address in the
server certificate), the real chrome MCP opens it into a real NSS database. The apps' side is tried by
hand with tests/manual/mobile-chat-server.py. Runs as root in a Debian trixie container, like the agent.

  python3 -I tests/integration/biaconfig-e2e.py <bia-mcp-web> <bia-mcp-chrome> <chrome MCP folder>
"""
import base64, json, os, re, shutil, stat, subprocess, sys, tempfile, threading, time

web_bin, chrome_bin, chrome_src = [os.path.abspath(x) for x in sys.argv[1:4]]
failed = []


def check(cond, what):
    print(time.strftime("%H:%M:%S ") + ("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        failed.append(what)


if subprocess.run(["id", "bia"], capture_output=True).returncode:
    subprocess.run(["useradd", "-m", "bia"], check=True)
files = "/home/bia/files"
os.makedirs(files, exist_ok=True)
subprocess.run(["chown", "-R", "bia:bia", "/home/bia"], check=True)
work = tempfile.mkdtemp(prefix="bia-config-e2e-")


class Mcp:
    def __init__(self, binary, cwd, env):
        self.p = subprocess.Popen([binary], cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(f"{work}/{os.path.basename(binary)}.log", "w"), env=dict(os.environ, BIA_FILES=files, **env), text=True, bufsize=1)
        self.results, self.rid = {}, 0
        threading.Thread(target=self.reader, daemon=True).start()
        self.rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}})

    def reader(self):
        for line in self.p.stdout:
            try:
                v = json.loads(line)
            except ValueError:
                continue
            if "id" in v:
                self.results[v["id"]] = v

    def rpc(self, method, params):
        self.rid += 1
        i = self.rid
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        end = time.time() + 90
        while time.time() < end:
            if i in self.results:
                return self.results.pop(i)
            time.sleep(0.02)
        raise SystemExit(f"no reply to {method} {params}")

    def tool(self, tool_name, /, **args):
        r = self.rpc("tools/call", {"name": tool_name, "arguments": args})["result"]
        text = "".join(c.get("text", "") for c in r.get("content", []))
        return ("error: " + text) if r.get("isError") else text


web = Mcp(web_bin, work, {"BIA_WEB_PORT": "8443", "BIA_WEB_HTTP_PORT": "0"})
mcp_dir = work + "/chrome"
shutil.copytree(chrome_src, mcp_dir, symlinks=True)
chrome = Mcp(chrome_bin, mcp_dir, {})


def sans():
    """What the running server's certificate names."""
    s = subprocess.run(["openssl", "s_client", "-connect", "127.0.0.1:8443"], stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=20).stdout
    return subprocess.run(["openssl", "x509", "-noout", "-ext", "subjectAltName"], input=s, capture_output=True, text=True).stdout


def wait_san(want):
    end = time.time() + 15
    while time.time() < end:
        try:
            if want in sans():
                return True
        except Exception:
            pass
        time.sleep(0.5)
    return False


# ---- bia makes one
check(wait_san("DNS"), "the server is up")
out = web.tool("pki_biaconfig", identity="alice", address="bia.test:8443", name="Test bia", password="short")
check(out.startswith("error") and "at least" in out, f"a weak password is refused: {out[:80]}")
check("alice" not in web.tool("pki_list"), "and no certificate was left behind")
out = web.tool("pki_biaconfig", identity="alice", address="bia.test/evil")
check(out.startswith("error"), f"a bad address is refused: {out[:80]}")
check("bia.test" not in sans(), "and changes nothing")

out = web.tool("pki_biaconfig", identity="alice", address="bia.test:8443", name="Test bia")
check(not out.startswith("error"), f"made: {out[:260]}")
path = out.split(",")[0]
password = re.search(r"password (\S+)\.", out).group(1)
serial = re.search(r"serial (\S+),", out).group(1)
check(len(password) == 19, "with a generated password")
st = os.stat(path)
check(stat.S_IMODE(st.st_mode) == 0o600 and st.st_uid == os.stat(files).st_uid, "the file is 0600 and bia's")
raw = open(path).read()
check("bia.test" not in raw and "BEGIN" not in raw and password not in raw, "nothing readable in it")
check(json.loads(raw)["fmt"] == "biaconfig" and json.loads(raw)["iter"] == 600000, "the envelope says what it is")
check(serial in web.tool("pki_list"), "its certificate is listed by serial")
check(wait_san("bia.test"), "the server certificate names the address within seconds")

out = web.tool("pki_biaconfig", identity="v6", address="[2001:db8::7]:9000", password="a-long-enough-password")
check(not out.startswith("error") and "[2001:db8::7]:9000" in out, f"an IPv6 address with a port: {out[:120]}")
check(wait_san("2001:DB8") or wait_san("2001:db8"), "is named as an IP")
out = web.tool("pki_biaconfig", identity="plain")
check(not out.startswith("error") and "bia.test:8443" in out, f"no address: the ones it knows: {out[:140]}")

# ---- bia opens one
bad = chrome.tool("cert_import", path=path, password="wrong-password-here")
check(bad.startswith("error") and "wrong password" in bad, f"the wrong password is refused: {bad[:100]}")
check("alice" not in chrome.tool("cert_list"), "and imports nothing")
out = chrome.tool("cert_import", path=path, password=password)
check("https://bia.test:8443/" in out and "alice" in out, f"opened: {out[:200]}")
listed = chrome.tool("cert_list")
check("alice" in listed, "the client certificate is in Chrome's database")
nss = subprocess.run(["runuser", "-u", "bia", "--", "certutil", "-L", "-d", "sql:/home/bia/.pki/nssdb"], capture_output=True, text=True).stdout
check(re.search(r"\bCA\b.*\s+C,,", nss) is not None, f"its CA is trusted for Chrome: {nss[-200:]}")
check(not os.listdir("/usr/local/share/ca-certificates") if os.path.isdir("/usr/local/share/ca-certificates") else True, "and not for this whole system")
open(f"{files}/fake.p12", "wb").write(b"not a certificate")
check(chrome.tool("cert_import", path=f"{files}/fake.p12", password="x").startswith("error"), "an ordinary import still goes the ordinary way")

# ---- the web app's Settings asks over HTTPS, root only
def curl(path, body=None, cert=None):
    a = ["curl", "-sk", "-o", f"{work}/answer", "-w", "%{http_code}", f"https://127.0.0.1:8443{path}"]
    if cert:
        a += ["--cert-type", "P12", "--cert", f"{cert}:12345678"]
    if body is not None:
        a += ["-X", "POST", "-H", "Content-Type: application/json", "-d", json.dumps(body)]
    return subprocess.run(a, capture_output=True, text=True).stdout.strip(), open(f"{work}/answer", "rb").read()


code, data = curl("/v1/pki/bootstrap/root", {})
check(code == "200", f"root's certificate from the first start: {code}")
root_p12 = f"{work}/root.p12"
open(root_p12, "wb").write(data)
code, _ = curl("/v1/pki/biaconfig", {"identity": "nobody"})
check(code != "200", f"no certificate, no file: {code}")
code, data = curl("/v1/pki/issue", {"identity": "carol"}, root_p12)
carol = f"{work}/carol.p12"
code2, p12 = curl(json.loads(data)["path"])
open(carol, "wb").write(p12)
check(code == "200" and code2 == "200", "a normal identity to try with")
code, _ = curl("/v1/pki/biaconfig", {"identity": "sneaky"}, carol)
check(code == "403", f"a normal identity is refused: {code}")
code, data = curl("/v1/pki/biaconfig", {"identity": "bad name"}, root_p12)
check(code == "400", f"a bad identity is refused: {code} {data[:60]}")
code, data = curl("/v1/pki/biaconfig", {"identity": "dave", "address": "web.test:8443", "name": "From the page"}, root_p12)
check(code == "200", f"root gets one: {code}")
page = json.loads(data)
check(page["file_name"].endswith(".biaconfig") and page["addresses"] == ["web.test:8443"] and len(page["password"]) == 19, f"with its name, address and password: {page['file_name']} {page['addresses']}")
check(not any("dave" in f for f in os.listdir(f"{files}/biaconfig")), "and no copy stays on the machine")
check(wait_san("web.test"), "the server certificate names that address")
open(f"{files}/page.biaconfig", "wb").write(base64.b64decode(page["file"]))
out = chrome.tool("cert_import", path=f"{files}/page.biaconfig", password=page["password"])
check("https://web.test:8443/" in out and "dave" in out, f"and it opens: {out[:120]}")

# ---- and it can be taken back
out = web.tool("pki_revoke", serial=serial)
check(not out.startswith("error"), f"revoked by its serial: {out[:80]}")
row = [l for l in web.tool("pki_list").splitlines() if serial in l]
check(row and "revoked" in row[0].lower(), f"listed as revoked: {row[:1]}")

for m in (web, chrome):
    m.p.terminate()
if failed:
    for log in os.listdir(work):
        if log.endswith(".log"):
            print(log, open(f"{work}/{log}").read()[-800:])
    print("FAILED:", len(failed))
    sys.exit(1)
print("all ok")
