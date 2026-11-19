"""Fixtures for web-keys-test.js: an EC client certificate of a throwaway CA packed the ways
people bring one to the web app: bia's own .p12 (PBES2/AES-256, SHA-256 MAC, password
12345678), bia cloud's download (3DES, its own password), and a .biaconfig around bia's file.

  python3 -I tests/integration/web-keys-fixtures.py <dir>
"""
import base64, datetime, json, os, sys
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.pbkdf2 import PBKDF2HMAC
from cryptography.hazmat.primitives.serialization import pkcs12
from cryptography.x509.oid import NameOID

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
now = datetime.datetime.now(datetime.timezone.utc)
ca_key = ec.generate_private_key(ec.SECP256R1())
ca_name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "bia-test agent CA")])
ca = (x509.CertificateBuilder().subject_name(ca_name).issuer_name(ca_name).public_key(ca_key.public_key())
      .serial_number(1).not_valid_before(now).not_valid_after(now + datetime.timedelta(days=30))
      .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True).sign(ca_key, hashes.SHA256()))
key = ec.generate_private_key(ec.SECP256R1())
leaf = (x509.CertificateBuilder().subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Identity: root")]))
        .issuer_name(ca_name).public_key(key.public_key()).serial_number(2)
        .not_valid_before(now).not_valid_after(now + datetime.timedelta(days=30)).sign(ca_key, hashes.SHA256()))

def p12(password, enc):
    return pkcs12.serialize_key_and_certificates(b"bia root", key, leaf, [ca], enc)

modern = (serialization.PrivateFormat.PKCS12.encryption_builder().kdf_rounds(2048)
          .key_cert_algorithm(pkcs12.PBES.PBESv2SHA256AndAES256CBC).hmac_hash(hashes.SHA256()).build(b"12345678"))
legacy = (serialization.PrivateFormat.PKCS12.encryption_builder()
          .key_cert_algorithm(pkcs12.PBES.PBESv1SHA1And3KeyTripleDESCBC).hmac_hash(hashes.SHA1()).build(b"cloud-pass-1234"))
custom = serialization.BestAvailableEncryption(b"my own password")
files = {"bia.p12": p12(b"12345678", modern), "cloud.p12": p12(b"cloud-pass-1234", legacy), "custom.p12": p12(b"my own password", custom)}

# .biaconfig (bia_pki::biaconfig v1) around bia's own .p12
pw = "abcd-efgh-jkmn-pqrs"
salt, nonce = os.urandom(16), os.urandom(12)
k = PBKDF2HMAC(algorithm=hashes.SHA256(), length=32, salt=salt, iterations=600000).derive(pw.encode())
plain = json.dumps({"v": 1, "kind": "bia", "bia": {"name": "test", "addresses": ["127.0.0.1:8443"],
                    "ca": ca.public_bytes(serialization.Encoding.PEM).decode(), "p12": base64.b64encode(files["bia.p12"]).decode()}}).encode()
files["test.biaconfig"] = json.dumps({"v": 1, "fmt": "biaconfig", "alg": "aes-256-gcm", "kdf": "pbkdf2-sha256", "iter": 600000,
    "salt": base64.b64encode(salt).decode(), "nonce": base64.b64encode(nonce).decode(),
    "data": base64.b64encode(AESGCM(k).encrypt(nonce, plain, b"biaconfig")).decode()}).encode()
for name, data in files.items():
    open(os.path.join(out, name), "wb").write(data)
open(os.path.join(out, "leaf.der"), "wb").write(leaf.public_bytes(serialization.Encoding.DER))
open(os.path.join(out, "ca.der"), "wb").write(ca.public_bytes(serialization.Encoding.DER))
json.dump({"biaconfig_password": pw, "cloud_password": "cloud-pass-1234", "custom_password": "my own password"}, open(os.path.join(out, "passwords.json"), "w"))
print("fixtures in", out)
