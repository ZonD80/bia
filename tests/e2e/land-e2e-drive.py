#!/usr/bin/env python3
"""Drive the land ISO install and a fly/land between two QEMU VMs (tests/e2e/land-e2e.sh run).

Env: ARCH, LE (work dir with bia-land.iso link, land.img, fly.img), MEM, CPUS.
Serial output of each VM goes to $LE/<vm>.log. Phases:
  land VM: ISO boots -> installer asks network (static) and disk -> copies -> reboot
           with the ISO ejected -> disk boots and waits in land
  fly VM:  agent up -> asked to fly -> CODE on both -> typed on land, confirmed on fly
  land VM: receives, unfolds, reboots -> flyer's agent starts on the land disk
"""
import os, re, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import e2e_vm
from e2e_vm import ARCH, LE, ROOT, VM, die, link, stamp

ISO = f"{ROOT}/dist/{ARCH}/bia-land.iso"
e2e_vm.SHOW = re.compile(r"bia land|bia-land|land:|fly:|CODE|Installed on|Erasing|copying\.\.\.|unfold:|both sides|"
                         r"code matches|code does not|landed|Kernel panic|archive|carry_|Waiting for another")


for f in (ISO, f"{LE}/land.img", f"{LE}/fly.img"):
    if not os.path.exists(f):
        sys.exit(f"missing {f}; run tests/e2e/land-e2e.sh images")

LAND_FAIL = r"bia-land: .*failed|Copy onto the disk failed|Partitions did not appear|is too small|No disk|Kernel panic|land: [a-z].*(timed out|failed|refused|mismatch|did not)"

# ---- 1. land VM: ISO install ----
land = VM("land", link(True, "52:54:00:77:00:02") + [
    "-drive", f"if=none,id=d0,format=raw,file={LE}/land.img", "-device", "virtio-blk-pci,drive=d0,serial=landdisk",
    "-device", "virtio-scsi-pci,id=scsi0",
    "-drive", f"if=none,id=cd0,media=cdrom,readonly=on,format=raw,file={ISO}",
    "-device", "scsi-cd,drive=cd0,bus=scsi0.0"])
land.expect("ISO boots into the installer", r"bia land \(setup\)", 600, fail=LAND_FAIL)
land.expect("network question", r"Use DHCP\? \[yes/no\]", 120, fail=LAND_FAIL)
time.sleep(1)
land.send("no")
land.expect("address question", r"Address with prefix", 60)
land.send("10.77.0.2/24")
land.expect("gateway question", r"Default gateway", 60)
land.send("10.77.0.3")  # the fly VM routes the link to the internet
land.expect("DNS question", r"DNS servers", 60)
land.send("")
land.expect("static summary", r"Static 10\.77\.0\.2/24 via 10\.77\.0\.3", 60)
m = land.expect("disk list", r"Disks that can be installed[^\n]*\n((?:\s+\S+[^\n]*\n)+)", 60)
disks = re.findall(r"^\s+(\S+)", m.group(1), re.M)
print(f"{stamp()} [land] disks offered: {disks}", flush=True)
if any(d.startswith("sr") for d in disks):
    die("installer offered the ISO drive", land)
if disks != ["vda"]:
    die(f"expected only vda, got {disks}", land)
land.expect("disk question", r"Type the disk name", 60)
land.send("vda")
land.expect("erase + install", r"Erasing /dev/vda", 60, fail=LAND_FAIL)
land.expect("install finished", r"Installed on /dev/vda\. 10\.77\.0\.2/24 via 10\.77\.0\.3", 1800, fail=LAND_FAIL)
land.expect("reboot prompt", r"Press enter to reboot", 120, fail=LAND_FAIL)
print(f"{stamp()} [land] ejecting the ISO: {land.mon('eject -f cd0').strip()[-120:]!r}", flush=True)
land.send("")

# ---- 2. land VM: installed disk waits ----
land.expect("disk boots in land (wait)", r"bia land \(wait\)", 900, fail=LAND_FAIL)
if re.search(r"bia land \(setup\)", land.text()[land.cursor:]):
    die("the ISO installer ran again", land)
