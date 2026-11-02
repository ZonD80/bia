"""Client certificates end to end: the real chrome MCP, real Chromium on Xvfb and Openbox with AT-SPI,
and an HTTPS server on localhost that demands a client certificate from a private CA. Runs as root in a
Debian trixie container, like the agent.

  python3 -I tests/integration/chrome-cert-e2e.py <bia-mcp-chrome> <chrome MCP folder> <bia_desktop.py>
"""
import http.server, json, os, re, shutil, ssl, subprocess, sys, tempfile, threading, time

binary, mcp_src, desktop = [os.path.abspath(x) for x in sys.argv[1:4]]
failed = []


def check(cond, what):
    print(time.strftime("%H:%M:%S ") + ("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        failed.append(what)


def sh(*a, **k):
    return subprocess.run(a, check=True, capture_output=True, **k)


if subprocess.run(["id", "bia"], capture_output=True).returncode:
    sh("useradd", "-m", "bia")
uid = subprocess.check_output(["id", "-u", "bia"], text=True).strip()
run = f"/run/user/{uid}"
sh("install", "-d", "-o", "bia", "-m", "700", run)
os.makedirs("/tmp/.X11-unix", exist_ok=True)
os.chmod("/tmp/.X11-unix", 0o1777)
os.environ["DISPLAY"] = ":0"
open("/home/bia/.Xauthority", "a").close()
sh("chown", "bia:bia", "/home/bia/.Xauthority")
xvfb = subprocess.Popen(["Xvfb", ":0", "-screen", "0", "1280x800x24", "-ac"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1)
wm = subprocess.Popen(["openbox"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1)
sh("runuser", "-u", "bia", "--", "env", f"XDG_RUNTIME_DIR={run}", "HOME=/home/bia", "DISPLAY=:0", "dbus-daemon", "--session", "--fork", f"--address=unix:path={run}/bus")

# the certificates a company would hand out
files = "/home/bia/files"
os.makedirs(files + "/state", exist_ok=True)
work = tempfile.mkdtemp(prefix="bia-cert-e2e-")
def ossl(*a):
    sh("openssl", *a, cwd=work)
ossl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "ca.key", "-out", "ca.pem", "-subj", "/CN=Acme Private CA/O=Acme", "-days", "30", "-addext", "basicConstraints=critical,CA:TRUE")
ossl("req", "-newkey", "rsa:2048", "-nodes", "-keyout", "srv.key", "-out", "srv.csr", "-subj", "/CN=localhost")
open(work + "/srv.ext", "w").write("subjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\n")
ossl("x509", "-req", "-in", "srv.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", "srv.pem", "-days", "30", "-extfile", "srv.ext")
open(work + "/cli.ext", "w").write("extendedKeyUsage=clientAuth\n")
for who in ("bia-robot", "other-person"):
    ossl("req", "-newkey", "rsa:2048", "-nodes", "-keyout", who + ".key", "-out", who + ".csr", "-subj", f"/CN={who}/O=Acme")
    ossl("x509", "-req", "-in", who + ".csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", who + ".pem", "-days", "30", "-extfile", "cli.ext")
ossl("pkcs12", "-export", "-in", "bia-robot.pem", "-inkey", "bia-robot.key", "-out", "bia-robot.p12", "-passout", "pass:p12-s3cret", "-name", "bia-robot")
# what a bia's own PKI makes: the same name twice, reissued, with a colon in it
for tag, who in (("dup1", "Identity: root"), ("dup2", "Identity: root"), ("zond", "Identity: zond")):
    ossl("req", "-newkey", "rsa:2048", "-nodes", "-keyout", tag + ".key", "-out", tag + ".csr", "-subj", f"/CN={who}")
    ossl("x509", "-req", "-in", tag + ".csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", tag + ".pem", "-days", "30", "-extfile", "cli.ext")
    ossl("pkcs12", "-export", "-in", tag + ".pem", "-inkey", tag + ".key", "-out", tag + ".p12", "-passout", "pass:x", "-name", who)
    shutil.copy(f"{work}/{tag}.p12", f"{files}/{tag}.p12")
def serial_of(tag):
    return subprocess.check_output(["openssl", "x509", "-in", f"{work}/{tag}.pem", "-noout", "-serial"], text=True).strip().split("=")[1]
for f in ("bia-robot.p12", "other-person.pem", "other-person.key", "ca.pem"):
    shutil.copy(f"{work}/{f}", f"{files}/{f}")
subprocess.run(["chown", "-R", "bia:bia", "/home/bia"])

seen = []
serials = []


class Page(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        cert = self.connection.getpeercert() or {}
        cn = dict(x[0] for x in cert.get("subject", ())).get("commonName", "nobody")
        if self.path == "/favicon.ico":
            self.send_response(404)
            self.end_headers()
            return
        seen.append(cn)
        serials.append(cert.get("serialNumber", ""))
        print(f"   site: {cn} at {time.strftime('%H:%M:%S')}", flush=True)
        body = f"<title>portal</title><p id=who>hello {cn}</p>".encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(work + "/srv.pem", work + "/srv.key")
ctx.load_verify_locations(work + "/ca.pem")
ctx.verify_mode = ssl.CERT_REQUIRED
srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Page)
srv.socket = ctx.wrap_socket(srv.socket, server_side=True, do_handshake_on_connect=False)
threading.Thread(target=srv.serve_forever, daemon=True).start()
port = srv.server_address[1]
url = f"https://localhost:{port}/"

# the MCP, from a writable copy of its folder
mcp_dir = work + "/mcp"
shutil.copytree(mcp_src, mcp_dir, symlinks=True)
os.makedirs("/opt/cr", exist_ok=True)
wrapper = "/opt/cr/chromium"  # a container has no sandbox for it
open(wrapper, "w").write('#!/bin/sh\nexec /usr/lib/chromium/chromium --no-sandbox "$@"\n')
os.chmod(wrapper, 0o755)
env = dict(os.environ, BIA_FILES=files, BIA_CHROME_BIN=wrapper, BIA_DESKTOP_SCRIPT=desktop)
p = subprocess.Popen([binary], cwd=mcp_dir, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(work + "/mcp.log", "w"), env=env, text=True, bufsize=1)
results, rid, inbound = {}, [0], []


def reader():
    for line in p.stdout:
        try:
            v = json.loads(line)
        except ValueError:
            continue
        d = (v.get("params") or {}).get("data")
        if isinstance(d, dict) and d.get("t") == "inbound":
            inbound.append(d)
        elif "id" in v:
            results[v["id"]] = v


threading.Thread(target=reader, daemon=True).start()


def rpc(method, params=None, wait=120):
    rid[0] += 1
    i = rid[0]
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params or {}}) + "\n")
    p.stdin.flush()
    end = time.time() + wait
    while time.time() < end:
        if i in results:
            return results.pop(i)
        time.sleep(0.02)
    return {"result": {"content": [{"text": f"no reply to {params}"}], "isError": True}}


