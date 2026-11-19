#!/usr/bin/env node
// Browser renewal path against a running bia (tests/e2e/peer-e2e.sh): the same web/pki.js
// the console uses makes the key and CSR, bia signs it, the .p12 is built here. Signing in is
// what the web app does (keys.js): challenge, signed auth, the session cookie.
//
//   node tests/e2e/peer-e2e-renew.js https://127.0.0.1:18443 <out dir> <root setup code>
//
// Steps: CA -> first root .p12 via /v1/pki/bootstrap/root with the setup code from bia's
// screen (password 12345678, read with openssl) -> sign in with it -> POST /v1/pki/renew {csr}
// -> the new key signs in, the old one is refused -> renewed .p12 written for import checks.
'use strict';
const crypto = require('crypto');
const fs = require('fs');
const https = require('https');
const path = require('path');
const { execFileSync } = require('child_process');
const BiaPki = require(path.join(__dirname, '../../source/crates/bia-mcp-web/web/pki.js'));

const [base, out, setupCode] = process.argv.slice(2);
if (!base || !out || !setupCode) { console.error('usage: peer-e2e-renew.js <https base> <out dir> <setup code>'); process.exit(2); }
fs.mkdirSync(out, { recursive: true });
const origin = new URL(base).origin;

// bia's own certificate on this address is self-signed: what counts is the signed sign-in
function req(method, url, { body, cookie } = {}) {
  return new Promise((resolve, reject) => {
    const u = new URL(url, base);
    const headers = body ? { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(body) } : {};
    if (cookie) Object.assign(headers, { Cookie: `bia_s=${cookie}`, Origin: origin });
    if (method === 'POST') headers.Origin = origin;
    const r = https.request(u, { method, rejectUnauthorized: false, headers }, res => {
      const chunks = [];
      res.on('data', c => chunks.push(c));
      res.on('end', () => resolve({ status: res.statusCode, body: Buffer.concat(chunks), headers: res.headers }));
    });
    r.on('error', reject);
    if (body) r.write(body);
    r.end();
  });
}

const b64u = b => Buffer.from(b).toString('base64url');

/** Sign in like the web app: returns [status, cookie]. */
async function signin(certPem, keyPem) {
  const ch = JSON.parse((await req('GET', '/v1/auth/challenge')).body);
  const ts = Math.floor(Date.now() / 1000);
  const msg = `bia-auth-v1\n${ch.nonce}\n${ch.ca_sha256}\n${ts}\n`;
  const sig = crypto.sign('sha256', Buffer.from(msg), { key: keyPem, dsaEncoding: 'ieee-p1363' });
  const r = await req('POST', '/v1/auth/signin', { body: JSON.stringify({ nonce: ch.nonce, chain: [b64u(BiaPki.pemCerts(certPem)[0])], ts, sig: b64u(sig) }) });
  const c = (r.headers['set-cookie'] || []).map(v => v.match(/^bia_s=([^;]+)/)).find(Boolean);
  return [r.status, c && c[1]];
}

const fail = msg => { console.log(`  FAIL ${msg}`); process.exitCode = 1; };
const ok = (cond, msg) => (cond ? console.log(`  ok   ${msg}`) : fail(msg));
const pemKey = pem => pem.match(/-----BEGIN PRIVATE KEY-----[\s\S]*?-----END PRIVATE KEY-----\n/)[0];

(async () => {
  const ca = (await req('GET', '/v1/pki/ca')).body.toString();
  ok(ca.includes('BEGIN CERTIFICATE'), 'CA downloaded');

  const wrong = await req('POST', '/v1/pki/bootstrap/root', { body: JSON.stringify({ code: 'nope-nope' }) });
  ok(wrong.status === 403, `root is not set up without the code from bia's screen (HTTP ${wrong.status})`);
  const boot = await req('POST', '/v1/pki/bootstrap/root', { body: JSON.stringify({ code: setupCode }) });
  ok(boot.status === 200, `first root .p12 issued with the setup code (HTTP ${boot.status})`);
  fs.writeFileSync(path.join(out, 'root-server.p12'), boot.body);
  const pemAll = execFileSync('openssl', ['pkcs12', '-in', path.join(out, 'root-server.p12'),
    '-passin', `pass:${BiaPki.PASSWORD}`, '-nodes'], { encoding: 'utf8' });
  const oldKey = pemKey(pemAll);
  const oldCert = BiaPki.pem('CERTIFICATE', BiaPki.pemCerts(pemAll)[0]);
  ok(!!oldKey && !!oldCert, `server .p12 opens with password ${BiaPki.PASSWORD} (OpenSSL 3, no -legacy)`);
  const [st0, cookie0] = await signin(oldCert, oldKey);
  const who0 = JSON.parse((await req('GET', '/v1/whoami', { cookie: cookie0 })).body);
  ok(st0 === 200 && who0.identity === 'root', `root signs in with it (serial ${who0.serial})`);

  const { csr, key } = await BiaPki.makeCsr('root');
  const ren = await req('POST', '/v1/pki/renew', { cookie: cookie0, body: JSON.stringify({ csr }) });
  const chain = ren.body.toString();
  ok(ren.status === 200 && !chain.includes('PRIVATE KEY') && BiaPki.pemCerts(chain).length === 2,
    `renew answers PEM cert + CA, no key (HTTP ${ren.status}, serial ${ren.headers['x-bia-serial']})`);

  const p12 = await BiaPki.p12(key, chain, 'root (bia)');
  fs.writeFileSync(path.join(out, 'root-renewed.p12'), Buffer.from(p12));
  const newPem = execFileSync('openssl', ['pkcs12', '-in', path.join(out, 'root-renewed.p12'),
    '-passin', `pass:${BiaPki.PASSWORD}`, '-nodes'], { encoding: 'utf8' });
  const newKey = pemKey(newPem);
  const newCert = BiaPki.pem('CERTIFICATE', BiaPki.pemCerts(newPem)[0]);
  ok(true, 'renewed .p12 built in the browser code opens with openssl');

  const [st1, cookie1] = await signin(newCert, newKey);
  const who1 = JSON.parse((await req('GET', '/v1/whoami', { cookie: cookie1 })).body);
  ok(st1 === 200 && who1.identity === 'root' && who1.trust === 'root' && who1.serial !== who0.serial,
    `new key signs in: ${who1.identity} trust=${who1.trust} serial ${who1.serial}`);
  const [stOld] = await signin(oldCert, oldKey);
  ok(stOld === 401, `old key refused after renewal (HTTP ${stOld})`);
  const noCsr = await req('POST', '/v1/pki/renew', { cookie: cookie1, body: '{}' });
  ok(noCsr.status === 400, `renew without a CSR is refused (HTTP ${noCsr.status})`);
  fs.writeFileSync(path.join(out, 'serials.json'), JSON.stringify({ old: who0.serial, new: who1.serial }));
})().catch(e => { fail(String(e && e.stack || e)); });
