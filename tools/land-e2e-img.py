#!/usr/bin/env python3
"""Image side of tools/land-e2e.sh. Runs in the bia-build container (root, loop devices).

  prep-fly    fly.img: FMM keys, a marker file to carry over, eth1 static on the test link
  check       land.img after the flight: the flyer's agent is there, land identity kept
"""
import importlib.util, os, shutil, sys

spec = importlib.util.spec_from_file_location("ut", "/work/tools/update-e2e-img.py")
ut = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ut)
ut.IMAGES.update({"fly": "/ut/fly.img", "land": "/ut/land.img"})

FLY_IP, LAND_IP = "10.77.0.3/24", "10.77.0.2"
MARKER = "home/bia/files/land-e2e-marker"


def prep_fly(token):
    with ut.Img("fly") as f:
        f.mount()
        ut.seed_fmm(f)
        # Current carry build (root.img may predate the last `make rust`).
        farch = "aarch64" if os.environ.get("ARCH", "arm64") == "arm64" else "x86_64"
        carry = f"{f.root}/home/bia/MCPs/carry/bin/{farch}/bia-mcp-carry"
        built = f"/work/dist/{os.environ.get('ARCH', 'arm64')}/bia-mcp-carry"
        if os.path.exists(built) and os.path.isdir(os.path.dirname(carry)):
            shutil.copy2(built, carry)
            print(f"fly: carry from {built}")
        ut.put(f"{f.root}/{MARKER}", f"{token}\n")
        os.chown(f"{f.root}/{MARKER}", 1000, 1000)
        ifs = f"{f.root}/etc/network/interfaces"
        s = open(ifs).read()
        if "iface eth1" not in s:
            s += f"\n# land-e2e: link to the landing VM\nallow-hotplug eth1\niface eth1 inet static\n    address {FLY_IP}\n"
            ut.put(ifs, s)
        router(f.root)
        print(f"fly: marker {token}, eth1 {FLY_IP}, NAT router for the land VM")


def router(root):
    """The land VM's only link is to the flyer: the flyer forwards and masquerades it
    out through its user-net eth0, so the land side has internet (bia-land waits for it)."""
    if not os.path.exists(f"{root}/usr/sbin/nft"):
        for d in ("proc", "sys", "dev"):
            ut.sh("mount", "--bind", f"/{d}", f"{root}/{d}")
        try:
            ut.sh("cp", "-L", "/etc/resolv.conf", f"{root}/etc/resolv.conf")
            ut.sh("chroot", root, "sh", "-c",
                  "DEBIAN_FRONTEND=noninteractive apt-get update -qq && "
                  "DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends nftables >/dev/null")
        finally:
            for d in ("dev", "sys", "proc"):
                ut.sh("umount", f"{root}/{d}", check=False)
    ut.put(f"{root}/etc/land-e2e-nat.nft",
           "table ip land_e2e {\n  chain post {\n    type nat hook postrouting priority 100;\n"
           "    ip saddr 10.77.0.0/24 oifname \"eth0\" masquerade\n  }\n}\n")
    ut.put(f"{root}/etc/systemd/system/land-e2e-router.service",
           "[Unit]\nDescription=land-e2e: route the land VM out through eth0\nAfter=network.target\n\n"
           "[Service]\nType=oneshot\nRemainAfterExit=yes\n"
           "ExecStart=/usr/sbin/sysctl -w net.ipv4.ip_forward=1\nExecStart=/usr/sbin/nft -f /etc/land-e2e-nat.nft\n\n"
           "[Install]\nWantedBy=multi-user.target\n")
    want = f"{root}/etc/systemd/system/multi-user.target.wants/land-e2e-router.service"
    if not os.path.islink(want):
        os.symlink("/etc/systemd/system/land-e2e-router.service", want)


def check(token):
    errs = []

    def want(cond, msg):
        print(("  ok   " if cond else "  FAIL ") + msg)
        if not cond:
            errs.append(msg)

    def read(p):
        try:
            return open(p).read()
        except OSError:
            return ""

    with ut.Img("land", ro=True) as d:
        d.mount()
        r = d.root
        want(ut.blk(d.root_dev, "LABEL") == "BIALAND", "root label BIALAND")
        want(not os.path.exists(f"{r}/etc/bia-land-mode"), "/etc/bia-land-mode removed (agent starts)")
        want(read(f"{r}/{MARKER}").strip() == token, "flyer's files arrived (marker token)")
        want(os.path.isfile(f"{r}/home/bia/files/state/fmm.json"), "flyer's FMM config arrived")
        want(os.path.isfile(f"{r}/home/bia/bia.toml"), "/home/bia/bia.toml present")
        want(read(f"{r}/etc/hostname").strip() == "bia-land", "hostname kept (bia-land)")
        ifs = read(f"{r}/etc/network/interfaces")
        want(f"address {LAND_IP}/24" in ifs and "eth1" not in ifs, "land network config kept, flyer's not copied")
        notice = read(f"{r}/home/bia/files/state/boot_notice")
        want("landed from another machine" in notice or not notice,
             "boot notice says landed (or was already shown and cleared)")
        farch = "aarch64" if os.environ.get("ARCH", "arm64") == "arm64" else "x86_64"
        want(os.access(f"{r}/home/bia/MCPs/carry/bin/{farch}/bia-mcp-carry", os.X_OK), "carry MCP binary present")
        want(os.path.isfile(f"{r}/etc/systemd/system/multi-user.target.wants/bia.service")
             or os.path.isfile(f"{r}/etc/systemd/system/bia.service"), "bia.service installed")
        want(os.path.exists(f"{d.esp}/vmlinuz-lts") and os.path.exists(f"{d.esp}/initramfs-lts"), "ESP has kernel + initramfs")
        cfgs = list(ut.grub_cfgs(d.esp))
        want(cfgs and all("root=LABEL=BIALAND" in read(c) for c in cfgs), "grub.cfg boots LABEL=BIALAND")
    if errs:
        sys.exit(f"CHECK land: {len(errs)} failure(s)")
    print("CHECK land: all good")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "prep-fly":
        prep_fly(sys.argv[2])
    elif cmd == "check":
        check(sys.argv[2])
    else:
        sys.exit(__doc__)
