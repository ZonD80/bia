#!/usr/bin/env node
// Browser renewal path against a running bia (tests/e2e/peer-e2e.sh): the same web/pki.js
// the console uses makes the key and CSR, bia signs it, the .p12 is built here.
//
//   node tests/e2e/peer-e2e-renew.js https://127.0.0.1:18443 <out dir>
//
// Steps: CA (trust on first use) -> first root .p12 via /v1/pki/bootstrap/root (password
// 12345678, read with openssl) -> POST /v1/pki/renew {csr} with that cert -> new cert
// works on /v1/whoami, the old one is refused -> renewed .p12 written for import checks.
'use strict';
const fs = require('fs');
const https = require('https');
const path = require('path');
const { execFileSync } = require('child_process');
const BiaPki = require(path.join(__dirname, '../../source/crates/bia-mcp-web/web/pki.js'));

const [base, out] = process.argv.slice(2);
if (!base || !out) { console.error('usage: peer-e2e-renew.js <https base> <out dir>'); process.exit(2); }
fs.mkdirSync(out, { recursive: true });

function req(method, url, { body, ca, cert, key } = {}) {
  return new Promise((resolve, reject) => {
    const u = new URL(url, base);
    const r = https.request(u, {
      method, ca, cert, key, rejectUnauthorized: !!ca,
      headers: body ? { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(body) } : {},
    }, res => {
      const chunks = [];
      res.on('data', c => chunks.push(c));
      res.on('end', () => resolve({ status: res.statusCode, body: Buffer.concat(chunks), headers: res.headers }));
    });
    r.on('error', reject);
    if (body) r.write(body);
    r.end();
  });
}

const fail = msg => { console.log(`  FAIL ${msg}`); process.exitCode = 1; };
const ok = (cond, msg) => (cond ? console.log(`  ok   ${msg}`) : fail(msg));

(async () => {
  const ca = (await req('GET', '/v1/pki/ca')).body.toString();
  ok(ca.includes('BEGIN CERTIFICATE'), 'CA downloaded');

  const boot = await req('POST', '/v1/pki/bootstrap/root', { ca });
  ok(boot.status === 200, `first root .p12 issued (HTTP ${boot.status})`);
  fs.writeFileSync(path.join(out, 'root-server.p12'), boot.body);
  const pemAll = execFileSync('openssl', ['pkcs12', '-in', path.join(out, 'root-server.p12'),
    '-passin', `pass:${BiaPki.PASSWORD}`, '-nodes'], { encoding: 'utf8' });
  const oldKey = pemAll.match(/-----BEGIN PRIVATE KEY-----[\s\S]*?-----END PRIVATE KEY-----\n/)[0];
  const oldCert = BiaPki.pem('CERTIFICATE', BiaPki.pemCerts(pemAll)[0]);
  ok(!!oldKey && !!oldCert, `server .p12 opens with password ${BiaPki.PASSWORD} (OpenSSL 3, no -legacy)`);
  const who0 = JSON.parse((await req('GET', '/v1/whoami', { ca, cert: oldCert, key: oldKey })).body);
  ok(who0.identity === 'root', `old cert works (serial ${who0.serial})`);

  const { csr, key } = await BiaPki.makeCsr('root');
  const ren = await req('POST', '/v1/pki/renew', { ca, cert: oldCert, key: oldKey, body: JSON.stringify({ csr }) });
  const chain = ren.body.toString();
  ok(ren.status === 200 && !chain.includes('PRIVATE KEY') && BiaPki.pemCerts(chain).length === 2,
    `renew answers PEM cert + CA, no key (HTTP ${ren.status}, serial ${ren.headers['x-bia-serial']})`);

  const p12 = await BiaPki.p12(key, chain, 'root (bia)');
  fs.writeFileSync(path.join(out, 'root-renewed.p12'), Buffer.from(p12));
  const newPem = execFileSync('openssl', ['pkcs12', '-in', path.join(out, 'root-renewed.p12'),
    '-passin', `pass:${BiaPki.PASSWORD}`, '-nodes'], { encoding: 'utf8' });
  const newKey = newPem.match(/-----BEGIN PRIVATE KEY-----[\s\S]*?-----END PRIVATE KEY-----\n/)[0];
  const newCert = BiaPki.pem('CERTIFICATE', BiaPki.pemCerts(newPem)[0]);
  ok(true, 'renewed .p12 built in the browser code opens with openssl');

  const who1 = JSON.parse((await req('GET', '/v1/whoami', { ca, cert: newCert, key: newKey })).body);
  ok(who1.identity === 'root' && who1.trust === 'root' && who1.serial !== who0.serial,
    `new cert works: ${who1.identity} trust=${who1.trust} serial ${who1.serial}`);
  const old = await req('GET', '/v1/whoami', { ca, cert: oldCert, key: oldKey });
  ok(old.status === 401, `old cert refused after renewal (HTTP ${old.status})`);
  const noCsr = await req('POST', '/v1/pki/renew', { ca, cert: newCert, key: newKey, body: '{}' });
  ok(noCsr.status === 400, `renew without a CSR is refused (HTTP ${noCsr.status})`);
  fs.writeFileSync(path.join(out, 'serials.json'), JSON.stringify({ old: who0.serial, new: who1.serial }));
})().catch(e => { fail(String(e && e.stack || e)); });