def tool(tool_name, /, **args):
    r = rpc("tools/call", {"name": tool_name, "arguments": args})["result"]
    text = "".join(c.get("text", "") for c in r.get("content", []))
    return ("error: " + text) if r.get("isError") else text


def desk(cmd, **args):
    env = {"HOME": "/home/bia", "DISPLAY": ":0", "XAUTHORITY": "/home/bia/.Xauthority", "XDG_RUNTIME_DIR": run, "DBUS_SESSION_BUS_ADDRESS": f"unix:path={run}/bus", "PATH": os.environ["PATH"]}
    out = subprocess.run(["runuser", "-u", "bia", "--", "env", *[f"{k}={v}" for k, v in env.items()], "python3", "-I", desktop, json.dumps({"cmd": cmd, "args": args})], capture_output=True, text=True, timeout=120)
    v = json.loads(out.stdout.strip().splitlines()[-1]) if out.stdout.strip() else {"ok": False, "error": out.stderr[-300:]}
    return v.get("text") if v.get("ok") else "error: " + str(v.get("error"))


rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})
names = {t["name"] for t in rpc("tools/list")["result"]["tools"]}
for t in ("cert_import", "cert_trust", "cert_list", "cert_select", "cert_unselect", "cert_remove", "choose_certificate", "navigate_page"):
    check(t in names, f"tool {t}")

