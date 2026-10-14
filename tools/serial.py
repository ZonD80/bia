#!/usr/bin/env python3
"""Talk to the VM's serial console over the unix socket from run-qemu.sh MODE=socket.
   serial.py wait "ready" [timeout]     -> print output until pattern appears
   serial.py send "text" [timeout] [pattern]   -> send a line, print output until pattern (default 'bia> ') or timeout
"""
import socket, sys, time, os
SOCK = os.environ.get("SOCK", "/tmp/bia-serial.sock")

def run(cmd, arg, timeout, pattern):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(SOCK)
    s.settimeout(0.5)
    if cmd == "send":
        s.sendall((arg + "\r").encode())
        pattern = pattern or "bia> "
    else:
        pattern = arg
    buf = b""
    end = time.time() + timeout
    while time.time() < end:
        try:
            d = s.recv(4096)
            if not d:
                break
            buf += d
            sys.stdout.write(d.decode(errors="replace")); sys.stdout.flush()
            if pattern and pattern.encode() in buf:
                # give trailing output a moment
                time.sleep(0.3)
                try:
                    while True:
                        d = s.recv(4096)
                        if not d: break
                        sys.stdout.write(d.decode(errors="replace"))
                except socket.timeout:
                    pass
                return 0
        except socket.timeout:
            continue
    print("\n[serial.py: timeout waiting for %r]" % pattern)
    return 1

if __name__ == "__main__":
    cmd, arg = sys.argv[1], sys.argv[2]
    timeout = float(sys.argv[3]) if len(sys.argv) > 3 else 120
    pattern = sys.argv[4] if len(sys.argv) > 4 else None
    sys.exit(run(cmd, arg, timeout, pattern))
