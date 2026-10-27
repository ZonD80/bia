"""Drive the takeover e2e (tests/e2e/takeover-e2e.sh run).

Ubuntu boots under UEFI with no display device; cloud-init (NoCloud over HTTP from
$TE/www) runs takeover.sh unattended with a config holding ./fmm.json's keys and setup
instructions. The driver downloads root.p12 from the gate page (hostfwd), lets the VM
reboot, watches bia's rescue write the disk, waits for bia's web app and checks it
with that certificate: identity root, the CA the takeover printed, the bootstrap route
closed, seeds moved into place, bia-net's interfaces, setup instructions handed over,
the virtual screen. Then takeover.sh and bia-deploy takeover, run on that bia through
root's shell (/ws/shell), must refuse: a misquoted `ssh host curl … | bash` runs them there.

DEPLOY=1: nothing runs on Ubuntu by itself; this computer's `bia-deploy server` logs in over
ssh as ubuntu with a password (sudo asks for it too): first contact trusts the host key, a
changed key is refused until --host-key names the new one; it plans, takes the server over
and saves root's certificate here.
"""
import base64, hashlib, http.server, json, os, re, shutil, socket, socketserver, ssl, subprocess, sys, tempfile, threading, time, urllib.request

ARCH = os.environ.get("ARCH", "arm64")
TE = os.environ["TE"]
UBUNTU = os.environ["UBUNTU"]
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
MEM, CPUS = os.environ.get("MEM", "6G"), os.environ.get("CPUS", "4")
TIMEOUT = int(os.environ.get("TIMEOUT", "40")) * 60
FW = "/opt/homebrew/share/qemu"
WWW = f"{TE}/www"
LOG = f"{TE}/serial.log"
GATE_PORT, WEB_PORT = 18444, 18443
ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
LAND = bool(os.environ.get("LAND"))
AGENT_LAND = os.environ.get("LAND") == "agent"
DEPLOY = bool(os.environ.get("DEPLOY"))
# DEPLOY: sudo must ask ubuntu's password (cloud images let it sudo without one)
SUDO_PASSWORD = """rm -f /etc/sudoers.d/90-cloud-init-users
echo "takeover-e2e: ready for ssh"
exit 0
"""
# NET=onlink: before the takeover Ubuntu switches to a static /32 with an on-link gateway
# (the Hetzner pattern); the takeover must carry that over or bia is unreachable after it.
NET = os.environ.get("NET", "dhcp")
ONLINK = """cat > /etc/netplan/60-onlink.yaml <<'NP'
network:
  version: 2
  ethernets:
    e2e:
      match: {macaddress: "52:54:00:7e:00:01"}
      dhcp4: false
      dhcp6: false
      addresses: [10.0.2.15/32]
      routes: [{to: default, via: 10.0.2.2, on-link: true}]
      nameservers: {addresses: [10.0.2.3]}
NP
chmod 600 /etc/netplan/60-onlink.yaml
rm -f /etc/netplan/50-cloud-init.yaml
echo 'network: {config: disabled}' > /etc/cloud/cloud.cfg.d/99-e2e-net.cfg
netplan apply
sleep 5
echo "takeover-e2e: network is now $(ip -br -4 addr show scope global | tr -s ' ') via $(ip -4 route show default)"
"""
SECRET = "TKE2-LAND-SECR-ET23-4567-ABCD-EFGH"
SETUP = "Takeover e2e: these are root's setup instructions. Say on the console that you received them; call no other tools."
t0 = time.time()


def say(msg):
    m, s = divmod(int(time.time() - t0), 60)
    print(f"[{m:3d}:{s:02d}] {msg}", flush=True)


def die(msg):
    say("FAIL " + msg)
    print("---- serial tail ----")
    print(ANSI.sub("", serial())[-4000:])
    sys.exit(1)


KMSG = re.compile(r"^\[\s*\d+\.\d+\] ", re.M)


def serial():
    """The serial log without kernel-log timestamps (run.sh writes through /dev/kmsg)."""
    try:
        return KMSG.sub("", open(LOG, errors="replace").read())
    except OSError:
        return ""


FIRMWARE = os.environ.get("FIRMWARE", "uefi")
# x86_64 runs under TCG (emulated): everything takes a few times longer
SCALE = float(os.environ.get("SCALE", "1" if ARCH == "arm64" else "4"))