land.expect("static address up", r"10\.77\.0\.2/24", 180, fail=LAND_FAIL)

# ---- 3. fly VM (also the land VM's router to the internet) ----
fly = VM("fly", ["-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0"]
         + link(False, "52:54:00:77:00:03")
         + ["-drive", f"if=none,id=d0,format=raw,file={LE}/fly.img", "-device", "virtio-blk-pci,drive=d0,serial=flash"])
land.expect("land waits for the internet", r"No internet on eth0 yet", 60, fail=LAND_FAIL)
land.expect("internet via the fly VM", r"Internet reachable", 900, fail=LAND_FAIL)
land.expect("land listening", r"land: listening on 0\.0\.0\.0:4791", 120, fail=LAND_FAIL)
fly.expect("agent boots", r"bia-update: (idle|boot)", 900, fail=r"Kernel panic")
fly.expect("agent console", r"->|Ready for commands|ready|FMM", 300)
time.sleep(15)

ASK = ("There is another bia computer on the local network waiting in land mode. "
       "Fly this computer to it now: call carry_fly without a host. Do not ask me first.")
code = None
for attempt in range(4):
    fly.send(ASK)
    end = time.time() + 300
    while time.time() < end:
        m = re.search(r"CODE (\d{6})", land.text()[land.cursor:])
        if m:
            code = m.group(1)
            break
        if re.search(r"several machines are landing|no machine is landing", fly.text()[fly.cursor:]):
            break
        if land.proc.poll() is not None or fly.proc.poll() is not None:
            die("a VM exited while waiting for the code")
        time.sleep(1)
    if code:
        break
    print(f"{stamp()} [fly] no code yet (attempt {attempt + 1}); asking again", flush=True)
if not code:
    die("no CODE on the land console after asking the flyer")
print(f"{stamp()} [land] code {code}", flush=True)
land.expect("code shown on land", rf"CODE {code}", 5)
fly_code = re.search(r"CODE (\d{6})", fly.text())
if fly_code and fly_code.group(1) != code:
    die(f"codes differ: land {code}, fly {fly_code.group(1)}")
print(f"{stamp()} [fly] code on the fly console: {fly_code.group(1) if fly_code else '(not printed)'}", flush=True)

time.sleep(2)
land.send(code)
land.expect("land accepts the code", r"code matches", 30, fail=r"code does not match")
fly.send(f"The landing computer shows code {code} and I typed it there. The two screens match. "
         f"Call carry_confirm with code {code} now.")

# ---- 4. transfer + unfold on land ----
land.expect("both sides confirmed", r"both sides confirmed", 300, fail=r"did not confirm|code does not match")
land.expect("archive received", r"land: archive accepted, unfolding", 1800, fail=LAND_FAIL + r"|checksum did not match")
# "replace busy ... (os error 26)" is normal: the running carry binary is unlinked and replaced
land.expect("unfold reboots", r"unfold: rebooting", 1800, fail=r"unfold: (?!replace busy|replaced|skip busy).*(failed|error)")
# the flyer's ack (carry_fly's job result) is checked in fly.img's session log afterwards: the
# console shows it only when it lands outside a model reply

# ---- 5. land VM boots the flyer's agent ----
land.expect("land disk reboots into the agent", r"bia-update: (idle|boot)", 900, fail=r"Kernel panic")
if re.search(r"bia land \(wait\)", land.text()[land.cursor:]):
    die("land mode ran again after the flight", land)
land.expect("agent console on the land machine", r"->|Ready for commands|ready|FMM", 300)
time.sleep(20)
notice = re.search(r"landed from another machine", land.text())
print(f"{stamp()} [land] boot notice on console: {'yes' if notice else 'not seen'}", flush=True)

for v in (land, fly):
    v.send("/poweroff")
for v in (land, fly):
    v.stop()
print(f"{stamp()} land e2e: run finished", flush=True)