check("no certificates" in tool("cert_list"), "an empty store")
out = tool("cert_import", path="bia-robot.p12", password="wrong")
check(out.startswith("error:") and "password" in out, f"a wrong password says so: {out[:110]}")
check("no client certificate" in tool("cert_select", site="localhost"), "nothing to select yet")
out = tool("cert_import", path="bia-robot.p12", password="p12-s3cret")
check("bia-robot" in out and "Acme Private CA" in out, f"a .p12 goes in: {out[:150]}")
check("p12-s3cret" not in subprocess.run("ps axww", shell=True, capture_output=True, text=True).stdout, "the password is on no command line")
check(not [d for d in os.listdir("/home/bia/.pki") if d.startswith("import-")], "the temporary copy is gone")
out = tool("cert_import", path="other-person.pem", key_path="other-person.key")
check("other-person" in out, f"a certificate and key pair goes in: {out[:120]}")
out = tool("cert_trust", path="key-does-not-exist.pem")
check("file not found" in out, "a missing file is said")
out = tool("cert_trust", path="ca.pem")
check("now trusted in Chrome and on this system" in out, f"a private CA: {out[:120]}")
check(subprocess.run(["curl", "-sS", "-o", "/dev/null", "--cert", work + "/bia-robot.pem", "--key", work + "/bia-robot.key", url]).returncode == 0, "the system trusts it: curl with no flags")
listing = tool("cert_list")
print(listing)
check('"bia-robot"' in listing and "client" in listing and "trusted CA" in listing, "the list tells them apart")
out = tool("cert_select", site="localhost")
check("several client certificates" in out, f"two certificates need a choice: {out[:90]}")

# no rule yet: Chrome asks in its own dialog, which can be read and answered as elements
out = tool("start")
check("pid" in out, f"Chrome starts: {out[:100]}")
check(subprocess.run(["pgrep", "-af", "force-renderer-accessibility"], capture_output=True).returncode == 0, "with accessibility on")
seen.clear()  # the curl above presented bia-robot
threading.Thread(target=lambda: tool("navigate_page", url=url, type="url"), daemon=True).start()


def notice(wait=45):
    end = time.time() + wait
    while time.time() < end:
        for d in inbound:
            if d.get("message", "").startswith("[certificate dialog opened]"):
                return d
        time.sleep(0.2)
    return None


d = notice()
print(d)
check(d is not None and d["channel"] == "chrome" and d["identity"] == "system", "the dialog is announced on system|chrome, like a file dialog")
text = d["message"] if d else ""
check(f"localhost:{port}" in text, "with the site asking")
check("bia-robot (issued by Acme Private CA, serial " in text and "other-person (issued by Acme Private CA, serial " in text, "and what is on offer")
time.sleep(2.5)
check(sum(1 for x in inbound if x.get("message", "").startswith("[certificate dialog opened]")) == 1, "once, not every second")
check(not seen, f"nothing reaches the site while the dialog waits: {seen}")
out = tool("choose_certificate", cert="no-such-one")
check("no such certificate" in out and "bia-robot" in out, f"an unknown certificate is refused: {out[:110]}")
out = tool("choose_certificate")
check("several match" in out, f"two on offer need a choice: {out[:90]}")
# the row that is not the current one first: this must really move the choice
out = tool("choose_certificate", cert="bia-robot")
check("Chose certificate bia-robot" in out, f"chosen: {out[:90]}")
deadline = time.time() + 25
while time.time() < deadline and not seen:
    time.sleep(0.3)
check(seen == ["bia-robot"], f"the site saw exactly the chosen certificate: {seen}")
out = tool("choose_certificate", cert="bia-robot")
check("No certificate dialog is open" in out, f"no dialog, no answer: {out[:90]}")

# a nickname from the store, on a new Chrome, which forgot the answer
seen.clear()
inbound.clear()
tool("restart")
threading.Thread(target=lambda: tool("navigate_page", url=url, type="url"), daemon=True).start()
check(notice() is not None, "announced again after a restart")
out = tool("choose_certificate", cert="other-person - Acme")
check("Chose certificate other-person" in out, f"a name from chrome_cert_list works: {out[:110]}")
deadline = time.time() + 25
while time.time() < deadline and not seen:
    time.sleep(0.3)
check(seen == ["other-person"], f"and that one reaches the site: {seen}")

