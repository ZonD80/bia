#!/usr/bin/env python3
"""Image side of tests/e2e/status-qemu.sh. Runs in the bia-build:<arch> container (privileged).

  prep      /se/root.img: the work tree's agent (/se/bin/bia-agent) and overlay scripts, the fake
            telegram MCP (tests/e2e/status-qemu-fake.py) in place of the real one, bia's name
            (Milo) and a log folder for the fake bus
  keys      /se/stick.img: ./fmm.json onto the ESP as EFI/bia/fmm.json (never into root.img)
  extract   /se/stick.img after the run: the fake bus's logs and the agent's state to /se/out
"""
import json, os, shutil, struct, subprocess, sys, uuid

STICK = "/se/stick.img"


def sh(*cmd, capture=False, check=True):
    r = subprocess.run(cmd, capture_output=capture, text=True)
    if check and r.returncode:
        sys.exit(f"{' '.join(cmd)} failed: {(r.stderr or '').strip()}")
    return (r.stdout or "").strip()


def parts(path):
    with open(path, "rb") as f:
        f.seek(512)
        h = f.read(92)
        part_lba, num, esz = struct.unpack_from("<Q", h, 72)[0], *struct.unpack_from("<II", h, 80)
        f.seek(part_lba * 512)
        out = []
        for _ in range(num):
            e = f.read(esz)
            if e[:16] == b"\0" * 16:
                continue
            first, last = struct.unpack_from("<QQ", e, 32)
            out.append((first * 512, (last - first + 1) * 512, e[56:128].decode("utf-16-le").rstrip("\0")))
    return out


class Mount:
    def __init__(self, path, fstype, start=None, size=None, ro=False):
        args = ["losetup", "-f", "--show"]
        if start is not None:
            args += ["-o", str(start), "--sizelimit", str(size)]
        self.dev = sh(*args, path, capture=True)
        self.mnt = f"/mnt/se-{fstype}"
        os.makedirs(self.mnt, exist_ok=True)
        sh("mount", "-t", fstype, "-o", "ro" if ro else "rw", self.dev, self.mnt)

    def __enter__(self):
        return self.mnt

    def __exit__(self, *a):
        subprocess.run(["umount", self.mnt])
        subprocess.run(["losetup", "-d", self.dev])


def prep():
    with Mount("/se/root.img", "ext4") as r:
        shutil.copy2("/se/bin/bia-agent", f"{r}/usr/local/sbin/bia-agent")
        sh("cp", "-a", "/work/source/os/overlay/usr/local/sbin/.", f"{r}/usr/local/sbin/")
        sh("cp", "-a", "/work/source/os/overlay/usr/local/share/bia/.", f"{r}/usr/local/share/bia/")
        tg = f"{r}/home/bia/MCPs/telegram"
        owner = os.stat(f"{r}/home/bia/MCPs")
        shutil.rmtree(tg, ignore_errors=True)
        os.makedirs(tg)
        shutil.copy2("/work/tests/e2e/status-qemu-fake.py", f"{tg}/fake.py")
        os.chmod(f"{tg}/fake.py", 0o755)
        json.dump({"name": "telegram", "version": "0", "description": "fake telegram (status e2e)", "command": "fake.py",
                   "boot_tool": "restore"}, open(f"{tg}/mcp.json", "w"))
        state = f"{r}/home/bia/files/state"
        os.makedirs(state, exist_ok=True)
        if os.path.exists(f"{state}/fmm.json"):
            sys.exit("root.img holds files/state/fmm.json; keys must never be in the image")
        json.dump({"name": "Milo", "gender": "male"}, open(f"{state}/self.json", "w"))
        # a session on disk makes the agent start the bus at boot, as a logged-in telegram
        tgf = f"{r}/home/bia/files/telegram"
        os.makedirs(tgf, exist_ok=True)
        open(f"{tgf}/ferogram.session", "w").close()
        for p in (tgf, f"{tgf}/ferogram.session"):
            os.chown(p, owner.st_uid, owner.st_gid)
        logs = f"{r}/home/bia/files/status-e2e"
        os.makedirs(logs, exist_ok=True)
        os.chmod(logs, 0o777)
        for p in (tg, f"{tg}/fake.py", f"{tg}/mcp.json", f"{state}/self.json", logs):
            os.chown(p, owner.st_uid, owner.st_gid)
        ver = open(f"{r}/etc/bia-version").read().strip() if os.path.exists(f"{r}/etc/bia-version") else "?"
        print(f"root.img (image {ver}): work-tree agent, fake telegram MCP, self.json Milo")


def keys():
    esp = parts(STICK)[0]
    with Mount(STICK, "vfat", esp[0], esp[1]) as m:
        os.makedirs(f"{m}/EFI/bia", exist_ok=True)
        shutil.copy("/work/fmm.json", f"{m}/EFI/bia/fmm.json")
    print("stick: ./fmm.json on the ESP as EFI/bia/fmm.json")


def extract():
    root = [p for p in parts(STICK) if p[2] == "BIAROOT"][0]
    os.makedirs("/se/out", exist_ok=True)
    with Mount(STICK, "ext4", root[0], root[1], ro=True) as m:
        for d in ("status-e2e",):
            src = f"{m}/home/bia/files/{d}"
            if os.path.isdir(src):
                shutil.copytree(src, f"/se/out/{d}", dirs_exist_ok=True)
        for f in ("conversation.json", "unanswered.json", "jobs.json", "addressbook.json"):
            src = f"{m}/home/bia/files/state/{f}"
            if os.path.exists(src):
                shutil.copy(src, f"/se/out/{f}")
        logs = f"{m}/home/bia/files/logs"
        if os.path.isdir(logs):
            shutil.copytree(logs, "/se/out/logs", dirs_exist_ok=True, ignore=shutil.ignore_patterns("*.gguf"))
    # the keys stay on the stick (deleted with it); nothing of fmm.json is copied out
    sh("chmod", "-R", "a+rX", "/se/out")
    print("extracted to /se/out")


{"prep": prep, "keys": keys, "extract": extract}[sys.argv[1]]()