def wait_for(pat, timeout, fail=None, since=0):
    end = time.time() + timeout * SCALE
    rx = re.compile(pat)
    fx = re.compile(fail) if fail else None
    while time.time() < end:
        text = ANSI.sub("", serial())[since:]
        m = rx.search(text)
        if m:
            return m
        if fx and fx.search(text):
            die(f"saw {fx.pattern!r} while waiting for {pat!r}")
        if proc.poll() is not None:
            die(f"QEMU exited while waiting for {pat!r}")
        time.sleep(2)
    die(f"timeout waiting for {pat!r}")


def wait_quiet(pat, timeout, since=0):
    """wait_for without failing: True when the pattern appeared in time."""
    end = time.time() + timeout * SCALE
    while time.time() < end:
        if re.search(pat, ANSI.sub("", serial())[since:]):
            return True
        if proc.poll() is not None:
            die(f"QEMU exited while waiting for {pat!r}")
        time.sleep(3)
    return False


# ---- web root for cloud-init and the release ----
port = None


class Quiet(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=WWW, **k)

    def log_message(self, *a):
        pass


srv = socketserver.ThreadingTCPServer(("0.0.0.0", 0), Quiet)
port = srv.server_address[1]
threading.Thread(target=srv.serve_forever, daemon=True).start()
host = f"http://10.0.2.2:{port}"
for f in ("go",):
    try:
        os.remove(f"{WWW}/{f}")
    except OSError:
        pass
# takeover.sh lives next to the release files, as in a real release
shutil.copyfile(f"{WWW}/takeover.sh", f"{WWW}/rel/takeover.sh")
fmm = json.load(open(f"{ROOT}/fmm.json"))
config = {
    "fmm": {k: v for k, v in fmm.items() if k in ("provider", "api_key", "model", "base_url", "ctx")},
    "wipe": "/dev/vda",
    "setup": SETUP,
    "lang": "en",
    "reboot": False,
    "gate_wait": 900,
    "pki": {"agent_name": "te-bia"},
}
if LAND:
    config = {"land": {"secret": SECRET}, "wipe": "/dev/vda", "reboot": False}
SSH_KEY = open(f"{TE}/fly_key.pub").read().strip() if AGENT_LAND else ""
open(f"{WWW}/config.json", "w").write(json.dumps(config))
open(f"{WWW}/run.sh", "w").write(f"""#!/bin/bash
# The serial getty hangs up the serial line when it starts; the kernel log reaches the
# serial console without that (and without its rate limit once printk_devkmsg is on).
sysctl -q -w kernel.printk_devkmsg=on
sysctl -q -w kernel.printk="7 4 1 7"
exec > >(while IFS= read -r l; do printf '%s\n' "$l" > /dev/kmsg; done) 2>&1
cd /root
{ONLINK if NET == "onlink" else ""}
{'echo "takeover-e2e: ready for ssh"; exit 0' if AGENT_LAND else ""}
{SUDO_PASSWORD if DEPLOY else ""}
echo "takeover-e2e: fetching takeover.sh"
curl -fsS {host}/takeover.sh -o takeover.sh
curl -fsS {host}/config.json -o config.json
BIA_RELEASE_URL={host}/rel bash takeover.sh --config /root/config.json --yes --no-test
echo "takeover-e2e: takeover exit $?"
until curl -fsS {host}/go >/dev/null 2>&1; do sleep 2; done
echo "takeover-e2e: rebooting"
systemctl reboot
""")
open(f"{WWW}/meta-data", "w").write("instance-id: bia-takeover-e2e\nlocal-hostname: ubuntu-te\n")
open(f"{WWW}/user-data", "w").write(f"""#cloud-config
password: ubuntu
chpasswd: {{expire: false}}
ssh_pwauth: false
disable_root: false
users:
  - name: root
    ssh_authorized_keys: ["{SSH_KEY}"]
runcmd:
  - [bash, -c, "curl -fsS {host}/run.sh | bash"]
""" if AGENT_LAND else f"""#cloud-config
password: ubuntu
chpasswd: {{expire: false}}
ssh_pwauth: true
runcmd:
  - [bash, -c, "curl -fsS {host}/run.sh | bash"]
""" if DEPLOY else f"""#cloud-config
password: ubuntu
chpasswd: {{expire: false}}
runcmd:
  - [bash, -c, "curl -fsS {host}/run.sh | bash"]
""")
open(f"{WWW}/vendor-data", "w").write("")

