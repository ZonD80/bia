#!/usr/bin/env python3
"""Drive the bia disk image boot test (tests/e2e/img-e2e.sh run).

Env: LE (dir with stick.img), ARCH, MODE (flash|dd), FIRMWARE (uefi|bios), MODEL, MEM, CPUS,
TIMEOUT (minutes per long phase). Serial output: $LE/stick.log.
"""
import os, re, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import e2e_vm
from e2e_vm import LE, VM, die, stamp

T = int(os.environ.get("TIMEOUT", "30")) * 60
MODE, FIRMWARE = os.environ.get("MODE", "flash"), os.environ.get("FIRMWARE", "uefi")
e2e_vm.SHOW = re.compile(r"(?i)DMI:|command line:|bia-grow|Ready for commands|Powering off|reboot: Power down|"
                         r"Kernel panic|FMM seeded")
READY = r"Ready for commands|<-(?:\x1b\[[0-9;]*m)* "  # the agent answered its boot notice (a first start greets instead)
FAIL = r"Kernel panic|bia-grow: .*(could not|failed|mismatch|overwritten|still sees)"

if not os.path.exists(f"{LE}/stick.img"):
    sys.exit(f"missing {LE}/stick.img; run tests/e2e/img-e2e.sh images")

DECOY = bool(os.environ.get("DECOY"))


def part3_guid(path):
    """The unique GUID of GPT partition 3 (LBA 2, 128-byte entries, GUID at +16)."""
    import uuid
    with open(path, "rb") as f:
        f.seek(512 * 2 + 128 * 2 + 16)
        return str(uuid.UUID(bytes_le=f.read(16)))


stick = ["-drive", f"if=none,id=d0,format=raw,file={LE}/stick.img",
         "-device", "virtio-blk-pci,drive=d0,serial=stick,bootindex=0"]
net = ["-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0"]
vm = VM("stick", net + stick, firmware=FIRMWARE)

# GRUB passes the PARTUUID of the root partition on the disk it booted from (the label
# would match a second bia drive too)
vm.expect("kernel booted by its own root's PARTUUID", r"(?i)command line:[^\n]*root=PARTUUID=[0-9a-f-]+", T)
if FIRMWARE == "bios":
    m = vm.expect("firmware is SeaBIOS (no UEFI)", r"DMI: [^\n]*BIOS ([^\n]*)", 300)
    if re.search(r"edk2|ovmf", m.group(1), re.I) or re.search(r"efi: EFI v", vm.text()):
        die(f"not a legacy BIOS boot: {m.group(1)}", vm)
if MODE == "dd":
    vm.expect("bia-grow grows BIAROOT over the disk", r"bia-grow: growing BIAROOT over the whole disk", T, fail=FAIL)
    vm.expect("bia-grow grows ext4", r"bia-grow: growing root to the whole partition", T, fail=FAIL)
else:
    vm.expect("bia-grow moves the model in", r"bia-grow: moving model", T, fail=FAIL)
    vm.expect("model ready", r"bia-grow: [^\n]*\.gguf: ready", T, fail=FAIL)
    if re.search(r"growing BIAROOT over the whole disk", vm.text()):
        die("bia-deploy left BIAROOT short of the disk end", vm)
vm.expect("agent up on the FMM keys", READY, T, fail=FAIL)
time.sleep(10)
vm.send("/poweroff")
vm.stop()
print(f"{stamp()} [stick] powered off", flush=True)

if DECOY:
    # again, with the release image on a disk the kernel sees first (vda), as a taken-over
    # server's: the same labels, and (being a raw copy) the image's own GUIDs
    decoy = ["-drive", f"if=none,id=d1,format=raw,file={LE}/decoy.img,snapshot=on",
             "-device", "virtio-blk-pci,drive=d1,serial=decoy"]
    vm = VM("stick-decoy", net + decoy + stick, firmware=FIRMWARE)
    m = vm.expect("kernel booted by its own root's PARTUUID", r"(?i)command line:[^\n]*root=PARTUUID=([0-9a-f-]+)", T)
    got, own, other = m.group(1).lower(), part3_guid(f"{LE}/stick.img"), part3_guid(f"{LE}/decoy.img")
    if got != own:
        die(f"root=PARTUUID={got} is not the stick's root ({own}; the decoy's is {other})", vm)
    if own == other:
        # in dd mode both are raw copies of one image: the stick's first boot (bia-esp-id)
        # must have given it GUIDs of its own
        die(f"the stick and the decoy share the root GUID {own} (bia-esp-id did not replace the image's)", vm)
    print(f"{stamp()} [stick] with the decoy first: root=PARTUUID={got} is the stick's (decoy {other})", flush=True)
    # the kernel mounted the stick's root (vdb: the decoy is vda), and the agent came up there
    # (a second boot need not say anything: wait for its ready line)
    vm.expect("root mounted from the stick (vdb3)", r"EXT4-fs \(vdb3\): mounted filesystem", T, fail=FAIL + r"|EXT4-fs \(vda3\): mounted filesystem")
    vm.expect("agent up from the stick", r"ready: \d+ tools", T, fail=FAIL)
    time.sleep(10)
    vm.send("/poweroff")
    vm.stop()
    print(f"{stamp()} [stick] powered off (decoy run)", flush=True)
