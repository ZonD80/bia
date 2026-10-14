#!/usr/bin/env python3
"""Package the bia OS of one architecture for a GitHub release (tools/release.sh).

  tools/os-bundle.py <arch> <out-dir> [--secrets FILE]

Reads dist/<arch>/{root.img, vmlinuz-lts, initramfs-lts, rescue.cpio.gz} and the
architecture's files from source/flash/{EFI,boot}. Writes into <out-dir>:

  bia-root-<arch>.NNN.zst   root.img in 1 GiB raw parts, each zstd-compressed alone
  bia-boot-<arch>.tar.zst   ESP tree + kernel, initramfs, rescue image
  os-<arch>.json            this architecture's entry for bia-os.json

--secrets FILE: one byte string per line that must not appear in anything shipped
(API keys, passwords, home paths); the raw files are scanned before compression.
The writer (installers/bia-flash/src/release.rs) reverses this.
"""
import argparse, hashlib, io, json, os, subprocess, sys, tarfile

PART = 1 << 30
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def zstd(data: bytes) -> bytes:
    r = subprocess.run(["zstd", "-q", "-19", "-T0", "-c"], input=data, capture_output=True)
    if r.returncode:
        sys.exit(f"zstd failed: {r.stderr.decode(errors='replace')}")
    return r.stdout


def scan(path, needles):
    """Refuse to ship a file holding any secret (chunked, overlap-safe)."""
    if not needles:
        return
    keep = max(len(n) for n in needles)
    tail = b""
    with open(path, "rb") as f:
        while True:
            chunk = f.read(64 << 20)
            if not chunk:
                return
            buf = tail + chunk
            for n in needles:
                if n in buf:
                    sys.exit(f"SECRET in {path}: {n[:6].decode(errors='replace')}… — refusing to package it")
            tail = buf[-keep:]


def boot_files(arch):
    """(archive name, source path) for this arch's ESP tree and kernel files."""
    flash, dist = f"{ROOT}/source/flash", f"{ROOT}/dist/{arch}"
    other = ("aa64",) if arch == "x86_64" else ("x64",)
    out = []
    for top, dirs, files in os.walk(f"{flash}/EFI"):
        dirs.sort()
        for n in sorted(files):
            rel = os.path.relpath(os.path.join(top, n), flash)
            low = n.lower()
            if rel.count("/") == 1 and (low.startswith("vmlinuz-lts") or low.startswith("initramfs-lts")):
                continue  # per-arch kernel copies from the build; the ESP boots /vmlinuz-lts
            if any(o in low for o in other):
                continue
            out.append((rel, os.path.join(top, n)))
    for top, dirs, files in os.walk(f"{flash}/boot"):
        dirs.sort()
        if arch != "x86_64" and "i386-pc" in top.split(os.sep):
            continue  # BIOS GRUB is x86 only
        for n in sorted(files):
            out.append((os.path.relpath(os.path.join(top, n), flash), os.path.join(top, n)))
    for n in ("vmlinuz-lts", "initramfs-lts", "rescue.cpio.gz"):
        p = f"{dist}/{n}"
        if os.path.isfile(p):
            out.append((n, p))
        elif n != "rescue.cpio.gz":
            sys.exit(f"missing {p} — make ARCH={arch} build first")
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("arch", choices=["x86_64", "arm64"])
    ap.add_argument("out")
    ap.add_argument("--secrets")
    a = ap.parse_args()
    needles = []
    if a.secrets:
        needles = [l.strip().encode() for l in open(a.secrets) if len(l.strip()) >= 8]
    os.makedirs(a.out, exist_ok=True)
    root_img = f"{ROOT}/dist/{a.arch}/root.img"
    if not os.path.isfile(root_img):
        sys.exit(f"missing {root_img} — make ARCH={a.arch} build first")

    files = boot_files(a.arch)
    print(f"== {a.arch}: scanning {len(files) + 1} files for secrets", flush=True)
    scan(root_img, needles)
    for _, p in files:
        scan(p, needles)

    size = os.path.getsize(root_img)
    parts = []
    with open(root_img, "rb") as f:
        i = 0
        while True:
            raw = f.read(PART)
            if not raw:
                break
            name = f"bia-root-{a.arch}.{i:03d}.zst"
            print(f"== {name}: compressing {len(raw) >> 20} MiB", flush=True)
            z = zstd(raw)
            with open(f"{a.out}/{name}", "wb") as o:
                o.write(z)
            parts.append({
                "file": name, "offset": i * PART, "size": len(raw),
                "sha256": hashlib.sha256(raw).hexdigest(),
                "zsize": len(z), "zsha256": hashlib.sha256(z).hexdigest(),
            })
            print(f"   {len(z) >> 20} MiB", flush=True)
            i += 1

    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=tarfile.PAX_FORMAT) as t:
        for rel, p in files:
            info = t.gettarinfo(p, arcname=rel)
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = 0
            info.mode = 0o644
            with open(p, "rb") as fh:
                t.addfile(info, fh)
    bz = zstd(buf.getvalue())
    boot = f"bia-boot-{a.arch}.tar.zst"
    with open(f"{a.out}/{boot}", "wb") as o:
        o.write(bz)
    print(f"== {boot}: {len(files)} files, {len(bz) >> 20} MiB", flush=True)

    entry = {
        "root": {"size": size, "parts": parts},
        "boot": {"file": boot, "size": len(bz), "sha256": hashlib.sha256(bz).hexdigest()},
    }
    with open(f"{a.out}/os-{a.arch}.json", "w") as o:
        json.dump(entry, o, indent=2)
    print(f"== {a.arch}: {len(parts)} parts, {sum(p['zsize'] for p in parts) >> 20} MiB to download", flush=True)


if __name__ == "__main__":
    main()
