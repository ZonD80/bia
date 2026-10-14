"""QEMU VMs driven over their serial sockets, for the two-VM e2e tests
(tools/land-e2e-drive.py, tools/peer-e2e-drive.py).

Env: ARCH (arm64), LE (work dir: images, sockets, logs), MEM (4G), CPUS (4).
Each VM's serial output goes to $LE/<name>.log; lines matching SHOW are echoed.
"""
import os, re, shutil, socket, subprocess, sys, threading, time

ARCH = os.environ.get("ARCH", "arm64")
LE = os.environ["LE"]
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MEM, CPUS = os.environ.get("MEM", "4G"), os.environ.get("CPUS", "4")
FW = "/opt/homebrew/share/qemu"
ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b[78()][0-9A-B]?")
SHOW = re.compile(r"Kernel panic")  # drivers replace this
t0 = time.time()
VMS = []


def stamp():
    m, s = divmod(int(time.time() - t0), 60)
    return f"[{m:3d}:{s:02d}]"


def base_cmd(name, firmware="uefi"):
    if firmware == "bios":
        # SeaBIOS (QEMU's default firmware): legacy boot, x86_64 only.
        return ["qemu-system-x86_64", "-M", "q35", "-accel", "tcg,thread=multi", "-cpu", "max"]
    vars_fd = f"{LE}/efivars-{name}.fd"
    if ARCH == "arm64":
        shutil.copyfile(f"{FW}/edk2-arm-vars.fd", vars_fd)
        return ["qemu-system-aarch64", "-M", "virt", "-accel", "hvf", "-cpu", "host",
                "-drive", f"if=pflash,format=raw,readonly=on,file={FW}/edk2-aarch64-code.fd",
                "-drive", f"if=pflash,format=raw,file={vars_fd}"]
    shutil.copyfile(f"{FW}/edk2-i386-vars.fd", vars_fd)
    return ["qemu-system-x86_64", "-M", "q35", "-accel", "tcg", "-cpu", "max",
            "-drive", f"if=pflash,format=raw,readonly=on,file={FW}/edk2-x86_64-code.fd",
            "-drive", f"if=pflash,format=raw,file={vars_fd}"]


def link(server, mac):
    """Private L2 between the two VMs over one unix stream socket (lossless; unix
    datagrams on macOS drop bursts). The land VM starts first and listens."""
    path = f"{LE}/link.sock"
    if server and os.path.exists(path):
        os.remove(path)
    return ["-netdev", f"stream,id=lk,server={'on' if server else 'off'},addr.type=unix,addr.path={path}",
            "-device", f"virtio-net-pci,netdev=lk,mac={mac}"]


class VM:
    def __init__(self, name, extra, firmware="uefi"):
        self.name = name
        self.sock, self.mon_path = f"{LE}/{name}.serial", f"{LE}/{name}.mon"
        for p in (self.sock, self.mon_path):
            if os.path.exists(p):
                os.remove(p)
        cmd = base_cmd(name, firmware) + ["-smp", CPUS, "-m", MEM, "-device", "virtio-rng-pci"] + extra + [
            "-device", "ramfb", "-device", "qemu-xhci", "-device", "usb-kbd", "-display", "none",
            "-monitor", f"unix:{self.mon_path},server,nowait", "-serial", f"unix:{self.sock},server,nowait"]
        self.qlog = open(f"{LE}/{name}.qemu.log", "w")
        print(f"{stamp()} [{name}] {' '.join(cmd)}", flush=True)
        self.proc = subprocess.Popen(cmd, stdout=self.qlog, stderr=subprocess.STDOUT)
        self.buf, self.lock, self.cursor = [], threading.Lock(), 0
        self.log = open(f"{LE}/{name}.log", "w")
        self.s = None
        for _ in range(300):
            if self.proc.poll() is not None:
                die(f"[{name}] QEMU exited early ({self.proc.returncode}); see {LE}/{name}.qemu.log")
            try:
                self.s = socket.socket(socket.AF_UNIX)
                self.s.connect(self.sock)
                break
            except OSError:
                self.s = None
                time.sleep(0.2)
        if self.s is None:
            die(f"[{name}] no serial socket")
        threading.Thread(target=self.reader, daemon=True).start()
        VMS.append(self)

    def reader(self):
        partial = ""
        while True:
            try:
                d = self.s.recv(65536)
            except OSError:
                break
            if not d:
                break
            t = d.decode(errors="replace")
            self.log.write(t)
            self.log.flush()
            t = ANSI.sub("", t)
            with self.lock:
                self.buf.append(t)
            partial += t
            *lines, partial = re.split(r"\r?\n", partial)
            for ln in lines:
                ln = ln.split("\r")[-1].strip()
                if ln and SHOW.search(ln):
                    print(f"{stamp()} [{self.name}] {ln[:200]}", flush=True)

    def text(self):
        with self.lock:
            return "".join(self.buf)

    def expect(self, what, pat, timeout, fail=None):
        print(f"{stamp()} [{self.name}] -- waiting: {what}", flush=True)
        end = time.time() + timeout
        while time.time() < end:
            t = self.text()
            m = re.search(pat, t[self.cursor:])
            if m:
                self.cursor += m.end()
                return m
            if fail:
                f = re.search(fail, t[self.cursor:])
                if f:
                    time.sleep(3)
                    die(f"[{self.name}] {what}: failure marker {f.group(0)!r}", self)
            if self.proc.poll() is not None:
                die(f"[{self.name}] {what}: QEMU exited ({self.proc.returncode})", self)
            time.sleep(0.5)
        die(f"[{self.name}] {what}: timeout after {timeout}s waiting for {pat!r}", self)

    def send(self, line):
        print(f"{stamp()} [{self.name}] >> {line!r}", flush=True)
        self.s.sendall((line + "\r").encode())

    def mon(self, c):
        try:
            m = socket.socket(socket.AF_UNIX)
            m.connect(self.mon_path)
            m.sendall((c + "\n").encode())
            time.sleep(0.5)
            out = m.recv(65536).decode(errors="replace")
            m.close()
            return out
        except OSError:
            return ""

    def stop(self, timeout=180):
        try:
            self.proc.wait(timeout)
        except subprocess.TimeoutExpired:
            print(f"{stamp()} [{self.name}] did not power off; quitting QEMU", flush=True)
            self.mon("quit")
            self.proc.wait(30)


def die(msg, vm=None):
    print(f"{stamp()} FAILED: {msg}", flush=True)
    for v in [vm] if vm else VMS:
        tail = v.text()[-6000:]
        print(f"---- last serial output of {v.name} ----\n"
              + "\n".join(l.split("\r")[-1] for l in tail.splitlines()[-50:]))
    for v in VMS:
        v.mon(f"screendump {LE}/{v.name}.ppm")
        v.mon("quit")
    time.sleep(2)
    for v in VMS:
        v.proc.kill()
    sys.exit(1)


