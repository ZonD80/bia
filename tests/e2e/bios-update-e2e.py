#!/usr/bin/env python3
"""Offline update through the rescue image on legacy BIOS (SeaBIOS, x86_64).

  ARCH=x86_64 tools/update-e2e.sh images && ARCH=x86_64 tools/update-e2e.sh patch
  tools/bios-update-e2e.py [disk|flash|rescue …]        (default: all three)

Uses the update-e2e images in dist/ut-x86_64 (prep/check from tools/update-e2e-img.py).
Scenarios, as they happen on a BIOS machine:
  disk    runs from the disk with no stick; the stick is hot-plugged ahead of the disk in
          the boot order (a user plugging in a new stick), then /update. SeaBIOS boots
          the stick next, so the rescue runs from the stick's ESP, copies onto the disk
          and powers off; the stick is removed and the disk boots and unfolds.
  flash   runs from the stick (first in the boot order) with the disk attached; /update
          copies the disk's newer OS onto the stick, which boots again and unfolds.
  rescue  runs from the stick; /rescue pushes it onto the disk, powers off, the stick is
          removed and the disk boots and unfolds.
Env: MEM (6G), CPUS (4), TIMEOUT (minutes per long phase, 30)
"""
import os, re, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
UT = f"{ROOT}/dist/ut-x86_64"
os.environ.setdefault("LE", UT)
os.environ.setdefault("MEM", "6G")
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import e2e_vm
from e2e_vm import VM, die, stamp

T = int(os.environ.get("TIMEOUT", "30")) * 60
e2e_vm.SHOW = re.compile(r"bia-update:|bia-rescue:|unfold:|Kernel panic|legacy BIOS|DMI: .*BIOS|Command line:")
FLASH, DISK = f"{UT}/flash.img", f"{UT}/disk.img"
NET = ["-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0"]
FAIL_UPD = r"bia-update: [^\n]*\[ FAIL \]"
FAIL_RESCUE = r"bia-rescue: [^\n]*\[ FAIL \]|Kernel panic"


def drive(img, name, serial, index):
    return ["-drive", f"if=none,id={name},format=raw,file={img}",
            "-device", f"virtio-blk-pci,drive={name},serial={serial},bootindex={index},id={name}dev"]


def boot(tag, stick=True, disk=True, stick_first=True):
    extra = list(NET)
    if stick:
        extra += drive(FLASH, "d0", "flash", 0 if stick_first else 1)
    if disk:
        extra += drive(DISK, "d1", "internal", 1 if stick_first else 0)
    if not stick:
        extra += ["-device", "pcie-root-port,id=rp1,chassis=1,slot=1"]  # room to plug the stick in
    vm = VM(tag, extra, firmware="bios")
    m = vm.expect("legacy BIOS boot", r"DMI: [^\n]*BIOS ([^\n]*)", T)
    if "edk2" in m.group(1).lower():
        die("not a legacy BIOS boot", vm)
    return vm


def image_tool(*args):
    r = subprocess.run(["docker", "run", "--rm", "-i", "--privileged", "--platform", "linux/amd64", "-e", "ARCH=x86_64",
                        "-v", f"{UT}:/ut:rw", "-v", f"{ROOT}:/work:ro", "-v", f"{ROOT}/tools/update-e2e-img.py:/img.py:ro",
                        "bia-build:x86_64", "python3", "/img.py", *args], capture_output=True, text=True)
    print(r.stdout + r.stderr, flush=True)
    if r.returncode:
        sys.exit(f"update-e2e-img {' '.join(args)} failed")


