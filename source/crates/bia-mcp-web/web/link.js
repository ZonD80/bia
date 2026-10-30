// bia web bus client (protocol 2, GET /ws): what the web app uses to talk to bia.
// One WebSocket carries everything: hello and catch-up, the chat's changes (`m`, `gone`),
// actions with acks, and files both ways (the receiver pulls: fget → binary chunks
// [fid 32 bytes][offset u64 BE][data] → fack / fdone). Actions that change the chat wait
// in an outbox (IndexedDB, with the files) while the link is down and go when it is back.
// See ARCHITECTURE.md, "The web bus protocol".
'use strict';

const BiaLink = (() => {
  // ---------------------------------------------------------------- SHA-256
  const K = new Uint32Array([
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2]);

  // Incremental SHA-256: update(bytes) as often as needed, then hex().
  function Sha256() {
    const H = new Uint32Array([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]);
    const W = new Uint32Array(64);
    const buf = new Uint8Array(64);
    let fill = 0, total = 0;
    const block = (b, o) => {
      for (let i = 0; i < 16; i++) W[i] = (b[o + 4 * i] << 24) | (b[o + 4 * i + 1] << 16) | (b[o + 4 * i + 2] << 8) | b[o + 4 * i + 3];
      for (let i = 16; i < 64; i++) {
        const a = W[i - 15], c = W[i - 2];
        const s0 = ((a >>> 7) | (a << 25)) ^ ((a >>> 18) | (a << 14)) ^ (a >>> 3);
        const s1 = ((c >>> 17) | (c << 15)) ^ ((c >>> 19) | (c << 13)) ^ (c >>> 10);
        W[i] = (W[i - 16] + s0 + W[i - 7] + s1) | 0;
      }
      let a = H[0], bb = H[1], c = H[2], d = H[3], e = H[4], f = H[5], g = H[6], h = H[7];
      for (let i = 0; i < 64; i++) {
        const S1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
        const t1 = (h + S1 + ((e & f) ^ (~e & g)) + K[i] + W[i]) | 0;
        const S0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
        const t2 = (S0 + ((a & bb) ^ (a & c) ^ (bb & c))) | 0;
        h = g; g = f; f = e; e = (d + t1) | 0; d = c; c = bb; bb = a; a = (t1 + t2) | 0;
      }
      H[0] += a; H[1] += bb; H[2] += c; H[3] += d; H[4] += e; H[5] += f; H[6] += g; H[7] += h;
    };
    return {
      update(bytes) {
        let o = 0;
        total += bytes.length;
        if (fill) {
          const n = Math.min(64 - fill, bytes.length);
          buf.set(bytes.subarray(0, n), fill); fill += n; o = n;
          if (fill === 64) { block(buf, 0); fill = 0; }
        }
        for (; o + 64 <= bytes.length; o += 64) block(bytes, o);
        if (o < bytes.length) { buf.set(bytes.subarray(o), 0); fill = bytes.length - o; }
        return this;
      },
      hex() {
        const bits = total * 8;
        const pad = new Uint8Array(((fill < 56 ? 56 : 120) - fill) + 8);
        pad[0] = 0x80;
        const dv = new DataView(pad.buffer);
        dv.setUint32(pad.length - 8, Math.floor(bits / 2 ** 32));
        dv.setUint32(pad.length - 4, bits >>> 0);
        this.update(pad);
        return [...H].map(x => (x >>> 0).toString(16).padStart(8, '0')).join('');
      },
    };
  }

  // sha256 of a Blob/File, read 2 MiB at a time (a big file is never in memory whole).
  async function sha256Blob(blob, onProgress) {
    const s = Sha256();
    const step = 2 << 20;
    for (let o = 0; o < blob.size; o += step) {
      s.update(new Uint8Array(await blob.slice(o, o + step).arrayBuffer()));
      if (onProgress) onProgress(Math.min(o + step, blob.size), blob.size);
    }
    return s.hex();
  }

  // ---------------------------------------------------------------- chunks
  const CHUNK = 32 * 1024;
  const WINDOW = 1 << 20;
  const hexBytes = h => new Uint8Array(h.match(/../g).map(x => parseInt(x, 16)));
  function chunk(fid, off, data) {
    const b = new Uint8Array(40 + data.length);
    b.set(hexBytes(fid), 0);
    new DataView(b.buffer).setBigUint64(32, BigInt(off));
    b.set(data, 40);
    return b.buffer;
  }
  function parseChunk(ab) {
    const b = new Uint8Array(ab);
    if (b.length < 40) return null;
    const fid = [...b.subarray(0, 32)].map(x => x.toString(16).padStart(2, '0')).join('');
    const off = Number(new DataView(ab).getBigUint64(32));
    return { fid, off, data: b.subarray(40) };
  }

  // ---------------------------------------------------------------- IndexedDB
  // Stores: `out` (cid → action), `blobs` (fid → Blob). Without IndexedDB (a private window)
  // the outbox lives in memory only.
  function openDb(name) {
    return new Promise(resolve => {
      try {
        const r = indexedDB.open(name, 1);
        r.onupgradeneeded = () => { r.result.createObjectStore('out'); r.result.createObjectStore('blobs'); };
        r.onsuccess = () => resolve(r.result);
        r.onerror = () => resolve(null);
      } catch { resolve(null); }
    });
  }
  const req = (db, store, mode, fn) => new Promise(resolve => {
    if (!db) return resolve(undefined);
    try {
      const t = db.transaction(store, mode);
      const r = fn(t.objectStore(store));
      t.oncomplete = () => resolve(r && r.result);
      t.onerror = t.onabort = () => resolve(undefined);
    } catch { resolve(undefined); }
  });

  // Actions that change the chat: they wait in the outbox while the link is down.
  const QUEUED = new Set(['send', 'edit', 'delete', 'react', 'pin', 'read', 'vote', 'button', 'close_poll', 'mute']);

  // ---------------------------------------------------------------- the link
  function create(opts) {
    const L = {
      state: 'connecting',     // connecting | online | blocked | replaced | banned
      rx: null,                // last revision seen (null: from now; the app loads history itself)
      instance: [...crypto.getRandomValues(new Uint8Array(8))].map(x => x.toString(16).padStart(2, '0')).join(''),
      ws: null, tries: 0, hello: null, bye: null, timer: 0, lastRx: 0,
      acks: new Map(),         // cid → {resolve, reject, queued}
      out: new Map(),          // cid → action (the outbox, mirrored in IndexedDB)
      blobs: new Map(),        // fid → Blob (files of ours the server may pull)
      jobs: new Map(),         // fid → {blob, off, end, acked} (being pulled from us)
      pulls: new Map(),        // fid → {parts, got, size, mime, resolve, reject, progress}
      urls: new Map(),         // fid → Promise<object URL>
      db: null, seq: 1,
    };
    const emit = (k, ...a) => { try { opts[k] && opts[k](...a); } catch (e) { console.error(e); } };
    const setState = s => { if (L.state !== s) { L.state = s; emit('onState', s); } };
    const open = () => L.ws && L.ws.readyState === 1 && L.hello;
    const sendJson = v => { if (L.ws && L.ws.readyState === 1) L.ws.send(JSON.stringify(v)); };

    async function load() {
      L.db = await openDb('bia-link-' + (opts.identity || 'me'));
      const keys = (await req(L.db, 'out', 'readonly', s => s.getAllKeys())) || [];
      const vals = (await req(L.db, 'out', 'readonly', s => s.getAll())) || [];
      keys.forEach((k, i) => L.out.set(k, vals[i]));
      emit('onOutbox', [...L.out.values()]);
    }

    function connect() {
      clearTimeout(L.timer);
      L.hello = null; L.bye = null;
      const ws = new WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws');
      ws.binaryType = 'arraybuffer';
      L.ws = ws;
      ws.onopen = () => ws.send(JSON.stringify({ t: 'hello', proto: 2, name: '', ver: 'web', instance: L.instance, rx: L.rx }));
      ws.onmessage = e => {
        L.lastRx = Date.now();
        if (typeof e.data !== 'string') { onChunk(e.data); return; }
        let v; try { v = JSON.parse(e.data); } catch { return; }
        if (!L.hello) {
          if (v.t === 'hello') {
            L.hello = v; L.tries = 0;
            if (v.rev < (L.rx || 0)) L.rx = null;
            setState('online');
            emit('onHello', v);
            for (const [cid, a] of L.out) ws.send(JSON.stringify(Object.assign({}, a, { cid })));
            for (const [fid, p] of L.pulls) sendJson({ t: 'fget', fid, off: p.got });
          } else if (v.t === 'bye') L.bye = v.reason;
          return;
        }
        onFrame(v);
      };
      ws.onclose = () => {
        if (L.ws !== ws) return;
        for (const [cid, a] of L.acks) if (!a.queued) { a.reject(new Error('not connected')); L.acks.delete(cid); }
        L.jobs.clear();
        const why = L.bye;
        let wait = Math.min(15000, 800 * Math.pow(1.6, L.tries++));
        if (why === 'duplicate_key') { setState('blocked'); wait = 30000; }
        else if (why === 'duplicate') { setState('replaced'); return; }
        else if (why === 'banned') { setState('banned'); wait = 600000; }
        else if (why === 'closed') return;
        else setState('connecting');
        L.timer = setTimeout(connect, wait);
      };
    }

    function onFrame(v) {
      switch (v.t) {
        case 'm': case 'gone':
          if (v.seq > (L.rx || 0)) L.rx = v.seq;
          if (v.t === 'm') {
            // our file arrived there: its copy here is no longer needed
            for (const f of v.m.files || []) if (f.fid && !f.pending && L.blobs.has(f.fid) && !L.jobs.has(f.fid)) dropBlob(f.fid);
          }
          emit('onFrame', v);
          return;
        case 'ack': {
          const a = L.acks.get(v.cid);
          if (L.out.has(v.cid)) { L.out.delete(v.cid); req(L.db, 'out', 'readwrite', s => s.delete(v.cid)); emit('onOutbox', [...L.out.values()]); }
          if (!a) return;
          L.acks.delete(v.cid);
          if (v.ok) a.resolve(v.r || {}); else a.reject(new Error(v.error || 'failed'));
          return;
        }
        case 'reset': L.rx = null; emit('onReset'); return;
        case 'fget': serve(v.fid, v.off || 0); return;
        case 'fack': { const j = L.jobs.get(v.fid); if (j) { j.acked = v.off; emit('onUpload', v.fid, v.off, j.end); } return; }
        case 'fdone': L.jobs.delete(v.fid); emit('onUpload', v.fid, -1, -1); return;
        case 'frefuse': case 'fcancel': {
          const j = L.jobs.get(v.fid); if (j) { L.jobs.delete(v.fid); emit('onUpload', v.fid, -2, v.reason || 'refused'); }
          const p = L.pulls.get(v.fid); if (p) { L.pulls.delete(v.fid); p.reject(new Error(v.reason || 'not available')); }
          return;
        }
        case 'bye': L.bye = v.reason; return;
        default: emit('onFrame', v);
      }
    }

    // ---- actions ----
    function act(t, payload = {}) {
      const cid = 'w' + L.instance.slice(0, 6) + (L.seq++).toString(36);
      const a = Object.assign({ t }, payload);
      const queued = QUEUED.has(t);
      return new Promise((resolve, reject) => {
        if (queued) {
          L.out.set(cid, a);
          req(L.db, 'out', 'readwrite', s => s.put(a, cid));
          emit('onOutbox', [...L.out.values()]);
        } else if (!open()) { reject(new Error('not connected')); return; }
        L.acks.set(cid, { resolve, reject, queued });
        if (open()) L.ws.send(JSON.stringify(Object.assign({}, a, { cid })));
      });
    }

    // A message with files: each is hashed here, kept (IndexedDB) until bia has pulled it.
    async function send({ id, text, reply_to, kind, files = [], onHash }) {
      const offered = [];
      for (const f of files) {
        const fid = await sha256Blob(f, (d, t) => onHash && onHash(f, d, t));
        L.blobs.set(fid, f);
        await req(L.db, 'blobs', 'readwrite', s => s.put(f, fid));
        offered.push({ fid, name: f.name || 'file', size: f.size, kind: kind || '' });
      }
      return act('send', { id, text: text || '', reply_to: reply_to || '', files: offered });
    }

    function dropBlob(fid) {
      L.blobs.delete(fid);
      req(L.db, 'blobs', 'readwrite', s => s.delete(fid));
    }

    // bia pulls a file of ours: stream it with about WINDOW bytes in flight.
    async function serve(fid, off) {
      let blob = L.blobs.get(fid);
      if (!blob) { blob = await req(L.db, 'blobs', 'readonly', s => s.get(fid)); if (blob) L.blobs.set(fid, blob); }
      if (!blob) { sendJson({ t: 'frefuse', fid, reason: 'this browser no longer has that file' }); return; }
      const job = { blob, off: Math.min(off, blob.size), end: blob.size, acked: off };
      L.jobs.set(fid, job);
      const ws = L.ws;
      while (L.jobs.get(fid) === job && job.off < job.end && ws.readyState === 1) {
        if (job.off - job.acked > WINDOW || ws.bufferedAmount > WINDOW) { await new Promise(r => setTimeout(r, 30)); continue; }
        const data = new Uint8Array(await blob.slice(job.off, job.off + CHUNK).arrayBuffer());
        ws.send(chunk(fid, job.off, data));
        job.off += data.length;
      }
    }

    // ---- files from bia ----
    function onChunk(ab) {
      const c = parseChunk(ab);
      const p = c && L.pulls.get(c.fid);
      if (!p) return;
      if (c.off !== p.got) { if (c.off > p.got) sendJson({ t: 'fget', fid: c.fid, off: p.got }); return; }
      const take = Math.min(c.data.length, p.size - p.got);
      p.parts.push(c.data.slice(0, take));
      p.got += take;
      sendJson({ t: 'fack', fid: c.fid, off: p.got });
      if (p.progress) p.progress(p.got, p.size);
      if (p.got >= p.size) finish(c.fid, p);
    }
    function finish(fid, p) {
      L.pulls.delete(fid);
      sendJson({ t: 'fdone', fid });
      p.resolve(new Blob(p.parts, { type: p.mime || '' }));
    }

    // A file of the chat as a Blob (pulled once).
    function blob(fid, size, mime, progress) {
      const have = L.blobs.get(fid);
      if (have) return Promise.resolve(have);
      return new Promise((resolve, reject) => {
        if (!size) { resolve(new Blob([], { type: mime || '' })); return; }
        const old = L.pulls.get(fid);
        if (old) { const r0 = old.resolve, j0 = old.reject; old.resolve = b => { r0(b); resolve(b); }; old.reject = e => { j0(e); reject(e); }; return; }
        L.pulls.set(fid, { parts: [], got: 0, size, mime, resolve, reject, progress });
        sendJson({ t: 'fget', fid, off: 0 });
      });
    }
    // …as an object URL for <img>, <video>, <audio>, links (cached for the page's life).
    function url(f, progress) {
      if (!f || !f.fid) return Promise.reject(new Error('no file'));
      if (!L.urls.has(f.fid)) {
        const p = blob(f.fid, f.size, f.mime, progress).then(b => URL.createObjectURL(b));
        p.catch(() => L.urls.delete(f.fid));
        L.urls.set(f.fid, p);
      }
      return L.urls.get(f.fid);
    }

    // The tab is closing: free the session at once (a reload starts a new one).
    function close() {
      clearTimeout(L.timer);
      try { sendJson({ t: 'bye', reason: 'closed' }); L.bye = 'closed'; L.ws && L.ws.close(); } catch {}
    }
    function retry() { L.tries = 0; setState('connecting'); connect(); }

    // a ping every 5 s; bia pings too, so 15 s of silence means the link is dead
    setInterval(() => {
      if (!open()) return;
      if (Date.now() - L.lastRx > 15000) { try { L.ws.close(); } catch {} return; }
      sendJson({ t: 'ping' });
    }, 5000);
    addEventListener('pagehide', close);
    // back from the back/forward cache: a new session
    addEventListener('pageshow', e => { if (e.persisted) retry(); });
    L.start = async () => { await load(); connect(); };
    Object.assign(L, { act, send, url, blob, close, retry, sendJson });
    return L;
  }

  return { create, Sha256, sha256Blob, chunk, parseChunk, QUEUED };
})();
if (typeof module !== 'undefined') module.exports = BiaLink;
