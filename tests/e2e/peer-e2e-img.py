#!/usr/bin/env python3
"""Image side of tests/e2e/peer-e2e.sh. Runs in the bia-build container (root, loop devices).

  prep <a|b>   FMM keys, hostname bia-<x>, eth1 static on the test link, and the current
               agent + system and web MCPs + overlay scripts from dist/ (root.img may be older)
  check        a.img after the run: the request was approved and collected, bia-b talked
"""
import importlib.util, json, os, re, shutil, sys

spec = importlib.util.spec_from_file_location("ut", "/work/tests/e2e/update-e2e-img.py")
ut = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ut)
ut.IMAGES.update({"a": "/ut/a.img", "b": "/ut/b.img"})
LINK_IP = {"a": "10.77.0.2/24", "b": "10.77.0.3/24"}
ARCH = os.environ.get("ARCH", "arm64")
FARCH = "aarch64" if ARCH == "arm64" else "x86_64"


def prep(name):
    with ut.Img(name) as im:
        im.mount()
        r = im.root
        ut.seed_fmm(im)
        ut.put(f"{r}/etc/hostname", f"bia-{name}\n")
        # not a first start and no pending release notes: the agent boots to "Ready for commands"
        ver = re.search(r'^version = "(.*)"', open("/work/Cargo.toml").read(), re.M).group(1)
        for f, text in (("welcomed", f"bia {ver}\n"), ("version", f"{ver}\n")):
            ut.put(f"{r}/home/bia/files/state/{f}", text)
            os.chown(f"{r}/home/bia/files/state/{f}", 1000, 1000)
        ifs = f"{r}/etc/network/interfaces"
        s = open(ifs).read()
        if "iface eth1" not in s:
            s += f"\n# peer-e2e: link to the other VM\nallow-hotplug eth1\niface eth1 inet static\n    address {LINK_IP[name]}\n"
            ut.put(ifs, s)
        out = f"/work/dist/{ARCH}"
        if os.path.exists(f"{out}/bia-agent"):
            shutil.copy2(f"{out}/bia-agent", f"{r}/usr/local/sbin/bia-agent")
        # the system MCP and the web MCP (the HTTPS API this test talks to) from this build
        for mcp in ("system", "web"):
            bindir = f"{r}/home/bia/MCPs/{mcp}/bin/{FARCH}"
            os.makedirs(bindir, exist_ok=True)
            if os.path.exists(f"{out}/bia-mcp-{mcp}"):
                shutil.copy2(f"{out}/bia-mcp-{mcp}", f"{bindir}/bia-mcp-{mcp}")
            shutil.copy2(f"/work/source/flash/MCPs/{mcp}/mcp.json", f"{r}/home/bia/MCPs/{mcp}/mcp.json")
        ut.sh("cp", "-a", "/work/source/os/overlay/usr/local/share/bia/.", f"{r}/usr/local/share/bia/")
        for d in (f"{r}/home/bia/files/state/pki/requests",):
            shutil.rmtree(d, ignore_errors=True)
        print(f"{name}: bia-{name}, eth1 {LINK_IP[name]}, agent + system and web MCPs from dist")


