#!/usr/bin/env python3
"""Exercise the HTTPS web console with a client PKCS#12.

  webconsole-test.py <base-url> <client.p12> "<line>" [seconds]

Uses empty PKCS#12 password. Skips TLS hostname verify (agent CA / LAN IP).
"""
import json
import os
import re
import ssl
import sys
import time
import urllib.request

try:
    import websocket
except ImportError:
    websocket = None

base, p12_path, line = sys.argv[1], sys.argv[2], sys.argv[3]
secs = float(sys.argv[4]) if len(sys.argv) > 4 else 20

ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
ctx.check_hostname = False
ctx.verify_mode = ssl.CERT_NONE
ctx.load_cert_chain(certfile=None)  # placeholder; load via PKCS12 below

# Prefer cryptography for PKCS#12 → PEM temp files for ssl
from cryptography.hazmat.primitives.serialization import Encoding, PrivateFormat, NoEncryption, pkcs12

with open(p12_path, "rb") as f:
    key, cert, _ = pkcs12.load_key_and_certificates(f.read(), b"")
import tempfile

td = tempfile.mkdtemp(prefix="bia-p12-")
key_path = os.path.join(td, "key.pem")
cert_path = os.path.join(td, "cert.pem")
with open(key_path, "wb") as f:
    f.write(key.private_bytes(Encoding.PEM, PrivateFormat.PKCS8, NoEncryption()))
with open(cert_path, "wb") as f:
    f.write(cert.public_bytes(Encoding.PEM))

ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
ctx.check_hostname = False
ctx.verify_mode = ssl.CERT_NONE
ctx.load_cert_chain(certfile=cert_path, keyfile=key_path)

r = urllib.request.urlopen(urllib.request.Request(base.rstrip("/") + "/v1/whoami"), context=ctx)
print("whoami:", r.status, r.read().decode())

if websocket is None:
    print("install websocket-client to exercise /ws; whoami OK")
    sys.exit(0)

ws_url = base.rstrip("/").replace("https://", "wss://").replace("http://", "ws://") + "/ws"
ws = websocket.create_connection(ws_url, sslopt={"cert_reqs": ssl.CERT_NONE, "keyfile": key_path, "certfile": cert_path})
ws.send(json.dumps({"t": "resize", "cols": 100, "rows": 30}))
time.sleep(0.3)
ws.send(json.dumps({"t": "line", "v": line}))
end = time.time() + secs
buf = ""
ws.settimeout(0.5)
while time.time() < end:
    try:
        buf += ws.recv()
    except Exception:
        continue
ws.close()
txt = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[78]", "", buf)
print(txt[-4000:])
