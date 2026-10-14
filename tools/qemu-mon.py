#!/usr/bin/env python3
"""QEMU monitor helper (MODE=headless).
   qemu-mon.py type "text"        -> sendkey each character (+ Enter)
   qemu-mon.py shot out.png       -> screendump and convert PPM -> PNG
"""
import os, socket, struct, sys, time, zlib
MON = os.environ.get("MON", "/tmp/bia-monitor.sock")

KEYS = {' ': 'spc', '.': 'dot', ',': 'comma', '/': 'slash', '-': 'minus', '=': 'equal', "'": 'apostrophe',
        ';': 'semicolon', '?': 'shift-slash', '!': 'shift-1', '+': 'shift-equal', ':': 'shift-semicolon', '\n': 'ret'}

def mon():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect(MON); s.settimeout(2)
    try: s.recv(4096)
    except socket.timeout: pass
    return s

def cmd(s, c):
    s.sendall((c + "\n").encode()); time.sleep(0.05)
    try: return s.recv(65536).decode(errors="replace")
    except socket.timeout: return ""

def type_text(text):
    s = mon()
    for ch in text + "\n":
        if ch in KEYS: k = KEYS[ch]
        elif ch.isupper(): k = "shift-" + ch.lower()
        elif ch.isdigit() or ch.isalpha(): k = ch
        else: continue
        cmd(s, f"sendkey {k}"); time.sleep(0.03)

def ppm_to_png(ppm, png):
    data = open(ppm, "rb").read()
    parts = data.split(maxsplit=4)
    assert parts[0] == b"P6", "not a P6 ppm"
    w, h = int(parts[1]), int(parts[2]); pix = parts[4][:w*h*3]
    raw = b"".join(b"\x00" + pix[y*w*3:(y+1)*w*3] for y in range(h))
    def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    out = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    open(png, "wb").write(out); print(f"{png}: {w}x{h}")

if __name__ == "__main__":
    if sys.argv[1] == "type": type_text(sys.argv[2])
    elif sys.argv[1] == "shot":
        s = mon(); ppm = os.path.abspath(sys.argv[2]) + ".ppm"
        cmd(s, f"screendump {ppm}"); time.sleep(1); ppm_to_png(ppm, sys.argv[2]); os.remove(ppm)
