// Client-side certificate keys: the browser makes its key pair, sends only a CSR, and
// packs the returned certificate with its key into a .p12 for the OS / browser store.
// The private key never leaves this page. WebCrypto only (works offline).
'use strict';
const BiaPki = (() => {
  const subtle = (globalThis.crypto || {}).subtle;
  const enc = new TextEncoder();

  // ---- DER ----
  const cat = (...parts) => {
    const n = parts.reduce((a, p) => a + p.length, 0);
    const out = new Uint8Array(n);
    let o = 0;
    for (const p of parts) { out.set(p, o); o += p.length; }
    return out;
  };
  const len = n => {
    if (n < 0x80) return Uint8Array.of(n);
    const b = [];
    while (n > 0) { b.unshift(n & 0xff); n = Math.floor(n / 256); }
    return Uint8Array.of(0x80 | b.length, ...b);
  };
  const tlv = (tag, body) => cat(Uint8Array.of(tag), len(body.length), body);
  const seq = (...xs) => tlv(0x30, cat(...xs));
  const set = (...xs) => tlv(0x31, cat(...xs));
  const octets = b => tlv(0x04, b);
  const bits = b => tlv(0x03, cat(Uint8Array.of(0), b));
  const nul = () => Uint8Array.of(0x05, 0x00);
  const ctx = (n, body) => tlv(0xa0 + n, body); // [n] EXPLICIT / constructed
  const int = v => {
    let b;
    if (typeof v === 'number') {
      const a = [];
      do { a.unshift(v & 0xff); v = Math.floor(v / 256); } while (v > 0);
      b = Uint8Array.from(a);
    } else {
      let i = 0;
      while (i < v.length - 1 && v[i] === 0) i++;
      b = v.slice(i);
    }
    if (b[0] & 0x80) b = cat(Uint8Array.of(0), b);
    return tlv(0x02, b);
  };
  const oid = s => {
    const p = s.split('.').map(Number);
    const out = [40 * p[0] + p[1]];
    for (const x of p.slice(2)) {
      const b = [x & 0x7f];
      for (let v = Math.floor(x / 128); v > 0; v = Math.floor(v / 128)) b.unshift(0x80 | (v & 0x7f));
      out.push(...b);
    }
    return tlv(0x06, Uint8Array.from(out));
  };
  const utf8 = s => tlv(0x0c, enc.encode(s));
  const bmp = s => {
    const b = new Uint8Array(s.length * 2);
    for (let i = 0; i < s.length; i++) { b[2 * i] = s.charCodeAt(i) >> 8; b[2 * i + 1] = s.charCodeAt(i) & 0xff; }
    return tlv(0x1e, b);
  };
  const OID = {
    cn: '2.5.4.3', ecPublicKey: '1.2.840.10045.2.1', ecdsaSha256: '1.2.840.10045.4.3.2',
    data: '1.2.840.113549.1.7.1', shroudedKeyBag: '1.2.840.113549.1.12.10.1.2', certBag: '1.2.840.113549.1.12.10.1.3',
    x509Cert: '1.2.840.113549.1.9.22.1', friendlyName: '1.2.840.113549.1.9.20', localKeyId: '1.2.840.113549.1.9.21',
    sha256: '2.16.840.1.101.3.4.2.1', pbes2: '1.2.840.113549.1.5.13', pbkdf2: '1.2.840.113549.1.5.12',
    hmacSha256: '1.2.840.113549.2.9', aes256cbc: '2.16.840.1.101.3.4.1.42',
  };
  /** Password of every .p12 bia hands out (macOS will not import one with an empty password). */
  const PASSWORD = '12345678';

  // ECDSA signature from WebCrypto (r || s) to DER Ecdsa-Sig-Value.
  const ecdsaDer = raw => {
    const h = raw.length / 2;
    return seq(int(raw.slice(0, h)), int(raw.slice(h)));
  };

  const pem = (label, der) => {
    let b64 = '';
    for (let i = 0; i < der.length; i += 0x8000) b64 += String.fromCharCode(...der.subarray(i, i + 0x8000));
    b64 = btoa(b64).replace(/(.{64})/g, '$1\n');
    return `-----BEGIN ${label}-----\n${b64.trim()}\n-----END ${label}-----\n`;
  };
  const pemCerts = text => [...text.matchAll(/-----BEGIN CERTIFICATE-----([\s\S]*?)-----END CERTIFICATE-----/g)]
    .map(m => Uint8Array.from(atob(m[1].replace(/\s+/g, '')), c => c.charCodeAt(0)));

  /** New P-256 key pair and a PKCS#10 CSR for it. The server ignores the subject. */
  async function makeCsr(cn) {
    const key = await subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
    const spki = new Uint8Array(await subtle.exportKey('spki', key.publicKey));
    const name = seq(set(seq(oid(OID.cn), utf8(cn || 'bia'))));
    const info = seq(int(0), name, spki, Uint8Array.of(0xa0, 0x00));
    const sig = new Uint8Array(await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, key.privateKey, info));
    const csr = seq(info, seq(oid(OID.ecdsaSha256)), bits(ecdsaDer(sig)));
    return { csr: pem('CERTIFICATE REQUEST', csr), key };
  }

  // RFC 7292 appendix B.2 key derivation with SHA-256 (MAC key: id 3).
  async function p12Kdf(password, salt, id, iterations, n) {
    const u = 32, v = 64;
    const pw = new Uint8Array((password.length + 1) * 2);
    for (let i = 0; i < password.length; i++) { pw[2 * i] = password.charCodeAt(i) >> 8; pw[2 * i + 1] = password.charCodeAt(i) & 0xff; }
    const fill = (src, size) => { const o = new Uint8Array(size); for (let i = 0; i < size; i++) o[i] = src[i % src.length]; return o; };
    const D = new Uint8Array(v).fill(id);
    const S = salt.length ? fill(salt, v * Math.ceil(salt.length / v)) : new Uint8Array(0);
    const P = fill(pw, v * Math.ceil(pw.length / v));
    let I = cat(S, P);
    const out = new Uint8Array(n);
    for (let done = 0; done < n;) {
      let A = new Uint8Array(await subtle.digest('SHA-256', cat(D, I)));
      for (let r = 1; r < iterations; r++) A = new Uint8Array(await subtle.digest('SHA-256', A));
      out.set(A.subarray(0, Math.min(u, n - done)), done);
      done += u;
      if (done >= n) break;
      const B = fill(A, v);
      for (let j = 0; j < I.length; j += v) {
        let c = 1;
        for (let k = v - 1; k >= 0; k--) { c += I[j + k] + B[k]; I[j + k] = c & 0xff; c >>= 8; }
      }
    }
    return out;
  }

  // PKCS#8 key encrypted with PBES2 (PBKDF2-HMAC-SHA256, AES-256-CBC), as OpenSSL 3 does.
  async function shroud(pkcs8, password, iterations) {
    const salt = crypto.getRandomValues(new Uint8Array(16));
    const iv = crypto.getRandomValues(new Uint8Array(16));
    const base = await subtle.importKey('raw', enc.encode(password), 'PBKDF2', false, ['deriveKey']);
    const aes = await subtle.deriveKey({ name: 'PBKDF2', salt, iterations, hash: 'SHA-256' }, base,
      { name: 'AES-CBC', length: 256 }, false, ['encrypt']);
    const ct = new Uint8Array(await subtle.encrypt({ name: 'AES-CBC', iv }, aes, pkcs8));
    const alg = seq(oid(OID.pbes2), seq(
      seq(oid(OID.pbkdf2), seq(octets(salt), int(iterations), seq(oid(OID.hmacSha256), nul()))),
      seq(oid(OID.aes256cbc), octets(iv))));
    return seq(alg, octets(ct));
  }

  /**
   * PKCS#12 like OpenSSL 3 writes by default: certificates, the key in a PBES2/AES-256
   * shrouded bag, SHA-256 HMAC. Password PASSWORD. Imports on macOS, Windows 10+, Linux
   * and Firefox.
   */
  async function p12(key, chainPem, friendly, password = PASSWORD) {
    const certs = pemCerts(chainPem);
    if (!certs.length) throw new Error('no certificate in the answer');
    const iterations = 2048;
    const pkcs8 = new Uint8Array(await subtle.exportKey('pkcs8', key.privateKey));
    const keyId = new Uint8Array(await subtle.digest('SHA-1', certs[0]));
    const attrs = set(seq(oid(OID.friendlyName), set(bmp(friendly))), seq(oid(OID.localKeyId), set(octets(keyId))));
    const certBag = (der, own) => seq(oid(OID.certBag), ctx(0, seq(oid(OID.x509Cert), ctx(0, octets(der)))), ...(own ? [attrs] : []));
    const certContents = seq(...certs.map((c, i) => certBag(c, i === 0)));
    const keyContents = seq(seq(oid(OID.shroudedKeyBag), ctx(0, await shroud(pkcs8, password, iterations)), attrs));
    const data = body => seq(oid(OID.data), ctx(0, octets(body)));
    const authSafe = seq(data(certContents), data(keyContents));
    const salt = crypto.getRandomValues(new Uint8Array(16));
    const macKey = await p12Kdf(password, salt, 3, iterations, 32);
    const hk = await subtle.importKey('raw', macKey, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
    const mac = new Uint8Array(await subtle.sign('HMAC', hk, authSafe));
    const macData = seq(seq(seq(oid(OID.sha256), nul()), octets(mac)), octets(salt), int(iterations));
    return seq(int(3), data(authSafe), macData);
  }

  return { makeCsr, p12, pem, pemCerts, PASSWORD };
})();
if (typeof module !== 'undefined') module.exports = BiaPki;