def land_run():
    """--land: the server waits for a flight with the land secret; a second bia flies there."""
    wait_for(r"fly to this server's address with land secret " + re.escape(SECRET), 30 * 60, fail=r"(?m)takeover-e2e: takeover exit [1-9]|^error: ")
    wait_for(r"takeover-e2e: takeover exit 0", 120)
    say("takeover staged in land mode")
    open(f"{WWW}/go", "w").write("go\n")
    mark = len(ANSI.sub("", serial()))
    wait_for(r"bia-rescue: takeover done", 40 * 60, fail=r"bia-rescue: .*\[ FAIL \]", since=mark)
    say("rescue wrote and verified the disk")
    wait_for(r"bia land \(wait\)", 15 * 60, since=mark)
    wait_for(r"Waiting for another bia to fly here over the internet", 20 * 60, since=mark)
    say("server waits in land mode with its secret")
    s = socket.create_connection(("127.0.0.1", 14791), timeout=10)
    s.sendall(b"BIA1" + bytes(32))
    s.settimeout(30)
    try:
        s.recv(64)
    except OSError:
        pass
    s.close()
    wait_for(r"land: a flight without this machine's land secret", 60, since=mark)
    say("a flight without the secret (BIA1) was refused; still waiting")
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import e2e_vm
    fly = e2e_vm.VM("fly", ["-netdev", "user,id=n1", "-device", "virtio-net-pci,netdev=n1",
                            "-drive", f"if=none,id=d1,format=raw,file={TE}/fly.img", "-device", "virtio-blk-pci,drive=d1,serial=flash"])
    try:
        fly.expect("flyer's agent boots", r"bia-update: (idle|boot)", 900, fail=r"Kernel panic")
        fly.expect("flyer's agent console", r"->|Ready for commands|ready|FMM", 300)
        time.sleep(15)
        ask = (f"A server I took over waits in land mode on the internet. Fly this computer to it now: call carry_fly "
               f"with host 10.0.2.2, port 14791 and secret {SECRET}. Do not ask me first.")
        for attempt in range(4):
            fly.send(ask)
            end = time.time() + 300
            hit = False
            while time.time() < end:
                if re.search(r"land: the flyer knows the land secret", ANSI.sub("", serial())[mark:]):
                    hit = True
                    break
                if proc.poll() is not None or fly.proc.poll() is not None:
                    die("a VM exited while waiting for the flight")
                time.sleep(2)
            if hit:
                break
            say(f"no flight yet (attempt {attempt + 1}); asking again")
        else:
            die("the flyer never proved the land secret")
        say("both sides proved the land secret; the agent travels encrypted")
        wait_for(r"land: archive accepted, unfolding", 30 * 60, fail=r"checksum did not match|did not decrypt", since=mark)
        wait_for(r"unfold: rebooting", 30 * 60, since=mark)
        say("server unfolded the flyer's agent")
        m2 = len(ANSI.sub("", serial()))
        wait_for(r"bia-update: (idle|boot)", 15 * 60, fail=r"Kernel panic", since=m2)
        if re.search(r"bia land \(wait\)", ANSI.sub("", serial())[m2:]):
            die("land mode ran again after the flight")
        wait_for(r"landed from another machine", 10 * 60, since=m2)
        say("PASS takeover e2e (land): the flyer's agent runs on the taken-over server")
        fly.send("/poweroff")
        fly.stop()
    except SystemExit:
        raise
    finally:
        if fly.proc.poll() is None:
            fly.proc.kill()


