#!/usr/bin/env python3
"""Image steps for tests/e2e/takeover-e2e.sh (runs in bia-build, /ut = dist/te-<arch>):

  prep-fly    fly.img: FMM keys, the work tree's carry build, a marker file to carry over
"""
import importlib.util, os, shutil, sys

spec = importlib.util.spec_from_file_location("ut", "/work/tests/e2e/update-e2e-img.py")
ut = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ut)
ut.IMAGES.update({"fly": "/ut/fly.img"})
MARKER = "home/bia/files/takeover-land-marker"


def prep_fly(token):
    with ut.Img("fly") as f:
        f.mount()
        ut.seed_fmm(f)
        arch = os.environ.get("ARCH", "arm64")
        farch = "aarch64" if arch == "arm64" else "x86_64"
        carry = f"{f.root}/home/bia/MCPs/carry/bin/{farch}/bia-mcp-carry"
        built = f"/work/dist/{arch}/bia-mcp-carry"
        if os.path.exists(built) and os.path.isdir(os.path.dirname(carry)):
            shutil.copy2(built, carry)
        ut.put(f"{f.root}/{MARKER}", f"{token}\n")
        os.chown(f"{f.root}/{MARKER}", 1000, 1000)
        # LAND=agent: the flyer's agent logs into the Ubuntu server over ssh itself
        if os.path.exists("/ut/fly_key"):
            ssh = f"{f.root}/root/.ssh"
            os.makedirs(ssh, exist_ok=True)
            os.chmod(ssh, 0o700)
            shutil.copy("/ut/fly_key", f"{ssh}/id_ed25519")
            shutil.copy("/ut/fly_key.pub", f"{ssh}/id_ed25519.pub")
            os.chmod(f"{ssh}/id_ed25519", 0o600)
            ut.put(f"{ssh}/config", "Host *\n    StrictHostKeyChecking accept-new\n")
            print("fly: ssh key in /root/.ssh")
        print(f"fly: keys, carry from {built}, marker {token}")


if __name__ == "__main__":
    if sys.argv[1] == "prep-fly":
        prep_fly(sys.argv[2])
    else:
        sys.exit(f"unknown step {sys.argv[1]}")
