#!/usr/bin/env python3
"""Drive the legacy BIOS boot + install test (tools/bios-e2e.sh run).

Env: LE (dir with flash.img, internal.img), MEM, CPUS, TIMEOUT (minutes per long phase).
Serial output: $LE/stick.log (first boot) and $LE/disk.log (after the stick is removed).
"""
import os, re, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import e2e_vm
from e2e_vm import LE, VM, die, stamp

T = int(os.environ.get("TIMEOUT", "60")) * 60
e2e_vm.SHOW = re.compile(r"DMI:|Command line:|bia-migrate|legacy BIOS|carry_to_disk|Ready for commands|"
                         r"rebooting|Powering off|reboot: Power down|Kernel panic|bia-update: (idle|boot)")
READY = r"Ready for commands"
FAIL = r"Kernel panic|bia-migrate: .*(failed|error|Error)"

for f in (f"{LE}/flash.img", f"{LE}/internal.img"):
    if not os.path.exists(f):
        sys.exit(f"missing {f}; run tools/bios-e2e.sh images")


def disks(with_stick):
    d = []
    if with_stick:
        d += ["-drive", f"if=none,id=d0,format=raw,file={LE}/flash.img",
              "-device", "virtio-blk-pci,drive=d0,serial=flash,bootindex=0"]
    d += ["-drive", f"if=none,id=d1,format=raw,file={LE}/internal.img",
          "-device", "virtio-blk-pci,drive=d1,serial=internal,bootindex=1"]
    return d


NET = ["-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0"]


def legacy_boot(vm, root_label):
    """SeaBIOS -> GRUB i386-pc -> kernel with root=LABEL=<root_label>, no UEFI anywhere."""
    vm.expect(f"kernel booted root=LABEL={root_label}", rf"Command line:[^\n]*root=LABEL={root_label}", T)
    m = vm.expect("firmware is SeaBIOS (no UEFI)", r"DMI: [^\n]*BIOS ([^\n]*)", 300)
    bios = m.group(1).strip()
    if "edk2" in bios.lower() or "ovmf" in bios.lower() or re.search(r"efi: EFI v", vm.text()):
        die(f"not a legacy BIOS boot: {bios}", vm)
    print(f"{stamp()} [{vm.name}] legacy BIOS: {bios}", flush=True)


# ---- 1. stick boots under SeaBIOS ----
stick = VM("stick", NET + disks(True), firmware="bios")
legacy_boot(stick, "BIAROOT")
stick.expect("agent up from the stick", READY, T, fail=FAIL)
time.sleep(20)

# ---- 2. install onto the internal disk ----
stick.send("Install this system onto the internal disk now: use bash lsblk to find the empty internal disk "
           "(serial internal, not the disk you booted from), then call carry_to_disk with that whole disk. "
           "Do not ask me first.")
stick.expect("installer starts", r"bia-migrate|carry_to_disk", 900, fail=FAIL)
# A reboot keeps QEMU running (SeaBIOS would boot the stick again); a poweroff ends it.
print(f"{stamp()} [stick] -- waiting: installer copies, then powers off (BIOS cannot be told to boot the disk)", flush=True)
end = time.time() + T
while stick.proc.poll() is None:
    t = stick.text()[stick.cursor:]
    if re.search(r"rebooting into the disk|Linux version", t.split("carry_to_disk", 1)[-1]):
        die("the installer rebooted instead of powering off: SeaBIOS boots the stick again", stick)
    f = re.search(FAIL, t)
    if f:
        die(f"installer failed: {f.group(0)!r}", stick)
    if time.time() > end:
        die(f"no poweroff within {T}s", stick)
    time.sleep(2)
said = bool(re.search(r"legacy BIOS boot: bia cannot change the boot order", stick.text()))
print(f"{stamp()} [stick] QEMU exited (poweroff); BIOS notice on console: {'yes' if said else 'not seen'}", flush=True)
print(f"{stamp()} [stick] powered off; removing the stick", flush=True)

# ---- 3. stick removed: SeaBIOS boots the internal disk ----
disk = VM("disk", NET + disks(False), firmware="bios")
legacy_boot(disk, "BIADISK")
disk.expect("agent up from the disk", READY, T, fail=r"Kernel panic")
time.sleep(15)
disk.send("/poweroff")
disk.stop()
print(f"{stamp()} BIOS e2e: run finished", flush=True)