def agent_land_run():
    """LAND=agent: the flyer's agent prepares the Ubuntu server as its land over ssh and flies there."""
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import e2e_vm
    fly = e2e_vm.VM("fly", ["-netdev", "user,id=n1", "-device", "virtio-net-pci,netdev=n1",
                            "-drive", f"if=none,id=d1,format=raw,file={TE}/fly.img", "-device", "virtio-blk-pci,drive=d1,serial=flash"])
    try:
        fly.expect("flyer's agent boots", r"bia-update: (idle|boot)", 900, fail=r"Kernel panic")
        fly.expect("flyer's agent console", r"->|Ready for commands|ready|FMM", 300)
        time.sleep(15)
        mark = len(serial())
        ask = (f"I rented an Ubuntu server for you. Prepare it as a land for yourself and fly there; I agree that it gets erased, "
               f"do not ask me again. You reach its ssh as root at 10.0.2.2 port 2222 (ssh -p 2222 root@10.0.2.2; your key is in /root/.ssh). "
               f"For this test its takeover script is at {host}/takeover.sh (not GitHub) and needs BIA_RELEASE_URL={host}/rel in its "
               f"environment, so run: curl -fsSL {host}/takeover.sh | BIA_RELEASE_URL={host}/rel bash -s -- --land --land-secret <secret> --yes --wipe <disk> --reboot. "
               f"From this computer, the server's land port 4791 is reachable at 10.0.2.2 port 14791 (carry_fly host 10.0.2.2, port 14791).")
        fly.send(ask)
        say("asked the flyer's agent to take the server over itself and fly there")
        wait_for(r"bia-rescue: takeover: replacing", 30 * 60, since=mark)
        say("the agent ran the takeover over ssh; bia's rescue started on the server")
        wait_for(r"bia-rescue: takeover done", 40 * 60, fail=r"bia-rescue: .*\[ FAIL \]", since=mark)
        say("rescue wrote and verified the disk")
        wait_for(r"Waiting for another bia to fly here over the internet", 20 * 60, since=mark)
        say("server waits in land mode with the agent's secret")
        # the agent may already be in carry_fly (it keeps trying); remind it once if not
        if not wait_quiet(r"land: the flyer knows the land secret", 10 * 60, since=mark):
            fly.send("The server is waiting in land mode now. Fly there with carry_fly (host 10.0.2.2, port 14791) and the land secret you made.")
            say("reminded the agent to fly")
            wait_for(r"land: the flyer knows the land secret", 20 * 60, since=mark)
        say("both sides proved the agent's own land secret; the agent travels encrypted")
        wait_for(r"land: archive accepted, unfolding", 30 * 60, fail=r"checksum did not match|did not decrypt", since=mark)
        wait_for(r"unfold: rebooting", 30 * 60, since=mark)
        m2 = len(serial())
        wait_for(r"bia-update: (idle|boot)", 15 * 60, fail=r"Kernel panic", since=m2)
        wait_for(r"landed from another machine", 10 * 60, since=m2)
        say("PASS takeover e2e (agent): the agent prepared a server over ssh by itself and now also runs there")
        fly.send("/poweroff")
        fly.stop()
    finally:
        if fly.proc.poll() is None:
            fly.proc.kill()


def deploy_run():
    """DEPLOY: bia-deploy server from this computer; root's certificate, CA PEM, CA fingerprint."""
    home = f"{TE}/deploy-home"
    shutil.rmtree(home, ignore_errors=True)
    env = dict(os.environ, BIA_DEPLOY_HOME=home, BIA_RELEASE_URL=f"{host}/rel", BIA_SSH_PASSWORD="ubuntu")
    cmd = [f"{ROOT}/target/release/bia-deploy", "server", "127.0.0.1", "--port", "2222", "--user", "ubuntu",
           "--fmm", f"{ROOT}/fmm.json", "--lang", "en", "--yes", "--no-test", "--no-trust", "--no-wait"]
    # first contact needs no confirmation: a wrong password gets as far as the login
    r = subprocess.run(cmd, env={**env, "BIA_SSH_PASSWORD": "wrong"}, capture_output=True, text=True, timeout=180)
    if r.returncode == 0 or "did not let ubuntu in" not in r.stderr:
        die("first contact did not reach the login: " + r.stderr[-800:])
    if os.path.exists(f"{home}/known_hosts.json"):
        die("a host key was remembered without a successful login")
    say("first contact goes straight to the login (no host key question)")
    # a key that changed since it was remembered stops it, naming the new one
    os.makedirs(home, exist_ok=True)
    json.dump({"127.0.0.1:2222": "SHA256:stale0000000000000000000000000000000000000000"}, open(f"{home}/known_hosts.json", "w"))
    r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=180)
    m = re.search(r"--host-key (SHA256:\S+)", r.stderr)
    if r.returncode == 0 or not m or "identity changed" not in r.stderr:
        die("a changed host key was not refused: " + r.stderr[-800:])
    say("a changed host key is refused until --host-key names the new one")
    log = open(f"{TE}/deploy.log", "w")
    p = subprocess.run(cmd + ["--host-key", m.group(1)], env=env, stdout=log, stderr=subprocess.STDOUT, text=True, timeout=int(40 * 60 * SCALE))
    out = open(f"{TE}/deploy.log").read()
    print("\n".join("  | " + l for l in out.splitlines()[-25:]), flush=True)
    if p.returncode != 0:
        die(f"bia-deploy server failed (exit {p.returncode}), see {TE}/deploy.log")
    if "root through sudo" not in out or "the server reboots into bia's installer now" not in out:
        die("bia-deploy did not go through sudo to the reboot")
    key = fmm.get("api_key", "")
    if len(key) > 12 and key[6:-4] in out:
        die("the API key reached bia-deploy's output")
    known = json.load(open(f"{home}/known_hosts.json"))
    if known.get("127.0.0.1:2222") != m.group(1):
        die(f"the host key was not remembered: {known}")
    fp = re.search(r"CA fingerprint \(SHA-256\): ([0-9a-f]{64})", out)
    if not fp:
        die("bia-deploy printed no CA fingerprint")
    d = f"{home}/servers/127.0.0.1"
    p12, ca_pem = open(f"{d}/bia-root.p12", "rb").read(), open(f"{d}/ca.crt", "rb").read()
    if hashlib.sha256(ssl.PEM_cert_to_DER_cert(ca_pem.decode())).hexdigest() != fp.group(1):
        die("the CA bia-deploy saved does not match the fingerprint")
    say(f"bia-deploy took the server over: root.p12 ({len(p12)} bytes) and the CA saved here")
    return p12, ca_pem, fp.group(1)