# cancel: the page gets no certificate, and the server refuses it
seen.clear()
inbound.clear()
tool("restart")
threading.Thread(target=lambda: tool("navigate_page", url=url, type="url"), daemon=True).start()
check(notice() is not None, "announced a third time")
out = tool("choose_certificate", cancel=True)
check("without choosing" in out, f"cancel: {out[:90]}")
time.sleep(3)
check(not seen, f"after cancel the site saw no certificate: {seen}")

# a rule: no dialog, the right certificate
out = tool("cert_select", site=url, cert="bia-robot")
check("without asking" in out, f"rule: {out}")
policy = json.load(open("/etc/chromium/policies/managed/bia-client-certs.json"))
entry = json.loads(policy["AutoSelectCertificateForUrls"][0])
check(entry["pattern"] == f"https://localhost:{port}" and entry["filter"]["SUBJECT"]["CN"] == "bia-robot", f"the policy names it: {entry}")
check(entry["filter"]["ISSUER"]["CN"] == "Acme Private CA", "and its issuer")
seen.clear()
tool("restart")
tool("navigate_page", url=url, type="url")
deadline = time.time() + 30
while time.time() < deadline and not seen:
    time.sleep(0.3)
check(seen and seen[-1] == "bia-robot", f"with a rule the site sees the chosen certificate, unasked: {seen}")
out = tool("take_snapshot")
check("hello bia-robot" in out, f"the page loaded: {out[:160]}")

# an OS update takes the policy and the system CA away: the MCP's next start puts them back
for d in ("/etc/chromium/policies/managed/bia-client-certs.json", "/usr/local/share/ca-certificates/bia-Acme-Private-CA.crt"):
    os.remove(d)
subprocess.run(["update-ca-certificates", "--fresh"], capture_output=True)
p.stdin.close()
p.terminate()
p.wait(15)
subprocess.run(["pkill", "-f", "/usr/lib/chromium/chromium"])
p = subprocess.Popen([binary], cwd=mcp_dir, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(work + "/mcp2.log", "w"), env=env, text=True, bufsize=1)
threading.Thread(target=reader, daemon=True).start()
rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})
check(os.path.exists("/etc/chromium/policies/managed/bia-client-certs.json"), "the rule is back after an OS update")
check(os.path.exists("/usr/local/share/ca-certificates/bia-Acme-Private-CA.crt"), "and the system CA")

check("no longer" in tool("cert_unselect", site=url), "unselect")
tool("cert_remove", name="other-person - Acme")
out = tool("cert_import", path="zond.p12", password="x")
check("imported client certificate \"Identity zond\"" in out, f"a colon in a name does not break it: {out[:90]}")
out = tool("cert_import", path="dup1.p12", password="x")
check("Identity root" in out, f"same name, first: {out[:90]}")
out = tool("cert_import", path="dup2.p12", password="x")
check("same subject and issuer" in out and "serial" in out, f"same name, second, and it says so: {out[:230]}")
out = tool("cert_import", path="dup2.p12", password="x")
check("nothing new" in out, f"the same file again adds nothing: {out[:80]}")
listing = tool("cert_list")
print(listing)
names = re.findall(r'^"(Identity root[^"]*)"', listing, re.M)
check(len(names) == 2 and len(set(names)) == 2, f"each has a name of its own: {names}")
check(serial_of("dup1") in listing and serial_of("dup2") in listing, "and the list shows their serials")
for n in names:
    out = tool("cert_select", site=url, cert=n)
    check(out.startswith("error:") and "same subject and issuer" in out, f"no rule that could send either: {out[:110]}")
check(not os.path.exists("/etc/chromium/policies/managed/bia-client-certs.json"), "and none was written")
seen.clear()
serials.clear()
inbound.clear()
tool("restart")
threading.Thread(target=lambda: tool("navigate_page", url=url, type="url"), daemon=True).start()
d = notice()
print(d)
check(d is not None and serial_of("dup1") in d["message"] and serial_of("dup2") in d["message"], "the notice tells the twins apart by serial")
out = tool("choose_certificate", cert="Identity: root")
check("several match" in out and serial_of("dup1") in out, f"their shared name is not enough: {out[:120]}")
out = tool("choose_certificate", cert="Identity: root", issuer="Acme Private CA")
check("several match" in out, "nor with the issuer")
out = tool("choose_certificate", serial=serial_of("dup2"))
check("Chose certificate Identity: root" in out, f"by serial: {out[:90]}")
deadline = time.time() + 25
while time.time() < deadline and not serials:
    time.sleep(0.3)
