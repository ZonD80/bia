#!/usr/bin/env python3
"""Package the bia OS of one architecture for a GitHub release (tools/release.sh).

  tools/os-bundle.py <arch> <out-dir> [--secrets FILE] [--level N]

Builds the minimal bia disk image dist/<arch>/bia-<arch>.img (tools/mkimg.sh: GPT, ESP,
BIOS GRUB on x86_64, BIAROOT; no keys, no model) and writes into <out-dir>:

  bia-<arch>.img.xz   the image, xz -T0 (Etcher, Rufus, Raspberry Pi Imager and dd write
                      it as it is; bia-deploy writes it over the whole stick with keys and
                      a model)
  os-<arch>.json      this architecture's entry for bia-os.json

--secrets FILE: one byte string per line that must not appear in anything shipped
(API keys, passwords, home paths); the raw image is scanned before compression.
The writer (installers/bia-deploy/src/release.rs) downloads and checks it.
"""
import argparse, hashlib, json, os, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GITHUB_LIMIT = 2 << 30


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


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(16 << 20):
            h.update(chunk)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("arch", choices=["x86_64", "arm64"])
    ap.add_argument("out")
    ap.add_argument("--secrets")
    ap.add_argument("--level", default="6", help="xz preset (default 6)")
    a = ap.parse_args()
    needles = []
    if a.secrets:
        needles = [l.strip().encode() for l in open(a.secrets) if len(l.strip()) >= 8]
    os.makedirs(a.out, exist_ok=True)

    img = f"{ROOT}/dist/{a.arch}/bia-{a.arch}.img"
    subprocess.run([f"{ROOT}/tools/mkimg.sh", a.arch], check=True, env={**os.environ, "OUT": img})
    print(f"== {a.arch}: scanning the image for secrets", flush=True)
    scan(img, needles)

    name = f"bia-{a.arch}.img.xz"
    xz = f"{a.out}/{name}"
    raw_size = os.path.getsize(img)
    print(f"== {name}: compressing {raw_size >> 20} MiB with xz -{a.level} -T0", flush=True)
    with open(xz + ".tmp", "wb") as o:
        subprocess.run(["xz", f"-{a.level}", "-T0", "-c", img], stdout=o, check=True)
    os.replace(xz + ".tmp", xz)
    size = os.path.getsize(xz)
    if size >= GITHUB_LIMIT:
        sys.exit(f"{xz} is {size >> 20} MiB, over GitHub's 2 GiB file limit")
    print(f"   {size >> 20} MiB", flush=True)

    entry = {"image": {
        "file": name, "size": size, "sha256": sha256(xz),
        "raw_size": raw_size, "raw_sha256": sha256(img),
    }}
    with open(f"{a.out}/os-{a.arch}.json", "w") as o:
        json.dump(entry, o, indent=2)


if __name__ == "__main__":
    main()