# ---- QEMU: UEFI, no display device, user net with the gate and bia's HTTPS forwarded ----
shutil.copyfile(f"{FW}/edk2-arm-vars.fd" if ARCH == "arm64" else f"{FW}/edk2-i386-vars.fd", f"{TE}/vars.fd")
if ARCH == "arm64":
    cmd = ["qemu-system-aarch64", "-M", "virt", "-accel", "hvf", "-cpu", "host",
           "-drive", f"if=pflash,format=raw,readonly=on,file={FW}/edk2-aarch64-code.fd",
           "-drive", f"if=pflash,format=raw,file={TE}/vars.fd"]
else:
    # no VGA: a cloud VM without any display device (bia must fall back to a virtual screen)
    cmd = ["qemu-system-x86_64", "-M", "q35", "-accel", "tcg,thread=multi", "-cpu", "max", "-vga", "none"]
    if FIRMWARE == "uefi":
        cmd += ["-drive", f"if=pflash,format=raw,readonly=on,file={FW}/edk2-x86_64-code.fd",
                "-drive", f"if=pflash,format=raw,file={TE}/vars.fd"]
mon = f"/tmp/bia-te-{os.getpid()}.mon"
cmd += ["-smp", CPUS, "-m", MEM, "-device", "virtio-rng-pci",
        "-drive", f"if=none,id=d0,format=qcow2,file={TE}/disk.qcow2", "-device", "virtio-blk-pci,drive=d0",
        "-netdev", f"user,id=n0,hostfwd=tcp:127.0.0.1:{GATE_PORT}-:8443,hostfwd=tcp:127.0.0.1:{WEB_PORT}-:443,hostfwd=tcp:127.0.0.1:14791-:4791,hostfwd=tcp:127.0.0.1:2222-:22",
        "-device", "virtio-net-pci,netdev=n0,mac=52:54:00:7e:00:01",
        "-smbios", f"type=1,serial=ds=nocloud;s={host}/",
        "-display", "none", "-monitor", f"unix:{mon},server,nowait", "-serial", f"file:{LOG}"]
if os.path.exists(LOG):
    os.remove(LOG)