def check():
    errs = []

    def want(cond, msg):
        print(("  ok   " if cond else "  FAIL ") + msg)
        if not cond:
            errs.append(msg)

    with ut.Img("a", ro=True) as a, ut.Img("b", ro=True) as b:
        a.mount(), b.mount()
        pki = f"{a.root}/home/bia/files/state/pki"
        reqs = []
        rdir = f"{pki}/requests"
        for n in sorted(os.listdir(rdir)) if os.path.isdir(rdir) else []:
            if n.endswith(".json"):
                reqs.append(json.load(open(f"{rdir}/{n}")))
        mine = [q for q in reqs if q.get("identity") == "bia-b"]
        want(len(mine) >= 1, f"bia-a has a certificate request from bia-b ({len(reqs)} total)")
        done = [q for q in mine if q.get("status") == "issued" and q.get("serial")]
        want(len(done) == 1, "exactly one request was approved and downloaded (status issued)")
        if done:
            q = done[0]
            want(q.get("decided_by", "").startswith("root"), f"decided by {q.get('decided_by')!r}")
            want(q.get("reason", "").startswith("MANGO-"), f"approved with root's message for bia-b {q.get('reason')!r}")
            want(q["decided"] - q["created"] >= 60, f"approval came {q['decided'] - q['created']}s after the request (requester had to wait)")
            meta = f"{pki}/issued/{q['serial']}.json"
            m = json.load(open(meta)) if os.path.exists(meta) else {}
            want(m.get("identity") == "bia-b" and m.get("role") == "agent" and m.get("trust") == "normal",
                 f"issued cert {q['serial']}: {m.get('identity')} role={m.get('role')} trust={m.get('trust')}")
            # CSR flow: bia-a never held bia-b's private key; bia-b's own key matches the cert
            stray = []
            for dp, _, fs in os.walk(pki):
                for f in fs:
                    path = os.path.join(dp, f)
                    if path in (f"{pki}/ca.key", f"{pki}/server.key"):
                        continue
                    if b"PRIVATE KEY" in open(path, "rb").read():
                        stray.append(path[len(pki) + 1:])
            want(not stray, f"no client private key on bia-a {stray or ''}")
            crt = f"{pki}/issued/{q['serial']}.crt"
            pub = ut.sh("openssl", "x509", "-in", crt, "-pubkey", "-noout", capture=True, check=False)
            match = []
            for dp, dirs, fs in os.walk(f"{b.root}/home/bia"):
                dirs[:] = [d for d in dirs if d not in ("MCPs", "model", ".cache", "chrome")]
                for f in fs:
                    path = os.path.join(dp, f)
                    try:
                        if os.path.getsize(path) > 20000 or b"PRIVATE KEY" not in open(path, "rb").read():
                            continue
                    except OSError:
                        continue
                    if pub and ut.sh("openssl", "pkey", "-in", path, "-pubout", capture=True, check=False) == pub:
                        match.append(path[len(b.root):])
            want(bool(match), f"bia-b holds the private key for the issued cert {match}")
        # renewal: bia-b's first cert revoked, a second one active for a new key only bia-b has
        issued = []
        for n in os.listdir(f"{pki}/issued"):
            if n.endswith(".json"):
                issued.append(json.load(open(f"{pki}/issued/{n}")))
        bb = sorted([m for m in issued if m["identity"] == "bia-b"], key=lambda m: m["created"])
        want(len(bb) == 2 and bb[0]["revoked"] and not bb[1]["revoked"],
             f"bia-b renewed: {[(m['serial'], 'revoked' if m['revoked'] else 'active') for m in bb]}")
        if len(bb) == 2:
            newpub = ut.sh("openssl", "x509", "-in", f"{pki}/issued/{bb[1]['serial']}.crt", "-pubkey", "-noout", capture=True, check=False)
            oldpub = ut.sh("openssl", "x509", "-in", f"{pki}/issued/{bb[0]['serial']}.crt", "-pubkey", "-noout", capture=True, check=False)
            want(newpub != oldpub, "the renewed cert is for a new key")
            found = []
            for dp, dirs, fs in os.walk(f"{b.root}/home/bia"):
                dirs[:] = [d for d in dirs if d not in ("MCPs", "model", ".cache", "chrome")]
                for f in fs:
                    path = os.path.join(dp, f)
                    try:
                        if os.path.getsize(path) > 20000 or b"PRIVATE KEY" not in open(path, "rb").read():
                            continue
                    except OSError:
                        continue
                    if ut.sh("openssl", "pkey", "-in", path, "-pubout", capture=True, check=False) == newpub:
                        found.append(path[len(b.root):])
            want(bool(found), f"bia-b holds the key for its renewed cert {found}")
        # renewal can land in the same second as the first cert: the revoked one is the older
        roots = sorted([m for m in issued if m["identity"] == "root"], key=lambda m: (m["created"], not m["revoked"]))
        want(len(roots) == 2 and roots[0]["revoked"] and not roots[1]["revoked"],
             f"root renewed from the browser code: {[(m['serial'], 'revoked' if m['revoked'] else 'active') for m in roots]}")
        stray = []
        for dp, _, fs in os.walk(pki):
            for f in fs:
                path = os.path.join(dp, f)
                if path not in (f"{pki}/ca.key", f"{pki}/server.key") and b"PRIVATE KEY" in open(path, "rb").read():
                    stray.append(path[len(pki) + 1:])
        want(not stray, f"after renewals still no client private key on bia-a {stray or ''}")
        chat = f"{a.root}/home/bia/files/state/web/bia-b"
        want(os.path.isdir(chat), "bia-a has a web chat for identity bia-b")
        text = ""
        for dp, _, fs in os.walk(chat):
            for f in fs:
                try:
                    text += open(os.path.join(dp, f), errors="replace").read()
                except OSError:
                    pass
        want("bia-b" in text or len(text) > 0, "the chat has messages")
        main = f"{chat}/main.json"
        msgs = json.loads(open(main).read(), strict=False) if os.path.exists(main) else []
        if isinstance(msgs, dict):
            msgs = msgs.get("messages", msgs.get("msgs", []))
        froms = [m.get("from") for m in msgs if isinstance(m, dict)]
        want("bia-b" in froms and "bia" in froms, f"bia-b's main chat has bia-b's message and bia's answer (from: {sorted(set(froms))})")
        want(not os.path.isdir(f"{a.root}/home/bia/files/state/web/bia"), "nothing written under web/bia")
    if errs:
        sys.exit(f"CHECK peer: {len(errs)} failure(s)")
    print("CHECK peer: all good")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "prep":
        prep(sys.argv[2])
    elif cmd == "check":
        check()
    else:
        sys.exit(__doc__)
