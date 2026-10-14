#!/usr/bin/env python3
"""Image side of tools/update-e2e.sh. Runs in the bia-build container (root, loop devices).

  make-disk          turn /ut/disk.img (a copy of the flash layout) into an internal
                     disk: LABEL=BIADISK / BIADBOOT, new ext4 UUID, fstab, grub.cfg
  prep <scenario>    releases (source = dst + 1), markers, clean /bia-update + rescue markers
  check <scenario>   assert the destination was updated and kept its agent
  patch              copy /work overlay scripts + /work/dist/<arch> binaries into both
                     images and rebuild the rescue overlay inside each root (fast iteration)
"""
import os, re, shutil, struct, subprocess, sys

UT = "/ut"
IMAGES = {"flash": f"{UT}/flash.img", "disk": f"{UT}/disk.img"}
LABEL = {"flash": "BIAROOT", "disk": "BIADISK"}
ESPLABEL = {"flash": "BIABOOT", "disk": "BIADBOOT"}
# scenario -> (destination, source)
SCEN = {"disk": ("disk", "flash"), "flash": ("flash", "disk"), "rescue": ("disk", "flash")}


def sh(*cmd, check=True, capture=False):
    r = subprocess.run(cmd, check=False, capture_output=capture, text=True)
    if check and r.returncode != 0:
        sys.exit(f"command failed ({r.returncode}): {' '.join(cmd)}\n{r.stdout or ''}{r.stderr or ''}")
    return (r.stdout or "").strip() if capture else r.returncode


def partitions(path):
    with open(path, "rb") as f:
        f.seek(512)
        h = f.read(92)
        assert h[0:8] == b"EFI PART", f"{path}: no GPT"
        part_lba, num, esz = struct.unpack_from("<Q", h, 72)[0], *struct.unpack_from("<II", h, 80)
        f.seek(part_lba * 512)
        out = []
        for i in range(num):
            e = f.read(esz)
            if e[0:16] == b"\0" * 16:
                continue
            first, last = struct.unpack_from("<QQ", e, 32)
            out.append((i + 1, first * 512, (last - first + 1) * 512))
        return out


class Img:
    """ESP (p1) and root (largest partition) of one image, looped and mounted."""

    def __init__(self, name, ro=False):
        self.name, self.ro = name, ro
        parts = partitions(IMAGES[name])
        self.parts = {}
        for n, off, size in parts:
            self.parts[n] = sh("losetup", "-f", "--show", "-o", str(off), "--sizelimit", str(size),
                               IMAGES[name], capture=True)
        self.esp_dev = self.parts[1]
        self.root_dev = self.parts[max(parts, key=lambda p: p[2])[0]]
        self.esp = f"/mnt/{name}-esp"
        self.root = f"/mnt/{name}-root"

    def mount(self):
        for d, dev, extra in ((self.root, self.root_dev, ["-t", "ext4"]), (self.esp, self.esp_dev, ["-t", "vfat"])):
            os.makedirs(d, exist_ok=True)
            sh("mount", *extra, "-o", "ro" if self.ro else "rw", dev, d)
        return self

    def close(self):
        for d in (self.esp, self.root):
            subprocess.run(["umount", d], capture_output=True)
        for dev in self.parts.values():
            subprocess.run(["losetup", "-d", dev], capture_output=True)

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.close()


def blk(dev, key):
    """Probe the device itself; the blkid cache goes stale when loop devices are reused."""
    return sh("blkid", "-p", "-s", key, "-o", "value", dev, check=False, capture=True)


def rel(root):
    p = f"{root}/etc/bia-release"
    return "".join(c for c in open(p).read() if c.isdigit()) if os.path.exists(p) else ""


