#!/usr/bin/env python3
"""Image side of tests/e2e/update-e2e.sh. Runs in the bia-build container (root, loop devices).

  make-disk          turn /ut/disk.img (a copy of the flash layout) into an internal
                     disk: LABEL=BIADISK / BIADBOOT, new ext4 UUID, fstab, grub.cfg
  prep <scenario>    versions (source = higher), markers, clean /bia-update + rescue markers;
                     internet: also a release in /ut/release (the real bia-<arch>.img stamped
                     9.9.9, xz, bia-os.json) that the disk downloads over QEMU's NAT
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
SCEN = {"disk": ("disk", "flash"), "flash": ("flash", "disk"), "rescue": ("disk", "flash"),
        # the disk updates from GitHub-style release files; the flash stays attached (not newer)
        "internet": ("disk", "flash"),
        # no update: the disk's root is damaged, checked at boot and by carry_fsck
        "fsck": ("disk", "flash")}
REL = f"{UT}/release"
NET_VERSION = "9.9.9"
# QEMU user networking: the host's loopback is 10.0.2.2 in the guest
RELEASE_URL = "http://10.0.2.2:8099"


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
            newer = name == src and scen != "internet"
            r = base + 1 if newer else base
            put(f"{im.root}/etc/bia-release", f"{r}\n")
            # bia-update compares versions, not the build stamp
            put(f"{im.root}/etc/bia-version", "1.0.1\n" if newer else "1.0.0\n")
            if os.path.exists(f"{im.root}/etc/bia-release-url"):
                os.remove(f"{im.root}/etc/bia-release-url")
        # Make the source ESP's initramfs differ (trailing zeros are ignored by
        # the kernel) so the check can see the destination ESP was refreshed.
        si, di = f"{imgs[src].esp}/initramfs-lts", f"{imgs[dst].esp}/initramfs-lts"
        if subprocess.run(["cmp", "-s", si, di]).returncode == 0:
            with open(si, "ab") as f:
                f.write(b"\0" * 4096)
        put(f"{imgs[src].root}/etc/ut-new-os", f"from {src}\n")
        if scen == "internet":
            put(f"{imgs[dst].root}/etc/bia-release-url", RELEASE_URL + "\n")
        put(f"{imgs[dst].root}/usr/local/share/ut-old-only", "must be deleted by the update\n")
        put(f"{imgs[dst].root}/home/bia/files/ut-agent-marker", f"agent of {dst}\n")
        os.chown(f"{imgs[dst].root}/home/bia/files/ut-agent-marker", 1000, 1000)
        for name, im in imgs.items():
            print(f"{name}: {LABEL[name]} version {open(im.root + '/etc/bia-version').read().strip()}"
                  + ("  <- source" if name == src and scen != "internet" else "  <- destination" if name == dst else ""))
    if scen == "internet":
        make_release()
    if scen == "fsck":
        damage_disk_root()


def damage_disk_root():
    """What the field showed: /usr/bin/df's inode fails its checksum (EFSBADCRC, "Bad
    message") and the superblock says "clean with errors"."""
    with Img("disk") as dk:
        dev = dk.root_dev
        sh("debugfs", "-w", "-R", "sif /usr/bin/df checksum 0xbad", dev)
        sh("debugfs", "-w", "-R", "ssv state 3", dev)
        sh("debugfs", "-w", "-R", "ssv error_count 9", dev)
        state = sh("dumpe2fs", "-h", dev, capture=True)
        print("disk root damaged: " + " ".join(l.strip() for l in state.splitlines()
                                                 if l.startswith(("Filesystem state:", "FS Error count:"))))
        bad = subprocess.run(["e2fsck", "-fn", dev], capture_output=True, text=True)
        if bad.returncode == 0:
            sys.exit("damage_disk_root: e2fsck -fn finds nothing wrong")


