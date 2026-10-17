#!/usr/bin/env python3
"""Image side of tools/bios-e2e.sh. Runs in the bia-build:x86_64 container.

  prep    flash.img: FMM keys, current agent + MCPs + overlay scripts from dist/ and /work
  check   internal.img after the install: GPT, BIOS GRUB in the MBR and BIOS boot
          partition, ESP grub.cfg for BIADISK, fstab, the agent's files
"""
import importlib.util, os, shutil, struct, sys, uuid

spec = importlib.util.spec_from_file_location("ut", "/work/tools/update-e2e-img.py")
ut = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ut)
ut.IMAGES.update({"flash": "/ut/flash.img", "internal": "/ut/internal.img"})
BIOS_TYPE = uuid.UUID("21686148-6449-6E6F-744E-656564454649")


def prep():
    with ut.Img("flash") as f:
        f.mount()
        r = f.root
        # FMM keys come from bia-flash (EFI/bia/fmm.json, seeded on first boot).
        out = "/work/dist/x86_64"
        if os.path.exists(f"{out}/bia-agent"):
            shutil.copy2(f"{out}/bia-agent", f"{r}/usr/local/sbin/bia-agent")
        for mcp in ("carry", "system"):
            dst = f"{r}/home/bia/MCPs/{mcp}/bin/x86_64/bia-mcp-{mcp}"
            if os.path.exists(f"{out}/bia-mcp-{mcp}") and os.path.isdir(os.path.dirname(dst)):
                shutil.copy2(f"{out}/bia-mcp-{mcp}", dst)
        ut.sh("cp", "-a", "/work/source/os/overlay/usr/local/sbin/.", f"{r}/usr/local/sbin/")
        ut.sh("cp", "-a", "/work/source/os/overlay/usr/local/share/bia/.", f"{r}/usr/local/share/bia/")
        print("flash: agent + carry/system MCP + overlay scripts from the work tree")


def gpt_types(path):
    with open(path, "rb") as f:
        f.seek(512)
        h = f.read(92)
        part_lba, num, esz = struct.unpack_from("<Q", h, 72)[0], *struct.unpack_from("<II", h, 80)
        f.seek(part_lba * 512)
        out = []
        for i in range(num):
            e = f.read(esz)
            if e[0:16] == b"\0" * 16:
                continue
            first, last = struct.unpack_from("<QQ", e, 32)
            out.append((i + 1, uuid.UUID(bytes_le=e[0:16]), first * 512, (last - first + 1) * 512))
        return out


def check():
    errs = []

    def want(cond, msg):
        print(("  ok   " if cond else "  FAIL ") + msg)
        if not cond:
            errs.append(msg)

    def read(p):
        try:
            return open(p, errors="replace").read()
        except OSError:
            return ""

    path = ut.IMAGES["internal"]
    with open(path, "rb") as f:
        mbr = f.read(512)
    want(mbr[:8] != b"\0" * 8 and b"GRUB" in mbr[:440], "MBR boot code is GRUB boot.img")
    parts = gpt_types(path)
    bios = [p for p in parts if p[1] == BIOS_TYPE]
    want(len(bios) == 1, f"one BIOS boot partition ({len(parts)} partitions)")
    if bios:
        with open(path, "rb") as f:
            f.seek(bios[0][2])
            core = f.read(512)
        want(core != b"\0" * 512, "BIOS boot partition holds core.img")
    with ut.Img("internal", ro=True) as d:
        d.mount()
        want(ut.blk(d.root_dev, "LABEL") == "BIADISK", f"root label {ut.blk(d.root_dev, 'LABEL')!r}")
        want(ut.blk(d.esp_dev, "LABEL") == "BIADBOOT", f"ESP label {ut.blk(d.esp_dev, 'LABEL')!r}")
        cfg = read(f"{d.esp}/boot/grub/grub.cfg")
        want("root=LABEL=BIADISK" in cfg, "ESP boot/grub/grub.cfg boots root=LABEL=BIADISK (read by BIOS GRUB)")
        want(os.path.isdir(f"{d.esp}/boot/grub/i386-pc"), "ESP has boot/grub/i386-pc modules")
        want(os.path.exists(f"{d.esp}/vmlinuz-lts") and os.path.exists(f"{d.esp}/initramfs-lts"), "ESP has kernel + initramfs")
        want("LABEL=BIADISK" in read(f"{d.root}/etc/fstab"), "fstab mounts LABEL=BIADISK")
        want(os.path.isfile(f"{d.root}/home/bia/bia.toml"), "/home/bia/bia.toml on the disk")
        want(os.path.isfile(f"{d.root}/home/bia/files/state/fmm.json"), "the agent's FMM config came along")
    if errs:
        sys.exit(f"CHECK bios: {len(errs)} failure(s)")
    print("CHECK bios: all good")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "prep":
        prep()
    elif cmd == "check":
        check()
    else:
        sys.exit(__doc__)
