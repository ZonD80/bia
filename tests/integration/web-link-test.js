// Unit test of the web app's web bus client (web/link.js): streaming SHA-256 against Node's,
// and the file chunk framing shared with the server ([fid 32 bytes][offset u64 BE][data]).
//   node tests/integration/web-link-test.js
'use strict';
const assert = require('assert');
const crypto = require('crypto');
const BiaLink = require('../../source/crates/bia-mcp-web/web/link.js');

let n = 0;
const ok = what => { n++; console.log('ok   ' + what); };

for (const len of [0, 1, 55, 56, 63, 64, 65, 119, 120, 1000, 100000]) {
  const data = crypto.randomBytes(len);
  const want = crypto.createHash('sha256').update(data).digest('hex');
  assert.strictEqual(BiaLink.Sha256().update(new Uint8Array(data)).hex(), want, `sha256 of ${len} bytes`);
  // the same bytes in odd pieces
  const s = BiaLink.Sha256();
  for (let o = 0; o < len; o += 7) s.update(new Uint8Array(data.subarray(o, o + 7)));
  assert.strictEqual(s.hex(), want, `sha256 of ${len} bytes in pieces`);
}
ok('streaming SHA-256 matches Node for every padding case, whole and in pieces');

(async () => {
  const data = crypto.randomBytes(5 << 20);
  const blob = new Blob([data]);
  let seen = 0;
  const got = await BiaLink.sha256Blob(blob, d => { seen = d; });
  assert.strictEqual(got, crypto.createHash('sha256').update(data).digest('hex'));
  assert.strictEqual(seen, data.length);
  ok('sha256Blob reads a 5 MiB blob in steps with progress');

  const fid = crypto.createHash('sha256').update('x').digest('hex');
  const body = new Uint8Array([1, 2, 3, 250]);
  const c = BiaLink.parseChunk(BiaLink.chunk(fid, 70000, body));
  assert.deepStrictEqual([c.fid, c.off, [...c.data]], [fid, 70000, [1, 2, 3, 250]]);
  const big = BiaLink.parseChunk(BiaLink.chunk(fid, 5 * 2 ** 32 + 7, body));
  assert.strictEqual(big.off, 5 * 2 ** 32 + 7);
  assert.strictEqual(BiaLink.parseChunk(new ArrayBuffer(39)), null);
  ok('chunk framing round-trips (offsets past 4 GiB too) and refuses short frames');

  assert(['send', 'edit', 'delete', 'react', 'read'].every(t => BiaLink.QUEUED.has(t)) && !BiaLink.QUEUED.has('hist') && !BiaLink.QUEUED.has('search'));
  ok('chat changes wait in the outbox; history and search do not');
  console.log(`\nall passed (${n})`);
})().catch(e => { console.error('FAIL', e); process.exit(1); });