def make_release():
    """The real release image, stamped as a newer version, packed like tools/release.sh does."""
    arch = os.environ.get("ARCH", "arm64")
    src = f"/work/dist/{arch}/bia-{arch}.img"
    if not os.path.exists(src):
        sys.exit(f"{src} is missing: run make img ARCH={arch} first")
    shutil.rmtree(REL, ignore_errors=True)
    os.makedirs(REL)
    raw = f"{REL}/os.img"
    sh("cp", "--sparse=always", src, raw)
    parts = partitions(raw)
    loops = [sh("losetup", "-f", "--show", "-o", str(o), "--sizelimit", str(s), raw, capture=True) for _, o, s in parts]
    root_dev = loops[max(range(len(parts)), key=lambda i: parts[i][2])]
    esp_dev = loops[0]
    try:
        for d, dev, fs in (("/mnt/rel-root", root_dev, "ext4"), ("/mnt/rel-esp", esp_dev, "vfat")):
            os.makedirs(d, exist_ok=True)
            sh("mount", "-t", fs, dev, d)
        put("/mnt/rel-root/etc/bia-version", NET_VERSION + "\n")
        put("/mnt/rel-root/etc/ut-new-os", "from internet\n")
        # a kernel file that differs from the old one, so the check sees the ESP was refreshed
        with open("/mnt/rel-esp/initramfs-lts", "ab") as f:
            f.write(b"\0" * 4096)
    finally:
        for d in ("/mnt/rel-esp", "/mnt/rel-root"):
            subprocess.run(["umount", d], capture_output=True)
        for l in loops:
            subprocess.run(["losetup", "-d", l], capture_output=True)
    name = f"bia-{arch}.img.xz"
    print(f"release: packing {name} (xz -1)", flush=True)
    with open(f"{REL}/{name}", "wb") as o:
        subprocess.run(["xz", "-1", "-T0", "-c", raw], stdout=o, check=True)
    import hashlib, json

    def sha(p):
        h = hashlib.sha256()
        with open(p, "rb") as f:
            while c := f.read(16 << 20):
                h.update(c)
        return h.hexdigest()
    m = {"version": NET_VERSION, "tag": f"v{NET_VERSION}", "os": {arch: {"image": {
        "file": name, "size": os.path.getsize(f"{REL}/{name}"), "sha256": sha(f"{REL}/{name}"),
        "raw_size": os.path.getsize(raw), "raw_sha256": sha(raw)}}}}
    with open(f"{REL}/bia-os.json", "w") as f:
        json.dump(m, f, indent=2)
    print(f"release: {NET_VERSION} at {RELEASE_URL} ({os.path.getsize(f'{REL}/{name}') >> 20} MiB)")


def check(scen):
    dst, src = SCEN[scen]
    errs = []

    def want(cond, msg):
        print(("  ok   " if cond else "  FAIL ") + msg)
        if not cond:
            errs.append(msg)

    if scen == "internet":
        return check_internet()
    if scen == "fsck":
        return check_fsck()
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


def check_fsck():
    errs = []

    def want(cond, msg):
        print(("  ok   " if cond else "  FAIL ") + msg)
        if not cond:
            errs.append(msg)

    with Img("disk") as dk:
        dev = dk.root_dev
        r = subprocess.run(["e2fsck", "-fn", dev], capture_output=True, text=True)
        want(r.returncode == 0, "e2fsck -fn finds the disk root clean" + ("" if r.returncode == 0 else ": " + r.stdout[-400:]))
        head = sh("dumpe2fs", "-h", dev, capture=True)
        state = next((l.split(":", 1)[1].strip() for l in head.splitlines() if l.startswith("Filesystem state:")), "?")
        want(state == "clean", f"filesystem state is clean ({state})")
        dk.mount()
        d = dk.root
        want(os.path.getsize(f"{d}/usr/bin/df") > 0 and open(f"{d}/usr/bin/df", "rb").read(4) == b"\x7fELF", "/usr/bin/df reads again")
        log = f"{d}/var/log/bia-fsck.log"
        want(os.path.exists(log) and "e2fsck" in open(log, errors="replace").read(), "the offline check left /var/log/bia-fsck.log")
        want(not os.path.exists(f"{d}/bia-update"), "/bia-update cleaned up")
        for rd in ("bia-rescue", "EFI/bia/bia-rescue"):
            want(not os.path.exists(f"{dk.esp}/{rd}/next.cfg"), f"no rescue marker {rd}/next.cfg")
    if errs:
        sys.exit(f"CHECK fsck: {len(errs)} failure(s)")
    print("CHECK fsck: all good")


