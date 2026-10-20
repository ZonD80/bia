#!/usr/bin/env python3
"""Drive one update scenario in QEMU over the serial socket (tests/e2e/update-e2e.sh run).

  update-e2e-drive.py <scenario> <logfile> -- <command that starts QEMU>

The command must use MODE=headless with SOCK= and MON= in its env list. Every
byte of serial output goes to <logfile>; bia-update/bia-rescue/fold lines are
echoed. Phases: first boot -> trigger (/update or /rescue) -> staged reboot ->
rescue image copies -> destination boots and unfolds -> final idle boot -> poweroff.
Env: TIMEOUT (minutes for the copy phase, default 20), TRIGGER=agent sends a
natural-language request instead of the slash command (needs a working model).
"""
import os, re, socket, subprocess, sys, threading, time

scen, logpath = sys.argv[1], sys.argv[2]
cmd = sys.argv[sys.argv.index("--") + 1:]
envs = dict(a.split("=", 1) for a in cmd if re.match(r"^[A-Z_]+=", a))
SOCK, MON = envs["SOCK"], envs["MON"]
COPY_TIMEOUT = int(os.environ.get("TIMEOUT", "20")) * 60
TRIGGER = os.environ.get("TRIGGER", "console")
ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b[78()][0-9A-B]?")
SHOW = re.compile(r"bia-update:|bia-rescue:|unfold:|fold: done|Kernel panic|rescue \(update|update:|rescue:|root file system|FILE SYSTEM WAS MODIFIED")

for p in (SOCK, MON):
    if os.path.exists(p):
        os.remove(p)
qlog = open(logpath + ".qemu", "w")
qemu = subprocess.Popen(cmd, stdout=qlog, stderr=subprocess.STDOUT)
t0 = time.time()


def stamp():
    m, s = divmod(int(time.time() - t0), 60)
    return f"[{m:3d}:{s:02d}]"


s = None
for _ in range(300):
    if qemu.poll() is not None:
        sys.exit(f"QEMU exited early ({qemu.returncode}); see {logpath}.qemu")
    try:
        s = socket.socket(socket.AF_UNIX)
        s.connect(SOCK)
        break
    except OSError:
        s = None
        time.sleep(0.2)
if s is None:
    qemu.kill()
    sys.exit("no serial socket")

buf = []  # cleaned text
lock = threading.Lock()
log = open(logpath, "w")


def reader():
    partial = ""
    while True:
        try:
            d = s.recv(65536)
        except OSError:
            break
        if not d:
            break
        t = d.decode(errors="replace")
        log.write(t)
        log.flush()
        t = ANSI.sub("", t)
        with lock:
            buf.append(t)
        partial += t
        *lines, partial = re.split(r"\r?\n", partial)
        for ln in lines:
            ln = ln.split("\r")[-1].strip()
            if ln and SHOW.search(ln):
                print(f"{stamp()} {ln[:200]}", flush=True)


threading.Thread(target=reader, daemon=True).start()
cursor = 0


def text():
    with lock:
        return "".join(buf)


def expect(what, pat, timeout, fail=None):
    """Wait for pat in output after the cursor; move the cursor past it."""
    global cursor
    print(f"{stamp()} -- waiting: {what}", flush=True)
    end = time.time() + timeout
    while time.time() < end:
        t = text()
        m = re.search(pat, t[cursor:])
        if m:
            cursor += m.end()
            return m
        if fail:
            f = re.search(fail, t[cursor:])
            if f:
                time.sleep(5)
                die(f"{what}: failure marker {f.group(0)!r}")
        if qemu.poll() is not None:
            die(f"{what}: QEMU exited ({qemu.returncode})")
        time.sleep(0.5)
    die(f"{what}: timeout after {timeout}s waiting for {pat!r}")


def send(line):
    print(f"{stamp()} >> {line}", flush=True)
    s.sendall((line + "\r").encode())


def mon(c):
    try:
        m = socket.socket(socket.AF_UNIX)
        m.connect(MON)
        m.sendall((c + "\n").encode())
        time.sleep(0.5)
        m.close()
    except OSError:
        pass


def die(msg):
    print(f"{stamp()} FAILED: {msg}", flush=True)
    tail = text()[-6000:]
    print("---- last serial output ----\n" + "\n".join(l.split("\r")[-1] for l in tail.splitlines()[-60:]))
    mon("screendump " + logpath + ".ppm")
    mon("quit")
    time.sleep(2)
    qemu.kill()
    sys.exit(1)


BOOTED = r"bia-update: (idle|boot)"
FAIL_UPD = r"bia-update: [^\n]*\[ FAIL \]"
FAIL_RESCUE = r"bia-rescue: [^\n]*\[ FAIL \]|Kernel panic"