def put(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def grub_cfgs(esp):
    for c in ("boot/grub/grub.cfg", "EFI/BOOT/grub.cfg", "EFI/debian/grub.cfg", "EFI/bia/grub.cfg"):
        if os.path.exists(f"{esp}/{c}"):
            yield f"{esp}/{c}"


def seed_fmm(im):
    """Test images only: run the agent on the developer's FMM model (./fmm.json,
    gitignored) instead of the slow local GGUF, and drop the GGUF. Shipped images
    never carry the keys."""
    src = "/work/fmm.json"
    if not os.path.isfile(src):
        print("  no ./fmm.json: the test agent loads the local model (needs MEM=8G)")
        return
    state = f"{im.root}/home/bia/files/state"
    os.makedirs(state, exist_ok=True)
    dst = f"{state}/fmm.json"
    shutil.copyfile(src, dst)
    os.chmod(dst, 0o600)
    os.chown(dst, 1000, 1000)
    print(f"  FMM keys from ./fmm.json -> {dst[len(im.root):]}")
    # FMM test images need no GGUF; leaving it out keeps fold and rescue copies quick.
    model = f"{im.root}/home/bia/model"
    for n in os.listdir(model) if os.path.isdir(model) else []:
        if n.endswith(".gguf"):
            os.remove(f"{model}/{n}")
            print(f"  removed test-image model {n}")


def make_disk():
    with Img("disk") as d:
        sh("fatlabel", d.esp_dev, "BIADBOOT")
        sh("e2fsck", "-fy", d.root_dev, check=False)
        sh("tune2fs", "-U", "random", d.root_dev)
        sh("e2label", d.root_dev, "BIADISK")
        d.mount()
        put(f"{d.root}/etc/fstab",
            "LABEL=BIADISK  /      ext4  noatime 0 1\nLABEL=BIADBOOT  /boot  vfat  noatime,nofail 0 2\n"
            "/swapfile      none  swap  sw,nofail 0 0\n")
        for cfg in grub_cfgs(d.esp):
            s = open(cfg).read()
            s = s.replace("root=LABEL=BIAROOT", "root=LABEL=BIADISK").replace('"bia flash"', '"bia disk"')
            s = s.replace("--label BIABOOT", "--label BIADBOOT")
            put(cfg, s)
        seed_fmm(d)
        print(f"disk: BIADISK uuid={blk(d.root_dev, 'UUID')} "
              f"esp={blk(d.esp_dev, 'UUID')}")
    with Img("flash") as f:
        f.mount()
        seed_fmm(f)
        print(f"flash: BIAROOT uuid={blk(f.root_dev, 'UUID')} "
              f"esp={blk(f.esp_dev, 'UUID')}")


def prep(scen):
    dst, src = SCEN[scen]
    with Img("flash") as fl, Img("disk") as dk:
        fl.mount(), dk.mount()
        imgs = {"flash": fl, "disk": dk}
        base = max(int(rel(fl.root) or 0), int(rel(dk.root) or 0))
        for name, im in imgs.items():
            shutil.rmtree(f"{im.root}/bia-update", ignore_errors=True)
            for p in ("etc/ut-new-os", "usr/local/share/ut-old-only", "home/bia/files/ut-agent-marker"):
                if os.path.exists(f"{im.root}/{p}"):
                    os.remove(f"{im.root}/{p}")
            for d in ("bia-rescue", "EFI/bia/bia-rescue"):
                if os.path.exists(f"{im.esp}/{d}/next.cfg"):
                    os.remove(f"{im.esp}/{d}/next.cfg")
            r = base + 1 if name == src else base
            put(f"{im.root}/etc/bia-release", f"{r}\n")
        # Make the source ESP's initramfs differ (trailing zeros are ignored by
        # the kernel) so the check can see the destination ESP was refreshed.
        si, di = f"{imgs[src].esp}/initramfs-lts", f"{imgs[dst].esp}/initramfs-lts"
        if subprocess.run(["cmp", "-s", si, di]).returncode == 0:
            with open(si, "ab") as f:
                f.write(b"\0" * 4096)
        put(f"{imgs[src].root}/etc/ut-new-os", f"from {src}\n")
        put(f"{imgs[dst].root}/usr/local/share/ut-old-only", "must be deleted by the update\n")
        put(f"{imgs[dst].root}/home/bia/files/ut-agent-marker", f"agent of {dst}\n")
        os.chown(f"{imgs[dst].root}/home/bia/files/ut-agent-marker", 1000, 1000)
        for name, im in imgs.items():
            print(f"{name}: {LABEL[name]} release {rel(im.root)}" + ("  <- source" if name == src else "  <- destination"))


def check(scen):
    dst, src = SCEN[scen]
    errs = []

    def want(cond, msg):
        print(("  ok   " if cond else "  FAIL ") + msg)
        if not cond:
            errs.append(msg)

    with Img(dst, ro=True) as d, Img(src, ro=True) as s:
        d.mount(), s.mount()
        srel = rel(s.root)
        want(rel(d.root) == srel and srel != "", f"{dst} release {rel(d.root)} == {src} release {srel}")
        want(os.path.exists(f"{d.root}/etc/ut-new-os") and open(f"{d.root}/etc/ut-new-os").read() == f"from {src}\n",
             "new OS file from the source is on the destination")
        want(not os.path.exists(f"{d.root}/usr/local/share/ut-old-only"), "file only in the old OS was deleted")
        mk = f"{d.root}/home/bia/files/ut-agent-marker"
        want(os.path.exists(mk) and open(mk).read() == f"agent of {dst}\n", "destination agent's files survived (fold/unfold)")
        want(not os.path.exists(f"{d.root}/bia-update"), "/bia-update cleaned up after unfold")
        fstab = open(f"{d.root}/etc/fstab").read()
        want(f"LABEL={LABEL[dst]} " in fstab or f"LABEL={LABEL[dst]}\t" in fstab, f"fstab still mounts LABEL={LABEL[dst]}")
        want(blk(d.root_dev, "LABEL") == LABEL[dst], f"root label still {LABEL[dst]}")
        for rd in ("bia-rescue", "EFI/bia/bia-rescue"):
            want(not os.path.exists(f"{d.esp}/{rd}/next.cfg"), f"no rescue marker {rd}/next.cfg on {dst} ESP")
            want(not os.path.exists(f"{s.esp}/{rd}/next.cfg"), f"no rescue marker {rd}/next.cfg on {src} ESP")
        for k in ("vmlinuz-lts", "initramfs-lts", "bia-rescue/rescue.cpio.gz"):
            same = os.path.exists(f"{d.esp}/{k}") and subprocess.run(["cmp", "-s", f"{d.esp}/{k}", f"{s.esp}/{k}"]).returncode == 0
            want(same, f"{dst} ESP {k} == {src} ESP {k}")
        for cfg in grub_cfgs(d.esp):
            c = open(cfg).read()
            want("# bia-rescue hook" in c and f"root=LABEL={LABEL[dst]}" in c, f"{cfg[len(d.esp):]}: rescue hook + root=LABEL={LABEL[dst]}")
        want(open(f"{s.root}/etc/ut-new-os").read() == f"from {src}\n" and rel(s.root) == srel, "source left untouched")
        farch = "aarch64" if os.environ.get("ARCH", "arm64") == "arm64" else "x86_64"
        diff = []
        for mcp in sorted(os.listdir(f"{s.root}/home/bia/MCPs")):
            bdir = f"home/bia/MCPs/{mcp}/bin/{farch}"
            if not os.path.isdir(f"{s.root}/{bdir}"):
                continue
            for b in os.listdir(f"{s.root}/{bdir}"):
                if subprocess.run(["cmp", "-s", f"{s.root}/{bdir}/{b}", f"{d.root}/{bdir}/{b}"]).returncode:
                    diff.append(f"{mcp}/{b}")
        want(not diff, f"MCP binaries are the new image's {diff or ''}")
        ok_unfold = os.path.exists(f"{d.root}/home/bia/bia.toml")
        want(ok_unfold, "destination /home/bia/bia.toml present")
    if errs:
        sys.exit(f"CHECK {scen}: {len(errs)} failure(s)")
    print(f"CHECK {scen}: all good")


def patch():
    arch = os.environ.get("ARCH", "arm64")
    out = f"/work/dist/{arch}"
    farch = "aarch64" if arch == "arm64" else "x86_64"
    for name in ("flash", "disk"):
        with Img(name) as im:
            im.mount()
            sh("cp", "-a", "/work/source/os/overlay/usr/local/sbin/.", f"{im.root}/usr/local/sbin/")
            sh("cp", "-a", "/work/source/os/overlay/usr/local/share/bia/.", f"{im.root}/usr/local/share/bia/")
            sh("cp", "-a", "/work/source/os/overlay/etc/systemd/system/.", f"{im.root}/etc/systemd/system/")
            for tty in ("ttyS0", "ttyAMA0"):  # same as builders/rootfs.sh
                link = f"{im.root}/etc/systemd/system/serial-getty@{tty}.service"
                if not os.path.islink(link):
                    os.symlink("/dev/null", link)
            if os.path.exists(f"{out}/bia-agent"):
                shutil.copy2(f"{out}/bia-agent", f"{im.root}/usr/local/sbin/bia-agent")
            carry = f"{im.root}/home/bia/MCPs/carry/bin/{farch}/bia-mcp-carry"
            if os.path.exists(f"{out}/bia-mcp-carry") and os.path.exists(os.path.dirname(carry)):
                shutil.copy2(f"{out}/bia-mcp-carry", carry)
            sh("/work/builders/rescue.sh", im.root, f"{im.root}/usr/local/share/bia/rescue.cpio.gz")
            sh(f"{im.root}/usr/local/sbin/bia-rescue-hook", im.esp, f"{im.root}/usr/local/share/bia/rescue.cpio.gz")
            print(f"patched {name}")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "make-disk":
        make_disk()
    elif cmd == "prep":
        prep(sys.argv[2])
    elif cmd == "check":
        check(sys.argv[2])
    elif cmd == "patch":
        patch()
    else:
        sys.exit(__doc__)
