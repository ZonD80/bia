"""bia's chat with made-up messages, for the apps' store screenshots (appstore-assets.sh, play-assets.sh).

  python3 installers/demo-chat/server.py <out dir> [port]

Serves bia's real web app (source/crates/bia-mcp-web/web) over HTTPS on localhost, with demo.js after
link.js: a fake bus, a conversation, and ?demo=chat | call | attach for the scene. No bia behind it.
Makes a throwaway CA, a server certificate for localhost and root's key, and writes <out>/ca.crt and
<out>/bia-root.p12 (password 12345678) as the apps keep them per bia, in servers/localhost:<port>/.
Prints READY once it listens.
"""
import http.server, json, os, ssl, subprocess, sys, tempfile

out = sys.argv[1]
port = int(sys.argv[2]) if len(sys.argv) > 2 else 18443
here = os.path.dirname(os.path.abspath(__file__))
web = os.path.join(here, "..", "..", "source", "crates", "bia-mcp-web", "web")
os.makedirs(out, exist_ok=True)
tmp = tempfile.mkdtemp(prefix="bia-demo-chat-")


def sh(*args):
    subprocess.run(args, check=True, cwd=tmp, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def ext(name, text):
    with open(os.path.join(tmp, name), "w") as f:
        f.write(text)


# the CA, the server (iOS refuses server certificates valid over 825 days) and root
sh("openssl", "ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", "ca.key")
ext("ca.ext", "basicConstraints=critical,CA:true\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\n")
sh("openssl", "req", "-new", "-key", "ca.key", "-subj", "/CN=bia agent CA", "-out", "ca.csr")
sh("openssl", "x509", "-req", "-in", "ca.csr", "-signkey", "ca.key", "-days", "3650", "-extfile", "ca.ext", "-out", "ca.crt")
for who, subj, extra in [
    ("server", "/CN=localhost", "subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n"),
    ("root", "/CN=root", "extendedKeyUsage=clientAuth\n"),
]:
    sh("openssl", "ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", f"{who}.key")
    ext(f"{who}.ext", "basicConstraints=CA:false\nkeyUsage=critical,digitalSignature,keyAgreement\n" + extra)
    sh("openssl", "req", "-new", "-key", f"{who}.key", "-subj", subj, "-out", f"{who}.csr")
    sh("openssl", "x509", "-req", "-in", f"{who}.csr", "-CA", "ca.crt", "-CAkey", "ca.key", "-CAcreateserial",
       "-days", "800", "-extfile", f"{who}.ext", "-out", f"{who}.crt")
# 3DES and SHA-1: what iOS and Android both import
sh("openssl", "pkcs12", "-export", "-inkey", "root.key", "-in", "root.crt", "-certfile", "ca.crt", "-name", "root",
   "-certpbe", "PBE-SHA1-3DES", "-keypbe", "PBE-SHA1-3DES", "-macalg", "sha1", "-passout", "pass:12345678",
   "-out", "bia-root.p12")
for f in ("ca.crt", "bia-root.p12"):
    with open(os.path.join(tmp, f), "rb") as a, open(os.path.join(out, f), "wb") as b:
        b.write(a.read())

API = {
    "/v1/whoami": {"identity": "root", "root": True, "files": "/home/bia/files"},
    "/v1/status": {"state": "idle", "model": "deepseek/deepseek-v4.1-flash"},
    "/v1/hitl": {"tasks": []},
    "/v1/qr": {"qr": []},
}


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=web, **k)

    def log_message(self, *a):
        pass

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def reply(self, body, ctype):
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?")[0]
        if path in ("/", "/index.html"):
            with open(os.path.join(web, "index.html")) as f:
                page = f.read().replace('<script src="/link.js"></script>', '<script src="/link.js"></script>\n<script src="/demo.js"></script>')
            return self.reply(page.encode(), "text/html; charset=utf-8")
        if path == "/demo.js":
            with open(os.path.join(here, "demo.js"), "rb") as f:
                return self.reply(f.read(), "text/javascript")
        if path == "/sw.js":
            self.send_error(404)
            return
        if path.startswith("/v1/"):
            return self.reply(json.dumps(API.get(path, {})).encode(), "application/json")
        super().do_GET()

    def do_POST(self):
        self.reply(b"{}", "application/json")


ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(os.path.join(tmp, "server.crt"), os.path.join(tmp, "server.key"))
# no client certificate is asked for: a browser would stop to pick one, and nothing here is private
srv = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
print("READY", flush=True)
srv.serve_forever()
