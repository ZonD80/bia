// bia terminal: a small xterm-compatible emulator rendered as DOM rows.
// Used for root's live console stream (status header pinned) and the root shell.
'use strict';
(function () {
  const PALETTE = (() => {
    const base = ['#1d1f24', '#e06c75', '#98c379', '#e5c07b', '#61afef', '#c678dd', '#56b6c2', '#c8ccd4',
      '#5c6370', '#ff7b86', '#b5e890', '#ffd58a', '#82c4ff', '#de9bf0', '#7fd6e0', '#ffffff'];
    const p = base.slice();
    const lv = [0, 95, 135, 175, 215, 255];
    for (let r = 0; r < 6; r++) for (let g = 0; g < 6; g++) for (let b = 0; b < 6; b++)
      p.push(`rgb(${lv[r]},${lv[g]},${lv[b]})`);
    for (let i = 0; i < 24; i++) { const v = 8 + i * 10; p.push(`rgb(${v},${v},${v})`); }
    return p;
  })();
  const DEC_GRAPHICS = {
    '`': '◆', a: '▒', f: '°', g: '±', j: '┘', k: '┐', l: '┌', m: '└', n: '┼', o: '⎺', p: '⎻',
    q: '─', r: '⎼', s: '⎽', t: '├', u: '┤', v: '┴', w: '┬', x: '│', y: '≤', z: '≥', '{': 'π', '|': '≠', '}': '£', '~': '·',
  };
  const BOLD = 1, DIM = 2, ITALIC = 4, UNDER = 8, BLINK = 16, INVERSE = 32, HIDDEN = 64, STRIKE = 128;

  function charWidth(cp) {
    if (cp < 0x300) return 1;
    if ((cp >= 0x300 && cp <= 0x36f) || (cp >= 0x200b && cp <= 0x200f) || (cp >= 0xfe00 && cp <= 0xfe0f) ||
        (cp >= 0x1ab0 && cp <= 0x1aff) || (cp >= 0x20d0 && cp <= 0x20ff) || cp === 0x200d) return 0;
    if ((cp >= 0x1100 && cp <= 0x115f) || (cp >= 0x2e80 && cp <= 0xa4cf && cp !== 0x303f) ||
        (cp >= 0xac00 && cp <= 0xd7a3) || (cp >= 0xf900 && cp <= 0xfaff) || (cp >= 0xfe30 && cp <= 0xfe4f) ||
        (cp >= 0xff00 && cp <= 0xff60) || (cp >= 0xffe0 && cp <= 0xffe6) || (cp >= 0x1f300 && cp <= 0x1f64f) ||
        (cp >= 0x1f900 && cp <= 0x1f9ff) || (cp >= 0x1f680 && cp <= 0x1f6ff) || (cp >= 0x20000 && cp <= 0x3fffd)) return 2;
    return 1;
  }

  const blank = (attr) => ({ ch: ' ', fg: attr.fg, bg: attr.bg, fl: attr.fl, w: 1 });
  const esc = s => s.replace(/[&<>]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));

  class Term {
    /**
     * opts.pinTop: rows kept above the scrollback (console status header).
     * opts.onData(str): bytes to send for keyboard input / device reports (shell mode).
     * opts.keyboard: true routes keys to onData.
     * opts.convertEol: LF also returns the carriage (raw stream without a TTY's onlcr).
     */
    constructor(host, opts = {}) {
      this.opts = opts;
      this.host = host;
      this.pinTop = opts.pinTop || 0;
      this.maxScrollback = opts.scrollback || 5000;
      host.classList.add('term');
      host.innerHTML = '';
      this.headEl = document.createElement('div');
      this.headEl.className = 'term-head';
      this.viewEl = document.createElement('div');
      this.viewEl.className = 'term-view';
      this.sbEl = document.createElement('div');
      this.scrEl = document.createElement('div');
      this.viewEl.append(this.sbEl, this.scrEl);
      this.measureEl = document.createElement('span');
      this.measureEl.className = 'term-measure';
      this.measureEl.textContent = 'WWWWWWWWWW';
      host.append(this.headEl, this.viewEl, this.measureEl);
      this.input = document.createElement('textarea');
      this.input.className = 'term-input';
      this.input.setAttribute('autocapitalize', 'off');
      this.input.setAttribute('autocomplete', 'off');
      this.input.setAttribute('autocorrect', 'off');
      this.input.spellcheck = false;
      host.appendChild(this.input);
      this.decoder = new TextDecoder('utf-8');
      this.rows = 24; this.cols = 80;
      this.follow = true;
      this.viewEl.addEventListener('scroll', () => {
        const v = this.viewEl;
        this.follow = v.scrollTop + v.clientHeight >= v.scrollHeight - 4;
      });
      this.reset();
      if (opts.keyboard) this.bindKeys();
      this.ro = new ResizeObserver(() => this.fit());
      this.ro.observe(host);
      this.fit();
    }

    reset() {
      this.attr = { fg: -1, bg: -1, fl: 0 };
      this.main = this.makeScreen(this.rows);
      this.alt = null;
      this.screen = this.main;
      this.cx = 0; this.cy = 0;
      this.top = 0; this.bottom = this.rows - 1;
      this.saved = null;
      this.wrapPending = false;
      this.autowrap = true;
      this.insert = false;
      this.appCursor = false;
      this.bracketed = false;
      this.cursorOn = true;
      this.g0 = 'B'; this.g1 = 'B'; this.gl = 0;
      this.state = 0; this.params = ''; this.osc = '';
      this.dirty = new Set();
      for (let i = 0; i < this.rows; i++) this.dirty.add(i);
      this.sbEl.innerHTML = '';
      this.sbCount = 0;
      this.schedule();
    }

    makeScreen(n) {
      const s = [];
      for (let i = 0; i < n; i++) s.push(this.makeLine());
      return s;
    }
    makeLine() {
      const l = [];
      for (let i = 0; i < this.cols; i++) l.push(blank(this.attr || { fg: -1, bg: -1, fl: 0 }));
      return l;
    }

    // ---- sizing ----
    cellSize() {
      const r = this.measureEl.getBoundingClientRect();
      return { w: r.width / 10 || 8, h: r.height || 16 };
    }
    fit() {
      const { w, h } = this.cellSize();
      const box = this.host.getBoundingClientRect();
      if (!box.width || !box.height) return;
      const cols = Math.max(20, Math.floor((box.width - 12) / w));
      const rows = Math.max(this.pinTop + 4, Math.floor((box.height - 8) / h));
      if (cols === this.cols && rows === this.rows) return;
      this.resize(rows, cols);
      if (this.opts.onResize) this.opts.onResize(rows, cols);
    }
    resize(rows, cols) {
      const fix = (scr) => {
        for (const l of scr) {
          while (l.length < cols) l.push(blank({ fg: -1, bg: -1, fl: 0 }));
          l.length = cols;
        }
        while (scr.length < rows) scr.push(this.lineOf(cols));
        while (scr.length > rows) {
          const l = scr.shift();
          if (scr === this.main) this.pushScrollback(l);
        }
        return scr;
      };
      const shrinkBy = Math.max(0, this.rows - rows);
      this.cols = cols;
      this.main = fix(this.main);
      if (this.alt) this.alt = fix(this.alt);
      this.screen = this.alt && this.screen !== this.main ? this.alt : this.main;
      this.cy = Math.max(0, Math.min(rows - 1, this.cy - shrinkBy));
      this.rows = rows;
      this.cx = Math.min(this.cx, cols - 1);
      this.top = 0; this.bottom = rows - 1;
      this.dirtyAll();
    }
    lineOf(cols) {
      const l = [];
      for (let i = 0; i < cols; i++) l.push(blank({ fg: -1, bg: -1, fl: 0 }));
      return l;
    }

    // ---- output ----
    write(data) {
      const s = typeof data === 'string' ? data : this.decoder.decode(data, { stream: true });
      for (const ch of s) this.feed(ch);
      this.schedule();
    }

    feed(ch) {
      const c = ch.codePointAt(0);
      switch (this.state) {
        case 0:
          if (c === 0x1b) { this.state = 1; return; }
          if (c < 0x20 || c === 0x7f) { this.control(c); return; }
          this.print(ch, c);
          return;
        case 1: // ESC
          this.escape(ch);
          return;
        case 2: // CSI
          if (c >= 0x40 && c <= 0x7e) { this.csi(ch); this.state = 0; return; }
          if (c === 0x18 || c === 0x1a) { this.state = 0; return; }
          if (c === 0x1b) { this.state = 1; return; }
          this.params += ch;
          if (this.params.length > 64) this.state = 0;
          return;
        case 3: // OSC
          if (c === 0x07) { this.oscDone(); this.state = 0; return; }
          if (c === 0x1b) { this.state = 4; return; }
          if (this.osc.length < 512) this.osc += ch;
          return;
        case 4: // OSC ESC
          this.oscDone();
          this.state = ch === '\\' ? 0 : 1;
          if (this.state === 1) this.escape(ch);
          return;
        case 5: // charset designation
          if (this.charsetSlot === 0) this.g0 = ch; else this.g1 = ch;
          this.state = 0;
          return;
        case 6: // DCS / SOS / PM / APC: swallow until ST
          if (c === 0x1b) this.state = 7;
          else if (c === 0x07) this.state = 0;
          return;
        case 7:
          this.state = ch === '\\' ? 0 : 6;
          return;
      }
    }

    oscDone() {
      const m = /^(\d+);(.*)$/s.exec(this.osc);
      if (m && (m[1] === '0' || m[1] === '2') && this.opts.onTitle) this.opts.onTitle(m[2]);
      this.osc = '';
    }

    control(c) {
      switch (c) {
        case 0x07: break;
        case 0x08: if (this.cx > 0) this.cx--; this.wrapPending = false; break;
        case 0x09: this.cx = Math.min(this.cols - 1, (Math.floor(this.cx / 8) + 1) * 8); break;
        case 0x0a: case 0x0b: case 0x0c: if (this.opts.convertEol) this.cx = 0; this.lineFeed(); break;
        case 0x0d: this.cx = 0; this.wrapPending = false; break;
        case 0x0e: this.gl = 1; break;
        case 0x0f: this.gl = 0; break;
      }
    }

    print(ch, cp) {
      const set = this.gl ? this.g1 : this.g0;
      if (set === '0' && DEC_GRAPHICS[ch]) ch = DEC_GRAPHICS[ch];
      const w = charWidth(cp);
      if (w === 0) {
        const x = this.cx > 0 ? this.cx - 1 : 0;
        const cell = this.screen[this.cy][this.wrapPending ? this.cols - 1 : x];
        if (cell) cell.ch += ch;
        this.dirty.add(this.cy);
        return;
      }
      if (this.wrapPending) {
        if (this.autowrap) { this.cx = 0; this.lineFeed(); }
        this.wrapPending = false;
      }
      if (w === 2 && this.cx === this.cols - 1) {
        if (this.autowrap) { this.screen[this.cy][this.cx] = blank(this.attr); this.cx = 0; this.lineFeed(); }
      }
      const line = this.screen[this.cy];
      if (this.insert) {
        for (let i = 0; i < w; i++) { line.splice(this.cx, 0, blank(this.attr)); line.pop(); }
      }
      line[this.cx] = { ch, fg: this.attr.fg, bg: this.attr.bg, fl: this.attr.fl, w };
      if (w === 2 && this.cx + 1 < this.cols) line[this.cx + 1] = { ch: '', fg: this.attr.fg, bg: this.attr.bg, fl: this.attr.fl, w: 0 };
      this.dirty.add(this.cy);
      if (this.cx + w >= this.cols) { this.cx = this.cols - 1; this.wrapPending = true; }
      else this.cx += w;
    }

    lineFeed() {
      this.wrapPending = false;
      if (this.cy === this.bottom) this.scrollUp(1);
      else if (this.cy < this.rows - 1) this.cy++;
    }

    pushScrollback(line) {
      const div = document.createElement('div');
      div.className = 'term-row';
      div.innerHTML = this.renderLine(line, -1);
      this.sbEl.appendChild(div);
      this.sbCount++;
      if (this.sbCount > this.maxScrollback) { this.sbEl.firstChild.remove(); this.sbCount--; }
    }

    scrollUp(n) {
      for (let i = 0; i < n; i++) {
        const gone = this.screen.splice(this.top, 1)[0];
        this.screen.splice(this.bottom, 0, this.makeBlankLine());
        // lines leaving the top of the (pinned-header) region go to scrollback
        if (this.screen === this.main && this.top <= this.pinTop && this.bottom === this.rows - 1) this.pushScrollback(gone);
      }
      for (let y = this.top; y <= this.bottom; y++) this.dirty.add(y);
    }
    scrollDown(n) {
      for (let i = 0; i < n; i++) {
        this.screen.splice(this.bottom, 1);
        this.screen.splice(this.top, 0, this.makeBlankLine());
      }
      for (let y = this.top; y <= this.bottom; y++) this.dirty.add(y);
    }
    makeBlankLine() {
      const l = [];
      const a = { fg: -1, bg: this.attr.bg, fl: 0 };
      for (let i = 0; i < this.cols; i++) l.push(blank(a));
      return l;
    }

    escape(ch) {
      this.state = 0;
      switch (ch) {
        case '[': this.state = 2; this.params = ''; return;
        case ']': this.state = 3; this.osc = ''; return;
        case 'P': case 'X': case '^': case '_': this.state = 6; return;
        case '(': this.state = 5; this.charsetSlot = 0; return;
        case ')': this.state = 5; this.charsetSlot = 1; return;
        case '*': case '+': case '-': case '.': this.state = 5; this.charsetSlot = 2; return;
        case '7': this.saveCursor(); return;
        case '8': this.restoreCursor(); return;
        case 'D': this.lineFeed(); return;
        case 'E': this.cx = 0; this.lineFeed(); return;
        case 'M':
          if (this.cy === this.top) this.scrollDown(1); else if (this.cy > 0) this.cy--;
          return;
        case 'c': this.reset(); return;
        case '=': case '>': return;
        case '#': this.state = 6; return;
      }
    }

    saveCursor() {
      this.saved = { cx: this.cx, cy: this.cy, attr: { ...this.attr }, g0: this.g0, g1: this.g1, gl: this.gl };
    }
    restoreCursor() {
      const s = this.saved;
      if (!s) { this.cx = 0; this.cy = 0; return; }
      this.cx = Math.min(s.cx, this.cols - 1); this.cy = Math.min(s.cy, this.rows - 1);
      this.attr = { ...s.attr }; this.g0 = s.g0; this.g1 = s.g1; this.gl = s.gl;
      this.wrapPending = false;
    }

    csi(final) {
      let p = this.params;
      let priv = '';
      if (p && '?>=<'.includes(p[0])) { priv = p[0]; p = p.slice(1); }
      const inter = p.replace(/[0-9;:]/g, '');
      p = p.replace(/[^0-9;:]/g, '');
      const args = p.split(';').map(x => (x === '' ? NaN : parseInt(x, 10)));
      const a = (i, d) => (Number.isNaN(args[i]) || args[i] === undefined || args[i] === 0 ? d : args[i]);
      const line = this.screen[this.cy];
      this.wrapPending = false;
      if (inter) {
        if (final === 'q' || final === 'p') return; // DECSCUSR / DECSTR etc.
      }
      switch (final) {
        case '@': { const n = Math.min(a(0, 1), this.cols - this.cx); for (let i = 0; i < n; i++) { line.splice(this.cx, 0, blank(this.attr)); line.pop(); } this.dirty.add(this.cy); break; }
        case 'A': this.cy = Math.max(this.cy < this.top ? 0 : this.top, this.cy - a(0, 1)); break;
        case 'B': case 'e': this.cy = Math.min(this.cy > this.bottom ? this.rows - 1 : this.bottom, this.cy + a(0, 1)); break;
        case 'C': case 'a': this.cx = Math.min(this.cols - 1, this.cx + a(0, 1)); break;
        case 'D': this.cx = Math.max(0, this.cx - a(0, 1)); break;
        case 'E': this.cx = 0; this.cy = Math.min(this.bottom, this.cy + a(0, 1)); break;
        case 'F': this.cx = 0; this.cy = Math.max(this.top, this.cy - a(0, 1)); break;
        case 'G': case '`': this.cx = Math.min(this.cols - 1, a(0, 1) - 1); break;
        case 'H': case 'f': this.cy = Math.min(this.rows - 1, a(0, 1) - 1); this.cx = Math.min(this.cols - 1, a(1, 1) - 1); break;
        case 'd': this.cy = Math.min(this.rows - 1, a(0, 1) - 1); break;
        case 'I': for (let i = 0; i < a(0, 1); i++) this.cx = Math.min(this.cols - 1, (Math.floor(this.cx / 8) + 1) * 8); break;
        case 'J': this.eraseDisplay(Number.isNaN(args[0]) ? 0 : args[0]); break;
        case 'K': this.eraseLine(Number.isNaN(args[0]) ? 0 : args[0]); break;
        case 'L': if (this.cy >= this.top && this.cy <= this.bottom) { const t = this.top; this.top = this.cy; this.scrollDown(a(0, 1)); this.top = t; } break;
        case 'M': if (this.cy >= this.top && this.cy <= this.bottom) { const t = this.top; this.top = this.cy; const ps = this.pinTop; this.pinTop = -1; this.scrollUp(a(0, 1)); this.pinTop = ps; this.top = t; } break;
        case 'P': { const n = Math.min(a(0, 1), this.cols - this.cx); line.splice(this.cx, n); for (let i = 0; i < n; i++) line.push(blank(this.attr)); this.dirty.add(this.cy); break; }
        case 'S': { const ps = this.pinTop; this.pinTop = -1; this.scrollUp(a(0, 1)); this.pinTop = ps; break; }
        case 'T': if (!priv) this.scrollDown(a(0, 1)); break;
        case 'X': { const n = Math.min(a(0, 1), this.cols - this.cx); for (let i = 0; i < n; i++) line[this.cx + i] = blank(this.attr); this.dirty.add(this.cy); break; }
        case 'b': { const prev = line[Math.max(0, this.cx - 1)]; for (let i = 0; i < a(0, 1); i++) this.print(prev.ch || ' ', (prev.ch || ' ').codePointAt(0)); break; }
        case 'c': if (!priv) this.reply('\x1b[?62;22c'); else if (priv === '>') this.reply('\x1b[>0;10;1c'); break;
        case 'n': if (args[0] === 6) this.reply(`\x1b[${this.cy + 1};${this.cx + 1}R`); else if (args[0] === 5) this.reply('\x1b[0n'); break;
        case 'h': case 'l': this.mode(priv, args, final === 'h'); break;
        case 'm': this.sgr(p); break;
        case 'r':
          if (!priv) {
            const t = a(0, 1) - 1, b = a(1, this.rows) - 1;
            if (t < b && b < this.rows) { this.top = t; this.bottom = b; this.cx = 0; this.cy = 0; }
          }
          break;
        case 's': if (!priv) this.saveCursor(); break;
        case 'u': if (!priv) this.restoreCursor(); break;
        case 't': break;
      }
    }

    reply(s) { if (this.opts.onData && this.opts.keyboard) this.opts.onData(s); }

    mode(priv, args, on) {
      for (const m of args) {
        if (priv === '?') {
          switch (m) {
            case 1: this.appCursor = on; break;
            case 7: this.autowrap = on; break;
            case 25: this.cursorOn = on; this.dirty.add(this.cy); break;
            case 47: case 1047: case 1049:
              if (on && this.screen === this.main) {
                if (m === 1049) this.saveCursor();
                this.alt = this.makeScreen(this.rows);
                this.screen = this.alt;
                this.viewEl.classList.add('alt');
              } else if (!on && this.screen !== this.main) {
                this.screen = this.main;
                this.alt = null;
                if (m === 1049) this.restoreCursor();
                this.viewEl.classList.remove('alt');
              }
              this.dirtyAll();
              break;
            case 2004: this.bracketed = on; break;
          }
        } else if (m === 4) this.insert = on;
      }
    }

    sgr(p) {
      const v = p === '' ? [0] : p.split(/[;:]/).map(x => (x === '' ? 0 : parseInt(x, 10)));
      for (let i = 0; i < v.length; i++) {
        const n = v[i];
        if (n === 0) this.attr = { fg: -1, bg: -1, fl: 0 };
        else if (n === 1) this.attr.fl |= BOLD;
        else if (n === 2) this.attr.fl |= DIM;
        else if (n === 3) this.attr.fl |= ITALIC;
        else if (n === 4) this.attr.fl |= UNDER;
        else if (n === 5 || n === 6) this.attr.fl |= BLINK;
        else if (n === 7) this.attr.fl |= INVERSE;
        else if (n === 8) this.attr.fl |= HIDDEN;
        else if (n === 9) this.attr.fl |= STRIKE;
        else if (n === 22) this.attr.fl &= ~(BOLD | DIM);
        else if (n === 23) this.attr.fl &= ~ITALIC;
        else if (n === 24) this.attr.fl &= ~UNDER;
        else if (n === 25) this.attr.fl &= ~BLINK;
        else if (n === 27) this.attr.fl &= ~INVERSE;
        else if (n === 28) this.attr.fl &= ~HIDDEN;
        else if (n === 29) this.attr.fl &= ~STRIKE;
        else if (n >= 30 && n <= 37) this.attr.fg = n - 30;
        else if (n === 39) this.attr.fg = -1;
        else if (n >= 40 && n <= 47) this.attr.bg = n - 40;
        else if (n === 49) this.attr.bg = -1;
        else if (n >= 90 && n <= 97) this.attr.fg = n - 90 + 8;
        else if (n >= 100 && n <= 107) this.attr.bg = n - 100 + 8;
        else if (n === 38 || n === 48) {
          let col = null;
          if (v[i + 1] === 5) { col = v[i + 2]; i += 2; }
          else if (v[i + 1] === 2) { col = `rgb(${v[i + 2] | 0},${v[i + 3] | 0},${v[i + 4] | 0})`; i += 4; }
          if (col !== null && col !== undefined) { if (n === 38) this.attr.fg = col; else this.attr.bg = col; }
        }
      }
    }

    eraseDisplay(m) {
      if (m === 0) {
        this.eraseLine(0);
        for (let y = this.cy + 1; y < this.rows; y++) { this.screen[y] = this.makeBlankLine(); this.dirty.add(y); }
      } else if (m === 1) {
        this.eraseLine(1);
        for (let y = 0; y < this.cy; y++) { this.screen[y] = this.makeBlankLine(); this.dirty.add(y); }
      } else if (m === 2 || m === 3) {
        for (let y = 0; y < this.rows; y++) { this.screen[y] = this.makeBlankLine(); this.dirty.add(y); }
        if (m === 3) { this.sbEl.innerHTML = ''; this.sbCount = 0; }
      }
    }
    eraseLine(m) {
      const l = this.screen[this.cy];
      const a = { fg: -1, bg: this.attr.bg, fl: 0 };
      const [s, e] = m === 0 ? [this.cx, this.cols] : m === 1 ? [0, this.cx + 1] : [0, this.cols];
      for (let x = s; x < e; x++) l[x] = blank(a);
      this.dirty.add(this.cy);
    }

    dirtyAll() { for (let i = 0; i < this.rows; i++) this.dirty.add(i); this.schedule(); }

    // ---- rendering ----
    schedule() {
      if (this.raf) return;
      this.raf = requestAnimationFrame(() => { this.raf = 0; this.render(); });
    }

    color(c, fallback) {
      if (c === -1) return fallback;
      if (typeof c === 'number') return PALETTE[c] || fallback;
      return c;
    }

    renderLine(line, cursorX) {
      let html = '';
      let run = '', key = null;
      const flush = () => {
        if (!run) return;
        html += key ? `<span style="${key}">${esc(run)}</span>` : esc(run);
        run = '';
      };
      for (let x = 0; x < line.length; x++) {
        const c = line[x];
        if (c.w === 0 && c.ch === '') continue;
        let fg = c.fg, bg = c.bg, fl = c.fl;
        if (fl & BOLD && typeof fg === 'number' && fg >= 0 && fg < 8) fg += 8;
        let style = '';
        let fgc = this.color(fg, ''), bgc = this.color(bg, '');
        if (fl & INVERSE) { const t = fgc || 'var(--term-fg)'; fgc = bgc || 'var(--term-bg)'; bgc = t; }
        if (x === cursorX) { style += 'outline:1px solid var(--term-cursor);background:var(--term-cursor);color:var(--term-bg);'; }
        else {
          if (fgc) style += `color:${fgc};`;
          if (bgc) style += `background:${bgc};`;
        }
        if (fl & BOLD) style += 'font-weight:700;';
        if (fl & DIM) style += 'opacity:.6;';
        if (fl & ITALIC) style += 'font-style:italic;';
        if (fl & (UNDER | STRIKE)) style += `text-decoration:${fl & UNDER ? 'underline ' : ''}${fl & STRIKE ? 'line-through' : ''};`;
        if (fl & HIDDEN) style += 'color:transparent;';
        if (c.w === 2) style += 'display:inline-block;width:2ch;';
        if (style !== key) { flush(); key = style; }
        run += c.ch || ' ';
      }
      flush();
      return html.replace(/(\s|&nbsp;)+$/, '') || ' ';
    }

    render() {
      const pin = Math.min(this.pinTop, this.rows);
      const usePin = pin > 0 && this.screen === this.main;
      // (re)build row elements when geometry changes
      const headN = usePin ? pin : 0;
      if (this.headEl.childElementCount !== headN || this.scrEl.childElementCount !== this.rows - headN) {
        this.headEl.innerHTML = '';
        this.scrEl.innerHTML = '';
        for (let y = 0; y < this.rows; y++) {
          const d = document.createElement('div');
          d.className = 'term-row';
          (y < headN ? this.headEl : this.scrEl).appendChild(d);
        }
        for (let y = 0; y < this.rows; y++) this.dirty.add(y);
      }
      const rowsEls = [...this.headEl.children, ...this.scrEl.children];
      if (this.lastCursorRow !== undefined && this.lastCursorRow !== this.cy) this.dirty.add(this.lastCursorRow);
      this.dirty.add(this.cy);
      const showCursor = this.cursorOn && this.focused && this.opts.keyboard;
      for (const y of this.dirty) {
        if (y >= this.rows || !rowsEls[y]) continue;
        rowsEls[y].innerHTML = this.renderLine(this.screen[y], showCursor && y === this.cy ? this.cx : -1);
      }
      this.lastCursorRow = this.cy;
      this.dirty.clear();
      if (this.follow) this.viewEl.scrollTop = this.viewEl.scrollHeight;
    }

    // ---- keyboard (shell mode) ----
    focus() { this.input.focus({ preventScroll: true }); }

    send(s) { if (this.opts.onData) this.opts.onData(s); }

    paste(text) {
      text = text.replace(/\r?\n/g, '\r');
      this.send(this.bracketed ? `\x1b[200~${text}\x1b[201~` : text);
    }

    bindKeys() {
      const inp = this.input;
      this.host.addEventListener('mouseup', () => {
        if (!String(window.getSelection() || '')) this.focus();
      });
      inp.addEventListener('focus', () => { this.focused = true; this.dirty.add(this.cy); this.schedule(); });
      inp.addEventListener('blur', () => { this.focused = false; this.dirty.add(this.cy); this.schedule(); });
      inp.addEventListener('paste', e => {
        e.preventDefault();
        this.paste(e.clipboardData.getData('text'));
      });
      inp.addEventListener('compositionend', e => { if (e.data) this.send(e.data); inp.value = ''; });
      inp.addEventListener('input', e => {
        if (e.isComposing) return;
        if (inp.value) { this.send(inp.value); inp.value = ''; }
      });
      inp.addEventListener('keydown', e => {
        if (e.isComposing) return;
        const seq = this.keySeq(e);
        if (seq === null) return;
        e.preventDefault();
        this.follow = true;
        this.send(seq);
      });
    }

    keySeq(e) {
      const mods = (e.shiftKey ? 1 : 0) | (e.altKey ? 2 : 0) | (e.ctrlKey ? 4 : 0);
      const csiMod = (n, final) => (mods ? `\x1b[${n};${mods + 1}${final}` : (n === 1 ? (this.appCursor ? `\x1bO${final}` : `\x1b[${final}`) : `\x1b[${n}${final}`));
      const tilde = n => (mods ? `\x1b[${n};${mods + 1}~` : `\x1b[${n}~`);
      if (e.metaKey) return null; // keep browser shortcuts (copy/paste)
      if (e.ctrlKey && e.shiftKey && (e.key === 'C' || e.key === 'V')) return null;
      switch (e.key) {
        case 'Enter': return e.altKey ? '\x1b\r' : '\r';
        case 'Backspace': return e.ctrlKey ? '\x08' : (e.altKey ? '\x1b\x7f' : '\x7f');
        case 'Tab': return e.shiftKey ? '\x1b[Z' : '\t';
        case 'Escape': return '\x1b';
        case 'ArrowUp': return csiMod(1, 'A');
        case 'ArrowDown': return csiMod(1, 'B');
        case 'ArrowRight': return csiMod(1, 'C');
        case 'ArrowLeft': return csiMod(1, 'D');
        case 'Home': return csiMod(1, 'H');
        case 'End': return csiMod(1, 'F');
        case 'PageUp': if (e.shiftKey) { this.viewEl.scrollTop -= this.viewEl.clientHeight * 0.9; this.follow = false; return ''; } return tilde(5);
        case 'PageDown': if (e.shiftKey) { this.viewEl.scrollTop += this.viewEl.clientHeight * 0.9; return ''; } return tilde(6);
        case 'Insert': return tilde(2);
        case 'Delete': return tilde(3);
        case 'F1': return mods ? `\x1b[1;${mods + 1}P` : '\x1bOP';
        case 'F2': return mods ? `\x1b[1;${mods + 1}Q` : '\x1bOQ';
        case 'F3': return mods ? `\x1b[1;${mods + 1}R` : '\x1bOR';
        case 'F4': return mods ? `\x1b[1;${mods + 1}S` : '\x1bOS';
        case 'F5': return tilde(15); case 'F6': return tilde(17); case 'F7': return tilde(18); case 'F8': return tilde(19);
        case 'F9': return tilde(20); case 'F10': return tilde(21); case 'F11': return null; case 'F12': return null;
      }
      if (e.key.length === 1 || /^\p{Extended_Pictographic}$/u.test(e.key)) {
        if (e.ctrlKey && !e.altKey) {
          const k = e.key.toLowerCase();
          if (k >= 'a' && k <= 'z') return String.fromCharCode(k.charCodeAt(0) - 96);
          const map = { ' ': '\x00', '@': '\x00', '[': '\x1b', '\\': '\x1c', ']': '\x1d', '^': '\x1e', '_': '\x1f', '/': '\x1f', '?': '\x7f', '2': '\x00', '3': '\x1b', '4': '\x1c', '5': '\x1d', '6': '\x1e', '7': '\x1f', '8': '\x7f' };
          return map[k] !== undefined ? map[k] : null;
        }
        if (e.altKey && !e.ctrlKey) return '\x1b' + e.key;
        return null; // printable: arrives through the input event (keeps IME / dead keys working)
      }
      return null;
    }

    text() {
      const lines = [];
      for (const d of this.sbEl.children) lines.push(d.textContent);
      for (const l of this.screen) lines.push(l.map(c => c.ch).join('').replace(/\s+$/, ''));
      return lines.join('\n');
    }

    clear() {
      this.sbEl.innerHTML = '';
      this.sbCount = 0;
      this.viewEl.scrollTop = this.viewEl.scrollHeight;
    }

    setFontSize(px) {
      this.host.style.setProperty('--term-font', px + 'px');
      this.fit();
      this.dirtyAll();
    }

    dispose() {
      this.ro.disconnect();
      cancelAnimationFrame(this.raf);
    }
  }

  window.BiaTerm = Term;
})();