say(f"booting Ubuntu ({ARCH}, {FIRMWARE}, network {NET})")
proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=open(f"{TE}/qemu.err", "w"))
try:
    if DEPLOY:
        wait_for(r"takeover-e2e: ready for ssh", 15 * 60)
        say("Ubuntu is up: ubuntu logs in with a password, sudo asks for it")
        mark = len(ANSI.sub("", serial()))
        p12, ca_pem, fp = deploy_run()
        open(f"{TE}/root.p12", "wb").write(p12)
        open(f"{TE}/ca.pem", "wb").write(ca_pem)
    if AGENT_LAND:
        wait_for(r"takeover-e2e: ready for ssh", 15 * 60)
        say("Ubuntu is up with the flyer's ssh key")
        agent_land_run()
        sys.exit(0)
    if not DEPLOY:
        wait_for(r"takeover-e2e: fetching takeover\.sh", 15 * 60)
        say("cloud-init started the takeover")
        if LAND:
            land_run()
            sys.exit(0)
        m = wait_for(r"https://\S+?:8443/([0-9a-f]{32})/", 30 * 60, fail=r"(?m)takeover-e2e: takeover exit [1-9]|^error: ")
        token = m.group(1)
        # the takeover's stderr (through /dev/kmsg) and its own console copy (announce) can
        # interleave on the serial line: take the first fingerprint after the heading
        fp = wait_for(r"(?s)CA fingerprint \(SHA-256\) of the new bia:.*?\b([0-9a-f]{64})\b", 60).group(1)
        say(f"gate up; CA {fp[:16]}…")
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
        p12 = urllib.request.urlopen(f"https://127.0.0.1:{GATE_PORT}/{token}/root.p12", context=ctx, timeout=30).read()
        open(f"{TE}/root.p12", "wb").write(p12)
        # the gate offers no separate CA: root.p12 carries it
        ca = subprocess.run(["openssl", "pkcs12", "-in", f"{TE}/root.p12", "-passin", "pass:12345678", "-cacerts", "-nokeys"],
                            capture_output=True, text=True)
        m_ca = re.search(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----", ca.stdout, re.S)
        if not m_ca:
            die("root.p12 holds no CA certificate: " + ca.stderr)
        ca_pem = (m_ca.group(0) + "\n").encode()
        open(f"{TE}/ca.pem", "wb").write(ca_pem)
        st404 = 0
        try:
            urllib.request.urlopen(f"https://127.0.0.1:{GATE_PORT}/{token}/ca.crt", context=ctx, timeout=30)
        except urllib.error.HTTPError as e:
            st404 = e.code
        if st404 != 404:
            die(f"the gate still serves a separate CA (HTTP {st404 or 200})")
        der = ssl.PEM_cert_to_DER_cert(ca_pem.decode())
        if hashlib.sha256(der).hexdigest() != fp:
            die("the gate's CA does not match the fingerprint the terminal showed")
        say(f"root.p12 downloaded from the gate ({len(p12)} bytes); the CA inside it matches")
        wait_for(r"takeover-e2e: takeover exit 0", 120)
        # the takeover also writes to every console itself (announce), without the gate's secrets
        mirrored = [l for l in ANSI.sub("", serial()).splitlines() if "bia takeover: " in l]
        if not any("ROOT'S CERTIFICATE MUST BE DOWNLOADED NOW" in l for l in mirrored) or \
                not any("root's certificate was downloaded from the gate page" in l for l in mirrored):
            die(f"the takeover did not announce itself on the console ({len(mirrored)} lines)")
        if any(token in l for l in mirrored):
            die("the gate's token reached the console")
        say(f"the takeover announced itself on the console ({len(mirrored)} lines), without the gate's token")
        open(f"{WWW}/go", "w").write("go\n")
        mark = len(ANSI.sub("", serial()))
    wait_for(r"bia-rescue: takeover: replacing", 10 * 60, since=mark)
    say("bia's rescue started from the one-shot boot entry")
    wait_for(r"bia-rescue: takeover done", 30 * 60, fail=r"bia-rescue: .*\[ FAIL \]", since=mark)
    say("rescue wrote and verified the disk")
    # ---- bia ----
    pem = subprocess.run(["openssl", "pkcs12", "-in", f"{TE}/root.p12", "-passin", "pass:12345678", "-nodes"],
                         capture_output=True, text=True)
    if pem.returncode != 0:
        die("openssl cannot read root.p12: " + pem.stderr)
    open(f"{TE}/root.pem", "w").write(pem.stdout)
    cctx = ssl.create_default_context(cafile=f"{TE}/ca.pem")
    cctx.check_hostname = False
    cctx.load_cert_chain(f"{TE}/root.pem")

    def get(path, ctx=cctx, method="GET"):
        req = urllib.request.Request(f"https://127.0.0.1:{WEB_PORT}{path}", method=method, data=b"{}" if method == "POST" else None)
        try:
            r = urllib.request.urlopen(req, context=ctx, timeout=20)
            return r.status, r.read()
        except urllib.error.HTTPError as e:
            return e.code, e.read()

    end = time.time() + 20 * 60
    while True:
        try:
            st, body = get("/v1/whoami")
            if st == 200:
                break
        except Exception:
            pass
        if time.time() > end:
            die("bia's web app did not answer with root's certificate")
        time.sleep(5)
    say("bia is up: " + body.decode()[:120])
    errs = []

    def want(ok, msg):
        print(("  ok   " if ok else "  FAIL ") + msg, flush=True)
        if not ok:
            errs.append(msg)

    want(json.loads(body).get("identity") == "root", "root's certificate from the gate works (identity root)")
    nctx = ssl.create_default_context(cafile=f"{TE}/ca.pem")
    nctx.check_hostname = False
    st, _ = get("/v1/pki/bootstrap/root", ctx=nctx, method="POST")
    want(st == 409, f"first-time root certificate route is closed (HTTP {st})")
    st, ca_now = get("/v1/pki/ca", ctx=nctx)
    want(hashlib.sha256(ssl.PEM_cert_to_DER_cert(ca_now.decode())).hexdigest() == fp, "bia serves the CA the takeover made")
    sc = subprocess.run(["openssl", "s_client", "-connect", f"127.0.0.1:{WEB_PORT}", "-showcerts"],
                        input="", capture_output=True, text=True, timeout=30)
    chain = re.findall(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----", sc.stdout, re.S)
    want(len(chain) == 2 and hashlib.sha256(ssl.PEM_cert_to_DER_cert(chain[1])).hexdigest() == fp,
         f"the web app sends the full chain: server certificate, then the CA ({len(chain)} certificate(s))")
    st, agent = get("/v1/pki/agent", ctx=nctx)
    want(st == 200 and json.loads(agent).get("name") == ("te-bia" if not DEPLOY else json.loads(agent).get("name") or "?"),
         "agent name from the config, agent CSR published")

    def fsget(p):
        from urllib.parse import quote
        return get("/v1/fs/get?path=" + quote(p))

    st, net = fsget("/etc/network/interfaces")
    if NET == "onlink":
        want(st == 200 and b"written by bia-net" in net and b"address 10.0.2.15/32" in net
             and b"route replace default via 10.0.2.2 dev" in net and b"onlink" in net,
             "bia-net wrote the static /32 with its on-link gateway (and bia is reachable through it)")
        st, rc = fsget("/etc/resolv.conf")
        want(st == 200 and b"nameserver 10.0.2.3" in rc, "name servers carried over")
    else:
        want(st == 200 and b"written by bia-net" in net and b"inet dhcp" in net, "bia-net wrote the interfaces (DHCP, by MAC)")
    st, _ = fsget("/etc/bia/net/5254007e0001.conf")
    want(st == 200, "network seed keyed by the VM's MAC")
    st, _ = fsget("/boot/EFI/bia/seed/setup.md")
    want(st != 200, "seeds were removed from the ESP")
    st, _ = fsget("/home/bia/files/state/fmm.json")
    want(st == 200, "FMM keys seeded")
    if DEPLOY:
        st, _ = fsget("/home/bia/files/state/setup.md")
        want(st != 200, "bia-deploy hands over no setup instructions")
    else:
        end = time.time() + 5 * 60
        while True:
            st, s = fsget("/home/bia/files/state/setup.done.md")
            if st == 200 or time.time() > end:
                break
            time.sleep(5)
        want(st == 200 and b"Takeover e2e" in s, "setup instructions handed to the model once (setup.done.md)")
    st, xlog = fsget("/var/log/Xorg.bia.log")
    want(st == 200 and b"dummy" in xlog.lower(), "no display device: Xorg runs on the virtual screen")

    def shell(line, timeout=180):
        """Type line into root's shell (/ws/shell); its output and exit status."""
        s = cctx.wrap_socket(socket.create_connection(("127.0.0.1", WEB_PORT), timeout=30))
        key = base64.b64encode(os.urandom(16)).decode()
        s.sendall(f"GET /ws/shell?rows=50&cols=400 HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                  f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n".encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            c = s.recv(4096)
            if not c:
                die("/ws/shell closed during the handshake")
            buf += c
        head, buf = buf.split(b"\r\n\r\n", 1)
        if b" 101 " not in head.split(b"\r\n")[0]:
            die("/ws/shell: " + head.decode(errors="replace")[:200])
        data = (line + "; echo TE-END-$?\n").encode()
        mask = os.urandom(4)
        n = len(data)
        hdr = bytes([0x82, 0x80 | n]) if n < 126 else bytes([0x82, 0x80 | 126]) + n.to_bytes(2, "big")
        s.sendall(hdr + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))
        out, end = b"", time.time() + timeout * SCALE
        while time.time() < end:
            while len(buf) >= 2:
                ln, off = buf[1] & 0x7F, 2
                if ln == 126:
                    ln, off = int.from_bytes(buf[2:4], "big"), 4
                elif ln == 127:
                    ln, off = int.from_bytes(buf[2:10], "big"), 10
                if len(buf) < off + ln:
                    break
                op, payload, buf = buf[0] & 0x0F, buf[off:off + ln], buf[off + ln:]
                if op == 8:
                    die("/ws/shell closed: " + ANSI.sub("", out.decode(errors="replace"))[-500:])
                if op in (1, 2):
                    out += payload
            # no \r: a line can start with one (bracketed paste off), which hides ^ matches
            text = ANSI.sub("", out.decode(errors="replace")).replace("\r", "")
            m = re.search(r"TE-END-(\d+)", text)
            if m:
                s.close()
                return text, int(m.group(1))
            try:
                c = s.recv(65536)
            except socket.timeout:
                continue
            if not c:
                break
            buf += c
        die("no end of the shell command: " + ANSI.sub("", out.decode(errors="replace"))[-500:])

    # user bia (Chrome's passkey native host) must reach /run/bia; the agent once made it 0700
    out, rc = shell("stat -c 'mode %a %U' /run/bia /run/bia/root; ls /run/bia/root")
    modes = re.findall(r"(?m)^mode (\d+) (\w+)", out)
    print(f"  ..   /run/bia, /run/bia/root: {modes}", flush=True)
    want(rc == 0 and modes == [("755", "root"), ("700", "root")] and re.search(r"(?m)^console\.sock", out),
         "/run/bia stays 0755 after the agent started; root's console socket is in /run/bia/root (0700)")
    out, rc = shell("runuser -u bia -- test -x /run/bia && echo bia-can''-enter")
    want(re.search(r"(?m)^bia-can-enter", out), "user bia can enter /run/bia")

    # web presence: the model hears when root's web app opens and closes, once each, not
    # for a second tab or a quick reconnect (bia-mcp-web bus.rs web_opened/web_closed)
    def ws_chat():
        s = cctx.wrap_socket(socket.create_connection(("127.0.0.1", WEB_PORT), timeout=30))
        key = base64.b64encode(os.urandom(16)).decode()
        s.sendall(f"GET /ws HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                  f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n".encode())
        head = b""
        while b"\r\n\r\n" not in head:
            c = s.recv(4096)
            if not c:
                die("/ws closed during the handshake")
            head += c
        if b" 101 " not in head.split(b"\r\n")[0]:
            die("/ws: " + head.decode(errors="replace")[:200])
        return s

    def ws_close(s):
        mask = os.urandom(4)
        try:
            s.sendall(bytes([0x88, 0x80]) + mask)
            s.close()
        except OSError:
            pass

    def conversation():
        st, b = fsget("/home/bia/files/state/conversation.json")
        return b.decode(errors="replace") if st == 200 else ""

    def wait_conv(pat, timeout):
        end = time.time() + timeout * SCALE
        while time.time() < end:
            if re.search(pat, conversation()):
                return True
            time.sleep(5)
        return False

    tab1 = ws_chat()
    want(wait_conv(r"\[web open\] root opened the web app", 240), "the model hears that root opened the web app")
    tab2 = ws_chat()          # a second tab
    time.sleep(3)
    ws_close(tab1)            # one of two tabs closes
    time.sleep(3)
    ws_close(tab2)            # the last one drops …
    time.sleep(3)
    tab3 = ws_chat()          # … and comes back within the grace
    time.sleep(75)            # past the 60 s grace of the drop
    conv = conversation()
    want(len(re.findall(r"\[web open\] root", conv)) == 1 and "[web closed] root" not in conv,
         "a second tab and a quick reconnect are not reported")
    ws_close(tab3)            # closed for good
    want(wait_conv(r"\[web closed\] root closed the web app", 240), "the model hears that root closed the web app (after the grace)")
    conv = conversation()
    want(len(re.findall(r"\[web open\] root", conv)) == 1 and len(re.findall(r"\[web closed\] root", conv)) == 1,
         "exactly one open and one close notice")
    for m in re.finditer(r"\[web (open|closed)\] root[^\"]{0,80}", conv):
        print(f"  ..   {m.group(0)}", flush=True)

    # the release image is the stick's ("bia flash"); a taken-over server's menu says "bia disk"
    out, rc = shell("grep -h '^menuentry' /boot/EFI/BOOT/grub.cfg /boot/boot/grub/grub.cfg 2>/dev/null | head -4")
    titles = re.findall(r'(?m)^menuentry "([^"]+)"', ANSI.sub("", out))
    want(titles and titles[0] == "bia disk" and "bia flash" not in titles, f"the boot menu entry is \"bia disk\" ({titles})")

    refused = "this machine already runs bia"
    out, rc = shell(f"curl -fsS {host}/takeover.sh | BIA_RELEASE_URL={host}/rel bash -s -- --land --land-secret {SECRET} --yes --wipe /dev/vda --reboot")
    want(rc != 0 and refused in out, f"takeover.sh refuses on bia itself (exit {rc})")
    out, rc = shell(f"rm -rf /tmp/te-tool && mkdir /tmp/te-tool && curl -fsS {host}/rel/bia-deploy-linux-{ARCH}.tar.gz | tar -C /tmp/te-tool -xz && "
                    f"/tmp/te-tool/bia-deploy/bia-deploy takeover --land --land-secret {SECRET} --yes --wipe /dev/vda --reboot")
    want(rc != 0 and refused in out, f"bia-deploy takeover refuses on bia itself (exit {rc})")
    st, _ = get("/v1/whoami")
    want(st == 200, "bia still runs after both refusals")
    if errs:
        die(f"{len(errs)} check(s) failed")
    say("PASS takeover e2e")
finally:
    try:
        s = socket.socket(socket.AF_UNIX)
        s.connect(mon)
        s.send(b"quit\n")
        s.close()
    except OSError:
        pass
    proc.wait(timeout=30)
    srv.shutdown()
