#!/usr/bin/env python3
"""Boot the status test's stick (tests/e2e/status-qemu.sh run) and wait for the fake bus's
STATUS-E2E-DONE, then /poweroff. Env: LE (dir with stick.img), ARCH, MEM, CPUS, TIMEOUT
(minutes for the whole scenario). Serial output: $LE/stick.log.
"""
import os, re, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import e2e_vm
from e2e_vm import LE, VM, die, stamp

T = int(os.environ.get("TIMEOUT", "25")) * 60
e2e_vm.SHOW = re.compile(r"(?i)Ready for commands|bia-grow: growing|<- \[|-> .*\[(Anna|Ben|Carl)|unanswered:|"
                         r"sent fallback|STATUS-E2E-DONE|Kernel panic|accepted job")
FAIL = r"Kernel panic|bia-grow: .*(could not|failed)"

stick = ["-drive", f"if=none,id=d0,format=raw,file={LE}/stick.img",
         "-device", "virtio-blk-pci,drive=d0,serial=stick,bootindex=0"]
net = ["-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0"]
vm = VM("stick", net + stick)
vm.expect("BIAROOT grown over the disk", r"bia-grow: growing root to the whole partition", 15 * 60, fail=FAIL)
vm.expect("the scenario ran to its end", r"STATUS-E2E-DONE", T, fail=FAIL)
time.sleep(20)  # the model's last turn
vm.send("/poweroff")
vm.stop()
print(f"{stamp()} [stick] powered off", flush=True)