def check_internet():
    dst = "disk"
    errs = []

    def want(cond, msg):
        print(("  ok   " if cond else "  FAIL ") + msg)
        if not cond:
            errs.append(msg)

    raw = f"{REL}/os.img"
    parts = partitions(raw)
    esp_loop = sh("losetup", "-f", "--show", "-r", "-o", str(parts[0][1]), "--sizelimit", str(parts[0][2]), raw, capture=True)
    os.makedirs("/mnt/rel-esp", exist_ok=True)
    sh("mount", "-t", "vfat", "-o", "ro", esp_loop, "/mnt/rel-esp")
    try:
        with Img(dst, ro=True) as d, Img("flash", ro=True) as fl:
            d.mount(), fl.mount()
            ver = open(f"{d.root}/etc/bia-version").read().strip()
            want(ver == NET_VERSION, f"disk runs the downloaded version ({ver})")
            want(os.path.exists(f"{d.root}/etc/ut-new-os") and open(f"{d.root}/etc/ut-new-os").read() == "from internet\n",
                 "new OS file from the release is on the disk (not from the flash)")
            want(not os.path.exists(f"{d.root}/usr/local/share/ut-old-only"), "file only in the old OS was deleted")
            mk = f"{d.root}/home/bia/files/ut-agent-marker"
            want(os.path.exists(mk) and open(mk).read() == f"agent of {dst}\n", "the disk agent's files survived (fold/unfold)")
            want(not os.path.exists(f"{d.root}/bia-update"), "/bia-update (and the download) cleaned up")
            fstab = open(f"{d.root}/etc/fstab").read()
            want("LABEL=BIADISK" in fstab, "fstab still mounts LABEL=BIADISK")
            want(blk(d.root_dev, "LABEL") == "BIADISK", "root label still BIADISK")
            for rd in ("bia-rescue", "EFI/bia/bia-rescue"):
                want(not os.path.exists(f"{d.esp}/{rd}/next.cfg"), f"no rescue marker {rd}/next.cfg on the disk ESP")
            for k in ("vmlinuz-lts", "initramfs-lts"):
                same = subprocess.run(["cmp", "-s", f"{d.esp}/{k}", f"/mnt/rel-esp/{k}"]).returncode == 0
                want(same, f"disk ESP {k} == release ESP {k}")
            for cfg in grub_cfgs(d.esp):
                c = open(cfg).read()
                want("# bia-rescue hook" in c and "root=LABEL=BIADISK" in c, f"{cfg[len(d.esp):]}: rescue hook + root=LABEL=BIADISK")
            want(open(f"{fl.root}/etc/bia-version").read().strip() == "1.0.0"
                 and open(f"{fl.root}/etc/ut-new-os").read() == "from flash\n", "flash left untouched")
            want(os.path.exists(f"{d.root}/home/bia/bia.toml"), "disk /home/bia/bia.toml present")
    finally:
        subprocess.run(["umount", "/mnt/rel-esp"], capture_output=True)
        subprocess.run(["losetup", "-d", esp_loop], capture_output=True)
    if errs:
        sys.exit(f"CHECK internet: {len(errs)} failure(s)")
    print("CHECK internet: all good")


def rebuild_initramfs(im):
    """update-initramfs in the image (same architecture as this container), then the ESP copy."""
    for m in ("proc", "sys", "dev"):
        sh("mount", "--bind", f"/{m}", f"{im.root}/{m}")
    try:
        sh("chroot", im.root, "update-initramfs", "-u", "-k", "all")
    finally:
        for m in ("dev", "sys", "proc"):
            subprocess.run(["umount", f"{im.root}/{m}"], capture_output=True)
    initrd = sorted(p for p in os.listdir(f"{im.root}/boot") if p.startswith("initrd.img-") and not p.endswith(".old"))[-1]
    for k in ("initramfs-lts", "EFI/bia/initramfs-lts"):
        if os.path.exists(f"{im.esp}/{k}"):
            shutil.copy2(f"{im.root}/boot/{initrd}", f"{im.esp}/{k}")


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
            sh("cp", "-a", "/work/source/os/overlay/etc/initramfs-tools/.", f"{im.root}/etc/initramfs-tools/")
            rebuild_initramfs(im)
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
