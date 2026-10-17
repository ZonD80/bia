#!/usr/bin/env python3
"""Image side of tools/img-e2e.sh. Runs in the bia-build:<arch> container.

  patch          /ie/root.img: overlay scripts, agent and carry/system MCPs from the work tree
  keys           /ie/stick.img (MODE=dd): ./fmm.json onto the ESP as EFI/bia/fmm.json, the way
                 a user who wrote the image with Etcher would add their keys
  check [MODEL]  /ie/stick.img after the boot: GPT over the whole disk, ext4 filling BIAROOT,
                 FMM keys seeded, the model moved in (MODE=flash), BIOS GRUB (x86_64)
"""
import os, shutil, struct, subprocess, sys, uuid

ARCH = os.environ.get("ARCH", "arm64")
MODE = os.environ.get("MODE", "flash")
STICK = "/ie/stick.img"
FILEARCH = {"arm64": "aarch64", "x86_64": "x86_64"}[ARCH]
BIOS_TYPE = uuid.UUID("21686148-6449-6E6F-744E-656564454649")


def sh(*cmd, capture=False, check=True):
    r = subprocess.run(cmd, capture_output=capture, text=True)
    if check and r.returncode:
        sys.exit(f"{' '.join(cmd)} failed: {(r.stderr or '').strip()}")
    return (r.stdout or "").strip()


def gpt(path):
    """(header fields, [(number, type, start bytes, size bytes, name)])."""
    with open(path, "rb") as f:
        f.seek(512)
        h = f.read(92)
        if h[:8] != b"EFI PART":
            sys.exit(f"{path}: no GPT")
        alt_lba, first_use, last_use = struct.unpack_from("<QQQ", h, 32)
        part_lba, num, esz = struct.unpack_from("<Q", h, 72)[0], *struct.unpack_from("<II", h, 80)
        f.seek(part_lba * 512)
        parts = []
        for i in range(num):
            e = f.read(esz)
            if e[:16] == b"\0" * 16:
                continue
            first, last = struct.unpack_from("<QQ", e, 32)
            name = e[56:128].decode("utf-16-le").rstrip("\0")
            parts.append((i + 1, uuid.UUID(bytes_le=e[:16]), first * 512, (last - first + 1) * 512, name))
    return {"alt_lba": alt_lba, "last_usable": last_use}, parts


class Part:
    """One partition of the stick, looped and mounted."""

    def __init__(self, start, size, fstype, ro=False):
        self.dev = sh("losetup", "-f", "--show", "-o", str(start), "--sizelimit", str(size), STICK, capture=True)
        self.mnt = f"/mnt/ie-{fstype}"
        os.makedirs(self.mnt, exist_ok=True)
        sh("mount", "-t", fstype, "-o", "ro" if ro else "rw", self.dev, self.mnt)

    def __enter__(self):
        return self

    def __exit__(self, *a):
        subprocess.run(["umount", self.mnt])
        subprocess.run(["losetup", "-d", self.dev])


def patch():
    dev = sh("losetup", "-f", "--show", "/ie/root.img", capture=True)
    try:
        os.makedirs("/mnt/ie-root", exist_ok=True)
        sh("mount", "-t", "ext4", dev, "/mnt/ie-root")
        r = "/mnt/ie-root"
        out = f"/work/dist/{ARCH}"
        if os.path.exists(f"{out}/bia-agent"):
            shutil.copy2(f"{out}/bia-agent", f"{r}/usr/local/sbin/bia-agent")
        for mcp in ("carry", "system"):
            dst = f"{r}/home/bia/MCPs/{mcp}/bin/{FILEARCH}/bia-mcp-{mcp}"
            if os.path.exists(f"{out}/bia-mcp-{mcp}") and os.path.isdir(os.path.dirname(dst)):
                shutil.copy2(f"{out}/bia-mcp-{mcp}", dst)
        sh("cp", "-a", "/work/source/os/overlay/usr/local/sbin/.", f"{r}/usr/local/sbin/")
        sh("cp", "-a", "/work/source/os/overlay/usr/local/share/bia/.", f"{r}/usr/local/share/bia/")
        state = f"{r}/home/bia/files/state"
        leaked = [n for n in os.listdir(state) if n == "fmm.json"] if os.path.isdir(state) else []
        if leaked:
            sys.exit("root.img holds files/state/fmm.json; keys must never be in the image")
        print("root.img: agent + carry/system MCP + overlay scripts from the work tree")
    finally:
        subprocess.run(["umount", "/mnt/ie-root"])
        subprocess.run(["losetup", "-d", dev])


