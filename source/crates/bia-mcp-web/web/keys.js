// The key this browser signs in with (ARCHITECTURE.md, "Sign-in" and "Keys in the browser").
// A non-extractable WebCrypto key and its certificate chain live in IndexedDB of this origin;
// page scripts can use the key but nobody can read it out. A browser gets one by importing a
// .biaconfig or .p12 (opened here, never sent anywhere) and enrolling a key of its own with
// it: the imported key is only used once, to ask bia for this browser's certificate.
'use strict';
const BiaKeys = (() => {
  const subtle = (globalThis.crypto || {}).subtle;
  const enc = new TextEncoder();
  const DB = 'bia-keys', STORE = 'keys', ME = 'me';
  /** What bia's own .p12 files are locked with; tried before asking. */
  const P12_DEFAULT = '12345678';

  // ---- bytes ----
  const b64u = bytes => {
    let s = '';
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  };
  const fromB64 = s => Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/').replace(/\s+/g, '')), c => c.charCodeAt(0));
  const hex = buf => [...new Uint8Array(buf)].map(b => b.toString(16).padStart(2, '0')).join('');
  const sha256hex = async der => hex(await subtle.digest('SHA-256', der));

  // ---- IndexedDB ----
  const db = () => new Promise((ok, no) => {
    const r = indexedDB.open(DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore(STORE);
    r.onsuccess = () => ok(r.result);
    r.onerror = () => no(r.error);
  });
  const tx = async (mode, f) => {
    const d = await db();
    return new Promise((ok, no) => {
      const t = d.transaction(STORE, mode);
      const req = f(t.objectStore(STORE));
      t.oncomplete = () => ok(req && req.result);
      t.onerror = () => no(t.error);
    });
  };
  /** This browser's key: {key: CryptoKey, chain: [base64url DER], identity, serial, not_after} or null. */
  const current = () => tx('readonly', s => s.get(ME)).catch(() => null);
  const save = rec => tx('readwrite', s => s.put(rec, ME));
  const forget = () => tx('readwrite', s => s.delete(ME));

  // ---- signing ----
  const algOf = key => key.algorithm.name === 'ECDSA'
    ? { name: 'ECDSA', hash: key.algorithm.namedCurve === 'P-384' ? 'SHA-384' : 'SHA-256' }
    : { name: key.algorithm.name };
  const sign = async (key, msg) => b64u(new Uint8Array(await subtle.sign(algOf(key), key, enc.encode(msg))));

  /** The `auth` answer to a challenge (no channel binding: a page cannot read it). */
  async function auth(rec, nonce, caSha) {
    const ts = Math.floor(Date.now() / 1000);
    const msg = `bia-auth-v1\n${nonce}\n${caSha}\n${ts}\n`;
    const sig = await sign(rec.key, msg);
    return { t: 'auth', chain: rec.chain, ts, sig, nonce };
  }

  /** The server's challenge for an HTTP sign-in: {nonce, ca_sha256}. */
  async function challenge() {
    const r = await fetch('/v1/auth/challenge', { cache: 'no-store' });
    if (!r.ok) throw new Error('HTTP ' + r.status);
    return r.json();
  }

  /** Sign in for the session cookie with `rec` (default: this browser's key). Returns who. */
  async function signin(rec) {
    rec = rec || await current();
    if (!rec) throw Object.assign(new Error('no key in this browser'), { kind: 'nokey' });
    const ch = await challenge();
    const body = JSON.stringify(await auth(rec, ch.nonce, ch.ca_sha256));
    const r = await fetch('/v1/auth/signin', { method: 'POST', body, headers: { 'Content-Type': 'application/json' }, credentials: 'same-origin' });
    if (!r.ok) {
      const text = await r.text();
      const kind = (text.match(/\((\w+)\)/) || [])[1] || 'refused';
      throw Object.assign(new Error(text || 'HTTP ' + r.status), { kind, status: r.status });
    }
    return r.json();
  }

  // ---- importing ----
  /** Is this a .biaconfig (JSON envelope) rather than a .p12? */
  function isBiaconfig(bytes) {
    try { return JSON.parse(new TextDecoder().decode(bytes)).fmt === 'biaconfig'; } catch { return false; }
  }

  /** Open a .biaconfig: {name, addresses, ca, p12 (bytes)}. Throws 'password' on a wrong one. */
  async function openBiaconfig(bytes, password) {
    const v = JSON.parse(new TextDecoder().decode(bytes));
    if (v.fmt !== 'biaconfig') throw new Error('this is not a .biaconfig file');
    if (v.v !== 1 || v.alg !== 'aes-256-gcm' || v.kdf !== 'pbkdf2-sha256') throw new Error('this .biaconfig was made by a newer bia');
    if (!(v.iter >= 100000 && v.iter <= 10000000)) throw new Error('this .biaconfig is damaged');
    const base = await subtle.importKey('raw', enc.encode(password), 'PBKDF2', false, ['deriveKey']);
    const key = await subtle.deriveKey({ name: 'PBKDF2', hash: 'SHA-256', salt: fromB64(v.salt), iterations: v.iter },
      base, { name: 'AES-GCM', length: 256 }, false, ['decrypt']);
    let plain;
    try {
      plain = await subtle.decrypt({ name: 'AES-GCM', iv: fromB64(v.nonce), additionalData: enc.encode('biaconfig') }, key, fromB64(v.data));
    } catch { throw Object.assign(new Error('wrong password'), { kind: 'password' }); }
    const inner = JSON.parse(new TextDecoder().decode(plain));
    const c = inner.bia || inner;
    return { name: c.name || '', addresses: c.addresses || [], ca: c.ca || '', p12: fromB64(c.p12) };
  }

  /** Read a .p12 with the vendored PKCS#12 reader: {pkcs8, chain: [DER], extra: [DER]}.
   *  `chain[0]` is the certificate that matches the key. Throws 'password' on a wrong one. */
  function readP12(bytes, password) {
    if (!globalThis.forge) throw new Error('the PKCS#12 reader did not load');
    const f = globalThis.forge;
    let p;
    try {
      let bin = '';
      for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      p = f.pkcs12.pkcs12FromAsn1(f.asn1.fromDer(bin), false, password);
    } catch (e) {
      if (/mac|password|decrypt|Invalid/i.test(String(e && e.message))) throw Object.assign(new Error('wrong password'), { kind: 'password' });
      throw new Error('this is not a .p12 file');
    }
    const bags = (type) => p.getBags({ bagType: type })[type] || [];
    const derOf = asn => Uint8Array.from(f.asn1.toDer(asn).getBytes(), c => c.charCodeAt(0));
    // forge parses RSA keys only; for EC keys (what bia issues) it leaves the decrypted
    // PrivateKeyInfo in `asn1`, which is PKCS#8 already
    const keyBag = [...bags(f.pki.oids.pkcs8ShroudedKeyBag), ...bags(f.pki.oids.keyBag)].find(b => b.key || b.asn1);
    if (!keyBag) throw new Error('this .p12 has no private key');
    const pkcs8 = keyBag.key ? derOf(f.pki.wrapRsaPrivateKey(f.pki.privateKeyToAsn1(keyBag.key))) : derOf(keyBag.asn1);
    const certs = bags(f.pki.oids.certBag).filter(b => b.cert || b.asn1);
    if (!certs.length) throw new Error('this .p12 has no certificate');
    const certDer = b => b.cert ? derOf(f.pki.certificateToAsn1(b.cert)) : derOf(b.asn1);
    const idOf = b => b.attributes && b.attributes.localKeyId && b.attributes.localKeyId[0];
    // the key's certificate shares its localKeyId; else the first one (bia writes it first)
    let own = idOf(keyBag) ? certs.findIndex(b => idOf(b) === idOf(keyBag)) : -1;
    if (own < 0) own = 0;
    const ders = certs.map(certDer);
    return { pkcs8, chain: [ders[own]], extra: ders.filter((_, i) => i !== own) };
  }

  /** WebCrypto key from PKCS#8: EC (P-256, P-384) or RSA. Non-extractable. */
  async function importPkcs8(pkcs8) {
    for (const [alg, curve] of [['ECDSA', 'P-256'], ['ECDSA', 'P-384'], ['RSASSA-PKCS1-v1_5', null]]) {
      try {
        const params = alg === 'ECDSA' ? { name: alg, namedCurve: curve } : { name: alg, hash: 'SHA-256' };
        return await subtle.importKey('pkcs8', pkcs8, params, false, ['sign']);
      } catch {}
    }
    throw new Error('this key type is not supported');
  }

  /**
   * Turn a dropped file into this browser's own key: open it (a .biaconfig with `password`;
   * a .p12 with bia's default first, then `password`), check it belongs to this bia, sign in
   * with it once, enroll a new non-extractable key for the same identity and keep that.
   * `askPassword()` is called when a password is needed; return null to give up.
   */
  async function importFile(bytes, askPassword) {
    let p12 = bytes, caPem = '', password = null;
    if (isBiaconfig(bytes)) {
      for (let wrong = false; ; wrong = true) {
        password = await askPassword('.biaconfig', wrong);
        if (password == null) return null;
        try { const c = await openBiaconfig(bytes, password); p12 = c.p12; caPem = c.ca; break; }
        catch (e) { if (e.kind !== 'password') throw e; }
      }
    }
    // bia's own .p12 files (and the one inside a .biaconfig) use 12345678: tried first, then
    // the .biaconfig's password, then the person is asked
    const tries = [P12_DEFAULT, ...(password != null && password !== P12_DEFAULT ? [password] : [])];
    let parsed = null;
    for (let wrong = false; ; ) {
      const pw = tries.length ? tries.shift() : await askPassword('.p12', wrong);
      if (pw == null) return null;
      try { parsed = readP12(p12, pw); break; }
      catch (e) { if (e.kind !== 'password') throw e; if (!tries.length) wrong = true; }
    }
    // a file for another bia: its CA is not this one
    const ch = await challenge();
    const cas = [...parsed.extra];
    if (caPem) cas.push(...BiaPki.pemCerts(caPem));
    if (cas.length) {
      const shas = await Promise.all(cas.map(sha256hex));
      if (!shas.includes(ch.ca_sha256)) throw Object.assign(new Error('this file is for another bia'), { kind: 'other_ca' });
    }
    const imported = { key: await importPkcs8(parsed.pkcs8), chain: parsed.chain.map(b64u) };
    const who = await signin(imported);
    return enroll(who);
  }

  /** With a session cookie for `who`: a key of this browser's own, its certificate, kept. */
  async function enroll(who) {
    const { csr, key } = await BiaPki.makeCsr(who.identity, { extractable: false });
    const r = await fetch('/v1/pki/enroll', { method: 'POST', body: JSON.stringify({ csr }), headers: { 'Content-Type': 'application/json' }, credentials: 'same-origin' });
    if (!r.ok) throw new Error('enroll: ' + (await r.text() || 'HTTP ' + r.status));
    const chain = BiaPki.pemCerts(await r.text()).map(b64u);
    const rec = { key: key.privateKey, chain, identity: who.identity, serial: r.headers.get('X-Bia-Serial') || '', not_after: Number(r.headers.get('X-Bia-Not-After') || 0) };
    await save(rec);
    if (navigator.storage && navigator.storage.persist) navigator.storage.persist().catch(() => {});
    return signin(rec);
  }

  /** A `#biaconfig=<base64url>` or `#p12=<base64url>` link (bia cloud's first access, the
   *  bia deploy apps): the file's bytes. That part of the hash is gone from the address bar and
   *  history at once; the rest of it (a page of the app) stays. */
  function fromHash() {
    const v = takeHash(/^(?:biaconfig|p12)=([A-Za-z0-9_-]+)$/);
    if (!v) return null;
    try { return fromB64(v); } catch { return null; }
  }

  /** `#enroll=<token>` of a link bia sent: a one-time key grant for whoever it was sent to. */
  const enrollToken = () => takeHash(/^enroll=([0-9a-f]{32})$/);

  // the part of the hash matching `re` (its first group), removed from the address and history
  function takeHash(re) {
    let found = null;
    const rest = location.hash.replace(/^#/, '').split('&').filter(p => {
      const m = !found && p.match(re);
      if (m) { found = m[1]; return false; }
      return p !== '';
    });
    if (found) history.replaceState(null, '', location.pathname + location.search + (rest.length ? '#' + rest.join('&') : ''));
    return found;
  }

  /** Sign up with a link's grant: a key of this browser's own, certified for the person the
   *  link was sent to. Returns who. */
  async function enrollWithToken(token) {
    const { csr, key } = await BiaPki.makeCsr('bia', { extractable: false });
    const r = await fetch('/v1/pki/link/' + token, { method: 'POST', body: JSON.stringify({ csr }), headers: { 'Content-Type': 'application/json' } });
    if (!r.ok) throw Object.assign(new Error(await r.text() || 'HTTP ' + r.status), { kind: r.status === 404 ? 'link' : 'refused' });
    const chain = BiaPki.pemCerts(await r.text()).map(b64u);
    const rec = { key: key.privateKey, chain, identity: r.headers.get('X-Bia-Identity') || '', serial: r.headers.get('X-Bia-Serial') || '', not_after: Number(r.headers.get('X-Bia-Not-After') || 0) };
    await save(rec);
    if (navigator.storage && navigator.storage.persist) navigator.storage.persist().catch(() => {});
    return signin(rec);
  }

  return { current, save, forget, signin, auth, challenge, importFile, enroll, enrollToken, enrollWithToken, fromHash, isBiaconfig, openBiaconfig, readP12, b64u, P12_DEFAULT };
})();
if (typeof module !== 'undefined') module.exports = BiaKeys;
