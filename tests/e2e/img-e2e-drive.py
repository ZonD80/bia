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

disk = ["-drive", f"if=none,id=d0,format=raw,file={LE}/stick.img",
        "-device", "virtio-blk-pci,drive=d0,serial=stick,bootindex=0"]
net = ["-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0"]
vm = VM("stick", net + disk, firmware=FIRMWARE)

vm.expect("kernel booted from BIAROOT", r"(?i)command line:[^\n]*root=LABEL=BIAROOT", T)
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