def keys():
    _, parts = gpt(STICK)
    esp = parts[0]
    with Part(esp[2], esp[3], "vfat") as p:
        if os.path.exists(f"{p.mnt}/EFI/bia/fmm.json"):
            sys.exit("the released image already has EFI/bia/fmm.json")
        os.makedirs(f"{p.mnt}/EFI/bia", exist_ok=True)
        shutil.copy("/work/fmm.json", f"{p.mnt}/EFI/bia/fmm.json")
    print("stick: ./fmm.json copied to the ESP as EFI/bia/fmm.json")


def check(model):
    errs = []

    def want(cond, msg):
        print(("  ok   " if cond else "  FAIL ") + msg)
        if not cond:
            errs.append(msg)

    disk = os.path.getsize(STICK)
    hdr, parts = gpt(STICK)
    want([p[4] for p in parts] == ["BIABOOT", "BIABIOS", "BIAROOT"], f"partitions {[p[4] for p in parts]}")
    root = parts[-1]
    want(hdr["alt_lba"] == disk // 512 - 1, f"backup GPT at the end of the disk (LBA {hdr['alt_lba']})")
    gap = disk - (root[2] + root[3])
    want(gap <= 16 << 20, f"BIAROOT reaches the end of the disk ({gap >> 10} KiB after it)")
    with open(STICK, "rb") as f:
        f.seek(disk - 512)
        tail = f.read(8)
    want(tail == b"EFI PART", "backup GPT header in the last sector")
    if ARCH == "x86_64":
        with open(STICK, "rb") as f:
            mbr = f.read(512)
            f.seek(parts[1][2] + 512)
            core = f.read(512)
        want(b"GRUB" in mbr[:440], "MBR boot code is GRUB boot.img")
        want(core != b"\0" * 512, "BIOS boot partition holds core.img")
        want(parts[1][1] == BIOS_TYPE, "partition 2 is a BIOS boot partition")
    with Part(root[2], root[3], "ext4", ro=True) as p:
        out = sh("dumpe2fs", "-h", p.dev, capture=True, check=False)
        f = dict(l.split(":", 1) for l in out.splitlines() if ":" in l)
        fs = int(f["Block count"]) * int(f["Block size"])
        want(root[3] - fs < 8 << 20, f"ext4 fills BIAROOT ({fs >> 20} of {root[3] >> 20} MiB)")
        want(os.path.isfile(f"{p.mnt}/home/bia/files/state/fmm.json"), "FMM keys seeded into /home/bia/files/state")
        if MODE == "flash":
            name = model.split(":", 1)[1] if ":" in model else ""
            got = os.listdir(f"{p.mnt}/home/bia/model") if os.path.isdir(f"{p.mnt}/home/bia/model") else []
            want(name in got if name else any(n.endswith(".gguf") for n in got), f"model in /home/bia/model ({got})")
            want(not any(n.endswith(".part") for n in got), "no half-moved model")
    with Part(parts[0][2], parts[0][3], "vfat", ro=True) as p:
        want(not os.path.exists(f"{p.mnt}/EFI/bia/models.txt"), "models.txt removed once the model is in")
    if errs:
        sys.exit(f"CHECK image ({MODE}): {len(errs)} failure(s)")
    print(f"CHECK image ({MODE}): all good")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "patch":
        patch()
    elif cmd == "keys":
        keys()
    elif cmd == "check":
        check(sys.argv[2] if len(sys.argv) > 2 else "")
    else:
        sys.exit(__doc__)
