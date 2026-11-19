// keys.js in Node (WebCrypto + the vendored forge): every way a person brings a key opens,
// the key comes out usable and matches its certificate, wrong passwords say so.
//   python3 -I tests/integration/web-keys-fixtures.py /tmp/wk && node tests/integration/web-keys-test.js /tmp/wk
'use strict';
const fs = require('fs'), path = require('path');
const web = path.join(__dirname, '../../source/crates/bia-mcp-web/web');
globalThis.window = globalThis; // the browser bundle names window
globalThis.forge = require(path.join(web, 'vendor/forge.min.js'));
globalThis.BiaPki = require(path.join(web, 'pki.js'));
const K = require(path.join(web, 'keys.js'));
const dir = process.argv[2];
const pw = JSON.parse(fs.readFileSync(path.join(dir, 'passwords.json')));
const read = f => new Uint8Array(fs.readFileSync(path.join(dir, f)));
let failed = false;
const check = (ok, what) => { console.log((ok ? 'ok   ' : 'FAIL ') + what); if (!ok) failed = true; };
const same = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);

async function usable(pkcs8, leafDer) {
  // the key signs, and the certificate's public key verifies it
  const key = await crypto.subtle.importKey('pkcs8', pkcs8, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign']);
  const msg = new TextEncoder().encode('bia-auth-v1\nnonce');
  const sig = await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, key, msg);
  const cert = new (require('crypto').X509Certificate)(Buffer.from(leafDer));
  const pub = await crypto.subtle.importKey('spki', cert.publicKey.export({ type: 'spki', format: 'der' }), { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);
  return crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, pub, sig, msg);
}

(async () => {
  const leaf = read('leaf.der'), ca = read('ca.der');
  for (const [file, password, what] of [['bia.p12', K.P12_DEFAULT, "bia's own .p12 (PBES2/AES, SHA-256 MAC)"], ['cloud.p12', pw.cloud_password, "bia cloud's .p12 (3DES)"], ['custom.p12', pw.custom_password, 'a .p12 with its own password']]) {
    const p = K.readP12(read(file), password);
    check(same(p.chain[0], leaf), `${what}: the key's certificate is found`);
    check(p.extra.some(c => same(c, ca)), `${what}: the CA comes with it`);
    check(await usable(p.pkcs8, leaf), `${what}: the key signs and matches the certificate`);
  }
  let e = null;
  try { K.readP12(read('cloud.p12'), K.P12_DEFAULT); } catch (x) { e = x; }
  check(e && e.kind === 'password', `a .p12 with another password says so: ${e && e.message}`);
  check(K.isBiaconfig(read('test.biaconfig')) && !K.isBiaconfig(read('bia.p12')), 'a .biaconfig is told from a .p12');
  e = null;
  try { await K.openBiaconfig(read('test.biaconfig'), 'wrong password!'); } catch (x) { e = x; }
  check(e && e.kind === 'password', 'a .biaconfig with a wrong password says so');
  const c = await K.openBiaconfig(read('test.biaconfig'), pw.biaconfig_password);
  check(c.addresses[0] === '127.0.0.1:8443' && c.ca.includes('BEGIN CERTIFICATE'), 'a .biaconfig opens');
  const p = K.readP12(c.p12, K.P12_DEFAULT);
  check(same(p.chain[0], leaf) && await usable(p.pkcs8, leaf), 'the .p12 inside opens with 12345678');
  console.log(failed ? 'FAILED' : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch(e => { console.error(e); process.exit(1); });