def follow_rescue(vm):
    """From the reboot into the rescue image to its hand-over. Returns True on poweroff."""
    vm.expect("staging (fold + marker)", r"rebooting into the rescue image", 1800, fail=FAIL_UPD)
    vm.expect("rescue image boots", r"bia-rescue: bia rescue image", 900, fail=FAIL_RESCUE)
    root = re.findall(r"Command line:[^\n]*", vm.text())
    vm.expect("rescue copies + verifies", r"bia-rescue: update copied", T, fail=FAIL_RESCUE)
    end = time.time() + 300
    while time.time() < end:
        t = vm.text()[vm.cursor:]
        if re.search(r"legacy BIOS: the firmware would start", t):
            vm.proc.wait(120)
            print(f"{stamp()} [{vm.name}] rescue powered off for the BIOS hand-over", flush=True)
            return True
        if re.search(r"bia-rescue: rebooting", t):
            return False
        if vm.proc.poll() is not None:
            die("QEMU exited without the BIOS notice", vm)
        time.sleep(1)
    die("no hand-over after the copy", vm)


def finish(vm):
    vm.expect("destination boot", r"bia-update: boot", 900, fail=FAIL_RESCUE)
    vm.expect("unfold", r"restoring this machine \.+ \[ OK \]|unfold: rebooting", 1800,
              fail=r"restoring this machine \.+ \[ FAIL \]|" + FAIL_UPD)
    vm.expect("final boot wipes /bia-update", r"bia-update: removed /bia-update", 900, fail=FAIL_UPD)
    vm.expect("final boot idle", r"bia-update: idle", 900, fail=FAIL_UPD)
    if re.search(r"bia-rescue: bia rescue image", vm.text()[vm.cursor:]):
        die("rescue image ran again", vm)
    vm.expect("agent console after update", r"Ready for commands", 600)
    time.sleep(10)
    vm.send("/poweroff")
    vm.stop()


def trigger(vm, cmd):
    for _ in range(6):
        vm.send(cmd)
        end = time.time() + 60
        while time.time() < end:
            if re.search(r"bia-update: (this system is|rescue —)", vm.text()[vm.cursor:]):
                return
            time.sleep(0.5)
    die(f"{cmd}: bia-update never started", vm)


def run(scen):
    print(f"======== BIOS scenario {scen}", flush=True)
    image_tool("prep", scen)
    e2e_vm.VMS.clear()
    if scen == "disk":
        vm = boot("disk1", stick=False, disk=True)
        vm.expect("agent on the disk", r"Ready for commands", T)
        time.sleep(10)
        # The user plugs the new stick in; the BIOS boot order puts USB first.
        vm.mon(f"drive_add 0 if=none,id=stick,format=raw,file={FLASH}")
        out = vm.mon("device_add virtio-blk-pci,drive=stick,serial=flash,bootindex=0,id=stickdev,bus=rp1")
        print(f"{stamp()} [disk1] stick hot-plugged ahead of the disk {out.strip()[-80:]!r}", flush=True)
        time.sleep(10)
        trigger(vm, "/update")
        vm.expect("other ESP marked for BIOS", r"legacy BIOS: the firmware picks the next drive", 600, fail=FAIL_UPD)
        if follow_rescue(vm):
            vm = boot("disk2", stick=False, disk=True)
        finish(vm)
    elif scen == "flash":
        vm = boot("flash1")
        vm.expect("agent on the stick", r"Ready for commands", T)
        time.sleep(10)
        trigger(vm, "/update")
        vm.expect("other ESP marked for BIOS", r"legacy BIOS: the firmware picks the next drive", 600, fail=FAIL_UPD)
        if follow_rescue(vm):
            die("the stick was the destination and booted the rescue; it should reboot, not power off", vm)
        finish(vm)
    elif scen == "rescue":
        vm = boot("rescue1")
        vm.expect("agent on the stick", r"Ready for commands", T)
        time.sleep(10)
        trigger(vm, "/rescue")
        vm.expect("other ESP marked for BIOS", r"legacy BIOS: the firmware picks the next drive", 600, fail=FAIL_UPD)
        if not follow_rescue(vm):
            die("the rescue rebooted into the stick instead of powering off for the disk", vm)
        vm = boot("rescue2", stick=False, disk=True)
        finish(vm)
    image_tool("check", scen)


scens = sys.argv[1:] or ["disk", "flash", "rescue"]
for s in scens:
    run(s)
print(f"======== BIOS update e2e passed: {' '.join(scens)}", flush=True)
