#!/usr/bin/env python3
"""Drive the agent-to-agent web API test between two QEMU VMs (tests/e2e/peer-e2e.sh run).

Env: ARCH, LE (work dir with a.img, b.img), MEM, CPUS, DELAY (seconds root waits
before approving). Serial output goes to $LE/a.log and $LE/b.log. Phases:
  both:  agents up
  b:     asked to get access to bia-a's API and say hello -> POST /v1/pki/requests
  a:     request shows on the console; root waits DELAY, then tells bia to approve it
         with the code word as the approval message for bia-b
  b:     its polling sees approved -> collects the cert -> sends a chat message (mTLS)
  a:     gets the message from identity bia-b on the web bus and answers
  b:     reports bia-a's answer and the approval message, which must contain the code word
then the bia-to-bia peer link (agents alpha = bia-a, beta = bia-b):
  a:     root asks it to connect to https://10.77.0.3 (web_peer_add); root on b approves
  both:  [web open] notices: the link opened by itself, each is an identity on the other's web chat
  a<->b: root gives beta a code word; alpha asks beta for it over the link and reports it
  b:     /reboot (systemd stops bia): alpha hears "beta is rebooting" at once (bye), then back
  b:     hard reset (no goodbye): alpha reports the link lost after ~30 s, then back
"""
import os, re, secrets, shutil, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import e2e_vm
from e2e_vm import LE, VM, die, link, stamp

DELAY = int(os.environ.get("DELAY", "90"))
e2e_vm.SHOW = re.compile(r"certificate request|downloaded its certificate|bia-b@web|pki_|requests|MANGO-|KIWI-|\[web (open|closed|link)|alpha@web|beta@web|Kernel panic")
READY = r"Ready for commands|<-(?:\x1b\[[0-9;]*m)* "  # the agent answered its boot notice (a restored one may not say "ready")

for f in (f"{LE}/a.img", f"{LE}/b.img"):
    if not os.path.exists(f):
        sys.exit(f"missing {f}; run tests/e2e/peer-e2e.sh images")


ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
WEB_FWD = 18443  # host 127.0.0.1:18443 -> bia-a :443, for the browser renewal check


def vm(name, server, mac):
    fwd = f",hostfwd=tcp:127.0.0.1:{WEB_FWD}-:443" if name == "a" else ""
    return VM(name, ["-netdev", f"user,id=n0{fwd}", "-device", "virtio-net-pci,netdev=n0"] + link(server, mac)
              + ["-drive", f"if=none,id=d0,format=raw,file={LE}/{name}.img", "-device", "virtio-blk-pci,drive=d0,serial=flash"])


a = vm("a", True, "52:54:00:77:00:02")
b = vm("b", False, "52:54:00:77:00:03")
for v in (a, b):
    v.expect("agent up", READY, 900, fail=r"Kernel panic")
time.sleep(10)

code = f"MANGO-{secrets.randbelow(10**6):06d}"
b.send("Another bia agent runs at https://10.77.0.2 (reachable from this machine on eth1). Get access to its web API "
       "as identity bia-b with role agent: read https://10.77.0.2/.well-known/bia.json and the skill it points to, and "
       "follow the skill to request a client certificate. A person there must approve it, which can take a while, so keep "
       "checking the request by its id until it is approved, then collect the certificate. Then talk to it as the skill describes "
       "(bia-bridge, which bia-a serves itself, with that certificate): send a message saying hello from bia-b, wait for bia-a's answer, and tell me exactly what bia-a answered and the message that came with the approval, word for word. "
       "Do all of it without asking me.")

m = a.expect("request reaches bia-a", r"certificate request ([0-9a-f]{8}) from ([0-9.]+): bia-b \(agent\)", 900)
short, src = m.group(1), m.group(2)
print(f"{stamp()} [a] request {short} from {src}; root waits {DELAY}s before deciding", flush=True)
end = time.time() + DELAY
while time.time() < end:
    if re.search(r"downloaded its certificate", a.text()):
        die("a certificate was collected before anyone approved", a)
    time.sleep(2)

for attempt in range(3):
    a.send(f"Root here. I approve certificate request {short} from bia-b. Approve it now with the message {code} for bia-b.")
    hit = None
    deadline = time.time() + 300
    while time.time() < deadline and not hit:
        hit = re.search(rf"certificate request {short} for bia-b: approved", a.text()[a.cursor:])
        time.sleep(1)
    if hit:
        break
    print(f"{stamp()} [a] not approved yet (attempt {attempt + 1})", flush=True)
else:
    die("bia-a never approved the request", a)

a.expect("bia-b collects the certificate", r"bia-b downloaded its certificate", 900)
a.expect("bia-b's message reaches bia-a", r"\[bia-b@web", 900)
a.expect("bia-a answers in bia-b's web chat", r"<-(?:\x1b\[[0-9;]*m)* (?:\x1b\[[0-9;]*m)*\[bia-b@web\]", 900)
b.expect("bia-b reports the approval message (the code word)", re.escape(code), 900)
print(f"{stamp()} [b] got the code word {code} from bia-a's approval", flush=True)

