// Page world: announce each WebAuthn call's challenge so the extension learns the calling origin.
(() => {
  const creds = navigator.credentials;
  if (!creds || !creds.create || !creds.get) return;
  function b64url(buf) {
    const bytes = buf instanceof ArrayBuffer ? new Uint8Array(buf) : new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
    let s = "";
    for (const b of bytes) s += String.fromCharCode(b);
    return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  }
  function announce(options) {
    try {
      const ch = options && options.publicKey && options.publicKey.challenge;
      if (ch) document.dispatchEvent(new CustomEvent("bia-webauthn", { detail: JSON.stringify({ challenge: b64url(ch), mediation: String(options.mediation || "") }) }));
    } catch (e) {}
  }
  const create = creds.create.bind(creds);
  const get = creds.get.bind(creds);
  creds.create = function (options) {
    announce(options);
    return create(options);
  };
  creds.get = function (options) {
    announce(options);
    return get(options);
  };
})();

// Page world, calls: in a tab opened with chrome_call_open, what the call plays is tapped and sent
// out as 16 kHz speech per voice, and the page's microphone is bia's voice. Tapped are every
// remote WebRTC audio track and every audio track given to a media element; when those carry
// nothing (Google Meet decodes audio itself and plays it through Web Audio), whatever the page
// connects to its audio output instead. In any other tab nothing here changes what the page gets.
(() => {
  const RTC = window.RTCPeerConnection;
  const md = navigator.mediaDevices;
  if (!RTC || !md || !md.getUserMedia) return;
  const MIC_ID = "bia-voice";
  const QUIET = 0.002;
  const TAIL_MS = 2000;

  let armed = null;
  const armedWaits = [];
  const remote = new Map();
  let trackSeq = 0;
  let pcs = 0;
  let ctx = null;
  let dest = null;
  let nextT = 0;
  let sources = [];
  let micServed = 0;
  let lastError = "";
  // frames that came from tracks; while there are none, the page's Web Audio output is the call
  let trackFrames = 0;
  let outSeq = 0;
  const pendingOut = [];
  const mine = new WeakSet();
  const pcList = [];
  // what the page uses, for chrome_call_status
  const probe = { insertable: 0, encoded: 0, transform: 0, elements: 0, outputs: 0, edges: [] };

  // one per loaded page: after a reload the voices are new ones
  const PAGE = Math.random().toString(36).slice(2, 10);

  function out(msg) {
    try {
      msg.page = PAGE;
      document.dispatchEvent(new CustomEvent("bia-voice-out", { detail: JSON.stringify(msg) }));
    } catch (e) {}
  }

  function isArmed() {
    if (armed !== null) return Promise.resolve(armed);
    return new Promise((resolve) => {
      const timer = setTimeout(() => resolve(false), 1500);
      armedWaits.push((on) => {
        clearTimeout(timer);
        resolve(on);
      });
    });
  }

  function b64(bytes) {
    let s = "";
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    return btoa(s);
  }

  function unb64(text) {
    const s = atob(text);
    const bytes = new Uint8Array(s.length);
    for (let i = 0; i < s.length; i++) bytes[i] = s.charCodeAt(i);
    return bytes;
  }

  // ---- hearing: one stream of speech per voice --------------------------------------------

  // First channel of an AudioData frame as floats.
  function mono(frame) {
    const n = frame.numberOfFrames;
    const buf = new Float32Array(n);
    try {
      frame.copyTo(buf, { planeIndex: 0, format: "f32-planar" });
      return buf;
    } catch (e) {}
    const ch = frame.numberOfChannels;
    const f = frame.format;
    if (f === "f32-planar") {
      frame.copyTo(buf, { planeIndex: 0 });
    } else if (f === "f32") {
      const all = new Float32Array(n * ch);
      frame.copyTo(all, { planeIndex: 0 });
      for (let i = 0; i < n; i++) buf[i] = all[i * ch];
    } else if (f === "s16" || f === "s16-planar") {
      const step = f === "s16" ? ch : 1;
      const all = new Int16Array(n * step);
      frame.copyTo(all, { planeIndex: 0 });
      for (let i = 0; i < n; i++) buf[i] = all[i * step] / 32768;
    } else {
      throw new Error("audio format " + f);
    }
    return buf;
  }

  // Who is in this slot now: conference servers that reuse a few audio slots for many people
  // (Google Meet) name the real speaker as the contributing source.
  function speakerOf(tap) {
    try {
      const list = tap.receiver && tap.receiver.getContributingSources ? tap.receiver.getContributingSources() : [];
      let best = null;
      for (const s of list) if (!best || s.timestamp > best.timestamp) best = s;
      if (best) {
        const now = best.timestamp > 1e11 ? Date.now() : performance.now();
        if (now - best.timestamp < 1500) return "c" + best.source;
      }
    } catch (e) {}
    return null;
  }

  function flush(tap) {
    if (!tap.fill) return;
    const pcm = new Int16Array(tap.fill);
    let peak = 0;
    for (let i = 0; i < tap.fill; i++) {
      const v = Math.max(-1, Math.min(1, tap.acc[i]));
      const a = v < 0 ? -v : v;
      if (a > peak) peak = a;
      pcm[i] = v * 32767;
    }
    const ms = (tap.fill * 1000) / tap.rate;
    tap.fill = 0;
    if (peak < QUIET) {
      tap.quietMs += ms;
      // a pause ends a phrase; after that silence says nothing new
      if (tap.quietMs > TAIL_MS) return;
    } else {
      tap.quietMs = 0;
      tap.loud++;
    }
    // the page's own output is the call only while no track carries it (else it would be heard twice)
    if (tap.out && trackFrames > 0) return;
    tap.sent++;
    out({ t: "audio", src: tap.src, rate: tap.rate, pcm: b64(new Uint8Array(pcm.buffer)) });
  }

  function feed(tap, x, rate) {
    // 48 kHz (what WebRTC gives) is averaged in threes down to 16 kHz; other rates go as they are
    const step = rate % 16000 === 0 ? rate / 16000 : 1;
    const outRate = rate / step;
    if (tap.rate !== outRate) {
      flush(tap);
      tap.rate = outRate;
      tap.acc = new Float32Array(Math.round(outRate / 10));
      tap.carry = [];
    }
    let i = 0;
    if (step > 1 && tap.carry.length) {
      while (tap.carry.length < step && i < x.length) tap.carry.push(x[i++]);
      if (tap.carry.length === step) {
        let s = 0;
        for (const v of tap.carry) s += v;
        tap.acc[tap.fill++] = s / step;
        tap.carry = [];
        if (tap.fill === tap.acc.length) flush(tap);
      }
    }
    for (; i + step <= x.length; i += step) {
      let s = 0;
      for (let k = 0; k < step; k++) s += x[i + k];
      tap.acc[tap.fill++] = s / step;
      if (tap.fill === tap.acc.length) flush(tap);
    }
    for (; i < x.length; i++) tap.carry.push(x[i]);
  }

  async function listen(tap) {
    if (tap.running || tap.track.readyState !== "live") return;
    if (typeof MediaStreamTrackProcessor !== "function") {
      lastError = "MediaStreamTrackProcessor is missing";
      return;
    }
    tap.running = true;
    let reader;
    try {
      reader = new MediaStreamTrackProcessor({ track: tap.track }).readable.getReader();
      out({ t: "track", src: tap.id, on: true });
      for (let n = 0; ; n++) {
        const { value, done } = await reader.read();
        if (done) break;
        let x;
        const rate = value.sampleRate;
        try {
          x = mono(value);
        } finally {
          value.close();
        }
        tap.frames++;
        if (!tap.out) trackFrames++;
        if (n % 5 === 0) {
          // nobody named: the slot still belongs to whoever spoke last (their pause ends the phrase)
          const who = speakerOf(tap);
          if (who && who !== tap.src) {
            flush(tap);
            tap.src = who;
            tap.quietMs = 0;
          }
        }
        feed(tap, x, rate);
      }
    } catch (e) {
      lastError = "tap " + tap.id + ": " + (e && e.message ? e.message : e);
    }
    tap.running = false;
    try {
      if (reader) reader.cancel();
    } catch (e) {}
    remote.delete(tap.track);
    out({ t: "track", src: tap.id, on: false });
  }

  function addRemote(track, receiver, kind) {
    if (!track || track.kind !== "audio" || remote.has(track) || mine.has(track)) return null;
    const id = (kind || "t") + ++trackSeq;
    const tap = {
      id, src: id, track, receiver, running: false, rate: 0, acc: null, fill: 0, carry: [],
      quietMs: 0, frames: 0, sent: 0, loud: 0, out: kind === "a",
    };
    remote.set(track, tap);
    track.addEventListener("ended", () => {
      if (!tap.running) remote.delete(track);
    });
    if (armed) listen(tap);
    return tap;
  }

  function watch(pc, config) {
    pcs++;
    try {
      if (config && config.encodedInsertableStreams) probe.insertable++;
      if (pcList.length < 8) pcList.push(pc);
      pc.addEventListener("track", (e) => addRemote(e.track, e.receiver));
    } catch (e) {}
  }

  const Wrapped = new Proxy(RTC, {
    construct(target, args, newTarget) {
      const pc = Reflect.construct(target, args, newTarget);
      watch(pc, args[0]);
      return pc;
    },
  });
  window.RTCPeerConnection = Wrapped;
  if (window.webkitRTCPeerConnection) window.webkitRTCPeerConnection = Wrapped;

  // a page that decodes the call's audio itself takes the encoded frames from the receivers
  try {
    const RR = window.RTCRtpReceiver;
    if (RR && RR.prototype.createEncodedStreams) {
      const real = RR.prototype.createEncodedStreams;
      RR.prototype.createEncodedStreams = function () {
        probe.encoded++;
        return real.apply(this, arguments);
      };
    }
    const td = RR && Object.getOwnPropertyDescriptor(RR.prototype, "transform");
    if (td && td.set) {
      Object.defineProperty(RR.prototype, "transform", {
        ...td,
        set(v) {
          if (v) probe.transform++;
          td.set.call(this, v);
        },
      });
    }
  } catch (e) {}

  // audio tracks a page plays through a media element (also ones it made itself)
  try {
    const sd = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, "srcObject");
    if (sd && sd.set) {
      Object.defineProperty(HTMLMediaElement.prototype, "srcObject", {
        ...sd,
        set(v) {
          sd.set.call(this, v);
          try {
            if (armed !== false && v instanceof MediaStream) {
              for (const t of v.getAudioTracks()) if (addRemote(t, null, "m")) probe.elements++;
            }
          } catch (e) {}
        },
      });
    }
  } catch (e) {}

  // what the page connects to its audio output: one voice per connected node
  const nodeIds = new WeakMap();
  let nodeSeq = 0;
  const tapped = new WeakMap();
  function nodeName(n) {
    let id = nodeIds.get(n);
    if (!id) {
      id = ++nodeSeq;
      nodeIds.set(n, id);
    }
    return ((n.constructor && n.constructor.name) || "Node").replace(/Node$/, "") + "#" + id;
  }
  const realConnect = window.AudioNode && AudioNode.prototype.connect;
  const realDisconnect = window.AudioNode && AudioNode.prototype.disconnect;

  function tapOutput(node, output) {
    if (armed === null) {
      if (pendingOut.length < 32) pendingOut.push([node, output]);
      return;
    }
    if (!armed) return;
    let outs = tapped.get(node);
    if (!outs) tapped.set(node, (outs = new Map()));
    if (outs.has(output)) return;
    const side = node.context.createMediaStreamDestination();
    realConnect.call(node, side, output);
    outs.set(output, side);
    probe.outputs++;
    const tap = addRemote(side.stream.getAudioTracks()[0], null, "a");
    if (tap) tap.node = nodeName(node);
  }

  if (realConnect) {
    AudioNode.prototype.connect = function (target, output) {
      const r = realConnect.apply(this, arguments);
      try {
        if (armed !== false && target instanceof AudioNode && this.context !== ctx) {
          if (probe.edges.length < 60) probe.edges.push(nodeName(this) + (output ? "." + output : "") + ">" + nodeName(target));
          if (target instanceof AudioDestinationNode && this.context instanceof AudioContext) tapOutput(this, output || 0);
        }
      } catch (e) {}
      return r;
    };
    AudioNode.prototype.disconnect = function (target) {
      const r = realDisconnect.apply(this, arguments);
      try {
        // taken off the output by the page: off the tap too (with no argument Chrome did that already)
        const outs = tapped.get(this);
        if (outs && target instanceof AudioDestinationNode) {
          for (const side of outs.values()) realDisconnect.call(this, side);
          tapped.delete(this);
        } else if (outs && target === undefined) {
          tapped.delete(this);
        }
      } catch (e) {}
      return r;
    };
  }

  // ---- speaking: bia's voice is the microphone -------------------------------------------

  function voice() {
    if (!ctx) {
      try {
        ctx = new AudioContext({ sampleRate: 48000, sinkId: { type: "none" } });
      } catch (e) {
        ctx = new AudioContext({ sampleRate: 48000 });
      }
      dest = ctx.createMediaStreamDestination();
      // a source that never stops keeps the track delivering silence between phrases
      const hum = ctx.createConstantSource();
      hum.offset.value = 0;
      (realConnect || hum.connect).call(hum, dest);
      hum.start();
    }
    if (ctx.state !== "running") ctx.resume().catch(() => {});
    return dest;
  }

  function micTrack() {
    const track = voice().stream.getAudioTracks()[0].clone();
    try {
      const settings = track.getSettings.bind(track);
      track.getSettings = () => ({ ...settings(), deviceId: MIC_ID, groupId: MIC_ID });
      Object.defineProperty(track, "label", { value: "bia voice", configurable: true });
    } catch (e) {}
    mine.add(track);
    return track;
  }

  const realGum = md.getUserMedia.bind(md);
  md.getUserMedia = async function (constraints) {
    if (!constraints || !constraints.audio || !(await isArmed())) return realGum(constraints);
    const stream = new MediaStream();
    if (constraints.video) {
      const cam = await realGum({ video: constraints.video });
      for (const t of cam.getVideoTracks()) stream.addTrack(t);
    }
    stream.addTrack(micTrack());
    micServed++;
    out({ t: "mic" });
    return stream;
  };

  if (md.enumerateDevices) {
    const realList = md.enumerateDevices.bind(md);
    md.enumerateDevices = async function () {
      const list = await realList();
      if (!(await isArmed())) return list;
      const info = { deviceId: MIC_ID, kind: "audioinput", label: "bia voice", groupId: MIC_ID };
      let mic;
      try {
        mic = Object.create((window.InputDeviceInfo || window.MediaDeviceInfo).prototype);
        for (const k of Object.keys(info)) Object.defineProperty(mic, k, { value: info[k], enumerable: true });
        Object.defineProperty(mic, "toJSON", { value: () => ({ ...info }) });
        if (window.InputDeviceInfo) Object.defineProperty(mic, "getCapabilities", { value: () => ({ deviceId: MIC_ID, groupId: MIC_ID }) });
      } catch (e) {
        mic = info;
      }
      return [mic, ...list.filter((d) => d.kind !== "audioinput")];
    };
  }

  if (navigator.permissions && navigator.permissions.query) {
    const perms = navigator.permissions;
    const realQuery = perms.query.bind(perms);
    perms.query = async function (desc) {
      if (desc && desc.name === "microphone" && (await isArmed())) {
        const status = new EventTarget();
        Object.defineProperties(status, {
          state: { value: "granted", enumerable: true },
          name: { value: "microphone", enumerable: true },
          onchange: { value: null, writable: true },
        });
        return status;
      }
      return realQuery(desc);
    };
  }

  function stopVoice() {
    for (const s of sources) {
      try {
        s.onended = null;
        s.stop();
      } catch (e) {}
    }
    sources = [];
    nextT = 0;
  }

  function play(msg) {
    if (!ctx || !micServed) return;
    voice();
    const bytes = unb64(msg.pcm);
    const pcm = new Int16Array(bytes.buffer, 0, bytes.length >> 1);
    if (!pcm.length) return;
    const buf = ctx.createBuffer(1, pcm.length, msg.rate);
    const data = buf.getChannelData(0);
    for (let i = 0; i < pcm.length; i++) data[i] = pcm[i] / 32768;
    const node = ctx.createBufferSource();
    node.buffer = buf;
    (realConnect || node.connect).call(node, dest);
    const at = Math.max(ctx.currentTime + 0.08, nextT);
    node.start(at);
    nextT = at + buf.duration;
    node.gen = msg.gen;
    sources.push(node);
    node.onended = () => {
      sources = sources.filter((s) => s !== node);
      if (node.last) out({ t: "played", gen: node.gen });
    };
  }

  // What each connection receives: packets that arrive but give no samples mean the page decodes
  // the audio itself.
  async function receiving() {
    const list = [];
    for (const pc of pcList) {
      const row = { state: pc.connectionState, audio: [], sent: [], channels: 0, channel_bytes: 0 };
      try {
        const stats = await pc.getStats();
        stats.forEach((s) => {
          if (s.type === "inbound-rtp" && s.kind === "audio" && row.audio.length < 6) {
            row.audio.push({ packets: s.packetsReceived || 0, samples: s.totalSamplesReceived || 0, level: s.audioLevel || 0 });
          } else if (s.type === "outbound-rtp" && s.kind === "audio" && row.sent.length < 4) {
            row.sent.push({ packets: s.packetsSent || 0, bytes: s.bytesSent || 0 });
          } else if (s.type === "media-source" && s.kind === "audio" && row.sent.length < 4) {
            // what the page's microphone track carries: energy grows while bia speaks
            row.sent.push({ level: s.audioLevel || 0, energy: s.totalAudioEnergy || 0 });
          } else if (s.type === "data-channel") {
            row.channels++;
            row.channel_bytes += s.bytesReceived || 0;
          }
        });
      } catch (e) {
        row.error = String(e && e.message ? e.message : e);
      }
      list.push(row);
    }
    return list;
  }

  async function state() {
    const taps = [];
    for (const tap of remote.values()) {
      taps.push({ id: tap.id, src: tap.src, node: tap.node, live: tap.track.readyState === "live", running: tap.running, frames: tap.frames, sent: tap.sent, loud: tap.loud });
    }
    out({
      t: "state", url: location.origin, armed: !!armed, pcs, taps, mic: micServed,
      audio: ctx ? ctx.state : "none", queued: sources.length, error: lastError,
      from_tracks: trackFrames > 0, uses: { ...probe, edges: probe.edges.slice() }, connections: await receiving(),
    });
  }

  // Audio that ran and then stands still (the page's audio output stopped): frames come all the
  // time, also in silence, so none for a while means nothing is heard any more.
  let seenFrames = 0;
  let stillFor = 0;
  let stalled = false;
  setInterval(() => {
    if (!armed) return;
    let total = 0;
    let running = 0;
    for (const tap of remote.values()) {
      total += tap.frames;
      if (tap.running) running++;
    }
    if (total !== seenFrames) {
      seenFrames = total;
      stillFor = 0;
      if (stalled) {
        stalled = false;
        out({ t: "stalled", on: false });
      }
    } else if (running && total > 0 && ++stillFor >= 4 && !stalled) {
      stalled = true;
      out({ t: "stalled", on: true });
    }
  }, 3000);

  document.addEventListener("bia-voice-in", (e) => {
    if (typeof e.detail !== "string") return;
    let msg;
    try {
      msg = JSON.parse(e.detail);
    } catch (err) {
      return;
    }
    if (msg.t === "armed") {
      armed = !!msg.on;
      for (const w of armedWaits.splice(0)) w(armed);
      for (const [node, output] of pendingOut.splice(0)) {
        try {
          tapOutput(node, output);
        } catch (err) {}
      }
      if (armed) for (const tap of remote.values()) listen(tap);
      else remote.clear();
    } else if (!armed) {
      return;
    } else if (msg.t === "play") {
      play(msg);
    } else if (msg.t === "end") {
      // the phrase is complete: say so when its last piece has played
      const mine = sources.filter((s) => s.gen === msg.gen);
      if (mine.length) mine[mine.length - 1].last = true;
      else if (ctx && micServed) out({ t: "played", gen: msg.gen });
    } else if (msg.t === "stop") {
      stopVoice();
    } else if (msg.t === "stat") {
      state();
    }
  });
  // a click in the page lets its audio run, should Chrome have held it back
  document.addEventListener("pointerdown", () => {
    if (ctx && ctx.state !== "running") ctx.resume().catch(() => {});
  }, true);
})();