check(serials == [serial_of("dup2")], f"the site saw exactly that one of the two: {serials} wanted {serial_of('dup2')}")
# the other, by its name in the list
first = next(n for n in names if serial_of("dup1") in next(l for l in listing.splitlines() if l.startswith(f'"{n}"')))
seen.clear()
serials.clear()
inbound.clear()
tool("restart")
threading.Thread(target=lambda: tool("navigate_page", url=url, type="url"), daemon=True).start()
check(notice() is not None, "announced again")
out = tool("choose_certificate", cert=first)
check("Chose certificate Identity: root" in out, f"by its name in the list: {out[:90]}")
deadline = time.time() + 25
while time.time() < deadline and not serials:
    time.sleep(0.3)
check(serials == [serial_of("dup1")], f"which also means exactly that one: {serials} wanted {serial_of('dup1')}")
# with one twin gone a rule is exact again, and the survivor's key still works
second = next(n for n in names if n != first)
check("removed" in tool("cert_remove", name=first), "remove one twin")
listing = tool("cert_list")
check(first not in re.findall(r'^"([^"]+)"', listing, re.M) and serial_of("dup2") in listing and serial_of("dup1") not in listing, "the other stays")
out = tool("cert_select", site=url, cert=second)
check("without asking" in out, f"then a rule is exact: {out[:90]}")
serials.clear()
tool("restart")
tool("navigate_page", url=url, type="url")
deadline = time.time() + 25
while time.time() < deadline and not serials:
    time.sleep(0.3)
check(serials == [serial_of("dup2")], f"and it sends the survivor, unasked, after its rename: {serials}")
tool("cert_unselect", site=url)
for n in re.findall(r'^"(Identity [^"]*)"', listing, re.M):
    tool("cert_remove", name=n)
out = tool("cert_import", path="other-person.pem", key_path="other-person.key")
check("other-person" in out, "back to the first two")

# in another language: the dialog is told and answered by its shape, not its words
p.stdin.close()
p.terminate()
p.wait(15)
subprocess.run(["pkill", "-f", "/usr/lib/chromium/chromium"])
time.sleep(1)
p = subprocess.Popen([binary], cwd=mcp_dir, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(work + "/mcp3.log", "w"), env=dict(env, LANGUAGE="ru"), text=True, bufsize=1)
threading.Thread(target=reader, daemon=True).start()
rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})
seen.clear()
inbound.clear()
tool("restart")
threading.Thread(target=lambda: tool("navigate_page", url=url, type="url"), daemon=True).start()
d = notice()
print(d)
check(d is not None and "bia-robot" in d["message"], "announced in a Russian Chrome")
check(d is not None and re.search("[а-яА-Я]", d["message"].split("On offer:")[0]) is not None, "whose dialog really is in Russian")
out = tool("choose_certificate", cert="bia-robot")
check("Chose certificate bia-robot" in out, f"and answered: {out[:100]}")
deadline = time.time() + 25
while time.time() < deadline and not seen:
    time.sleep(0.3)
check(seen == ["bia-robot"], f"the site saw it: {seen}")
check(not os.path.exists("/etc/chromium/policies/managed/bia-client-certs.json"), "the last rule takes the policy file with it")
check("removed" in tool("cert_remove", name="bia-robot"), "remove a client certificate")
check("bia-robot" not in tool("cert_list"), "with its key")
out = tool("cert_remove", name="Acme Private CA")
check("from this system" in out, f"remove the CA: {out}")
check(subprocess.run(["curl", "-sS", "-o", "/dev/null", "--cert", work + "/bia-robot.pem", "--key", work + "/bia-robot.key", url], capture_output=True).returncode != 0, "the system no longer trusts it")

p.terminate()
wm.terminate()
xvfb.terminate()
subprocess.run(["pkill", "-f", "chromium"])
if failed:
    print(open(work + "/mcp.log").read()[-1500:])
    print("FAILED:", len(failed))
    sys.exit(1)
print("all ok")