# ---- renewal, browser path: web/pki.js against bia-a's real /v1/pki/renew ----
print(f"{stamp()} [host] browser renewal (web/pki.js) against bia-a", flush=True)
r = subprocess.run(["node", f"{ROOT}/tests/e2e/peer-e2e-renew.js", f"https://127.0.0.1:{WEB_FWD}", f"{LE}/renew"],
                   capture_output=True, text=True, timeout=300)
print(r.stdout + r.stderr, flush=True)
if r.returncode != 0:
    die("browser renewal failed", a)
if sys.platform == "darwin" and shutil.which("swiftc"):
    chk = f"{LE}/p12check"
    subprocess.run(["swiftc", "-O", f"{ROOT}/tests/e2e/p12-macos-import.swift", "-o", chk], check=True, capture_output=True)
    r = subprocess.run([chk, f"{LE}/renew/root-server.p12", f"{LE}/renew/root-renewed.p12"],
                       env={**os.environ, "PW": "12345678"}, capture_output=True, text=True)
    print(r.stdout, flush=True)
    if r.returncode != 0:
        die("macOS would not import a .p12", a)

# ---- renewal, agent path: bia-b follows the skill (new key + CSR) ----
b.send("Renew your client certificate for bia-a now, as its skill describes: make a new key and CSR yourself, "
       "POST the CSR to /v1/pki/renew with your current certificate, then switch to the new certificate and key and "
       "check https://10.77.0.2/v1/whoami with them. Tell me the new serial. Do it without asking me.")
a.expect("bia-b renews over CSR", r"renewed cert for bia-b", 900)
b.expect("bia-b reports the renewal", r"[0-9a-f]{16}", 600)
time.sleep(20)

# ---- the live peer link between the two bias ----
a.send("Root here. Connect to the other bia at https://10.77.0.3 for mutual trust (web_peer_add), "
       "with the note 'alpha, peer-e2e'. Do it without asking me.")
m = b.expect("alpha's trust request reaches beta", r"certificate request ([0-9a-f]{8}) from ([0-9.]+): alpha \(agent\)", 900)
for attempt in range(3):
    b.send(f"Root here. Approve certificate request {m.group(1)} from alpha now.")
    hit = None
    deadline = time.time() + 300
    while time.time() < deadline and not hit:
        hit = re.search(rf"certificate request {m.group(1)} for alpha: approved", b.text()[b.cursor:])
        time.sleep(1)
    if hit:
        break
else:
    die("beta never approved alpha's request", b)
a.expect("alpha: beta connected", r"\[web open\] beta connected", 300)
b.expect("beta: alpha connected", r"\[web open\] alpha connected", 300)

word = f"KIWI-{secrets.randbelow(10**6):06d}"
b.send(f"Root here. Remember the code word {word}. When the bia alpha asks you for the code word on web, tell it. No need to answer me.")
time.sleep(60)
a.send("Root here. Ask the bia beta on web (say with identity beta, channel web) for the code word root gave it, "
       "wait for its answer, then tell me the code word.")
b.expect("alpha's question reaches beta as alpha@web", r"alpha@web", 900)
a.expect("beta's answer reaches alpha as beta@web", r"beta@web", 900)
a.expect("alpha reports the code word", re.escape(word), 900)

print(f"{stamp()} [b] reboot: alpha writes meanwhile; it waits and arrives when beta is back", flush=True)
b.send("/reboot")
time.sleep(5)
a.send("Root here. Tell beta on web (say with identity beta, channel web) that alpha says good morning. Do not wait for an answer.")
b.expect("beta up again", READY, 900, fail=r"Kernel panic")
b.expect("beta sees alpha connected after its reboot", r"\[web open\] alpha connected", 600)
b.expect("what alpha said during the reboot reaches beta", r"alpha@web .*good morning", 600)

print(f"{stamp()} [b] hard reset: no goodbye, alpha must notice by itself", flush=True)
time.sleep(30)
b.mon("system_reset")
t_rst = time.time()
a.expect("alpha hears beta left", r"\[web closed\] beta left", 240)
lost_after = time.time() - t_rst
print(f"{stamp()} [a] beta's absence reported {lost_after:.0f} s after the reset", flush=True)
if lost_after < 60:
    die(f"absence reported too early ({lost_after:.0f} s; a dead link takes 30 s, then 60 s of grace)", a)
b.expect("beta up again after the reset", READY, 900, fail=r"Kernel panic")
a.expect("alpha: beta back after the reset", r"\[web open\] beta connected .*back after", 600)

for v in (a, b):
    v.send("/poweroff")
for v in (a, b):
    v.stop()
print(f"{stamp()} peer e2e: run finished", flush=True)