# 1. first boot, agent up
if scen == "fsck":
    # The initramfs checks the damaged root before mounting it and repairs it. Its output
    # goes to /dev/console (the screen on arm64), but it leaves /run/initramfs/fsck-root
    # only after a check that passed, and systemd then skips its own root check.
    expect("boot check of the root (initramfs)", r"systemd-fsck-root\.service[^\n]*skipped[^\n]*/run/initramfs/fsck-root", 300,
           fail=r"requires a manual fsck|Kernel panic")
expect("first boot (bia-update.service)", BOOTED, 600, fail=FAIL_UPD)
expect("agent console", r"->|Ready for commands|ready|FMM", 300)
time.sleep(15)

if scen == "fsck":
    send("Check and repair this computer's disk now. Call carry_fsck." if TRIGGER == "agent" else "/fsck")
    expect("disk check staged", r"rebooting into the rescue image", 600, fail=FAIL_UPD)
    expect("rescue image boots", r"bia-rescue: bia rescue image", 600, fail=FAIL_RESCUE)
    expect("offline e2fsck", r"bia-rescue: fsck \S+ \.+ \[ OK \]", COPY_TIMEOUT, fail=FAIL_RESCUE)
    expect("back on the disk", r"bia-update: disk check: no errors", 600, fail=FAIL_UPD)
    expect("idle after the check", r"bia-update: idle", 600, fail=FAIL_UPD)
    if re.search(r"bia-rescue: bia rescue image", text()[cursor:]):
        die("rescue image ran again")
    expect("agent console after the check", r"->|Ready for commands|ready|FMM", 300)
    time.sleep(10)
    send("/poweroff")
    try:
        qemu.wait(180)
    except subprocess.TimeoutExpired:
        mon("quit")
        qemu.wait(30)
    print(f"{stamp()} scenario fsck: run finished", flush=True)
    sys.exit(0)

# 2. trigger
if TRIGGER == "agent":
    req = {"disk": "Update this computer's OS from the flash stick now. Call carry_update.",
           "flash": "Update this flash system from the internal disk now. Call carry_update.",
           "rescue": "Rescue the internal disk from this flash now. Call carry_rescue.",
           "internet": "Update this computer's OS from the internet now. Call carry_update with from='internet'."}[scen]
else:
    req = {"disk": "/update", "flash": "/update", "rescue": "/rescue", "internet": "/update internet"}[scen]
for attempt in range(6):
    send(req)
    try_end = time.time() + (240 if TRIGGER == "agent" else 60)
    hit = None
    while time.time() < try_end and not hit:
        hit = re.search(r"bia-update: (this system is|rescue —)", text()[cursor:])
        time.sleep(0.5)
    if hit:
        break
else:
    die("trigger: bia-update never started")

# 3. stage -> reboot into rescue
if scen == "internet":
    expect("release found", r"bia-update: newest release 9\.9\.9", 300, fail=FAIL_UPD)
    expect("download checked", r"download bia-\S+ \.+ \[ OK \]", 1800, fail=FAIL_UPD)
    expect("release unpacked", r"unpacked release 9\.9\.9 \.+ \[ OK \]", 1800, fail=FAIL_UPD)
expect("staging (fold + marker)", r"rebooting into the rescue image", 1800, fail=FAIL_UPD)
expect("rescue image boots", r"bia-rescue: bia rescue image", 600, fail=FAIL_RESCUE)
expect("rescue copies + verifies", r"bia-rescue: update copied", COPY_TIMEOUT, fail=FAIL_RESCUE)

# 4. destination boots and unfolds
expect("destination boot", r"bia-update: boot", 600, fail=FAIL_RESCUE)
expect("unfold", r"restoring this machine \.+ \[ OK \]|unfold: rebooting", 1800,
       fail=r"restoring this machine \.+ \[ FAIL \]|" + FAIL_UPD)
expect("final boot wipes /bia-update", r"bia-update: removed /bia-update", 900, fail=FAIL_UPD)
expect("final boot idle", r"bia-update: idle", 900, fail=FAIL_UPD)
if re.search(r"bia-rescue: bia rescue image", text()[cursor:]):
    die("rescue image ran again")
expect("agent console after update", r"->|Ready for commands|ready|FMM", 300)
time.sleep(10)

# 5. power off cleanly so the images are consistent for the checks
send("/poweroff")
try:
    qemu.wait(180)
except subprocess.TimeoutExpired:
    print(f"{stamp()} poweroff timed out; quitting QEMU", flush=True)
    mon("quit")
    qemu.wait(30)
print(f"{stamp()} scenario {scen}: run finished", flush=True)
