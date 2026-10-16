// bia web console: Telegram-style chats with the agent; for root also the live console,
// a root shell, a file manager, human-in-the-loop tasks, the display and certificate admin.
'use strict';

// ======================================================================
// utilities
// ======================================================================
const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];

function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v === null || v === undefined || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k === 'style') el.style.cssText = v;
      else if (k === 'html') el.innerHTML = v;
      else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
      else if (k in el && typeof v !== 'string') el[k] = v;
      else el.setAttribute(k, v === true ? '' : v);
    }
  }
  const add = (k) => {
    if (k === null || k === undefined || k === false) return;
    if (Array.isArray(k)) k.forEach(add);
    else el.appendChild(k instanceof Node ? k : document.createTextNode(String(k)));
  };
  kids.forEach(add);
  return el;
}

const ICONS = {
  menu: 'M4 6h16M4 12h16M4 18h16',
  search: '<circle cx="11" cy="11" r="7"/><path d="M20 20l-4-4"/>',
  send: 'M3.5 11.2 20 4l-7.2 16.5-2.3-7z',
  attach: 'M20.5 11.5l-8.6 8.6a5 5 0 0 1-7-7l8.6-8.6a3.4 3.4 0 0 1 4.8 4.8l-8.6 8.6a1.7 1.7 0 0 1-2.4-2.4l7.9-7.9',
  mic: '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3"/>',
  micOff: '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3M3 3l18 18"/>',
  phone: 'M6.6 3h3l1.6 4.6-2.1 1.3a11 11 0 0 0 6 6l1.3-2.1 4.6 1.6v3a2 2 0 0 1-2.2 2A17 17 0 0 1 4.6 5.2 2 2 0 0 1 6.6 3z',
  phoneOff: '<path d="M6.6 3h3l1.6 4.6-2.1 1.3a11 11 0 0 0 6 6l1.3-2.1 4.6 1.6v3a2 2 0 0 1-2.2 2A17 17 0 0 1 4.6 5.2 2 2 0 0 1 6.6 3z"/><path d="M3 3l18 18"/>',
  smile: '<circle cx="12" cy="12" r="9"/><path d="M8 14.5a5 5 0 0 0 8 0M9 9.5h.01M15 9.5h.01"/>',
  close: 'M6 6l12 12M18 6 6 18',
  reply: 'M10 7 4 12.5l6 5.5M4 12.5h10a6 6 0 0 1 6 6v1',
  edit: 'M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4',
  trash: 'M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3',
  pin: 'M12 17v5M8 3h8l-1.5 6.5L18 13H6l3.5-3.5z',
  unpin: 'M12 17v5M8 3h8l-1.5 6.5L18 13H6l3.5-3.5zM3 3l18 18',
  forward: 'M14 7l6 5.5-6 5.5M20 12.5H10a6 6 0 0 0-6 6v1',
  copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3"/>',
  check: 'M5 12.5l4.5 4.5L19 7',
  checks: 'M2 12.5 6.5 17 16 7M11 15.5l1.5 1.5L22 7',
  clock: '<circle cx="12" cy="12" r="8"/><path d="M12 8v4l2.5 2"/>',
  more: '<circle cx="12" cy="5" r="1.2" fill="currentColor"/><circle cx="12" cy="12" r="1.2" fill="currentColor"/><circle cx="12" cy="19" r="1.2" fill="currentColor"/>',
  back: 'M15 5l-7 7 7 7',
  down: 'M6 9l6 6 6-6',
  up: 'M6 15l6-6 6 6',
  file: 'M6 2.5h8l5 5V21.5H6zM14 2.5v5h5',
  folder: 'M3 6.5a1.5 1.5 0 0 1 1.5-1.5h5l2 2.5h8a1.5 1.5 0 0 1 1.5 1.5v9.5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5z',
  folderPlus: 'M3 6.5a1.5 1.5 0 0 1 1.5-1.5h5l2 2.5h8a1.5 1.5 0 0 1 1.5 1.5v9.5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5zM12 11v6M9 14h6',
  filePlus: 'M6 2.5h8l5 5V21.5H6zM14 2.5v5h5M12 11v6M9 14h6',
  download: 'M12 4v11M7 10l5 5 5-5M5 20h14',
  upload: 'M12 20V9M7 14l5-5 5 5M5 4h14',
  terminal: '<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M7 9l3 3-3 3M12.5 15H17"/>',
  console: '<rect x="3" y="4" width="18" height="12.5" rx="2"/><path d="M8.5 20.5h7M12 16.5v4M6.5 8.5h6M6.5 11.5h9"/>',
  tasks: '<rect x="4" y="4" width="16" height="16" rx="3.5"/><path d="M8.5 12l2.5 2.5 4.5-5"/>',
  display: '<rect x="2.5" y="4" width="19" height="13" rx="2"/><path d="M8 21h8M12 17v4"/>',
  settings: '<circle cx="12" cy="12" r="3.2"/><path d="M12 2.5v2.8M12 18.7v2.8M2.5 12h2.8M18.7 12h2.8M5.3 5.3l2 2M16.7 16.7l2 2M5.3 18.7l2-2M16.7 7.3l2-2"/>',
  plus: 'M12 5v14M5 12h14',
  stop: '<rect x="7" y="7" width="10" height="10" rx="2" fill="currentColor"/>',
  select: '<circle cx="12" cy="12" r="9"/><path d="M8 12.5l3 3 5-6"/>',
  refresh: 'M20 12a8 8 0 1 1-2.3-5.7M20 4v5h-5',
  home: 'M3.5 11 12 3.5l8.5 7.5M6 9.5V20h12V9.5',
  root: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5v9M8 12h8"/>',
  shield: 'M12 2.5 19.5 5.5v6c0 4.8-3.3 8.6-7.5 10-4.2-1.4-7.5-5.2-7.5-10v-6zM8.5 12l2.5 2.5 4.5-5',
  moon: 'M20 14.5A8.5 8.5 0 1 1 9.5 4a7 7 0 0 0 10.5 10.5z',
  bell: 'M6 16.5V11a6 6 0 0 1 12 0v5.5l2 2H4zM10 21a2 2 0 0 0 4 0',
  bellOff: 'M6 16.5V11a6 6 0 0 1 9.2-5M18 11v5.5l2 2H8M10 21a2 2 0 0 0 4 0M3 3l18 18',
  play: '<path d="M8 5.5v13l11-6.5z" fill="currentColor"/>',
  pause: '<path d="M7.5 5h3v14h-3zM13.5 5h3v14h-3z" fill="currentColor"/>',
  eye: 'M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
  link: 'M10 14a4 4 0 0 0 5.7 0l3.5-3.5a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3.5 3.5a4 4 0 0 0 5.7 5.7l1-1',
  key: '<circle cx="8" cy="15" r="4.5"/><path d="M11.3 11.7 20 3M16 7l3 3"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5h.01"/>',
  image: '<rect x="3" y="4" width="18" height="16" rx="2.5"/><circle cx="9" cy="10" r="2"/><path d="M21 16l-5-5-9 9"/>',
  video: '<rect x="2.5" y="5.5" width="14" height="13" rx="2.5"/><path d="M16.5 10.5l5-3v9l-5-3z"/>',
  audio: '<path d="M9 18V5.5l11-2V16"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="17.5" cy="16" r="2.5"/>',
  archive: '<rect x="3" y="4" width="18" height="5" rx="1.5"/><path d="M5 9v10.5h14V9M10 13h4"/>',
  code: 'M8.5 7 3.5 12l5 5M15.5 7l5 5-5 5',
  cut: '<circle cx="6" cy="6.5" r="2.8"/><circle cx="6" cy="17.5" r="2.8"/><path d="M8.3 8.2 20 19M8.3 15.8 20 5"/>',
  paste: '<rect x="5" y="4.5" width="14" height="17" rx="2"/><path d="M9 3h6v3H9z"/>',
  chat: 'M4 5h16v11.5H9L4 20.5z',
  user: '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>',
  logout: 'M15 4h4v16h-4M10 8l-4 4 4 4M6 12h11',
  hand: 'M8 13V5.5a1.5 1.5 0 0 1 3 0V11M11 10.5V4a1.5 1.5 0 0 1 3 0v6.5M14 10.5V5.5a1.5 1.5 0 0 1 3 0V14a7 7 0 0 1-7 7h-1a6 6 0 0 1-5-3l-2-3.5a1.5 1.5 0 0 1 2.5-1.6L8 15',
  poll: 'M5 20V10M12 20V4M19 20v-7',
  external: 'M14 4h6v6M20 4l-9 9M18 14v5.5H4.5V6H10',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
  fontUp: 'M4 18 9 6l5 12M5.8 14h6.4M17 8v6M14 11h6',
  fontDown: 'M4 18 9 6l5 12M5.8 14h6.4M14 11h6',
  eraser: 'M4 20h16M6.5 16.5 15 8l4 4-6.5 6.5zM12 11l4 4',
};
function ic(name, cls) {
  const d = ICONS[name] || ICONS.info;
  const body = d.trim().startsWith('<') ? d : `<path d="${d}"/>`;
  const t = document.createElement('template');
  t.innerHTML = `<svg class="i ${cls || ''}" viewBox="0 0 24 24" aria-hidden="true">${body}</svg>`;
  return t.content.firstChild;
}
function iconBtn(name, title, onclick, cls) {
  return h('button', { class: 'btn-icon ' + (cls || ''), title, 'aria-label': title, onclick }, ic(name));
}

// replaceChildren that skips null/false and flattens arrays (DOM would print "null")
function set(el, ...kids) {
  el.replaceChildren(...kids.flat(9).filter(k => k !== null && k !== undefined && k !== false));
  return el;
}

const esc = s => String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

function fmtBytes(n) {
  n = Number(n) || 0;
  if (n < 1024) return n + ' B';
  const u = ['KB', 'MB', 'GB', 'TB'];
  let i = -1;
  do { n /= 1024; i++; } while (n >= 1024 && i < u.length - 1);
  return (n < 10 ? n.toFixed(1) : Math.round(n)) + ' ' + u[i];
}
const pad2 = n => String(n).padStart(2, '0');
function fmtTime(ms) { const d = new Date(ms); return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`; }
function sameDay(a, b) { const x = new Date(a), y = new Date(b); return x.toDateString() === y.toDateString(); }
function fmtDay(ms) {
  const d = new Date(ms), now = new Date();
  if (sameDay(ms, now)) return 'Today';
  if (sameDay(ms, now.getTime() - 864e5)) return 'Yesterday';
  const opts = { weekday: 'long', day: 'numeric', month: 'long' };
  if (d.getFullYear() !== now.getFullYear()) opts.year = 'numeric';
  return d.toLocaleDateString(undefined, opts);
}
function fmtListTime(ms) {
  if (!ms) return '';
  const now = Date.now();
  if (sameDay(ms, now)) return fmtTime(ms);
  if (now - ms < 6 * 864e5) return new Date(ms).toLocaleDateString(undefined, { weekday: 'short' });
  return new Date(ms).toLocaleDateString(undefined, { day: '2-digit', month: '2-digit', year: '2-digit' });
}
function fmtDate(sec) { return sec ? new Date(sec * 1000).toLocaleString() : '—'; }
function fmtDur(s) { s = Math.max(0, Math.round(s || 0)); return `${Math.floor(s / 60)}:${pad2(s % 60)}`; }
function initials(s) { return (String(s || '?').trim()[0] || '?').toUpperCase(); }
function colorOf(s) { let x = 0; for (const c of String(s)) x = (x * 31 + c.charCodeAt(0)) >>> 0; return 'c' + (x % 7); }
function debounce(fn, ms) { let t; return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); }; }

function toast(text, err) {
  const t = h('div', { class: 'toast' + (err ? ' err' : '') }, text);
  $('#toasts').appendChild(t);
  setTimeout(() => t.remove(), err ? 5000 : 2600);
}
async function copyText(s, what) {
  try { await navigator.clipboard.writeText(s); toast((what || 'Text') + ' copied'); }
  catch {
    const ta = h('textarea', { style: 'position:fixed;opacity:0' }); ta.value = s; document.body.appendChild(ta); ta.select();
    try { document.execCommand('copy'); toast((what || 'Text') + ' copied'); } catch { toast('Copy failed', true); }
    ta.remove();
  }
}
function saveBlob(blob, name) {
  const a = h('a', { href: URL.createObjectURL(blob), download: name });
  document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 30000);
}
function download(url, name) {
  const a = h('a', { href: url, download: name || '' });
  document.body.appendChild(a); a.click(); a.remove();
}
function filenameOf(resp, fallback) {
  const cd = resp.headers.get('content-disposition') || '';
  const m = /filename\*=UTF-8''([^;]+)/i.exec(cd) || /filename="([^"]+)"/i.exec(cd);
  return m ? decodeURIComponent(m[1]) : fallback;
}

// ---- modal / menus ----
function modal({ title, body, buttons, cls, onClose, dismiss = true }) {
  return new Promise(resolve => {
    const back = h('div', { class: 'modal-back' });
    const close = (v) => { back.remove(); document.removeEventListener('keydown', key, true); if (onClose) onClose(v); resolve(v); };
    const foot = h('div', { class: 'mfoot' }, (buttons || []).map(b => h('button', {
      class: 'btn ' + (b.cls || 'flat'),
      onclick: async () => {
        if (b.onClick) { const r = await b.onClick(); if (r === false) return; close(r === undefined ? b.value : r); }
        else close(b.value);
      },
    }, b.text)));
    const m = h('div', { class: 'modal ' + (cls || '') },
      title ? h('div', { class: 'mhead' }, title) : null,
      h('div', { class: 'mbody' }, body), buttons && buttons.length ? foot : null);
    back.appendChild(m);
    back.addEventListener('mousedown', e => { if (e.target === back && dismiss) close(undefined); });
    const key = e => {
      if (e.key === 'Escape' && dismiss) { e.stopPropagation(); close(undefined); }
      if (e.key === 'Enter' && !e.shiftKey && e.target.tagName !== 'TEXTAREA') {
        const p = (buttons || []).find(b => b.primary);
        if (p) { e.preventDefault(); foot.children[(buttons || []).indexOf(p)].click(); }
      }
    };
    document.addEventListener('keydown', key, true);
    document.body.appendChild(back);
    m.close = close;
    const f = m.querySelector('input, textarea, select');
    if (f) setTimeout(() => f.focus(), 30);
  });
}
function confirmBox(text, okText = 'OK', danger = false) {
  return modal({
    title: text.title || null, body: h('div', null, text.body || text),
    buttons: [{ text: 'Cancel', value: false }, { text: okText, value: true, primary: true, cls: danger ? 'flat danger' : 'flat' }],
  }).then(v => !!v);
}
function promptBox(title, value = '', opts = {}) {
  const inp = h(opts.multiline ? 'textarea' : 'input', { class: 'input', value, placeholder: opts.placeholder || '', rows: opts.multiline ? 4 : null });
  inp.value = value;
  if (opts.select !== false) setTimeout(() => {
    inp.focus();
    const dot = opts.selectStem ? value.lastIndexOf('.') : -1;
    inp.setSelectionRange(0, dot > 0 ? dot : value.length);
  }, 40);
  return modal({
    title, body: h('div', null, opts.text ? h('p', { class: 'muted' }, opts.text) : null, inp),
    buttons: [{ text: 'Cancel', value: null }, { text: opts.ok || 'OK', primary: true, onClick: () => inp.value }],
  });
}

let openMenuEl = null;
function closeMenu() { if (openMenuEl) { openMenuEl.remove(); openMenuEl = null; } }
function menu(x, y, items, top) {
  closeMenu();
  const m = h('div', { class: 'menu', role: 'menu' });
  if (top) m.appendChild(top);
  for (const it of items) {
    if (!it) continue;
    if (it === '-') { m.appendChild(h('div', { class: 'sep' })); continue; }
    m.appendChild(h('div', {
      class: 'mi' + (it.danger ? ' danger' : ''), role: 'menuitem',
      onclick: (e) => { e.stopPropagation(); closeMenu(); it.onClick(); },
    }, it.icon ? ic(it.icon) : null, it.text));
  }
  document.body.appendChild(m);
  const r = m.getBoundingClientRect();
  m.style.left = Math.max(6, Math.min(x, innerWidth - r.width - 6)) + 'px';
  m.style.top = Math.max(6, Math.min(y, innerHeight - r.height - 6)) + 'px';
  openMenuEl = m;
  return m;
}
document.addEventListener('mousedown', e => { if (openMenuEl && !openMenuEl.contains(e.target)) closeMenu(); }, true);
document.addEventListener('keydown', e => { if (e.key === 'Escape' && openMenuEl) { closeMenu(); e.stopPropagation(); } }, true);
window.addEventListener('blur', closeMenu);

function onLongPress(el, fn) {
  let t = null, sx = 0, sy = 0;
  el.addEventListener('touchstart', e => {
    const p = e.touches[0]; sx = p.clientX; sy = p.clientY;
    t = setTimeout(() => { t = null; fn({ clientX: sx, clientY: sy, preventDefault() {}, target: e.target }); }, 480);
  }, { passive: true });
  const cancel = e => {
    if (!t) return;
    if (e.type === 'touchmove') { const p = e.touches[0]; if (Math.hypot(p.clientX - sx, p.clientY - sy) < 10) return; }
    clearTimeout(t); t = null;
  };
  el.addEventListener('touchmove', cancel, { passive: true });
  el.addEventListener('touchend', cancel);
  el.addEventListener('touchcancel', cancel);
}

// ---- HTTP ----
async function http(method, url, body, raw) {
  const opts = { method, credentials: 'include', headers: {} };
  if (body !== undefined) {
    if (body instanceof Blob || typeof body === 'string') opts.body = body;
    else { opts.body = JSON.stringify(body); opts.headers['Content-Type'] = 'application/json'; }
  }
  const r = await fetch(url, opts);
  if (raw) return r;
  if (!r.ok) throw new Error((await r.text().catch(() => '')) || `HTTP ${r.status}`);
  const ct = r.headers.get('content-type') || '';
  return ct.includes('json') ? r.json() : r.text();
}
const GET = (u) => http('GET', u);
const POST = (u, b) => http('POST', u, b === undefined ? {} : b);

function xhrUpload(url, blob, onProgress) {
  return new Promise((resolve, reject) => {
    const x = new XMLHttpRequest();
    x.open('POST', url);
    x.withCredentials = true;
    x.setRequestHeader('Content-Type', 'application/octet-stream');
    x.upload.onprogress = e => onProgress && onProgress(e.loaded, e.lengthComputable ? e.total : blob.size);
    x.onload = () => {
      if (x.status >= 200 && x.status < 300) { try { resolve(JSON.parse(x.responseText)); } catch { resolve({}); } }
      else { const e = new Error(x.responseText || 'HTTP ' + x.status); e.status = x.status; reject(e); }
    };
    x.onerror = () => reject(new Error('network error'));
    x.onabort = () => reject(new Error('cancelled'));
    x.send(blob);
    if (onProgress) onProgress(0, blob.size, x);
  });
}

// ======================================================================
// state
// ======================================================================
const S = {
  me: null, root: false,
  chats: new Map(),        // id → summary
  order: [],               // chat ids, most recent first
  cache: new Map(),        // id → {list, byId, moreBefore, moreAfter, loading}
  drafts: new Map(),       // chat → {id, text}
  typing: new Set(),
  pending: new Map(),      // chat → [{cid, text, files, progress, reply_to, kind}]
  cur: null,
  route: '',
  ws: null, wsOk: false, wsTry: 0, hellos: 0,
  acks: new Map(), cidSeq: 1,
  tasks: [], taskSeen: new Set(),
  status: null,
  reply: null, edit: null,
  selecting: false, sel: new Set(),
  prefs: loadPrefs(),
};
function loadPrefs() {
  try { return Object.assign({ theme: 'auto', fs: 15, notify: true, sound: true, enterSends: true, termFs: 14 }, JSON.parse(localStorage.getItem('bia.prefs') || '{}')); }
  catch { return { theme: 'auto', fs: 15, notify: true, sound: true, enterSends: true, termFs: 14 }; }
}
function savePrefs() { localStorage.setItem('bia.prefs', JSON.stringify(S.prefs)); applyPrefs(); }
function applyPrefs() {
  const light = S.prefs.theme === 'light' || (S.prefs.theme === 'auto' && matchMedia('(prefers-color-scheme: light)').matches);
  document.documentElement.classList.toggle('light', light);
  try { localStorage.setItem('bia.theme', S.prefs.theme); } catch {}
  document.documentElement.style.setProperty('--fs', S.prefs.fs + 'px');
  const meta = $('meta[name=theme-color]'); if (meta) meta.content = light ? '#ffffff' : '#17212b';
}
matchMedia('(prefers-color-scheme: light)').addEventListener('change', applyPrefs);
applyPrefs();

const chatTitle = id => (S.chats.get(id) || {}).title || (id === 'main' ? 'bia' : id);
const isMine = m => m.from !== 'bia';

// ======================================================================
// start: client certificate or onboarding
// ======================================================================
(async function boot() {
  let who = null;
  try { const r = await fetch('/v1/whoami', { credentials: 'include' }); if (r.ok) who = await r.json(); } catch {}
  if (who && who.identity) start(who);
  else showBoot();
})();

async function showBoot() {
  const box = $('#boot');
  box.hidden = false;
  let info = { root_issued: true, ca_sha256: '' };
  try { info = await GET('/v1/pki/bootstrap'); } catch {}
  const err = h('div', { class: 'err-line' });
  const fp = (info.ca_sha256 || '').toUpperCase().match(/.{1,2}/g);
  let p12 = null;

  const caStep = h('div', { class: 'step' },
    h('h2', null, h('span', { class: 'num' }, '1'), 'Trust this machine'),
    h('p', null, 'Download the bia certificate authority and trust it, so your browser recognises this machine without warnings. Check that the fingerprint matches the one shown on the machine (/web on its console).'),
    fp ? h('div', { class: 'fp' }, 'SHA-256 ' + fp.join(':')) : null,
    h('div', { class: 'actions' },
      h('button', { class: 'btn', onclick: () => { download('/v1/pki/ca', 'bia-ca.crt'); caStep.classList.add('done'); } }, ic('download'), 'CA certificate (.crt)'),
      h('button', { class: 'btn ghost', onclick: () => { download('/v1/pki/ca?format=der', 'bia-ca.cer'); caStep.classList.add('done'); } }, 'DER (.cer)')),
    howto('How to trust it', [
      ['macOS', 'Open bia-ca.crt → Keychain Access adds it to “login” → double-click “bia agent CA” → Trust → “Always Trust”.'],
      ['Windows', 'Open bia-ca.cer → Install Certificate → Current User → “Trusted Root Certification Authorities”.'],
      ['Linux (Chrome)', 'chrome://settings/certificates → Authorities → Import → bia-ca.crt → trust for websites. System-wide: copy to /usr/local/share/ca-certificates/ and run update-ca-certificates.'],
      ['Firefox', 'Settings → Privacy & Security → Certificates → View Certificates → Authorities → Import.'],
      ['iPhone / iPad', 'Open the .crt in Safari → Settings → Profile Downloaded → Install, then Settings → General → About → Certificate Trust Settings → enable bia agent CA.'],
      ['Android', 'Settings → Security → Encryption & credentials → Install a certificate → CA certificate.'],
    ]));

  const issueBtn = h('button', { class: 'btn' }, ic('key'), 'Issue root certificate');
  const dlBtn = h('button', { class: 'btn', hidden: true }, ic('download'), 'Download root.p12');
  issueBtn.onclick = async () => {
    err.textContent = ''; issueBtn.disabled = true;
    try {
      const r = await fetch('/v1/pki/bootstrap/root', { method: 'POST' });
      if (r.status === 409) { err.textContent = 'A root certificate was already issued. Use a pickup link from the machine.'; return; }
      if (!r.ok) throw new Error('HTTP ' + r.status);
      p12 = await r.blob();
      issueBtn.hidden = true; dlBtn.hidden = false;
      certStep.classList.add('done');
      saveBlob(p12, 'root.p12');
    } catch (e) { err.textContent = 'Could not issue: ' + e.message; }
    finally { issueBtn.disabled = false; }
  };
  dlBtn.onclick = () => p12 && saveBlob(p12, 'root.p12');

  const pickIn = h('input', { class: 'input', placeholder: 'https://…/v1/pki/pickup/… or token' });
  const pickBtn = h('button', { class: 'btn ghost' }, 'Get certificate');
  pickBtn.onclick = async () => {
    err.textContent = '';
    const v = pickIn.value.trim();
    const tok = (v.match(/pickup\/([A-Za-z0-9_-]+)/) || [null, v])[1];
    if (!tok || !/^[A-Za-z0-9_-]+$/.test(tok)) { err.textContent = 'Paste the pickup link bia gave you.'; return; }
    try {
      const r = await fetch('/v1/pki/pickup/' + tok);
      if (!r.ok) throw new Error(r.status === 404 ? 'link expired or already used' : 'HTTP ' + r.status);
      saveBlob(await r.blob(), filenameOf(r, 'bia.p12'));
      certStep.classList.add('done');
    } catch (e) { err.textContent = 'Pickup failed: ' + e.message; }
  };

  const certStep = h('div', { class: 'step' },
    h('h2', null, h('span', { class: 'num' }, '2'), 'Get your certificate'),
    info.root_issued
      ? h('p', null, 'Access needs a personal client certificate. Ask root (or bia on the machine: /pki issue <name>) for a one-time pickup link, valid 15 minutes.')
      : h('p', null, 'First-time setup: no root certificate exists yet. Issue it here — the file stays only in this tab until you download it. Its password is 12345678.'),
    !info.root_issued ? h('div', { class: 'actions' }, issueBtn, dlBtn) : null,
    h('div', { class: 'row', style: 'margin-top:10px' }, pickIn, pickBtn));

  const importStep = h('div', { class: 'step' },
    h('h2', null, h('span', { class: 'num' }, '3'), 'Import it and reload'),
    h('p', null, 'Import the .p12 file into your browser or OS (password 12345678), then reload this page and pick the certificate when asked.'),
    howto('How to import', [
      ['macOS', 'Double-click the .p12 → login keychain → password 12345678. Chrome and Safari use it; Firefox: Settings → Certificates → Your Certificates → Import.'],
      ['Windows', 'Double-click the .p12 → Current User → password 12345678 → automatic store.'],
      ['Linux (Chrome)', 'chrome://settings/certificates → Your certificates → Import.'],
      ['iPhone / iPad', 'Open the .p12 → Settings → Profile Downloaded → Install.'],
      ['Android', 'Settings → Security → Encryption & credentials → Install a certificate → VPN & app user certificate.'],
    ]),
    h('div', { class: 'actions', style: 'margin-top:10px' }, h('button', { class: 'btn', onclick: () => location.reload() }, ic('refresh'), 'Reload')));

  set(box, h('div', { class: 'wrap' },
    h('img', { class: 'logo', src: '/icon.svg', alt: '' }),
    h('h1', null, 'bia'),
    h('div', { class: 'sub' }, 'This machine talks to you over HTTPS with client certificates. Three steps to get in.'),
    caStep, certStep, importStep, err,
    h('p', { class: 'muted', style: 'text-align:center;font-size:13px' },
      h('a', { href: '/openapi.yaml', target: '_blank' }, 'OpenAPI'), ' · ',
      h('a', { href: '/skills/web-console/SKILL.md', target: '_blank' }, 'Skill for agents'), ' · ',
      h('a', { href: '/.well-known/bia.json', target: '_blank' }, 'Discovery'))));
}
function howto(title, rows) {
  return h('details', { class: 'howto' }, h('summary', null, title),
    h('ul', null, rows.map(([k, v]) => h('li', null, h('b', null, k + ': '), v))));
}

async function start(who) {
  S.me = who;
  S.root = !!who.root;
  $('#app').hidden = false;
  buildSide();
  connect();
  window.addEventListener('hashchange', route);
  route();
  if (S.root) { pollTasks(); setInterval(pollTasks, 4000); pollStatus(); setInterval(pollStatus, 5000); }
  if (who.days_left !== undefined && who.days_left < 30) toast(`Your certificate expires in ${who.days_left} days — renew it in Settings`, true);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) maybeRead(); updateTitle(); });
  window.addEventListener('focus', () => maybeRead());
  setInterval(() => { if (S.ws && S.ws.readyState === 1) S.ws.send('{"t":"ping"}'); }, 30000);
}

// ======================================================================
// websocket
// ======================================================================
function connect() {
  const ws = new WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws');
  S.ws = ws;
  let helloSeen = false;
  ws.onopen = () => { S.wsTry = 0; };
  ws.onmessage = e => {
    let v; try { v = JSON.parse(e.data); } catch { return; }
    if (v.t === 'hello') { helloSeen = true; onHello(v); return; }
    if (!helloSeen) return;
    onEvent(v);
  };
  ws.onclose = () => {
    S.wsOk = false; setConn();
    for (const [, a] of S.acks) a.reject(new Error('disconnected'));
    S.acks.clear();
    const delay = Math.min(10000, 800 * Math.pow(1.6, S.wsTry++));
    setTimeout(connect, delay);
  };
}

function act(t, payload = {}) {
  const cid = 'c' + (S.cidSeq++);
  const msg = Object.assign({ t, cid }, payload);
  if (!S.ws || S.ws.readyState !== 1) {
    return POST('/v1/chats/action', msg);
  }
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { S.acks.delete(cid); reject(new Error('timeout')); }, 30000);
    S.acks.set(cid, {
      resolve: (r) => { clearTimeout(timer); resolve(r); },
      reject: (e) => { clearTimeout(timer); reject(e); },
    });
    S.ws.send(JSON.stringify(msg));
  });
}

function onHello(v) {
  S.wsOk = true;
  S.chats.clear();
  for (const c of v.chats || []) S.chats.set(c.id, c);
  S.drafts.clear();
  for (const d of v.drafts || []) S.drafts.set(d.chat, { id: d.id, text: d.text });
  S.typing = new Set(v.typing || []);
  sortChats();
  setConn();
  renderChatList();
  // resync after a reconnect (events may have been missed)
  if (S.hellos++) {
    for (const id of [...S.cache.keys()]) if (id !== S.cur) S.cache.delete(id);
    if (S.cur && S.route.startsWith('chat/')) { S.cache.delete(S.cur); openChat(S.cur, { keepScroll: false }); }
  } else if (S.cur && S.route.startsWith('chat/')) { renderChatHead(); renderPinbar(); }
  updateTitle();
}

function onEvent(v) {
  switch (v.t) {
    case 'ack': {
      const a = S.acks.get(v.cid);
      if (!a) return;
      S.acks.delete(v.cid);
      if (v.ok) a.resolve(v.r); else a.reject(new Error(v.error || 'failed'));
      return;
    }
    case 'chat': {
      const prev = S.chats.get(v.c.id);
      S.chats.set(v.c.id, v.c);
      sortChats();
      renderChatList();
      if (S.cur === v.c.id && S.route.startsWith('chat/')) { renderChatHead(); renderPinbar(); }
      if (!prev || prev.unread !== v.c.unread) updateTitle();
      return;
    }
    case 'chat_del':
      S.chats.delete(v.id); S.cache.delete(v.id); sortChats(); renderChatList(); updateTitle();
      if (S.cur === v.id) go('chat/main');
      return;
    case 'msg': onMsg(v.chat, v.m); return;
    case 'ring': showRing(v); return;
    case 'ring_end': if (RING.chat === v.chat) stopRing(); return;
    case 'call': return;
    case 'upd': {
      const c = S.cache.get(v.chat);
      if (c && c.byId.has(v.m.id)) {
        const i = c.list.findIndex(x => x.id === v.m.id);
        c.list[i] = v.m; c.byId.set(v.m.id, v.m);
        if (S.cur === v.chat) rerenderRow(v.m.id);
      }
      return;
    }
    case 'del': {
      const c = S.cache.get(v.chat);
      if (c) {
        c.list = c.list.filter(m => !v.ids.includes(m.id));
        v.ids.forEach(id => c.byId.delete(id));
        if (S.cur === v.chat) { v.ids.forEach(id => S.sel.delete(id)); renderMessages({ keep: true }); }
      }
      return;
    }
    case 'read': {
      const c = S.cache.get(v.chat);
      if (!c) return;
      for (const id of v.ids) { const m = c.byId.get(id); if (m) { m.read = true; if (S.cur === v.chat) rerenderRow(id); } }
      return;
    }
    case 'clear': {
      const c = S.cache.get(v.chat);
      if (c) { c.list = []; c.byId.clear(); c.moreBefore = false; c.moreAfter = false; }
      if (S.cur === v.chat) renderMessages();
      return;
    }
    case 'typing':
      if (v.on) S.typing.add(v.chat); else S.typing.delete(v.chat);
      renderChatList();
      if (S.cur === v.chat) { renderChatHead(); renderTail(); updateSendBtn(); }
      return;
    case 'draft': {
      const d = S.drafts.get(v.chat);
      if (d && d.id === v.id) d.text += v.d;
      else S.drafts.set(v.chat, { id: v.id, text: v.d });
      if (S.cur === v.chat) scheduleDraft();
      return;
    }
    case 'draft_end':
      S.drafts.delete(v.chat);
      if (S.cur === v.chat) renderTail();
      return;
  }
}

function onMsg(chat, m) {
  const d = S.drafts.get(chat);
  if (d && d.id === m.id) S.drafts.delete(chat);
  // an optimistic bubble of ours becomes the real message
  const pend = S.pending.get(chat) || [];
  if (isMine(m)) {
    const i = pend.findIndex(p => p.sent && !p.matched && (p.text || '') === (m.text || '') && (p.files || []).length === (m.files || []).length);
    if (i >= 0) pend.splice(i, 1);
  }
  const c = S.cache.get(chat);
  if (c && !c.moreAfter && !c.byId.has(m.id)) {
    c.list.push(m); c.byId.set(m.id, m);
    if (S.cur === chat) appendRow(m);
  } else if (c && c.moreAfter && S.cur === chat) {
    S.jumpUnread = (S.jumpUnread || 0) + (isMine(m) ? 0 : 1); renderJump();
  }
  if (S.cur === chat) renderTail();
  if (!isMine(m)) notifyMsg(chat, m);
}

function sortChats() {
  S.order = [...S.chats.values()]
    .sort((a, b) => ((b.last && b.last.ts) || b.created) - ((a.last && a.last.ts) || a.created))
    .map(c => c.id);
  // main chat stays on top
  S.order = ['main', ...S.order.filter(x => x !== 'main')].filter(x => S.chats.has(x));
}

function setConn() {
  const el = $('#connState');
  if (el) { el.hidden = S.wsOk; }
  renderChatHead();
}

// ======================================================================
// notifications / title
// ======================================================================
let audioCtx = null;
function blip() {
  if (!S.prefs.sound) return;
  try {
    audioCtx = audioCtx || new AudioContext();
    const o = audioCtx.createOscillator(), g = audioCtx.createGain();
    o.type = 'sine'; o.frequency.value = 880;
    g.gain.setValueAtTime(0.0001, audioCtx.currentTime);
    g.gain.exponentialRampToValueAtTime(0.08, audioCtx.currentTime + 0.01);
    g.gain.exponentialRampToValueAtTime(0.0001, audioCtx.currentTime + 0.25);
    o.connect(g).connect(audioCtx.destination);
    o.start(); o.stop(audioCtx.currentTime + 0.26);
  } catch {}
}
function notifyMsg(chat, m) {
  const c = S.chats.get(chat);
  const visible = !document.hidden && S.cur === chat && S.route.startsWith('chat/');
  if (visible || (c && c.muted)) { if (visible) maybeRead(); return; }
  blip();
  if (S.prefs.notify && 'Notification' in window && Notification.permission === 'granted' && document.hidden) {
    try {
      const n = new Notification(chatTitle(chat), { body: snippet(m), icon: '/icon.svg', tag: 'bia-' + chat });
      n.onclick = () => { window.focus(); go('chat/' + chat); n.close(); };
    } catch {}
  }
}
function updateTitle() {
  let n = 0;
  for (const c of S.chats.values()) if (!c.muted) n += c.unread || 0;
  const t = S.tasks.length;
  document.title = (n || t ? `(${n + t}) ` : '') + 'bia';
}
// markdown → plain text for previews
function plain(t) {
  return String(t).replace(/```[\w+-]*\n?/g, '').replace(/\*\*|__|~~|\|\||`/g, '').replace(/\[([^\]]+)\]\([^)]+\)/g, '$1')
    .replace(/^#{1,6}\s+/gm, '').replace(/^\s*>\s?/gm, '').replace(/\s+/g, ' ').trim();
}
function snippet(m) {
  if (!m) return '';
  if (m.text && m.text.trim()) return plain(m.text).slice(0, 120);
  const f = (m.files || [])[0];
  if (f) return { photo: '🖼 Photo', video: '📹 Video', voice: '🎤 Voice message', audio: '🎵 ' + f.name }[f.kind] || '📄 ' + f.name;
  if (m.poll) return '📊 ' + m.poll.question;
  return '';
}

// ======================================================================
// router
// ======================================================================
function go(r) { if (location.hash.slice(1) !== r) location.hash = r; else route(); }
function route() {
  const r = decodeURIComponent(location.hash.slice(1));
  const prev = S.route;
  S.route = r;
  closeMenu();
  if (prev === 'display' && r !== 'display') { const f = $('#displayFrame'); if (f) f.src = 'about:blank'; }
  document.body.classList.toggle('in-view', r !== '');
  let viewKey = r.split(/[/=]/)[0];
  if (!r) {
    if (innerWidth > 760) { go('chat/main'); return; }
    viewKey = 'empty';
  }
  if (!S.root && ['console', 'shell', 'files', 'tasks', 'task', 'display'].includes(viewKey)) { go('chat/main'); return; }
  switch (viewKey) {
    case 'chat': openChat(r.slice(5) || 'main'); break;
    case 'console': showView('console', buildConsole); break;
    case 'shell': showView('shell', buildShell); break;
    case 'files': showView('files', buildFiles); filesNavigate(r.startsWith('files=') ? r.slice(6) : null); break;
    case 'tasks': showView('tasks', buildTasks); renderTasks(); break;
    case 'task': showView('task', buildTaskFrame); openTaskFrame(r.slice(5)); break;
    case 'display': showView('display', buildDisplay); $('#displayFrame').src = '/gui'; break;
    case 'settings': showView('settings', buildSettings); renderSettings(); break;
    default: showView('empty', () => h('section', { class: 'view chatbg', id: 'v-empty' }, h('div', { class: 'pill' }, 'Select a chat to start messaging')));
  }
  if (viewKey !== 'chat') S.cur = null;
  renderChatList();
}
function showView(key, build) {
  let v = $('#v-' + key);
  if (!v) { v = build(); v.id = 'v-' + key; $('#main').appendChild(v); }
  for (const el of $('#main').children) el.hidden = el !== v;
  return v;
}
function backBtn() { return iconBtn('back', 'Back', () => go(''), 'back-btn'); }

// ======================================================================
// sidebar
// ======================================================================
function buildSide() {
  const side = $('#side');
  const search = h('input', { id: 'gsearch', placeholder: 'Search', autocomplete: 'off', type: 'search' });
  search.addEventListener('input', debounce(() => globalSearch(search.value), 250));
  search.addEventListener('keydown', e => { if (e.key === 'Escape') { search.value = ''; globalSearch(''); search.blur(); } });
  side.append(
    h('div', { class: 'side-top' },
      iconBtn('menu', 'Menu', e => sideMenu(e)),
      h('div', { class: 'search' }, ic('search'), search)),
    h('div', { id: 'connState', class: 'side-label', style: 'color:var(--warn)', hidden: true }, 'Connecting…'),
    h('div', { class: 'side-scroll', id: 'sideScroll' },
      h('div', { id: 'searchRes', class: 'search-res', hidden: true }),
      h('div', { id: 'chatList' }),
      S.root ? h('div', { id: 'toolList' }) : null),
    h('button', { class: 'fab', title: 'New chat', onclick: newChat }, ic('edit')));
}

function sideMenu(e) {
  const r = e.currentTarget.getBoundingClientRect();
  const light = document.documentElement.classList.contains('light');
  menu(r.left, r.bottom + 4, [
    { icon: 'edit', text: 'New chat', onClick: newChat },
    S.root && { icon: 'console', text: 'Console', onClick: () => go('console') },
    S.root && { icon: 'terminal', text: 'Root shell', onClick: () => go('shell') },
    S.root && { icon: 'folder', text: 'Files', onClick: () => go('files') },
    S.root && { icon: 'tasks', text: 'Human tasks', onClick: () => go('tasks') },
    S.root && { icon: 'display', text: 'Display', onClick: () => go('display') },
    '-',
    { icon: light ? 'moon' : 'sun', text: light ? 'Night mode' : 'Day mode', onClick: () => { S.prefs.theme = light ? 'dark' : 'light'; savePrefs(); } },
    { icon: 'settings', text: 'Settings', onClick: () => go('settings') },
  ]);
}

async function newChat() {
  const t = await promptBox('New chat', '', { placeholder: 'Chat name', text: 'A separate conversation with bia (its own channel).', ok: 'Create' });
  if (t === null || t === undefined) return;
  try { const r = await act('chat_new', { title: t }); go('chat/' + r.id); }
  catch (e) { toast(e.message, true); }
}

function renderChatList() {
  const list = $('#chatList');
  if (!list) return;
  const active = S.route.startsWith('chat/') ? S.cur : null;
  set(list, ...S.order.map(id => {
    const c = S.chats.get(id);
    const typing = S.typing.has(id);
    const last = c.last;
    let prev;
    if (typing) prev = h('span', { class: 'cprev typing' }, 'typing…');
    else if (last) prev = h('span', { class: 'cprev' }, isMine(last) ? h('b', null, 'You: ') : null, last.service ? last.text : snippet(last));
    else prev = h('span', { class: 'cprev' }, id === 'main' ? 'Your conversation with bia' : 'No messages yet');
    const item = h('div', { class: 'citem' + (id === active ? ' active' : ''), onclick: () => go('chat/' + id) },
      h('div', { class: 'avatar ' + (id === 'main' ? '' : colorOf(id)) }, id === 'main' ? h('img', { src: '/icon.svg', style: 'width:100%;height:100%;border-radius:50%' }) : initials(c.title)),
      h('div', { class: 'cbody' },
        h('div', { class: 'ctop' }, h('span', { class: 'ctitle' }, c.title), c.muted ? ic('bellOff', 'mutei') : null,
          last && isMine(last) ? ic(last.read ? 'checks' : 'check', 'mutei') : null,
          h('span', { class: 'ctime' }, fmtListTime(last && last.ts))),
        h('div', { class: 'cbot' }, prev,
          (c.pinned || []).length > 0 && !c.unread ? ic('pin', 'mutei') : null,
          c.unread ? h('span', { class: 'badge' + (c.muted ? ' mute' : '') }, c.unread) : null)));
    item.addEventListener('contextmenu', e => { e.preventDefault(); chatMenu(e.clientX, e.clientY, id); });
    onLongPress(item, e => chatMenu(e.clientX, e.clientY, id));
    return item;
  }));
  const tools = $('#toolList');
  if (tools) {
    const tool = (r, icon, title, sub, badge) => h('div', { class: 'citem' + (S.route.split(/[/=]/)[0] === r ? ' active' : ''), onclick: () => go(r) },
      h('div', { class: 'avatar sm tool' }, ic(icon)),
      h('div', { class: 'cbody' }, h('div', { class: 'ctop' }, h('span', { class: 'ctitle' }, title)),
        h('div', { class: 'cbot' }, h('span', { class: 'cprev' }, sub), badge ? h('span', { class: 'badge warn' }, badge) : null)));
    const st = S.status;
    set(tools, 
      h('div', { class: 'side-label' }, 'Machine'),
      tool('console', 'console', 'Console', st && st.model ? `${/^(thinking|tool|talking)/.test(st.state || '') ? st.state.split(' ')[0] : 'idle'} · ${st.model}` : 'live agent console'),
      tool('shell', 'terminal', 'Root shell', 'bash on this machine'),
      tool('files', 'folder', 'Files', 'browse, upload, edit'),
      tool('tasks', 'hand', 'Human tasks', S.tasks.length ? S.tasks.map(t => t.title).join(', ') : 'nothing waiting', S.tasks.length || null),
      tool('display', 'display', 'Display', 'screen :0 over VNC'),
      tool('settings', 'settings', 'Settings', 'certificates, appearance'));
  }
}

function chatMenu(x, y, id) {
  const c = S.chats.get(id);
  menu(x, y, [
    { icon: c.muted ? 'bell' : 'bellOff', text: c.muted ? 'Unmute' : 'Mute', onClick: () => act('chat_mute', { chat: id, on: !c.muted }) },
    c.unread ? { icon: 'checks', text: 'Mark as read', onClick: () => act('read', { chat: id, upto: '' }) } : null,
    { icon: 'edit', text: 'Rename', onClick: () => renameChat(id) },
    { icon: 'eraser', text: 'Clear history', danger: true, onClick: () => clearChat(id) },
    id !== 'main' ? { icon: 'trash', text: 'Delete chat', danger: true, onClick: () => deleteChat(id) } : null,
  ]);
}
async function renameChat(id) {
  const t = await promptBox('Rename chat', chatTitle(id));
  if (t) act('chat_rename', { chat: id, title: t }).catch(e => toast(e.message, true));
}
async function clearChat(id) {
  if (await confirmBox(`Clear all messages in “${chatTitle(id)}”? bia keeps its own memory of the conversation.`, 'Clear', true))
    act('chat_clear', { chat: id }).catch(e => toast(e.message, true));
}
async function deleteChat(id) {
  if (await confirmBox(`Delete “${chatTitle(id)}” and all its messages?`, 'Delete', true))
    act('chat_delete', { chat: id }).catch(e => toast(e.message, true));
}

async function globalSearch(q) {
  const res = $('#searchRes'), list = $('#chatList'), tools = $('#toolList');
  q = q.trim();
  if (!q) { res.hidden = true; list.hidden = false; if (tools) tools.hidden = false; return; }
  res.hidden = false; list.hidden = true; if (tools) tools.hidden = true;
  const ql = q.toLowerCase();
  const chats = S.order.filter(id => chatTitle(id).toLowerCase().includes(ql));
  set(res, h('div', { class: 'side-label' }, 'Searching…'));
  let j = { results: [] };
  try { j = await GET('/v1/search?q=' + encodeURIComponent(q)); } catch {}
  set(res, 
    chats.length ? h('div', { class: 'side-label' }, 'Chats') : null,
    chats.map(id => h('div', { class: 'citem', onclick: () => go('chat/' + id) },
      h('div', { class: 'avatar sm ' + colorOf(id) }, initials(chatTitle(id))), h('div', { class: 'cbody ctitle' }, chatTitle(id)))),
    h('div', { class: 'side-label' }, j.results.length ? `Messages (${j.results.length})` : 'No messages found'),
    j.results.map(r => searchRow(r, q, () => jumpTo(r.chat, r.m.id))));
}
function searchRow(r, q, onclick) {
  const text = snippet(r.m);
  const i = text.toLowerCase().indexOf(q.toLowerCase());
  const body = i < 0 ? text : [text.slice(Math.max(0, i - 30), i), h('mark', null, text.slice(i, i + q.length)), text.slice(i + q.length)];
  return h('div', { class: 'sr', onclick },
    h('div', { class: 'st' }, h('span', null, (r.title || chatTitle(r.chat)) + ' · ' + (isMine(r.m) ? 'You' : 'bia')), h('span', null, fmtListTime(r.m.ts))),
    h('div', { class: 'sx' }, body));
}

// ======================================================================
// chat view
// ======================================================================
function buildChat() {
  const v = h('section', { class: 'view chatbg' });
  const head = h('div', { class: 'vhead', id: 'chatHead' });
  const msgs = h('div', { class: 'msgs', id: 'msgs' }, h('div', { class: 'msgs-inner', id: 'msgsIn' }));
  msgs.addEventListener('scroll', onMsgsScroll);
  msgs.addEventListener('click', onMsgsClick);
  const jump = h('button', { class: 'jump', id: 'jumpBtn', hidden: true, onclick: jumpBottom }, ic('down'));
  v.append(head, h('div', { id: 'pinbar' }), h('div', { id: 'taskbar' }), h('div', { id: 'chatSearch' }), msgs, jump, buildComposer());
  // drag & drop files onto the chat
  let depth = 0;
  const overlay = h('div', { class: 'drop-overlay', hidden: true }, h('div', { class: 'dz' }, ic('upload'), 'Drop files to send'));
  v.appendChild(overlay);
  v.addEventListener('dragenter', e => { if (hasFiles(e)) { e.preventDefault(); depth++; overlay.hidden = false; } });
  v.addEventListener('dragover', e => { if (hasFiles(e)) e.preventDefault(); });
  v.addEventListener('dragleave', () => { if (--depth <= 0) { depth = 0; overlay.hidden = true; } });
  v.addEventListener('drop', e => {
    if (!hasFiles(e)) return;
    e.preventDefault(); depth = 0; overlay.hidden = true;
    attachDialog([...e.dataTransfer.files]);
  });
  return v;
}
const hasFiles = e => e.dataTransfer && [...e.dataTransfer.types].includes('Files');

async function openChat(id, opts = {}) {
  if (!S.chats.has(id) && S.chats.size) { go('chat/main'); return; }
  const changed = S.cur !== id;
  if (changed) {
    saveComposerDraft();
    S.cur = id; S.reply = null; S.edit = null; S.selecting = false; S.sel.clear(); S.jumpUnread = 0;
    S.chatSearch = null; S.pinIdx = undefined;
  }
  showView('chat', buildChat);
  renderChatHead(); renderPinbar(); renderTaskbar(); renderChatSearch();
  if (changed) loadComposerDraft();
  renderComposerCtx();
  let c = S.cache.get(id);
  if (!c) {
    c = { list: [], byId: new Map(), moreBefore: false, moreAfter: false, loading: true };
    S.cache.set(id, c);
    renderMessages();
    try {
      const j = await GET(`/v1/chats/${encodeURIComponent(id)}/messages?limit=60`);
      c.list = j.messages; c.byId = new Map(j.messages.map(m => [m.id, m]));
      c.moreBefore = j.more_before; c.moreAfter = false;
    } catch (e) { toast('Could not load messages: ' + e.message, true); }
    c.loading = false;
    if (S.cur !== id) return;
    const sum = S.chats.get(id);
    S.unreadFrom = null;
    if (sum && sum.unread) {
      const first = c.list.find(m => m.from === 'bia' && !m.read && !m.service);
      if (first) S.unreadFrom = first.id;
    }
    renderMessages();
    if (S.unreadFrom) scrollToId(S.unreadFrom, false, 'start'); else scrollBottom();
  } else if (changed || opts.keepScroll === false) {
    renderMessages(); scrollBottom();
  }
  if (innerWidth > 760 && !('ontouchstart' in window)) $('#composerInput').focus({ preventScroll: true });
  maybeRead();
}

function renderChatHead() {
  const head = $('#chatHead');
  if (!head || !S.cur) return;
  if (S.selecting) {
    set(head, 
      iconBtn('close', 'Cancel', () => setSelecting(false)),
      h('div', { class: 'tbox title' }, `${S.sel.size} selected`),
      iconBtn('forward', 'Forward', () => forwardMsgs([...S.sel])),
      iconBtn('copy', 'Copy', copySelected),
      iconBtn('trash', 'Delete', () => deleteMsgs([...S.sel])));
    return;
  }
  const c = S.chats.get(S.cur) || { title: chatTitle(S.cur) };
  let sub, on = false;
  if (!S.wsOk) sub = 'connecting…';
  else if (S.typing.has(S.cur)) { sub = 'typing…'; on = true; }
  else if (S.root && S.status && /^(thinking|tool|talking|reflecting)/.test(S.status.state || '')) { sub = S.status.state.split(' ')[0] + '…'; on = true; }
  else sub = S.root && S.status && S.status.model ? `online · ${S.status.model}` : 'online';
  set(head, 
    backBtn(),
    h('div', { class: 'avatar xs ' + (S.cur === 'main' ? '' : colorOf(S.cur)) }, S.cur === 'main' ? h('img', { src: '/icon.svg', style: 'width:100%;height:100%;border-radius:50%' }) : initials(c.title)),
    h('div', { class: 'tbox' }, h('div', { class: 'title ellipsis' }, c.title), h('div', { class: 'subtitle' + (on ? ' on' : '') }, sub)),
    iconBtn('phone', 'Call bia', () => startCall(S.cur)),
    iconBtn('search', 'Search in chat', () => { S.chatSearch = S.chatSearch ? null : { q: '' }; renderChatSearch(); }),
    S.root && S.typing.has(S.cur) ? iconBtn('stop', 'Stop generating', () => act('stop', { chat: S.cur })) : null,
    iconBtn('more', 'More', e => {
      const r = e.currentTarget.getBoundingClientRect();
      menu(r.right - 220, r.bottom + 4, [
        { icon: 'select', text: 'Select messages', onClick: () => setSelecting(true) },
        { icon: c.muted ? 'bell' : 'bellOff', text: c.muted ? 'Unmute' : 'Mute', onClick: () => act('chat_mute', { chat: S.cur, on: !c.muted }) },
        { icon: 'edit', text: 'Rename', onClick: () => renameChat(S.cur) },
        { icon: 'download', text: 'Export as text', onClick: () => exportChat(S.cur) },
        '-',
        { icon: 'eraser', text: 'Clear history', danger: true, onClick: () => clearChat(S.cur) },
        S.cur !== 'main' ? { icon: 'trash', text: 'Delete chat', danger: true, onClick: () => deleteChat(S.cur) } : null,
      ]);
    }));
}

async function exportChat(id) {
  const all = [];
  let before = '';
  for (let i = 0; i < 100; i++) {
    const j = await GET(`/v1/chats/${encodeURIComponent(id)}/messages?limit=200${before ? '&before=' + before : ''}`);
    all.unshift(...j.messages);
    if (!j.more_before || !j.messages.length) break;
    before = j.messages[0].id;
  }
  const lines = all.map(m => `[${new Date(m.ts).toLocaleString()}] ${m.from}: ${m.text || ''}${(m.files || []).map(f => `\n  [${f.kind}] ${f.name}`).join('')}`);
  saveBlob(new Blob([lines.join('\n') + '\n'], { type: 'text/plain' }), `bia-${chatTitle(id)}.txt`);
}

// ---- pinned bar ----
function renderPinbar() {
  const el = $('#pinbar');
  if (!el) return;
  const c = S.chats.get(S.cur);
  const ids = (c && c.pinned) || [];
  if (!ids.length) { set(el); return; }
  if (S.pinIdx === undefined || S.pinIdx >= ids.length) S.pinIdx = ids.length - 1;
  const id = ids[S.pinIdx];
  const cache = S.cache.get(S.cur);
  const m = cache && cache.byId.get(id);
  set(el, h('div', {
    class: 'pinbar', onclick: () => {
      jumpTo(S.cur, id);
      S.pinIdx = (S.pinIdx - 1 + ids.length) % ids.length;
      renderPinbar();
    },
  },
  h('div', { class: 'pline' }),
  h('div', { class: 'grow' },
    h('div', { class: 'ptitle' }, ids.length > 1 ? `Pinned message #${S.pinIdx + 1}` : 'Pinned message'),
    h('div', { class: 'ptext' }, m ? snippet(m) : '…')),
  iconBtn('unpin', 'Unpin', e => { e.stopPropagation(); act('pin', { chat: S.cur, id, on: false }); })));
  if (!m) {
    GET(`/v1/chats/${encodeURIComponent(S.cur)}/messages?around=${id}&limit=1`).then(j => {
      const x = j.messages.find(q => q.id === id);
      if (x && $('.ptext', el)) $('.ptext', el).textContent = snippet(x);
    }).catch(() => {});
  }
}

function renderTaskbar() {
  const el = $('#taskbar');
  if (!el) return;
  if (!S.root || !S.tasks.length) { set(el); return; }
  const t = S.tasks[0];
  set(el, h('div', { class: 'taskbar' }, ic('hand'),
    h('div', { class: 'grow ellipsis' }, h('b', null, 'bia needs you: '), t.title + (S.tasks.length > 1 ? ` (+${S.tasks.length - 1})` : '')),
    h('button', { class: 'btn small', onclick: () => go('task/' + t.id) }, 'Open')));
}

// ---- in-chat search ----
function renderChatSearch() {
  const el = $('#chatSearch');
  if (!el) return;
  if (!S.chatSearch) { set(el); return; }
  const inp = h('input', { class: 'input', placeholder: 'Search in this chat', value: S.chatSearch.q });
  const res = h('div', { class: 'results search-res', hidden: true });
  const run = debounce(async () => {
    const q = inp.value.trim();
    S.chatSearch.q = q;
    if (!q) { res.hidden = true; return; }
    const j = await GET(`/v1/search?chat=${encodeURIComponent(S.cur)}&q=${encodeURIComponent(q)}`).catch(() => ({ results: [] }));
    res.hidden = false;
    set(res, j.results.length ? j.results.map(r => searchRow(r, q, () => { res.hidden = true; jumpTo(S.cur, r.m.id); })) : h('div', { class: 'muted', style: 'padding:10px' }, 'Nothing found'));
  }, 250);
  inp.addEventListener('input', run);
  inp.addEventListener('keydown', e => { if (e.key === 'Escape') { S.chatSearch = null; renderChatSearch(); } });
  set(el, h('div', { class: 'chat-search' }, ic('search'), inp, iconBtn('close', 'Close', () => { S.chatSearch = null; renderChatSearch(); })), res);
  setTimeout(() => inp.focus(), 20);
}

// ---- message list ----
function groupInfo(list, i) {
  const m = list[i], p = list[i - 1], n = list[i + 1];
  const same = (a, b) => a && b && !a.service && !b.service && a.from === b.from && Math.abs(b.ts - a.ts) < 5 * 60e3 && sameDay(a.ts, b.ts);
  return { first: !same(p, m), last: !same(m, n), newDay: !p || !sameDay(p.ts, m.ts) };
}

function renderMessages(opts = {}) {
  const box = $('#msgsIn');
  if (!box || !S.cur) return;
  const c = S.cache.get(S.cur);
  const sc = $('#msgs');
  const anchor = opts.keep ? firstVisibleRow() : null;
  const frag = document.createDocumentFragment();
  if (!c || c.loading) { set(box, h('div', { class: 'loading-more' }, 'Loading…')); return; }
  if (c.moreBefore) frag.appendChild(h('div', { class: 'loading-more', id: 'moreBefore' }, ' '));
  if (!c.list.length && !c.moreBefore) {
    frag.appendChild(h('div', { class: 'service', style: 'margin-top:30vh' },
      S.cur === 'main' ? 'Say hi to bia. Messages, files, voice notes — everything goes here.' : 'No messages here yet'));
  }
  c.list.forEach((m, i) => {
    const g = groupInfo(c.list, i);
    if (g.newDay) frag.appendChild(h('div', { class: 'daysep' }, fmtDay(m.ts)));
    if (S.unreadFrom === m.id) frag.appendChild(h('div', { class: 'service', id: 'unreadSep' }, 'Unread messages'));
    frag.appendChild(buildRow(m, g));
  });
  frag.appendChild(h('div', { id: 'tail' }));
  set(box, frag);
  renderTail();
  if (anchor) {
    const el = box.querySelector(`[data-id="${anchor.id}"]`);
    if (el) sc.scrollTop += el.getBoundingClientRect().top - anchor.top;
  }
  renderJump();
}
function firstVisibleRow() {
  const sc = $('#msgs'), top = sc.getBoundingClientRect().top;
  for (const r of $$('.mrow[data-id]', sc)) {
    const b = r.getBoundingClientRect();
    if (b.bottom > top) return { id: r.dataset.id, top: b.top };
  }
  return null;
}

function appendRow(m) {
  const c = S.cache.get(S.cur);
  const box = $('#msgsIn');
  if (!box || !c) return;
  const atBottom = nearBottom();
  const i = c.list.length - 1;
  const g = groupInfo(c.list, i);
  // the previous row may lose its tail
  if (i > 0) rerenderRow(c.list[i - 1].id);
  const tail = $('#tail');
  const placeholder = box.querySelector('.service[style]');
  if (placeholder && c.list.length === 1) placeholder.remove();
  if (g.newDay) box.insertBefore(h('div', { class: 'daysep' }, fmtDay(m.ts)), tail);
  box.insertBefore(buildRow(m, g), tail);
  if (atBottom || isMine(m)) scrollBottom(true);
  else { S.jumpUnread = (S.jumpUnread || 0) + 1; renderJump(); }
  maybeRead();
}

function rerenderRow(id) {
  const c = S.cache.get(S.cur);
  const old = $(`#msgsIn .mrow[data-id="${CSS.escape(id)}"]`);
  if (!c || !old) return;
  const i = c.list.findIndex(m => m.id === id);
  if (i < 0) return;
  old.replaceWith(buildRow(c.list[i], groupInfo(c.list, i)));
}

// tail: pending uploads/sends, live draft from bia, typing dots
function renderTail() {
  const tail = $('#tail');
  if (!tail) return;
  const atBottom = nearBottom();
  const kids = [];
  for (const p of S.pending.get(S.cur) || []) kids.push(buildPendingRow(p));
  const d = S.drafts.get(S.cur);
  if (d && d.text.trim()) kids.push(buildRow({ id: d.id, from: 'bia', ts: Date.now(), text: d.text }, { first: true, last: true }, true));
  else if (S.typing.has(S.cur)) kids.push(h('div', { class: 'mrow in gap last' }, h('div', { class: 'bcol' }, h('div', { class: 'bubble typing-bubble' }, h('span', { class: 'dots' }, h('i'), h('i'), h('i'))))));
  set(tail, ...kids);
  if (atBottom) scrollBottom();
}
let draftRaf = 0;
function scheduleDraft() {
  if (draftRaf) return;
  draftRaf = requestAnimationFrame(() => { draftRaf = 0; renderTail(); });
}

function nearBottom() { const s = $('#msgs'); return !s || s.scrollTop + s.clientHeight >= s.scrollHeight - 80; }
function scrollBottom(smooth) {
  const s = $('#msgs');
  if (!s) return;
  requestAnimationFrame(() => s.scrollTo({ top: s.scrollHeight, behavior: smooth ? 'smooth' : 'auto' }));
}
function scrollToId(id, flash, block = 'center') {
  const el = $(`#msgsIn .mrow[data-id="${CSS.escape(id)}"]`);
  if (!el) return false;
  el.scrollIntoView({ block, behavior: 'auto' });
  if (flash) { el.classList.remove('flash'); void el.offsetWidth; el.classList.add('flash'); }
  return true;
}

async function jumpTo(chat, id) {
  if (S.cur !== chat || !S.route.startsWith('chat/')) { go('chat/' + chat); await new Promise(r => setTimeout(r, 50)); while (S.cache.get(chat) && S.cache.get(chat).loading) await new Promise(r => setTimeout(r, 50)); }
  if (scrollToId(id, true)) return;
  const c = S.cache.get(chat);
  try {
    const j = await GET(`/v1/chats/${encodeURIComponent(chat)}/messages?around=${encodeURIComponent(id)}&limit=80`);
    c.list = j.messages; c.byId = new Map(j.messages.map(m => [m.id, m]));
    c.moreBefore = j.more_before; c.moreAfter = j.more_after;
    renderMessages();
    if (!scrollToId(id, true)) toast('Message not found');
  } catch (e) { toast(e.message, true); }
}

async function jumpBottom() {
  const c = S.cache.get(S.cur);
  S.jumpUnread = 0;
  if (c && c.moreAfter) {
    const j = await GET(`/v1/chats/${encodeURIComponent(S.cur)}/messages?limit=60`);
    c.list = j.messages; c.byId = new Map(j.messages.map(m => [m.id, m]));
    c.moreBefore = j.more_before; c.moreAfter = false;
    renderMessages();
  }
  scrollBottom(true);
  renderJump();
}
function renderJump() {
  const b = $('#jumpBtn');
  if (!b) return;
  const c = S.cache.get(S.cur);
  const show = !nearBottom() || (c && c.moreAfter);
  b.hidden = !show;
  const old = b.querySelector('.badge'); if (old) old.remove();
  if (show && S.jumpUnread) b.appendChild(h('span', { class: 'badge' }, S.jumpUnread));
}

async function onMsgsScroll() {
  const s = $('#msgs');
  const c = S.cache.get(S.cur);
  renderJump();
  if (nearBottom()) { S.jumpUnread = 0; maybeRead(); }
  if (!c || c.loading || c.paging) return;
  if (s.scrollTop < 300 && c.moreBefore && c.list.length) {
    c.paging = true;
    try {
      const j = await GET(`/v1/chats/${encodeURIComponent(S.cur)}/messages?before=${encodeURIComponent(c.list[0].id)}&limit=60`);
      const fresh = j.messages.filter(m => !c.byId.has(m.id));
      c.list = fresh.concat(c.list); fresh.forEach(m => c.byId.set(m.id, m));
      c.moreBefore = j.more_before;
      c.paging = false;
      renderMessages({ keep: true });
    } catch { c.paging = false; }
  } else if (c.moreAfter && s.scrollTop + s.clientHeight > s.scrollHeight - 300) {
    c.paging = true;
    try {
      const j = await GET(`/v1/chats/${encodeURIComponent(S.cur)}/messages?after=${encodeURIComponent(c.list[c.list.length - 1].id)}&limit=60`);
      const fresh = j.messages.filter(m => !c.byId.has(m.id));
      c.list = c.list.concat(fresh); fresh.forEach(m => c.byId.set(m.id, m));
      c.moreAfter = j.more_after;
      c.paging = false;
      renderMessages({ keep: true });
    } catch { c.paging = false; }
  }
}

const maybeRead = debounce(() => {
  if (!S.cur || document.hidden || !S.route.startsWith('chat/') || !nearBottom()) return;
  const sum = S.chats.get(S.cur);
  const c = S.cache.get(S.cur);
  if (!sum || !sum.unread || !c || c.moreAfter) return;
  const last = [...c.list].reverse().find(m => m.from === 'bia');
  sum.unread = 0;
  renderChatList(); updateTitle();
  act('read', { chat: S.cur, upto: last ? last.id : '' }).catch(() => {});
}, 300);

// ---- one message row ----
function buildRow(m, g, isDraft) {
  if (m.service) return h('div', { class: 'service', 'data-id': m.id }, m.text);
  const mine = isMine(m);
  const row = h('div', { class: `mrow ${mine ? 'out' : 'in'}${g.first ? ' gap' : ''}${g.last ? ' last' : ''}${S.sel.has(m.id) ? ' selected' : ''}${isDraft ? ' draft' : ''}`, 'data-id': m.id });
  const bubble = h('div', { class: 'bubble' });
  const col = h('div', { class: 'bcol' }, bubble);
  row.append(h('div', { class: 'selbox' }), col);

  if (m.fwd) bubble.appendChild(h('div', { class: 'fwd' }, 'Forwarded from ', h('b', null, m.fwd.from === 'bia' ? 'bia' : m.fwd.from)));
  if (m.reply_to) bubble.appendChild(buildQuote(m.reply_to));

  const files = m.files || [];
  const media = files.map((f, i) => ({ f, i })).filter(x => x.f.kind === 'photo' || x.f.kind === 'video');
  const others = files.map((f, i) => ({ f, i })).filter(x => x.f.kind !== 'photo' && x.f.kind !== 'video');
  const hasText = m.text && m.text.trim();
  if (media.length) {
    const al = h('div', { class: `album n${Math.min(media.length, 10)}${!hasText && !others.length ? ' only' : ''}` });
    if (m.fwd || m.reply_to) al.style.marginTop = '2px';
    for (const { f, i } of media) al.appendChild(buildMedia(m, f, i, media));
    bubble.appendChild(al);
  }
  for (const { f, i } of others) bubble.appendChild(f.kind === 'voice' ? buildVoice(m, f, i) : buildDoc(m, f, i));
  if (m.poll) bubble.appendChild(buildPoll(m));

  const meta = buildMeta(m, isDraft);
  if (hasText) {
    const big = isEmojiOnly(m.text) && !files.length && !m.reply_to && !m.fwd;
    const t = h('div', { class: 'text md' + (big ? ' big' : ''), html: md(m.text) });
    if (big) { bubble.classList.add('bare'); }
    const lastBlock = t.lastElementChild;
    (lastBlock && lastBlock.tagName === 'P' ? lastBlock : t).appendChild(meta);
    bubble.appendChild(t);
  } else if (media.length && !others.length && !m.poll) {
    meta.classList.add('over');
    bubble.querySelector('.album').appendChild(meta);
    bubble.style.padding = '6px 10px';
  } else {
    bubble.appendChild(meta);
    bubble.appendChild(h('div', { style: 'clear:both' }));
  }
  if (m.reactions && Object.keys(m.reactions).length) bubble.appendChild(buildReacts(m));
  if (m.buttons && m.buttons.length) col.appendChild(buildButtons(m));

  if (!isDraft) {
    row.addEventListener('contextmenu', e => { if (S.selecting) return; e.preventDefault(); msgMenu(e, m); });
    onLongPress(bubble, e => { if (!S.selecting) msgMenu(e, m); });
    row.addEventListener('dblclick', e => { if (!S.selecting && !e.target.closest('a,button,.media,.doc,.voice,.poll')) setReply(m); });
    row.addEventListener('click', e => {
      if (!S.selecting) return;
      e.preventDefault(); e.stopPropagation();
      toggleSel(m.id);
    }, true);
  }
  return row;
}

function buildMeta(m, isDraft) {
  const mine = isMine(m);
  return h('span', { class: 'meta' },
    m.pinned ? ic('pin', 'pin') : null,
    m.edited ? h('span', { class: 'edited' }, 'edited') : null,
    isDraft ? 'typing' : fmtTime(m.ts),
    mine && !isDraft ? ic(m.read ? 'checks' : 'check') : null);
}

function buildQuote(id) {
  const c = S.cache.get(S.cur);
  const o = c && c.byId.get(id);
  const q = h('div', { class: 'quote', onclick: (e) => { e.stopPropagation(); jumpTo(S.cur, id); } },
    h('div', { class: 'grow' },
      h('div', { class: 'qname' }, o ? (isMine(o) ? 'You' : 'bia') : 'Message'),
      h('div', { class: 'qtext' }, o ? snippet(o) || 'Message' : 'Original message')));
  return q;
}

function attUrl(m, i, dl) { return `/v1/att?chat=${encodeURIComponent(S.cur)}&id=${encodeURIComponent(m.id)}&i=${i}${dl ? '&dl=1' : ''}`; }

function buildMedia(m, f, i, all) {
  const url = attUrl(m, i);
  const box = h('div', { class: 'media', onclick: (e) => { e.stopPropagation(); if (!S.selecting) openViewer(all.map(x => ({ url: attUrl(m, x.i), dl: attUrl(m, x.i, true), name: x.f.name, kind: x.f.kind })), all.findIndex(x => x.i === i)); } });
  if (f.kind === 'photo') {
    const img = h('img', { src: url, loading: 'lazy', alt: f.name, decoding: 'async' });
    img.addEventListener('load', () => { if (nearBottomish()) scrollBottom(); }, { once: true });
    box.appendChild(img);
  } else {
    const v = h('video', { src: url + '#t=0.1', preload: 'metadata', muted: true, playsinline: true });
    const dur = h('span', { class: 'dur' }, 'video');
    v.addEventListener('loadedmetadata', () => { dur.textContent = fmtDur(v.duration); });
    box.append(v, h('div', { class: 'play' }, ic('play')), dur);
  }
  return box;
}
function nearBottomish() { const s = $('#msgs'); return s && s.scrollTop + s.clientHeight >= s.scrollHeight - 400; }

function fileIcon(name, kind) {
  const ext = (name.split('.').pop() || '').toLowerCase();
  if (kind === 'audio') return 'audio';
  if (['zip', 'tar', 'gz', 'tgz', 'xz', 'bz2', '7z', 'rar', 'zst'].includes(ext)) return 'archive';
  if (['rs', 'js', 'py', 'sh', 'c', 'h', 'go', 'ts', 'json', 'toml', 'yaml', 'yml', 'html', 'css'].includes(ext)) return 'code';
  return 'file';
}

function buildDoc(m, f, i) {
  if (f.kind === 'audio') return buildAudio(m, f, i);
  return h('div', { class: 'doc', onclick: (e) => { e.stopPropagation(); if (!S.selecting) openFileAtt(m, f, i); } },
    h('div', { class: 'dicon' }, ic(fileIcon(f.name, f.kind))),
    h('div', { class: 'grow' }, h('div', { class: 'dname' }, f.name), h('div', { class: 'dsize' }, fmtBytes(f.size) + ' · ' + (f.name.split('.').pop() || '').toUpperCase())));
}
function openFileAtt(m, f, i) {
  const viewable = /^(application\/pdf|text\/|image\/|video\/|audio\/|application\/json)/.test(f.mime);
  if (viewable) openViewer([{ url: attUrl(m, i), dl: attUrl(m, i, true), name: f.name, kind: f.mime.startsWith('text/') || f.mime.includes('json') ? 'text' : f.mime === 'application/pdf' ? 'pdf' : f.kind }], 0);
  else download(attUrl(m, i, true), f.name);
}

function buildAudio(m, f, i) {
  const audio = new Audio();
  audio.preload = 'none';
  const btn = h('button', { class: 'dicon', style: 'border:0;cursor:pointer' }, ic('play'));
  const sub = h('div', { class: 'dsize' }, fmtBytes(f.size));
  btn.onclick = (e) => {
    e.stopPropagation();
    if (!audio.src) audio.src = attUrl(m, i);
    if (audio.paused) { stopOtherAudio(audio); audio.play(); } else audio.pause();
  };
  audio.onplay = () => set(btn, ic('pause'));
  audio.onpause = audio.onended = () => set(btn, ic('play'));
  audio.ontimeupdate = () => { sub.textContent = `${fmtDur(audio.currentTime)} / ${fmtDur(audio.duration)}`; };
  return h('div', { class: 'doc' }, btn,
    h('div', { class: 'grow' }, h('div', { class: 'dname' }, f.name), sub),
    iconBtn('download', 'Download', (e) => { e.stopPropagation(); download(attUrl(m, i, true), f.name); }));
}
let playingAudio = null;
function stopOtherAudio(a) { if (playingAudio && playingAudio !== a) playingAudio.pause(); playingAudio = a; }

function buildVoice(m, f, i) {
  const audio = new Audio();
  audio.preload = 'metadata';
  audio.src = attUrl(m, i);
  let seed = 0; for (const c of m.id) seed = (seed * 33 + c.charCodeAt(0)) >>> 0;
  const bars = [];
  for (let k = 0; k < 36; k++) { seed = (seed * 1103515245 + 12345) >>> 0; bars.push(h('i', { style: `height:${25 + (seed % 75)}%` })); }
  const wave = h('div', { class: 'wave' }, bars);
  const time = h('span', { class: 'vtime' }, '0:00');
  const btn = h('button', { class: 'vplay' }, ic('play'));
  btn.onclick = (e) => { e.stopPropagation(); if (audio.paused) { stopOtherAudio(audio); audio.play(); } else audio.pause(); };
  audio.onloadedmetadata = () => { if (isFinite(audio.duration)) time.textContent = fmtDur(audio.duration); };
  audio.onplay = () => set(btn, ic('pause'));
  audio.onpause = () => set(btn, ic('play'));
  audio.onended = () => { set(btn, ic('play')); bars.forEach(b => b.classList.remove('p')); };
  audio.ontimeupdate = () => {
    const p = audio.duration ? audio.currentTime / audio.duration : 0;
    bars.forEach((b, k) => b.classList.toggle('p', k / bars.length < p));
    time.textContent = fmtDur(audio.currentTime);
  };
  wave.onclick = (e) => {
    e.stopPropagation();
    const r = wave.getBoundingClientRect();
    if (audio.duration) audio.currentTime = ((e.clientX - r.left) / r.width) * audio.duration;
    if (audio.paused) { stopOtherAudio(audio); audio.play(); }
  };
  const row = h('div', { class: 'voice' }, btn, h('div', { class: 'grow' }, wave, time));
  // transcript of a voice message (theirs: speech to text; bia's: what it said)
  return f.transcript ? h('div', { class: 'voicebox' }, row, h('div', { class: 'vtext' }, f.transcript)) : row;
}

function buildPoll(m) {
  const p = m.poll;
  const me = S.me.identity;
  const total = p.options.reduce((a, o) => a + o.voters.length, 0);
  const voted = p.options.some(o => o.voters.includes(me));
  const showRes = voted || p.closed;
  const picks = new Set();
  const box = h('div', { class: 'poll' + (p.multi ? ' multi' : '') + (showRes ? ' voted' : '') });
  box.appendChild(h('div', { class: 'pq' }, p.question));
  box.appendChild(h('div', { class: 'pkind' }, p.closed ? 'Final results' : p.multi ? 'Poll · multiple answers' : 'Poll'));
  const voteBtn = h('button', { class: 'btn small pvote', hidden: true, onclick: (e) => { e.stopPropagation(); act('vote', { chat: S.cur, id: m.id, options: [...picks] }).catch(er => toast(er.message, true)); } }, 'Vote');
  p.options.forEach((o, i) => {
    const pct = total ? Math.round((o.voters.length / total) * 100) : 0;
    const mineOpt = o.voters.includes(me);
    const opt = h('div', { class: 'opt' + (mineOpt ? ' picked' : '') },
      showRes ? h('span', { class: 'pct' }, pct + '%') : h('span', { class: 'radio' }),
      h('div', null, o.text, showRes && mineOpt ? ' ✓' : ''),
      showRes ? h('div', { class: 'bar', style: `width:${Math.max(2, pct)}%` }) : null);
    if (!showRes) opt.onclick = (e) => {
      e.stopPropagation();
      if (!p.multi) { act('vote', { chat: S.cur, id: m.id, options: [i] }).catch(er => toast(er.message, true)); return; }
      if (picks.has(i)) picks.delete(i); else picks.add(i);
      opt.classList.toggle('picked', picks.has(i));
      voteBtn.hidden = !picks.size;
    };
    box.appendChild(opt);
  });
  if (p.multi && !showRes) box.appendChild(voteBtn);
  box.appendChild(h('div', { class: 'pfoot' }, total === 1 ? '1 vote' : `${total} votes`));
  return box;
}

function buildReacts(m) {
  const me = S.me.identity;
  return h('div', { class: 'reacts' }, Object.entries(m.reactions).map(([e, who]) =>
    h('button', {
      class: 'react' + (who.includes(me) ? ' mine' : ''), title: who.join(', '),
      onclick: (ev) => { ev.stopPropagation(); act('react', { chat: S.cur, id: m.id, emoji: e }).catch(er => toast(er.message, true)); },
    }, e, who.length > 1 ? h('span', { class: 'n' }, who.length) : h('span', { class: 'who' }, initials(who[0])))));
}

function buildButtons(m) {
  return h('div', { class: 'kb' }, m.buttons.map((row, r) => h('div', { class: 'kbrow' }, row.map((b, c) =>
    h('button', {
      title: b.url || b.text,
      onclick: async (e) => {
        e.stopPropagation();
        if (b.url) {
          if (b.url.startsWith('/') || await confirmBox({ title: 'Open link?', body: b.url }, 'Open')) window.open(b.url, '_blank', 'noopener');
          return;
        }
        e.currentTarget.classList.add('pressed');
        try { await act('button', { chat: S.cur, id: m.id, row: r, col: c }); toast(`Sent: ${b.text}`); }
        catch (er) { toast(er.message, true); }
      },
    }, b.url ? [b.text, ' ↗'] : b.text)))));
}

function buildPendingRow(p) {
  const bubble = h('div', { class: 'bubble' });
  if (p.reply_to) bubble.appendChild(buildQuote(p.reply_to));
  for (const f of p.files || []) {
    const pct = p.progress && p.progress[f.name] !== undefined ? p.progress[f.name] : 0;
    const R = 21, C = 2 * Math.PI * R;
    const icon = h('div', { class: 'dicon', html: `<svg class="ring" viewBox="0 0 48 48"><circle cx="24" cy="24" r="${R}" stroke-dasharray="${C}" stroke-dashoffset="${C * (1 - pct)}"/></svg>` });
    icon.prepend(ic(p.error ? 'close' : 'upload'));
    bubble.appendChild(h('div', { class: 'doc' }, icon,
      h('div', { class: 'grow' }, h('div', { class: 'dname' }, f.name), h('div', { class: 'dsize' }, p.error ? 'failed' : `${Math.round(pct * 100)}% of ${fmtBytes(f.size)}`))));
  }
  const meta = h('span', { class: 'meta' }, fmtTime(p.ts), p.error ? h('span', { style: 'color:var(--err)' }, '!') : ic('clock'));
  if (p.text) { const t = h('div', { class: 'text md', html: md(p.text) }); t.appendChild(meta); bubble.appendChild(t); }
  else { bubble.append(meta, h('div', { style: 'clear:both' })); }
  const row = h('div', { class: 'mrow out gap last pending' }, h('div', { class: 'selbox' }), h('div', { class: 'bcol' }, bubble));
  if (p.error) {
    row.title = p.error;
    row.onclick = async () => {
      if (await confirmBox({ title: 'Not sent', body: p.error }, 'Discard', true)) {
        const list = S.pending.get(p.chat) || [];
        list.splice(list.indexOf(p), 1);
        renderTail();
      }
    };
  }
  return row;
}

function isEmojiOnly(s) {
  const t = s.trim();
  if (!t || t.length > 24) return false;
  if (!/^(?:\p{Extended_Pictographic}|\p{Emoji_Component}|‍|️|\s)+$/u.test(t)) return false;
  if (/^[\d#*\s]+$/.test(t)) return false;
  const n = [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(t.replace(/\s/g, ''))].length;
  return n <= 3;
}

function onMsgsClick(e) {
  const sp = e.target.closest('.spoiler');
  if (sp) { sp.classList.add('shown'); return; }
  const lang = e.target.closest('pre .lang');
  if (lang) { e.stopPropagation(); copyText(lang.parentElement.querySelector('code').textContent, 'Code'); return; }
  const pl = e.target.closest('a.plink');
  if (pl) { e.preventDefault(); openPathInFiles(pl.dataset.path); }
}
function openPathInFiles(p) {
  const dir = p.endsWith('/') ? p : p.replace(/\/[^/]*$/, '') || '/';
  S.filesFocus = p.endsWith('/') ? null : p.split('/').pop();
  go('files=' + dir);
}

// ---- markdown ----
function md(src) {
  const lines = String(src).replace(/\r\n?/g, '\n').split('\n');
  const out = [];
  let i = 0;
  const isFence = l => /^\s*```/.test(l);
  const isList = l => /^\s*([-*+•]|\d+[.)])\s+/.test(l);
  const isTable = (l, n) => /^\s*\|.*\|\s*$/.test(l) && n !== undefined && /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(n);
  const special = (k) => isFence(lines[k]) || /^#{1,6}\s/.test(lines[k]) || isList(lines[k]) || /^\s*>/.test(lines[k]) || /^\s*(---|\*\*\*|___)\s*$/.test(lines[k]) || isTable(lines[k], lines[k + 1]);
  while (i < lines.length) {
    const l = lines[i];
    const fence = /^\s*```\s*([\w+#.-]*)/.exec(l);
    if (fence) {
      const buf = []; i++;
      while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) buf.push(lines[i++]);
      i++;
      out.push(`<pre><span class="lang" title="Copy">${esc(fence[1] || 'copy')}</span><code>${esc(buf.join('\n'))}</code></pre>`);
      continue;
    }
    if (/^\s*(---|\*\*\*|___)\s*$/.test(l)) { out.push('<hr>'); i++; continue; }
    const hd = /^(#{1,6})\s+(.*)$/.exec(l);
    if (hd) { out.push(`<h3>${inline(hd[2])}</h3>`); i++; continue; }
    if (/^\s*>/.test(l)) {
      const buf = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) buf.push(lines[i++].replace(/^\s*>\s?/, ''));
      out.push(`<blockquote>${md(buf.join('\n'))}</blockquote>`);
      continue;
    }
    if (isTable(l, lines[i + 1])) {
      const row = s => s.trim().replace(/^\||\|$/g, '').split('|').map(x => x.trim());
      const head = row(l); i += 2;
      const body = [];
      while (i < lines.length && /^\s*\|.*\|\s*$/.test(lines[i])) body.push(row(lines[i++]));
      out.push(`<table><thead><tr>${head.map(c => `<th>${inline(c)}</th>`).join('')}</tr></thead><tbody>${body.map(r => `<tr>${r.map(c => `<td>${inline(c)}</td>`).join('')}</tr>`).join('')}</tbody></table>`);
      continue;
    }
    if (isList(l)) {
      const ordered = /^\s*\d/.test(l);
      const start = ordered ? parseInt(l, 10) : 1;
      const items = [];
      while (i < lines.length) {
        const m = /^\s*([-*+•]|\d+[.)])\s+(.*)$/.exec(lines[i]);
        if (m) { items.push(m[2]); i++; continue; }
        if (items.length && /^\s{2,}\S/.test(lines[i])) { items[items.length - 1] += '\n' + lines[i].trim(); i++; continue; }
        break;
      }
      const tag = ordered ? 'ol' : 'ul';
      out.push(`<${tag}${ordered && start !== 1 ? ` start="${start}"` : ''}>${items.map(x => `<li>${x.split('\n').map(inline).join('<br>')}</li>`).join('')}</${tag}>`);
      continue;
    }
    if (!l.trim()) { i++; continue; }
    const buf = [l]; i++;
    while (i < lines.length && lines[i].trim() && !special(i)) buf.push(lines[i++]);
    out.push(`<p>${buf.map(inline).join('<br>')}</p>`);
  }
  return out.join('');
}
function inline(s) {
  return s.split(/(`[^`\n]+`)/g).map(p => {
    if (p.length > 1 && p[0] === '`' && p.endsWith('`')) return `<code>${esc(p.slice(1, -1))}</code>`;
    const hold = [];
    const keep = x => { hold.push(x); return `\u0000${hold.length - 1}\u0000`; };
    let t = p.replace(/\[([^\]\n]+)\]\(((?:https?:\/\/|\/)[^)\s]*)\)/g, (m, a, u) => keep(`<a href="${esc(u)}" target="_blank" rel="noopener noreferrer">${esc(a)}</a>`));
    t = t.replace(/\bhttps?:\/\/[^\s<>"'`]*[^\s<>"'`.,;:!?)\]]/g, u => keep(`<a href="${esc(u)}" target="_blank" rel="noopener noreferrer">${esc(u)}</a>`));
    if (S.root) t = t.replace(/(^|[\s(])(\/(?:home|tmp|etc|var|root|usr|opt|srv|mnt|media|run)\/[^\s<>"'`)]*[^\s<>"'`.,;:!?)\]])/g,
      (m, pre, pth) => pre + keep(`<a href="#" class="plink" data-path="${esc(pth)}">${esc(pth)}</a>`));
    t = esc(t)
      .replace(/\*\*(?=\S)([\s\S]*?\S)\*\*/g, '<b>$1</b>')
      .replace(/__(?=\S)([\s\S]*?\S)__/g, '<u>$1</u>')
      .replace(/~~(?=\S)([\s\S]*?\S)~~/g, '<s>$1</s>')
      .replace(/\|\|(?=\S)([\s\S]*?\S)\|\|/g, '<span class="spoiler">$1</span>')
      .replace(/(^|[^\w*])\*(?=\S)([^*\n]*?\S)\*(?![\w*])/g, '$1<i>$2</i>')
      .replace(/(^|[^\w])_(?=\S)([^_\n]*?\S)_(?!\w)/g, '$1<i>$2</i>');
    return t.replace(/\u0000(\d+)\u0000/g, (m, n) => hold[n]);
  }).join('');
}

// ---- context menu / actions ----
function msgMenu(e, m) {
  e.preventDefault();
  const mine = isMine(m);
  const reacts = h('div', { class: 'react-row' }, BIA_REACTIONS.slice(0, 7).map(em =>
    h('button', { onclick: () => { closeMenu(); act('react', { chat: S.cur, id: m.id, emoji: em }).catch(er => toast(er.message, true)); } }, em)),
  h('button', { title: 'More', onclick: (ev) => { const r = ev.currentTarget.getBoundingClientRect(); closeMenu(); emojiPopup(r.left, r.top, em => act('react', { chat: S.cur, id: m.id, emoji: em })); } }, '＋'));
  const files = m.files || [];
  const sel = String(window.getSelection() || '');
  menu(e.clientX, e.clientY, [
    { icon: 'reply', text: 'Reply', onClick: () => setReply(m) },
    mine && (m.text || files.length) && !m.poll ? { icon: 'edit', text: 'Edit', onClick: () => setEdit(m) } : null,
    m.text ? { icon: 'copy', text: sel ? 'Copy selected text' : 'Copy text', onClick: () => copyText(sel || m.text) } : null,
    { icon: m.pinned ? 'unpin' : 'pin', text: m.pinned ? 'Unpin' : 'Pin', onClick: () => act('pin', { chat: S.cur, id: m.id, on: !m.pinned }) },
    { icon: 'forward', text: 'Forward', onClick: () => forwardMsgs([m.id]) },
    files.length ? { icon: 'download', text: files.length > 1 ? `Download all (${files.length})` : 'Download', onClick: () => files.forEach((f, i) => setTimeout(() => download(attUrl(m, i, true), f.name), i * 400)) } : null,
    S.root && files.length ? { icon: 'folder', text: 'Show in Files', onClick: () => openPathInFiles(files[0].path.startsWith('/') ? files[0].path : (S.me.files + '/' + files[0].path)) } : null,
    m.poll && m.poll.options.some(o => o.voters.includes(S.me.identity)) && !m.poll.closed ? { icon: 'poll', text: 'Retract vote', onClick: () => act('vote', { chat: S.cur, id: m.id, options: [] }) } : null,
    { icon: 'select', text: 'Select', onClick: () => { setSelecting(true); toggleSel(m.id); } },
    S.root ? { icon: 'info', text: 'Copy message ID', onClick: () => copyText(m.id, 'ID') } : null,
    '-',
    { icon: 'trash', text: 'Delete', danger: true, onClick: () => deleteMsgs([m.id]) },
  ], reacts);
}

function setSelecting(on) {
  S.selecting = on;
  if (!on) S.sel.clear();
  $('#msgs').classList.toggle('selecting', on);
  $$('#msgsIn .mrow.selected').forEach(r => r.classList.remove('selected'));
  renderChatHead();
}
function toggleSel(id) {
  if (S.sel.has(id)) S.sel.delete(id); else S.sel.add(id);
  const r = $(`#msgsIn .mrow[data-id="${CSS.escape(id)}"]`);
  if (r) r.classList.toggle('selected', S.sel.has(id));
  if (!S.sel.size) setSelecting(false); else renderChatHead();
}
function copySelected() {
  const c = S.cache.get(S.cur);
  const txt = c.list.filter(m => S.sel.has(m.id)).map(m => `${isMine(m) ? 'You' : 'bia'}, [${new Date(m.ts).toLocaleString()}]\n${m.text || ''}`).join('\n\n');
  copyText(txt, 'Messages');
  setSelecting(false);
}
async function deleteMsgs(ids) {
  if (!ids.length) return;
  const ok = await confirmBox({ title: ids.length > 1 ? `Delete ${ids.length} messages?` : 'Delete message?', body: 'They are removed for everyone, and bia is told which ones were deleted.' }, 'Delete', true);
  if (!ok) return;
  try { await act('delete', { chat: S.cur, ids }); setSelecting(false); }
  catch (e) { toast(e.message, true); }
}
async function forwardMsgs(ids) {
  if (!ids.length) return;
  const from = S.cur;
  const to = await pickChat('Forward to…');
  if (!to) return;
  try {
    await act('forward', { chat: from, ids, to });
    setSelecting(false);
    if (to !== from) go('chat/' + to);
    toast(ids.length > 1 ? `${ids.length} messages forwarded` : 'Message forwarded');
  } catch (e) { toast(e.message, true); }
}
function pickChat(title) {
  let pick = null;
  return modal({
    title,
    body: h('div', { class: 'chat-pick' }, S.order.map(id => h('div', {
      class: 'citem', onclick: (e) => { pick = id; e.currentTarget.closest('.modal').close(id); },
    }, h('div', { class: 'avatar sm ' + (id === 'main' ? '' : colorOf(id)) }, initials(chatTitle(id))), h('div', { class: 'cbody ctitle' }, chatTitle(id))))),
    buttons: [{ text: 'Cancel', value: null }],
  }).then(v => v || pick);
}

// ---- emoji popup (reactions) ----
function emojiPopup(x, y, onPick) {
  const grid = h('div', { class: 'egrid', style: 'height:260px;overflow:auto' });
  for (const cat of BIA_EMOJI) {
    grid.appendChild(h('div', { class: 'elabel' }, cat.name));
    for (const e of cat.list) grid.appendChild(h('button', { onclick: () => { closeMenu(); onPick(e); } }, e));
  }
  const m = menu(x, y, [], h('div', { class: 'emoji-panel', style: 'position:static;width:320px;height:auto;box-shadow:none;border:0' }, grid));
  m.style.padding = '0';
}

// ======================================================================
// composer
// ======================================================================
function buildComposer() {
  const ta = h('textarea', { id: 'composerInput', rows: 1, placeholder: 'Message', autocomplete: 'off' });
  const send = h('button', { class: 'sendbtn', id: 'sendBtn', title: 'Send' }, ic('send'));
  const emoBtn = iconBtn('smile', 'Emoji', () => toggleEmojiPanel());
  const attBtn = iconBtn('attach', 'Attach', (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    menu(r.left - 150, r.top - 110, [
      { icon: 'image', text: 'Photo or video', onClick: () => pickFiles('image/*,video/*') },
      { icon: 'file', text: 'File', onClick: () => pickFiles('') },
      S.root ? { icon: 'folder', text: 'From machine…', onClick: () => go('files') } : null,
    ]);
  });
  ta.addEventListener('input', () => { autoGrow(ta); updateSendBtn(); saveComposerDraftSoon(); });
  ta.addEventListener('keydown', composerKey);
  ta.addEventListener('paste', e => {
    const files = [...(e.clipboardData && e.clipboardData.files) || []];
    if (files.length) { e.preventDefault(); attachDialog(files); }
  });
  send.addEventListener('click', onSendBtn);
  const wrap = h('div', { class: 'composer-wrap', id: 'composerWrap' },
    h('div', { class: 'composer-inner' },
      h('div', { class: 'composer', id: 'composer' },
        h('div', { id: 'composerCtx' }),
        h('div', { class: 'crow', id: 'composerRow' }, emoBtn, ta, attBtn)),
      send));
  return wrap;
}
function autoGrow(ta) { ta.style.height = 'auto'; ta.style.height = Math.min(ta.scrollHeight, innerHeight * 0.4) + 'px'; }

function updateSendBtn() {
  const b = $('#sendBtn'), ta = $('#composerInput');
  if (!b || !ta) return;
  if (S.rec) { b.className = 'sendbtn'; set(b, ic('send')); b.title = 'Send voice message'; return; }
  const has = ta.value.trim() || S.edit;
  if (has) { b.className = 'sendbtn'; set(b, ic(S.edit ? 'check' : 'send')); b.title = S.edit ? 'Save' : 'Send'; }
  else if (S.root && S.typing.has(S.cur)) { b.className = 'sendbtn stop'; set(b, ic('stop')); b.title = 'Stop generating'; }
  else { b.className = 'sendbtn'; set(b, ic('mic')); b.title = 'Record voice message'; }
}
function onSendBtn() {
  const ta = $('#composerInput');
  if (S.rec) { stopRecording(true); return; }
  if (ta.value.trim() || S.edit) { sendText(); return; }
  if (S.root && S.typing.has(S.cur)) { act('stop', { chat: S.cur }); return; }
  startRecording();
}

function composerKey(e) {
  const ta = e.target;
  if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && (S.prefs.enterSends || e.ctrlKey || e.metaKey)) {
    e.preventDefault(); sendText(); return;
  }
  if (e.key === 'Escape') {
    if (S.edit || S.reply) { e.preventDefault(); cancelCtx(); return; }
    if (S.selecting) { setSelecting(false); return; }
  }
  if (e.key === 'ArrowUp' && !ta.value && !S.edit) {
    const c = S.cache.get(S.cur);
    const last = c && [...c.list].reverse().find(m => isMine(m) && m.text && !m.poll);
    if (last) { e.preventDefault(); setEdit(last); }
    return;
  }
  const mod = e.ctrlKey || e.metaKey;
  if (!mod) return;
  const wraps = { b: '**', i: '_', u: '__' };
  const k = e.key.toLowerCase();
  let w = !e.shiftKey ? wraps[k] : ({ x: '~~', m: '`', p: '||' })[k];
  if (!w) return;
  e.preventDefault();
  const s = ta.selectionStart, en = ta.selectionEnd;
  const sel = ta.value.slice(s, en);
  ta.setRangeText(w + sel + w, s, en, 'end');
  if (!sel) ta.setSelectionRange(s + w.length, s + w.length);
  ta.dispatchEvent(new Event('input'));
}

function renderComposerCtx() {
  const el = $('#composerCtx');
  if (!el) return;
  const c = S.cache.get(S.cur);
  if (S.edit) {
    set(el, h('div', { class: 'ctx' }, ic('edit'), h('div', { class: 'cline' }),
      h('div', { class: 'grow' }, h('div', { class: 'ctitle' }, 'Edit message'), h('div', { class: 'ctext' }, snippet(S.edit))),
      iconBtn('close', 'Cancel', cancelCtx)));
  } else if (S.reply) {
    const o = (c && c.byId.get(S.reply)) || {};
    set(el, h('div', { class: 'ctx' }, ic('reply'), h('div', { class: 'cline' }),
      h('div', { class: 'grow', style: 'cursor:pointer', onclick: () => jumpTo(S.cur, S.reply) },
        h('div', { class: 'ctitle' }, 'Reply to ' + (isMine(o) ? 'yourself' : 'bia')), h('div', { class: 'ctext' }, snippet(o))),
      iconBtn('close', 'Cancel', cancelCtx)));
  } else set(el);
}
function setReply(m) { S.edit = null; S.reply = m.id; renderComposerCtx(); updateSendBtn(); $('#composerInput').focus(); }
function setEdit(m) {
  S.reply = null; S.edit = m;
  const ta = $('#composerInput');
  ta.value = m.text || '';
  autoGrow(ta); renderComposerCtx(); updateSendBtn(); ta.focus();
  ta.setSelectionRange(ta.value.length, ta.value.length);
}
function cancelCtx() {
  if (S.edit) { $('#composerInput').value = ''; autoGrow($('#composerInput')); }
  S.edit = null; S.reply = null; renderComposerCtx(); updateSendBtn();
}

const draftKey = id => `bia.draft.${S.me && S.me.identity}.${id}`;
function saveComposerDraft() {
  const ta = $('#composerInput');
  if (!ta || !S.cur || S.edit) return;
  if (ta.value.trim()) localStorage.setItem(draftKey(S.cur), ta.value); else localStorage.removeItem(draftKey(S.cur));
}
const saveComposerDraftSoon = debounce(saveComposerDraft, 400);
function loadComposerDraft() {
  const ta = $('#composerInput');
  if (!ta) return;
  ta.value = localStorage.getItem(draftKey(S.cur)) || '';
  autoGrow(ta); updateSendBtn();
}

async function sendText() {
  const ta = $('#composerInput');
  const text = ta.value;
  if (S.edit) {
    const m = S.edit;
    if (text === m.text) { cancelCtx(); return; }
    try { await act('edit', { chat: S.cur, id: m.id, text }); cancelCtx(); }
    catch (e) { toast(e.message, true); }
    return;
  }
  if (!text.trim()) return;
  ta.value = ''; autoGrow(ta); localStorage.removeItem(draftKey(S.cur));
  const reply = S.reply; S.reply = null; renderComposerCtx(); updateSendBtn();
  await sendMessage({ text, files: [], reply_to: reply });
}

// optimistic send; files are uploaded first with progress
async function sendMessage({ text, files, reply_to, kind }) {
  const chat = S.cur;
  const p = { chat, text: text || '', files: files.map(f => ({ name: f.name, size: f.size })), progress: {}, reply_to, ts: Date.now() };
  if (!S.pending.has(chat)) S.pending.set(chat, []);
  S.pending.get(chat).push(p);
  renderTail(); scrollBottom(true);
  try {
    const paths = [];
    for (const f of files) {
      const j = await xhrUpload('/v1/upload?name=' + encodeURIComponent(f.name), f, (done, total) => {
        p.progress[f.name] = total ? done / total : 0;
        if (S.cur === chat) scheduleDraft();
      });
      paths.push(j.path);
    }
    p.sent = true;
    const r = await act('send', { chat, text: text || '', files: paths, reply_to: reply_to || '', kind: kind || '' });
    const list = S.pending.get(chat) || [];
    const i = list.indexOf(p);
    if (i >= 0) list.splice(i, 1);
    // the msg event normally arrived first; make sure the message shows
    const c = S.cache.get(chat);
    if (r && r.m && c && !c.byId.has(r.m.id) && !c.moreAfter) { c.list.push(r.m); c.byId.set(r.m.id, r.m); if (S.cur === chat) appendRow(r.m); }
    if (S.cur === chat) renderTail();
  } catch (e) {
    p.error = e.message || String(e);
    if (S.cur === chat) renderTail();
    toast('Not sent: ' + p.error, true);
  }
}

function pickFiles(accept) {
  const inp = h('input', { type: 'file', multiple: true, accept, style: 'display:none' });
  inp.onchange = () => { attachDialog([...inp.files]); inp.remove(); };
  document.body.appendChild(inp);
  inp.click();
}

function attachDialog(files) {
  files = files.filter(f => f && (f.size || f.type));
  if (!files.length) return;
  const visual = files.every(f => /^(image|video)\//.test(f.type));
  const cap = h('textarea', { class: 'input', rows: 2, placeholder: 'Add a caption…' });
  cap.value = $('#composerInput') ? $('#composerInput').value : '';
  const urls = [];
  const list = visual
    ? h('div', { class: 'attach-grid' }, files.map(f => { const u = URL.createObjectURL(f); urls.push(u); return f.type.startsWith('video/') ? h('video', { src: u, muted: true }) : h('img', { src: u }); }))
    : h('div', { class: 'attach-list' }, files.map(f => {
      let thumb;
      if (f.type.startsWith('image/')) { const u = URL.createObjectURL(f); urls.push(u); thumb = h('img', { class: 'thumb', src: u }); }
      else thumb = h('div', { class: 'thumb' }, ic('file'));
      return h('div', { class: 'attach-item' }, thumb, h('div', { class: 'grow' }, h('div', { class: 'ellipsis' }, f.name), h('div', { class: 'muted' }, fmtBytes(f.size))));
    }));
  const title = visual ? (files.length > 1 ? `Send ${files.length} ${files.every(f => f.type.startsWith('image/')) ? 'photos' : 'media'}` : (files[0].type.startsWith('image/') ? 'Send photo' : 'Send video')) : (files.length > 1 ? `Send ${files.length} files` : 'Send file');
  modal({
    title, cls: 'wide',
    body: h('div', null, list, cap),
    buttons: [{ text: 'Cancel', value: false }, { text: 'Send', value: true, primary: true }],
    onClose: () => urls.forEach(u => URL.revokeObjectURL(u)),
  }).then(ok => {
    if (!ok) return;
    const text = cap.value;
    const ta = $('#composerInput');
    if (ta && ta.value && text.includes(ta.value)) { ta.value = ''; autoGrow(ta); localStorage.removeItem(draftKey(S.cur)); }
    const reply = S.reply; S.reply = null; renderComposerCtx(); updateSendBtn();
    sendMessage({ text, files, reply_to: reply });
  });
}

// ---- emoji panel ----
function toggleEmojiPanel() {
  const old = $('#emojiPanel');
  if (old) { old.remove(); return; }
  let cat = 0;
  const grid = h('div', { class: 'egrid' });
  const tabs = h('div', { class: 'etabs' });
  const show = (k) => {
    cat = k;
    [...tabs.children].forEach((b, j) => b.classList.toggle('on', j === k));
    set(grid, ...BIA_EMOJI[k].list.map(e => h('button', {
      onclick: () => {
        const ta = $('#composerInput');
        ta.setRangeText(e, ta.selectionStart, ta.selectionEnd, 'end');
        ta.dispatchEvent(new Event('input')); ta.focus();
      },
    }, e)));
    grid.scrollTop = 0;
  };
  BIA_EMOJI.forEach((c, k) => tabs.appendChild(h('button', { title: c.name, onclick: () => show(k) }, c.icon)));
  const panel = h('div', { class: 'emoji-panel', id: 'emojiPanel' }, tabs, grid);
  $('#v-chat').appendChild(panel);
  show(cat);
  const away = (e) => { if (!panel.contains(e.target) && !e.target.closest('[title=Emoji]')) { panel.remove(); document.removeEventListener('mousedown', away); } };
  setTimeout(() => document.addEventListener('mousedown', away), 0);
}

// ---- voice messages ----
async function startRecording() {
  if (!navigator.mediaDevices || !window.MediaRecorder) { toast('Voice recording is not supported here', true); return; }
  let stream;
  try { stream = await navigator.mediaDevices.getUserMedia({ audio: true }); }
  catch (e) { toast('Microphone: ' + e.message, true); return; }
  const type = ['audio/ogg;codecs=opus', 'audio/webm;codecs=opus', 'audio/mp4', 'audio/webm'].find(t => MediaRecorder.isTypeSupported(t)) || '';
  const rec = new MediaRecorder(stream, type ? { mimeType: type } : undefined);
  const chunks = [];
  rec.ondataavailable = e => { if (e.data.size) chunks.push(e.data); };
  const t0 = Date.now();
  const timer = h('span', { class: 'mono' }, '0:00');
  const tick = setInterval(() => { timer.textContent = fmtDur((Date.now() - t0) / 1000); }, 250);
  S.rec = { rec, stream, chunks, tick, type };
  const row = $('#composerRow');
  row.hidden = true;
  const bar = h('div', { class: 'recbar', id: 'recBar' }, h('span', { class: 'dot' }), timer, h('span', { class: 'grow muted' }, 'Recording…'),
    h('button', { class: 'btn flat danger', onclick: () => stopRecording(false) }, 'Cancel'));
  $('#composer').appendChild(bar);
  rec.start(250);
  updateSendBtn();
  $('#sendBtn').classList.add('rec');
}
function stopRecording(send) {
  const r = S.rec;
  if (!r) return;
  S.rec = null;
  clearInterval(r.tick);
  r.rec.onstop = () => {
    r.stream.getTracks().forEach(t => t.stop());
    if (!send) return;
    const mime = (r.type || 'audio/webm').split(';')[0];
    const ext = mime.includes('ogg') ? 'ogg' : mime.includes('mp4') ? 'm4a' : 'webm';
    const blob = new Blob(r.chunks, { type: mime });
    if (blob.size < 1200) { toast('Too short'); return; }
    const d = new Date();
    const f = new File([blob], `voice-${d.getFullYear()}${pad2(d.getMonth() + 1)}${pad2(d.getDate())}-${pad2(d.getHours())}${pad2(d.getMinutes())}${pad2(d.getSeconds())}.${ext}`, { type: mime });
    const reply = S.reply; S.reply = null; renderComposerCtx();
    sendMessage({ text: '', files: [f], reply_to: reply, kind: 'voice' });
  };
  r.rec.stop();
  const bar = $('#recBar'); if (bar) bar.remove();
  $('#composerRow').hidden = false;
  updateSendBtn();
}

// ======================================================================
// media viewer
// ======================================================================
function openViewer(items, idx) {
  let i = Math.max(0, idx);
  const stage = h('div', { class: 'vstage' });
  const name = h('div', { class: 'grow ellipsis' });
  const dlBtn = iconBtn('download', 'Download', () => download(items[i].dl, items[i].name));
  const v = h('div', { class: 'viewer' },
    h('div', { class: 'vtop' }, name, h('span', { class: 'muted', id: 'vcount' }), dlBtn,
      iconBtn('external', 'Open in new tab', () => window.open(items[i].url, '_blank', 'noopener')),
      iconBtn('close', 'Close', () => close())),
    stage);
  const show = () => {
    const it = items[i];
    name.textContent = it.name;
    $('#vcount', v).textContent = items.length > 1 ? `${i + 1} of ${items.length}` : '';
    let el;
    if (it.kind === 'photo') el = h('img', { src: it.url, alt: it.name });
    else if (it.kind === 'video') el = h('video', { src: it.url, controls: true, autoplay: true, playsinline: true });
    else if (it.kind === 'audio' || it.kind === 'voice') el = h('audio', { src: it.url, controls: true, autoplay: true });
    else if (it.kind === 'pdf') el = h('iframe', { src: it.url });
    else if (it.kind === 'text') {
      el = h('pre', { class: 'editor', style: 'max-width:1000px;height:100%;margin:0;overflow:auto;white-space:pre-wrap' }, 'Loading…');
      fetch(it.url, { credentials: 'include' }).then(r => r.text()).then(t => { el.textContent = t.length > 2e6 ? t.slice(0, 2e6) + '\n…' : t; });
    } else el = h('div', { class: 'muted' }, 'No preview');
    set(stage, el);
    if (items.length > 1) stage.append(
      iconBtn('back', 'Previous', () => { i = (i - 1 + items.length) % items.length; show(); }, 'vnav l'),
      h('button', { class: 'btn-icon vnav r', title: 'Next', onclick: () => { i = (i + 1) % items.length; show(); } }, ic('back', '')));
    const r = stage.querySelector('.vnav.r svg'); if (r) r.style.transform = 'scaleX(-1)';
  };
  const key = (e) => {
    if (e.key === 'Escape') { e.stopPropagation(); close(); }
    else if (e.key === 'ArrowLeft' && items.length > 1) { i = (i - 1 + items.length) % items.length; show(); }
    else if (e.key === 'ArrowRight' && items.length > 1) { i = (i + 1) % items.length; show(); }
  };
  const close = () => { v.remove(); document.removeEventListener('keydown', key, true); };
  stage.addEventListener('click', e => { if (e.target === stage) close(); });
  document.addEventListener('keydown', key, true);
  document.body.appendChild(v);
  show();
}

// ======================================================================
// root: live console
// ======================================================================
const CON = { term: null, ws: null, hist: [], hi: -1 };
function buildConsole() {
  const state = h('span', { class: 'conn-state' }, 'connecting');
  const host = h('div', { class: 'term' });
  const inp = h('input', { placeholder: 'Type to the agent console (slash commands work: /help, /status…)', autocomplete: 'off', spellcheck: false });
  const v = h('section', { class: 'view' },
    h('div', { class: 'vhead' }, backBtn(), h('div', { class: 'avatar xs tool' }, ic('console')),
      h('div', { class: 'tbox' }, h('div', { class: 'title' }, 'Console'), h('div', { class: 'subtitle' }, 'live agent console · same as the machine screen')),
      state,
      iconBtn('fontDown', 'Smaller', () => termFont(-1)), iconBtn('fontUp', 'Larger', () => termFont(1)),
      iconBtn('copy', 'Copy all', () => copyText(CON.term.text(), 'Console')),
      iconBtn('eraser', 'Clear scrollback', () => CON.term.clear()),
      iconBtn('refresh', 'Reconnect', () => { if (CON.ws) CON.ws.close(); else conConnect(); })),
    host,
    h('div', { class: 'termbar' }, h('span', { class: 'prompt' }, '->'), inp,
      h('button', { class: 'btn danger small', title: 'Interrupt the model (like Enter on an empty line)', onclick: () => conSend('') }, ic('stop'), 'Stop'),
      h('button', { class: 'btn small', onclick: () => { conSend(inp.value); inp.value = ''; } }, 'Send')));
  CON.state = state;
  CON.term = new BiaTerm(host, {
    pinTop: 3,
    convertEol: true,
    onResize: (rows, cols) => { if (CON.ws && CON.ws.readyState === 1) CON.ws.send(JSON.stringify({ t: 'resize', rows, cols })); },
  });
  CON.term.setFontSize(S.prefs.termFs);
  inp.addEventListener('keydown', e => {
    if (e.key === 'Enter') { e.preventDefault(); conSend(inp.value); inp.value = ''; }
    else if (e.key === 'ArrowUp') { e.preventDefault(); if (CON.hist.length) { CON.hi = CON.hi < 0 ? CON.hist.length - 1 : Math.max(0, CON.hi - 1); inp.value = CON.hist[CON.hi]; } }
    else if (e.key === 'ArrowDown') { e.preventDefault(); if (CON.hi >= 0) { CON.hi++; if (CON.hi >= CON.hist.length) { CON.hi = -1; inp.value = ''; } else inp.value = CON.hist[CON.hi]; } }
  });
  whenSized(CON.term, conConnect);
  setTimeout(() => inp.focus(), 50);
  return v;
}
// connect once the terminal has its real size, so the replay wraps at the right width
function whenSized(t, fn) {
  const tick = () => { if (t.host.clientWidth > 0) { t.fit(); fn(); } else requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
}
function termFont(d) {
  S.prefs.termFs = Math.max(9, Math.min(24, S.prefs.termFs + d)); savePrefs();
  if (CON.term) CON.term.setFontSize(S.prefs.termFs);
  if (SH.term) SH.term.setFontSize(S.prefs.termFs);
}
function conConnect() {
  const t = CON.term;
  t.reset();
  const ws = new WebSocket(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws/term?rows=${t.rows}&cols=${t.cols}`);
  ws.binaryType = 'arraybuffer';
  CON.ws = ws;
  CON.state.textContent = 'connecting'; CON.state.className = 'conn-state';
  ws.onopen = () => { CON.state.textContent = 'live'; CON.state.className = 'conn-state ok'; };
  ws.onmessage = e => t.write(typeof e.data === 'string' ? e.data : new Uint8Array(e.data));
  ws.onclose = () => {
    CON.state.textContent = 'disconnected'; CON.state.className = 'conn-state bad';
    if (CON.ws === ws) { CON.ws = null; setTimeout(() => { if (!CON.ws) conConnect(); }, 3000); }
  };
}
function conSend(line) {
  if (!CON.ws || CON.ws.readyState !== 1) { toast('Console not connected', true); return; }
  if (line.trim()) { CON.hist.push(line); if (CON.hist.length > 200) CON.hist.shift(); }
  CON.hi = -1;
  CON.term.follow = true;
  CON.ws.send(JSON.stringify({ t: 'line', v: line }));
}

// ======================================================================
// root: shell
// ======================================================================
const SH = { term: null, ws: null, ctrl: false, alt: false };
function buildShell() {
  const state = h('span', { class: 'conn-state' }, 'connecting');
  const host = h('div', { class: 'term' });
  const enc = new TextEncoder();
  const keybar = h('div', { class: 'keybar' });
  const v = h('section', { class: 'view' },
    h('div', { class: 'vhead' }, backBtn(), h('div', { class: 'avatar xs tool' }, ic('terminal')),
      h('div', { class: 'tbox' }, h('div', { class: 'title', id: 'shTitle' }, 'Root shell'), h('div', { class: 'subtitle' }, 'bash -l as root on this machine')),
      state,
      iconBtn('fontDown', 'Smaller', () => termFont(-1)), iconBtn('fontUp', 'Larger', () => termFont(1)),
      iconBtn('paste', 'Paste', async () => { try { SH.term.paste(await navigator.clipboard.readText()); } catch { toast('Clipboard not available', true); } }),
      iconBtn('refresh', 'New session', () => { if (SH.ws) { SH.ws.onclose = null; SH.ws.close(); } shConnect(); })),
    host, keybar);
  SH.state = state;
  SH.term = new BiaTerm(host, {
    keyboard: true,
    onData: s => {
      if (!SH.ws || SH.ws.readyState !== 1) return;
      if ((SH.ctrl || SH.alt) && s.length === 1) {
        let c = s;
        if (SH.ctrl) { const k = s.toLowerCase(); if (k >= 'a' && k <= 'z') c = String.fromCharCode(k.charCodeAt(0) - 96); }
        if (SH.alt) c = '\x1b' + c;
        s = c; SH.ctrl = SH.alt = false; renderKeybar();
      }
      SH.ws.send(enc.encode(s));
    },
    onResize: (rows, cols) => { if (SH.ws && SH.ws.readyState === 1) SH.ws.send(JSON.stringify({ t: 'resize', rows, cols })); },
    onTitle: t => { const el = $('#shTitle'); if (el) el.textContent = t || 'Root shell'; },
  });
  SH.term.setFontSize(S.prefs.termFs);
  const keys = [['Esc', '\x1b'], ['Tab', '\t'], ['Ctrl', 'ctrl'], ['Alt', 'alt'], ['←', '\x1b[D'], ['↑', '\x1b[A'], ['↓', '\x1b[B'], ['→', '\x1b[C'], ['Home', '\x1b[H'], ['End', '\x1b[F'], ['PgUp', '\x1b[5~'], ['PgDn', '\x1b[6~'], ['|', '|'], ['~', '~'], ['/', '/'], ['-', '-']];
  function renderKeybar() {
    set(keybar, ...keys.map(([label, seq]) => h('button', {
      class: (seq === 'ctrl' && SH.ctrl) || (seq === 'alt' && SH.alt) ? 'on' : '',
      onmousedown: e => e.preventDefault(),
      onclick: () => {
        if (seq === 'ctrl') SH.ctrl = !SH.ctrl; else if (seq === 'alt') SH.alt = !SH.alt;
        else SH.term.opts.onData(seq);
        renderKeybar(); SH.term.focus();
      },
    }, label)));
  }
  renderKeybar();
  whenSized(SH.term, () => { shConnect(); SH.term.focus(); });
  return v;
}
function shConnect() {
  const t = SH.term;
  t.reset();
  const ws = new WebSocket(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws/shell?rows=${t.rows}&cols=${t.cols}`);
  ws.binaryType = 'arraybuffer';
  SH.ws = ws;
  SH.state.textContent = 'connecting'; SH.state.className = 'conn-state';
  ws.onopen = () => { SH.state.textContent = 'live'; SH.state.className = 'conn-state ok'; t.focus(); };
  ws.onmessage = e => t.write(typeof e.data === 'string' ? e.data : new Uint8Array(e.data));
  ws.onclose = () => {
    SH.state.textContent = 'closed'; SH.state.className = 'conn-state bad';
    t.write('\r\n\x1b[90m[session closed — press the refresh button for a new shell]\x1b[0m\r\n');
  };
}

// ======================================================================
// root: files
// ======================================================================
const F = { path: null, home: '', entries: [], sel: new Set(), anchor: null, sort: 'name', asc: true, hidden: false, filter: '', clip: null, hist: [] };
function buildFiles() {
  const filter = h('input', { class: 'input ffilter', placeholder: 'Filter', type: 'search' });
  filter.addEventListener('input', () => { F.filter = filter.value; renderFiles(); });
  const v = h('section', { class: 'view' },
    h('div', { class: 'vhead' }, backBtn(), h('div', { class: 'avatar xs tool' }, ic('folder')),
      h('div', { class: 'tbox' }, h('div', { class: 'title' }, 'Files'), h('div', { class: 'subtitle', id: 'fsub' }, '')),
      iconBtn('upload', 'Upload files', () => fsPickUpload(false)),
      iconBtn('folderPlus', 'New folder', fsMkdir),
      iconBtn('filePlus', 'New file', fsTouch),
      iconBtn('more', 'More', e => {
        const r = e.currentTarget.getBoundingClientRect();
        menu(r.right - 230, r.bottom + 4, [
          { icon: 'upload', text: 'Upload folder', onClick: () => fsPickUpload(true) },
          { icon: 'eye', text: F.hidden ? 'Hide hidden files' : 'Show hidden files', onClick: () => { F.hidden = !F.hidden; renderFiles(); } },
          { icon: 'download', text: 'Download this folder (.tar)', onClick: () => download(fsUrl(F.path, true)) },
          { icon: 'copy', text: 'Copy path', onClick: () => copyText(F.path, 'Path') },
          { icon: 'terminal', text: 'Open shell', onClick: () => go('shell') },
        ]);
      })),
    h('div', { class: 'fbar' },
      iconBtn('back', 'Back', () => { F.hist.pop(); const p = F.hist.pop(); if (p) filesLoad(p); }),
      iconBtn('up', 'Up', () => filesLoad(parentOf(F.path))),
      iconBtn('home', 'bia files', () => filesLoad(F.home || '/home/bia/files')),
      iconBtn('root', 'Root /', () => filesLoad('/')),
      h('div', { class: 'crumbs', id: 'crumbs' }),
      filter,
      iconBtn('refresh', 'Refresh', () => filesLoad(F.path))),
    h('div', { class: 'flist', id: 'flist', tabindex: 0 }),
    h('div', { class: 'fstatus', id: 'fstatus' }));
  const list = $('#flist', v);
  let depth = 0;
  list.addEventListener('dragenter', e => { if (hasFiles(e)) { e.preventDefault(); depth++; list.style.outline = '2px dashed var(--acc)'; } });
  list.addEventListener('dragover', e => { if (hasFiles(e)) e.preventDefault(); });
  list.addEventListener('dragleave', () => { if (--depth <= 0) { depth = 0; list.style.outline = ''; } });
  list.addEventListener('drop', async e => {
    if (!hasFiles(e)) return;
    e.preventDefault(); depth = 0; list.style.outline = '';
    const items = [...e.dataTransfer.items].map(i => i.webkitGetAsEntry && i.webkitGetAsEntry()).filter(Boolean);
    const files = items.length ? await walkEntries(items) : [...e.dataTransfer.files].map(f => ({ file: f, rel: f.name }));
    fsUpload(files);
  });
  list.addEventListener('contextmenu', e => { if (e.target === list || e.target.closest('.fempty')) { e.preventDefault(); bgMenu(e); } });
  list.addEventListener('click', e => { if (e.target === list) { F.sel.clear(); renderFiles(); } });
  list.addEventListener('keydown', fsKey);
  return v;
}
const parentOf = p => (p && p !== '/' ? p.replace(/\/[^/]+\/?$/, '') || '/' : '/');
const joinPath = (d, n) => (d === '/' ? '/' + n : d + '/' + n);
const fsUrl = (p, dl) => `/v1/fs/get?path=${encodeURIComponent(p)}${dl ? '&dl=1' : ''}`;

function filesNavigate(p) {
  if (p) { filesLoad(p); return; }
  if (!F.path) filesLoad(S.me.files || '/home/bia/files');
}
async function filesLoad(p) {
  try {
    const j = await GET('/v1/fs/list?path=' + encodeURIComponent(p));
    F.path = j.path; F.home = j.home; F.entries = j.entries; F.error = j.error;
    if (F.hist[F.hist.length - 1] !== F.path) F.hist.push(F.path);
    if (F.hist.length > 50) F.hist.shift();
    F.sel.clear();
    if (S.filesFocus) { F.sel.add(S.filesFocus); S.filesFocus = null; }
    const want = 'files=' + F.path;
    if (decodeURIComponent(location.hash.slice(1)) !== want && S.route.startsWith('files')) { history.replaceState(null, '', '#' + want); S.route = want; }
    renderFiles();
    const s = F.sel.size && $('#flist .frow.sel'); if (s) s.scrollIntoView({ block: 'center' });
  } catch (e) { toast(e.message, true); if (!F.path) { F.path = '/'; } }
}
function fsKind(e) {
  if (e.dir) return 'dir';
  const m = e.mime || '';
  if (m.startsWith('image/')) return 'img';
  if (m.startsWith('video/')) return 'vid';
  if (m.startsWith('audio/')) return 'aud';
  if (m.startsWith('text/') || m.includes('json')) return 'txt';
  if (/\.(zip|tar|gz|tgz|xz|bz2|7z|zst|deb)$/i.test(e.name)) return 'arc';
  return 'bin';
}
const KIND_ICON = { dir: 'folder', img: 'image', vid: 'video', aud: 'audio', txt: 'file', arc: 'archive', bin: 'file' };
function renderFiles() {
  const list = $('#flist');
  if (!list) return;
  const crumbs = $('#crumbs');
  const parts = F.path === '/' ? [] : F.path.split('/').filter(Boolean);
  const cr = [h('button', { onclick: () => filesLoad('/') }, '/')];
  parts.forEach((p, i) => {
    if (i) cr.push(h('span', { class: 'sep' }, '/'));
    const full = '/' + parts.slice(0, i + 1).join('/');
    cr.push(h('button', { onclick: () => filesLoad(full) }, p));
  });
  set(crumbs, ...cr);
  crumbs.scrollLeft = crumbs.scrollWidth;
  crumbs.ondblclick = () => {
    const inp = h('input', { class: 'input', value: F.path });
    inp.onkeydown = e => { if (e.key === 'Enter') filesLoad(inp.value.trim() || '/'); if (e.key === 'Escape') renderFiles(); };
    inp.onblur = () => renderFiles();
    set(crumbs, inp); inp.focus(); inp.select();
  };
  $('#fsub').textContent = F.path;
  let ents = F.entries.filter(e => (F.hidden || !e.name.startsWith('.')) && (!F.filter || e.name.toLowerCase().includes(F.filter.toLowerCase())));
  const key = F.sort, dir = F.asc ? 1 : -1;
  ents.sort((a, b) => (b.dir - a.dir) || dir * (key === 'size' ? a.size - b.size : key === 'mtime' ? a.mtime - b.mtime : a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' })));
  F.view = ents;
  const th = (k, t, cls) => h('th', { class: cls || '', onclick: () => { if (F.sort === k) F.asc = !F.asc; else { F.sort = k; F.asc = true; } renderFiles(); } }, t, F.sort === k ? (F.asc ? ' ▲' : ' ▼') : '');
  const rows = ents.map((e, idx) => {
    const full = joinPath(F.path, e.name);
    const kind = fsKind(e);
    const tr = h('tr', { class: 'frow' + (F.sel.has(e.name) ? ' sel' : ''), 'data-name': e.name, title: e.link ? `→ ${e.link}` : '' },
      h('td', { class: 'fname' }, h('div', { class: 'fn' }, h('div', { class: `ficon ${kind}${e.link ? ' link' : ''}` }, ic(KIND_ICON[kind])), h('span', null, e.name))),
      h('td', { class: 'num' }, e.dir ? '' : fmtBytes(e.size)),
      h('td', { class: 'num' }, e.mtime ? new Date(e.mtime * 1000).toLocaleString(undefined, { year: '2-digit', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }) : ''),
      h('td', { class: 'dim' }, e.mode));
    tr.addEventListener('click', ev => {
      if (ev.pointerType === 'touch' && !F.sel.size) { fsOpen(e); return; }
      fsSelect(e.name, idx, ev);
    });
    tr.addEventListener('dblclick', () => fsOpen(e));
    tr.addEventListener('contextmenu', ev => {
      ev.preventDefault();
      if (!F.sel.has(e.name)) { F.sel.clear(); F.sel.add(e.name); renderFiles(); }
      fileMenu(ev.clientX, ev.clientY, e, full);
    });
    onLongPress(tr, ev => { if (!F.sel.has(e.name)) { F.sel.clear(); F.sel.add(e.name); renderFiles(); } fileMenu(ev.clientX, ev.clientY, e, full); });
    return tr;
  });
  const sc = list.scrollTop;
  set(list, 
    F.error ? h('div', { class: 'fempty', style: 'color:var(--err)' }, F.error) : null,
    h('table', { class: 'ftable' }, h('thead', null, h('tr', null, th('name', 'Name'), th('size', 'Size', 'num'), th('mtime', 'Modified', 'num'), h('th', null, 'Mode'))), h('tbody', null, rows)),
    !ents.length && !F.error ? h('div', { class: 'fempty' }, F.filter ? 'Nothing matches' : 'Empty folder — drop files here to upload') : null,
    h('div', { id: 'uploads' }));
  list.scrollTop = sc;
  const selSize = ents.filter(e => F.sel.has(e.name)).reduce((a, e) => a + (e.size || 0), 0);
  set($('#fstatus'), 
    h('span', null, `${ents.length} items`),
    F.sel.size ? h('span', null, `${F.sel.size} selected${selSize ? ' · ' + fmtBytes(selSize) : ''}`) : null,
    F.clip ? h('span', null, `${F.clip.paths.length} in clipboard (${F.clip.op}) — `, h('a', { href: '#', onclick: e => { e.preventDefault(); fsPaste(); } }, 'paste here')) : null);
  renderUploads();
}
function fsSelect(name, idx, ev) {
  if (ev.shiftKey && F.anchor !== null) {
    const [a, b] = [Math.min(F.anchor, idx), Math.max(F.anchor, idx)];
    if (!(ev.ctrlKey || ev.metaKey)) F.sel.clear();
    for (let i = a; i <= b; i++) F.sel.add(F.view[i].name);
  } else if (ev.ctrlKey || ev.metaKey) {
    if (F.sel.has(name)) F.sel.delete(name); else F.sel.add(name);
    F.anchor = idx;
  } else { F.sel.clear(); F.sel.add(name); F.anchor = idx; }
  renderFiles();
}
function fsKey(e) {
  const v = F.view || [];
  if ((e.ctrlKey || e.metaKey) && e.key === 'a') { e.preventDefault(); v.forEach(x => F.sel.add(x.name)); renderFiles(); return; }
  if ((e.ctrlKey || e.metaKey) && e.key === 'c' && F.sel.size) { F.clip = { op: 'copy', paths: selPaths() }; renderFiles(); toast('Copied'); return; }
  if ((e.ctrlKey || e.metaKey) && e.key === 'x' && F.sel.size) { F.clip = { op: 'cut', paths: selPaths() }; renderFiles(); toast('Cut'); return; }
  if ((e.ctrlKey || e.metaKey) && e.key === 'v' && F.clip) { fsPaste(); return; }
  if (e.key === 'Delete' && F.sel.size) { fsDelete(selPaths()); return; }
  if (e.key === 'F2' && F.sel.size === 1) { fsRename(joinPath(F.path, [...F.sel][0])); return; }
  if (e.key === 'Backspace') { filesLoad(parentOf(F.path)); return; }
  if (e.key === 'Enter' && F.sel.size === 1) { const x = v.find(q => F.sel.has(q.name)); if (x) fsOpen(x); return; }
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault();
    let i = v.findIndex(x => F.sel.has(x.name));
    i = Math.max(0, Math.min(v.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)));
    if (v[i]) { F.sel.clear(); F.sel.add(v[i].name); F.anchor = i; renderFiles(); const r = $(`#flist .frow[data-name="${CSS.escape(v[i].name)}"]`); if (r) r.scrollIntoView({ block: 'nearest' }); }
  }
}
const selPaths = () => [...F.sel].map(n => joinPath(F.path, n));

function fsOpen(e) {
  const full = joinPath(F.path, e.name);
  if (e.dir) { filesLoad(full); return; }
  const k = fsKind(e);
  if (k === 'txt' || (k === 'bin' && e.size < 256 * 1024 && !/\.(bin|img|iso|so|o|a|gguf|p12|der|db|sqlite)$/i.test(e.name))) { fsEdit(full, e); return; }
  if (k === 'img' || k === 'vid' || k === 'aud' || e.mime === 'application/pdf') {
    const media = (F.view || []).filter(x => ['img', 'vid'].includes(fsKind(x)));
    if (k !== 'aud' && e.mime !== 'application/pdf' && media.length > 1) {
      openViewer(media.map(x => ({ url: fsUrl(joinPath(F.path, x.name)), dl: fsUrl(joinPath(F.path, x.name), true), name: x.name, kind: fsKind(x) === 'img' ? 'photo' : 'video' })), media.findIndex(x => x.name === e.name));
    } else openViewer([{ url: fsUrl(full), dl: fsUrl(full, true), name: e.name, kind: k === 'img' ? 'photo' : k === 'vid' ? 'video' : k === 'aud' ? 'audio' : 'pdf' }], 0);
    return;
  }
  download(fsUrl(full, true), e.name);
}

function fileMenu(x, y, e, full) {
  const many = F.sel.size > 1;
  const paths = selPaths();
  menu(x, y, [
    !many ? { icon: e.dir ? 'folder' : 'eye', text: 'Open', onClick: () => fsOpen(e) } : null,
    !many && !e.dir ? { icon: 'edit', text: 'Edit as text', onClick: () => fsEdit(full, e) } : null,
    { icon: 'download', text: many ? `Download ${paths.length} items` : (e.dir ? 'Download (.tar)' : 'Download'), onClick: () => paths.forEach((p, i) => setTimeout(() => download(fsUrl(p, true)), i * 400)) },
    paths.length && !(F.entries.filter(q => F.sel.has(q.name)).some(q => q.dir)) ? { icon: 'chat', text: 'Send to chat…', onClick: () => fsSendToChat(paths) } : null,
    '-',
    !many ? { icon: 'edit', text: 'Rename', onClick: () => fsRename(full) } : null,
    { icon: 'copy', text: 'Copy', onClick: () => { F.clip = { op: 'copy', paths }; renderFiles(); } },
    { icon: 'cut', text: 'Cut', onClick: () => { F.clip = { op: 'cut', paths }; renderFiles(); } },
    F.clip ? { icon: 'paste', text: 'Paste here', onClick: fsPaste } : null,
    { icon: 'link', text: 'Copy path', onClick: () => copyText(paths.join('\n'), 'Path') },
    !many ? { icon: 'info', text: 'Properties', onClick: () => fsProps(full, e) } : null,
    '-',
    { icon: 'trash', text: 'Delete', danger: true, onClick: () => fsDelete(paths) },
  ]);
}
function bgMenu(e) {
  menu(e.clientX, e.clientY, [
    { icon: 'upload', text: 'Upload files', onClick: () => fsPickUpload(false) },
    { icon: 'folderPlus', text: 'New folder', onClick: fsMkdir },
    { icon: 'filePlus', text: 'New file', onClick: fsTouch },
    F.clip ? { icon: 'paste', text: `Paste ${F.clip.paths.length} item(s)`, onClick: fsPaste } : null,
    { icon: 'refresh', text: 'Refresh', onClick: () => filesLoad(F.path) },
  ]);
}
async function fsOp(body, okMsg) {
  try { const r = await POST('/v1/fs/op', body); if (okMsg) toast(okMsg); return r; }
  catch (e) { toast(e.message, true); return null; }
  finally { filesLoad(F.path); }
}
async function fsMkdir() {
  const n = await promptBox('New folder', 'New folder');
  if (n) fsOp({ op: 'mkdir', path: joinPath(F.path, n) });
}
async function fsTouch() {
  const n = await promptBox('New file', 'untitled.txt', { selectStem: true });
  if (!n) return;
  const p = joinPath(F.path, n);
  if (await fsOp({ op: 'touch', path: p })) fsEdit(p, { name: n, size: 0 });
}
async function fsRename(full) {
  const old = full.split('/').pop();
  const n = await promptBox('Rename', old, { selectStem: true });
  if (n && n !== old) fsOp({ op: 'rename', paths: [full], to: joinPath(parentOf(full), n) });
}
async function fsDelete(paths) {
  const ok = await confirmBox({ title: paths.length > 1 ? `Delete ${paths.length} items?` : `Delete ${paths[0].split('/').pop()}?`, body: 'Folders are deleted with everything inside. This cannot be undone.' }, 'Delete', true);
  if (ok) fsOp({ op: 'delete', paths }, 'Deleted');
}
async function fsPaste() {
  const c = F.clip;
  if (!c) return;
  const r = await fsOp({ op: c.op === 'cut' ? 'move' : 'copy', paths: c.paths, to: F.path }, c.op === 'cut' ? 'Moved' : 'Copied');
  if (r && c.op === 'cut') F.clip = null;
}
async function fsProps(full, e) {
  const body = h('div', { class: 'card', style: 'margin:0' }, h('div', { class: 'kv' },
    h('div', null, 'Path'), h('div', { class: 'mono', style: 'word-break:break-all' }, full),
    h('div', null, 'Type'), h('div', null, e.dir ? 'folder' : (e.mime || 'file')),
    e.link ? [h('div', null, 'Link to'), h('div', { class: 'mono' }, e.link)] : null,
    h('div', null, 'Size'), h('div', { id: 'propSize' }, e.dir ? 'counting…' : `${fmtBytes(e.size)} (${e.size} bytes)`),
    h('div', null, 'Modified'), h('div', null, fmtDate(e.mtime)),
    h('div', null, 'Mode'), h('div', null, h('input', { class: 'input mono', id: 'propMode', value: e.mode, style: 'width:100px' }))));
  if (e.dir) POST('/v1/fs/op', { op: 'du', path: full }).then(r => { const el = $('#propSize'); if (el) el.textContent = `${fmtBytes(r.bytes)} in ${r.files} files`; }).catch(() => {});
  const ok = await modal({ title: e.name, body, buttons: [{ text: 'Close', value: false }, { text: 'Apply mode', value: true }] });
  const mode = $('#propMode') ? $('#propMode').value : e.mode;
  if (ok && mode !== e.mode) fsOp({ op: 'chmod', path: full, mode }, 'Mode changed');
}
async function fsSendToChat(paths) {
  const chat = await pickChat('Send to chat…');
  if (!chat) return;
  try { await act('send', { chat, text: '', files: paths }); toast('Sent'); go('chat/' + chat); }
  catch (e) { toast(e.message, true); }
}

async function fsEdit(full, e) {
  let text = '';
  try {
    const r = await fetch(fsUrl(full), { credentials: 'include' });
    if (!r.ok) throw new Error(await r.text());
    const buf = await r.arrayBuffer();
    if (buf.byteLength > 5 * 1024 * 1024) { toast('Too large to edit here (5 MB max) — download it instead', true); return; }
    const u8 = new Uint8Array(buf);
    if (u8.slice(0, 8000).includes(0) && !(await confirmBox('This looks like a binary file. Open it as text anyway?', 'Open'))) return;
    text = new TextDecoder().decode(u8);
  } catch (er) { toast(er.message, true); return; }
  const ed = h('textarea', { class: 'editor', spellcheck: false });
  ed.value = text;
  let dirty = false;
  const status = h('span', { class: 'muted' }, '');
  ed.addEventListener('input', () => { dirty = true; status.textContent = 'modified'; });
  ed.addEventListener('keydown', ev => {
    if ((ev.ctrlKey || ev.metaKey) && ev.key === 's') { ev.preventDefault(); save(); }
    if (ev.key === 'Tab') { ev.preventDefault(); ed.setRangeText('\t', ed.selectionStart, ed.selectionEnd, 'end'); dirty = true; }
  });
  const save = async () => {
    try {
      const r = await fetch(`/v1/fs/put?path=${encodeURIComponent(full)}&overwrite=1`, { method: 'POST', credentials: 'include', body: new Blob([ed.value]) });
      if (!r.ok) throw new Error(await r.text());
      dirty = false; status.textContent = 'saved ' + fmtTime(Date.now());
      return true;
    } catch (er) { toast('Save failed: ' + er.message, true); return false; }
  };
  await modal({
    title: h('span', { class: 'row grow' }, ic('file'), h('span', { class: 'ellipsis grow' }, full), status),
    cls: 'full', body: ed, dismiss: false,
    buttons: [
      { text: 'Download', onClick: () => { download(fsUrl(full, true)); return false; } },
      { text: 'Close', onClick: async () => !dirty || await confirmBox('Discard unsaved changes?', 'Discard', true) || false },
      { text: 'Save', primary: true, cls: '', onClick: async () => { await save(); return false; } },
    ],
  });
  filesLoad(F.path);
}

// ---- uploads ----
const UP = { queue: [], active: null, done: 0, policy: null };
function fsPickUpload(folder) {
  const inp = h('input', { type: 'file', multiple: true, style: 'display:none' });
  if (folder) inp.webkitdirectory = true;
  inp.onchange = () => { fsUpload([...inp.files].map(f => ({ file: f, rel: f.webkitRelativePath || f.name }))); inp.remove(); };
  document.body.appendChild(inp); inp.click();
}
async function walkEntries(entries, prefix = '') {
  const out = [];
  for (const en of entries) {
    if (en.isFile) out.push({ file: await new Promise((res, rej) => en.file(res, rej)), rel: prefix + en.name });
    else if (en.isDirectory) {
      const reader = en.createReader();
      let batch;
      const all = [];
      do { batch = await new Promise((res, rej) => reader.readEntries(res, rej)); all.push(...batch); } while (batch.length);
      out.push(...await walkEntries(all, prefix + en.name + '/'));
    }
  }
  return out;
}
function fsUpload(items) {
  const dir = F.path;
  for (const it of items) UP.queue.push({ ...it, dir, pct: 0 });
  UP.policy = null;
  renderUploads();
  if (!UP.active) uploadNext();
}
async function uploadNext() {
  const it = UP.queue.shift();
  if (!it) { UP.active = null; renderUploads(); filesLoad(F.path); return; }
  UP.active = it;
  const dest = joinPath(it.dir, it.rel);
  const put = (ow) => xhrUpload(`/v1/fs/put?path=${encodeURIComponent(dest)}${ow ? '&overwrite=1' : ''}`, it.file, (d, t, x) => { if (x) it.xhr = x; it.pct = t ? d / t : 0; renderUploads(); });
  try {
    await put(false);
  } catch (e) {
    if (e.status === 409) {
      let choice = UP.policy;
      if (!choice) {
        choice = await modal({
          title: 'File exists', body: h('div', null, h('span', { class: 'mono' }, dest), ' already exists.'),
          buttons: [{ text: 'Skip', value: 'skip' }, { text: 'Skip all', value: 'skipall' }, { text: 'Replace all', value: 'replaceall' }, { text: 'Replace', value: 'replace', primary: true }],
        }) || 'skip';
        if (choice.endsWith('all')) UP.policy = choice;
      }
      if (choice.startsWith('replace')) { try { await put(true); } catch (e2) { toast(`${it.rel}: ${e2.message}`, true); } }
    } else toast(`${it.rel}: ${e.message}`, true);
  }
  UP.done++;
  uploadNext();
}
function renderUploads() {
  const box = $('#uploads');
  if (!box) return;
  if (!UP.active && !UP.queue.length) { set(box); box.className = ''; UP.done = 0; return; }
  box.className = 'uploads';
  const a = UP.active;
  set(box, 
    h('div', { class: 'row' }, h('b', { class: 'grow' }, `Uploading ${UP.done + 1} of ${UP.done + 1 + UP.queue.length}`),
      h('button', { class: 'btn flat small', onclick: () => { UP.queue.length = 0; if (a && a.xhr) a.xhr.abort(); } }, 'Cancel')),
    a ? h('div', { class: 'u' }, h('div', { class: 'ellipsis' }, a.rel), h('div', { class: 'ubar' }, h('i', { style: `width:${Math.round(a.pct * 100)}%` })), h('div', { class: 'muted' }, `${Math.round(a.pct * 100)}% of ${fmtBytes(a.file.size)}`)) : null);
}

// ======================================================================
// root: human tasks
// ======================================================================
async function pollTasks() {
  let j;
  try { j = await GET('/v1/hitl'); } catch { return; }
  const tasks = j.tasks || [];
  for (const t of tasks) {
    if (!S.taskSeen.has(t.id)) {
      S.taskSeen.add(t.id);
      toast('bia needs you: ' + t.title);
      blip();
      if (S.prefs.notify && document.hidden && 'Notification' in window && Notification.permission === 'granted') {
        try { const n = new Notification('bia needs you', { body: t.title, icon: '/icon.svg', tag: 'hitl-' + t.id }); n.onclick = () => { window.focus(); go('task/' + t.id); }; } catch {}
      }
    }
  }
  const changed = JSON.stringify(tasks.map(t => t.id)) !== JSON.stringify(S.tasks.map(t => t.id));
  S.tasks = tasks;
  if (changed) { renderChatList(); renderTaskbar(); renderTasks(); updateTitle(); }
}
async function pollStatus() {
  try {
    const st = await GET('/v1/status');
    const was = S.status && S.status.state;
    S.status = st;
    if (was !== st.state) { renderChatHead(); renderChatList(); }
  } catch {}
}
function buildTasks() {
  return h('section', { class: 'view' },
    h('div', { class: 'vhead' }, backBtn(), h('div', { class: 'avatar xs tool' }, ic('hand')),
      h('div', { class: 'tbox' }, h('div', { class: 'title' }, 'Human tasks'), h('div', { class: 'subtitle' }, 'steps bia asked you to do (ask_human)')),
      iconBtn('refresh', 'Refresh', pollTasks)),
    h('div', { class: 'page' }, h('div', { class: 'page-inner', id: 'taskList' })));
}
function renderTasks() {
  const el = $('#taskList');
  if (!el) return;
  if (!S.tasks.length) { set(el, h('div', { class: 'fempty' }, 'Nothing is waiting for you.')); return; }
  set(el, ...S.tasks.map(t => h('div', { class: 'card task' },
    h('div', { class: 'avatar sm c2' }, ic('hand')),
    h('div', { class: 'tbody' },
      h('div', { class: 'ttitle' }, t.title),
      h('div', { class: 'muted', style: 'font-size:13px' }, t.created ? 'asked ' + fmtDate(t.created) : ''),
      t.message ? h('div', { class: 'tmsg' }, t.message) : null,
      h('div', { class: 'actions' },
        h('button', { class: 'btn', onclick: () => go('task/' + t.id) }, ic('display'), 'Open'),
        h('button', { class: 'btn ghost', onclick: () => window.open('/hitl/' + t.id, '_blank', 'noopener') }, ic('external'), 'New tab'),
        // Login with ChatGPT is finished by picking a model on its page
        t.kind === 'chatgpt' ? null : h('button', { class: 'btn ghost', onclick: () => finishTask(t, true) }, ic('check'), 'Done'),
        h('button', { class: 'btn flat danger', onclick: () => finishTask(t, false) }, "Can't do it"))))));
}
async function finishTask(t, ok) {
  const note = await promptBox(ok ? `Done: ${t.title}` : `Failed: ${t.title}`, '', { placeholder: 'Note for bia (optional)', multiline: true, ok: ok ? 'Done' : 'Report failure' });
  if (note === null || note === undefined) return;
  try {
    await POST(`/v1/hitl/${encodeURIComponent(t.id)}/finish`, { ok, note });
    toast(ok ? 'Told bia it is done' : 'Told bia it failed');
    await pollTasks();
    if (S.route.startsWith('task/')) go('tasks');
  } catch (e) { toast(e.message, true); }
}
function buildTaskFrame() {
  return h('section', { class: 'view' },
    h('div', { class: 'vhead', id: 'taskHead' }),
    h('div', { class: 'frame-wrap' }, h('iframe', { id: 'taskFrame', allow: 'clipboard-read; clipboard-write; fullscreen' })));
}
function openTaskFrame(id) {
  const t = S.tasks.find(x => x.id === id) || { id, title: 'Human task' };
  set($('#taskHead'), iconBtn('back', 'Back', () => go('tasks')), h('div', { class: 'avatar xs c2' }, ic('hand')),
    h('div', { class: 'tbox' }, h('div', { class: 'title ellipsis' }, t.title), h('div', { class: 'subtitle ellipsis' }, t.message || 'do the step on the machine display, then press Done')),
    t.kind === 'chatgpt' ? null : h('button', { class: 'btn small', onclick: () => finishTask(t, true) }, ic('check'), 'Done'),
    h('button', { class: 'btn flat danger small', onclick: () => finishTask(t, false) }, 'Failed'),
    iconBtn('external', 'New tab', () => window.open('/hitl/' + id, '_blank', 'noopener')));
  const f = $('#taskFrame');
  if (f.dataset.id !== id) { f.dataset.id = id; f.src = '/hitl/' + encodeURIComponent(id); }
}

// ======================================================================
// root: display
// ======================================================================
function buildDisplay() {
  return h('section', { class: 'view' },
    h('div', { class: 'vhead' }, backBtn(), h('div', { class: 'avatar xs tool' }, ic('display')),
      h('div', { class: 'tbox' }, h('div', { class: 'title' }, 'Display'), h('div', { class: 'subtitle' }, 'screen :0 over VNC (root only)')),
      iconBtn('refresh', 'Reconnect', () => { const f = $('#displayFrame'); f.src = 'about:blank'; setTimeout(() => { f.src = '/gui'; }, 50); }),
      iconBtn('external', 'New tab', () => window.open('/gui', '_blank', 'noopener'))),
    h('div', { class: 'frame-wrap' }, h('iframe', { id: 'displayFrame', allow: 'clipboard-read; clipboard-write; fullscreen' })));
}

// ======================================================================
// settings
// ======================================================================
function buildSettings() {
  return h('section', { class: 'view' },
    h('div', { class: 'vhead' }, backBtn(), h('div', { class: 'tbox' }, h('div', { class: 'title' }, 'Settings'))),
    h('div', { class: 'page' }, h('div', { class: 'page-inner', id: 'settingsBody' })));
}
async function renderSettings() {
  const el = $('#settingsBody');
  if (!el) return;
  const me = S.me;
  const sw = (checked, on) => { const i = h('input', { type: 'checkbox', checked }); i.onchange = () => on(i.checked); return h('label', { class: 'switch' }, i, h('span')); };
  const seg = (opts, cur, on) => h('div', { class: 'seg' }, opts.map(([v, t]) => h('button', { class: v === cur ? 'on' : '', onclick: () => on(v) }, t)));

  const profile = h('div', { class: 'card' },
    h('div', { class: 'row', style: 'gap:14px' }, h('div', { class: 'avatar ' + colorOf(me.identity) }, initials(me.identity)),
      h('div', { class: 'grow' }, h('div', { style: 'font-size:18px;font-weight:600' }, me.identity),
        h('div', { class: 'muted' }, `${me.role} · trust ${me.trust}${S.root ? ' · root' : ''}`))),
    h('div', { class: 'kv', style: 'margin-top:14px' },
      h('div', null, 'Certificate'), h('div', { class: 'mono' }, me.serial || '—'),
      h('div', null, 'Expires'), h('div', null, fmtDate(me.not_after), ' ', h('span', { class: 'tag ' + (me.days_left < 30 ? 'bad' : 'ok') }, `${me.days_left} days left`))),
    h('div', { class: 'row', style: 'margin-top:12px;flex-wrap:wrap' },
      h('button', { class: 'btn', onclick: renewCert }, ic('refresh'), 'Renew my certificate'),
      h('button', { class: 'btn ghost', onclick: () => download('/v1/pki/ca', 'bia-ca.crt') }, ic('download'), 'CA (.crt)'),
      h('button', { class: 'btn ghost', onclick: () => download('/v1/pki/ca?format=der', 'bia-ca.cer') }, 'CA (.cer)')));

  const appearance = h('div', { class: 'card' }, h('h3', null, ic('moon'), 'Appearance'),
    h('div', { class: 'setrow' }, h('div', { class: 'grow' }, 'Theme'), seg([['auto', 'Auto'], ['light', 'Day'], ['dark', 'Night']], S.prefs.theme, v => { S.prefs.theme = v; savePrefs(); renderSettings(); })),
    h('div', { class: 'setrow' }, h('div', { class: 'grow' }, `Message text size (${S.prefs.fs}px)`),
      h('input', { type: 'range', min: 12, max: 20, value: S.prefs.fs, oninput: e => { S.prefs.fs = +e.target.value; savePrefs(); e.target.parentElement.firstChild.textContent = `Message text size (${S.prefs.fs}px)`; } })),
    h('div', { class: 'setrow' }, h('div', { class: 'grow' }, 'Send with Enter (Shift+Enter for a new line)'), sw(S.prefs.enterSends, v => { S.prefs.enterSends = v; savePrefs(); })));

  const notif = h('div', { class: 'card' }, h('h3', null, ic('bell'), 'Notifications'),
    h('div', { class: 'setrow' }, h('div', { class: 'grow' }, 'Desktop notifications', h('div', { class: 'muted', style: 'font-size:13px' }, 'Notification' in window ? `browser permission: ${Notification.permission}` : 'not supported by this browser')),
      sw(S.prefs.notify && 'Notification' in window && Notification.permission === 'granted', async v => {
        S.prefs.notify = v; savePrefs();
        if (v && 'Notification' in window && Notification.permission !== 'granted') { await Notification.requestPermission(); renderSettings(); }
      })),
    h('div', { class: 'setrow' }, h('div', { class: 'grow' }, 'Sound'), sw(S.prefs.sound, v => { S.prefs.sound = v; savePrefs(); if (v) blip(); })));

  const kids = [profile, appearance, notif];
  if (S.root) {
    const issueId = h('input', { class: 'input', placeholder: 'identity, e.g. alice or my-laptop' });
    const role = h('select', { class: 'input' }, h('option', { value: 'user' }, 'user (a person)'), h('option', { value: 'agent' }, 'agent (a program)'));
    const trust = h('select', { class: 'input' }, h('option', { value: 'normal' }, 'normal'), h('option', { value: 'root' }, 'root (full control)'));
    const out = h('div');
    const issue = async () => {
      set(out);
      try {
        const r = await POST('/v1/pki/issue', { identity: issueId.value.trim(), role: role.value, trust: trust.value });
        set(out, h('p', { class: 'muted' }, `One-time pickup link for ${r.identity} (valid 15 minutes, works once, .p12 password 12345678):`),
          ...(r.urls.length ? r.urls : [location.origin + r.path]).map(u => h('div', { class: 'copyline' }, h('code', null, u), iconBtn('copy', 'Copy', () => copyText(u, 'Link')))));
        loadCerts();
      } catch (e) { set(out, h('div', { class: 'err-line' }, e.message)); }
    };
    const certs = h('div', { id: 'certList' }, 'Loading…');
    kids.push(h('div', { class: 'card' }, h('h3', null, ic('shield'), 'Certificates'),
      h('p', { class: 'muted', style: 'margin-top:0' }, 'Give someone (or another agent) access: issue a certificate and send them the pickup link. They also need the CA.'),
      h('div', { class: 'field' }, h('label', null, 'Identity'), issueId),
      h('div', { class: 'row' }, h('div', { class: 'field grow' }, h('label', null, 'Role'), role), h('div', { class: 'field grow' }, h('label', null, 'Trust'), trust)),
      h('button', { class: 'btn', onclick: issue }, ic('key'), 'Issue certificate'), out,
      h('h3', { style: 'margin-top:18px' }, 'Requests from other agents'), h('div', { id: 'reqList' }, 'Loading…'),
      h('h3', { style: 'margin-top:18px' }, 'Issued'), certs));
    const st = h('div', { class: 'kv' }, 'Loading…');
    kids.push(h('div', { class: 'card' }, h('h3', null, ic('console'), 'Machine'), st));
    GET('/v1/status').then(s => {
      const up = s.uptime || 0;
      set(st, 
        h('div', null, 'Version'), h('div', null, s.version || me.version),
        h('div', null, 'System'), h('div', null, `${s.arch} · ${s.kernel}`),
        h('div', null, 'Model'), h('div', null, `${s.model} ${s.model_detail || ''}`),
        h('div', null, 'State'), h('div', null, s.state || 'idle'),
        h('div', null, 'Context'), h('div', null, `${s.ctx_used} / ${s.ctx_total} tokens · ${s.messages} messages`),
        h('div', null, 'Speed'), h('div', null, `${(s.tps || 0).toFixed(1)} tok/s`),
        h('div', null, 'Tools'), h('div', null, String(s.tools)),
        h('div', null, 'Uptime'), h('div', null, `${Math.floor(up / 86400)}d ${Math.floor(up % 86400 / 3600)}h ${Math.floor(up % 3600 / 60)}m`),
        h('div', null, 'Addresses'), h('div', { class: 'mono' }, (s.origins || []).join('  ') || '—'));
    }).catch(e => set(st, e.message));
    setTimeout(() => { loadCerts(); loadRequests(); }, 0);
  }
  kids.push(h('div', { class: 'card' }, h('h3', null, ic('info'), 'About'),
    h('p', { class: 'muted', style: 'margin:0' }, `bia ${me.version || ''} — a machine that boots into an agent. `,
      h('a', { href: '/openapi.yaml', target: '_blank' }, 'OpenAPI'), ' · ', h('a', { href: '/skills/web-console/SKILL.md', target: '_blank' }, 'Skill'), ' · ',
      h('a', { href: '/.well-known/bia.json', target: '_blank' }, 'Discovery'))));
  set(el, ...kids);
}
async function loadCerts() {
  const box = $('#certList');
  if (!box) return;
  try {
    const j = await GET('/v1/pki/certs');
    set(box, h('table', { class: 'certs' },
      h('thead', null, h('tr', null, h('th', null, 'Identity'), h('th', null, 'Role'), h('th', null, 'Expires'), h('th', null, 'Status'), h('th'))),
      h('tbody', null, j.certs.map(c => h('tr', null,
        h('td', null, h('b', null, c.identity), c.current ? h('span', { class: 'tag acc', style: 'margin-left:6px' }, 'this browser') : null, h('div', { class: 'muted mono', style: 'font-size:11px' }, c.serial)),
        h('td', null, `${c.role} · ${c.trust}`),
        h('td', null, c.not_after ? new Date(c.not_after * 1000).toLocaleDateString() : '—'),
        h('td', null, c.revoked ? h('span', { class: 'tag bad' }, 'revoked') : c.expired ? h('span', { class: 'tag bad' }, 'expired') : h('span', { class: 'tag ok' }, 'active')),
        h('td', null, !c.revoked && !c.expired ? h('button', {
          class: 'btn flat danger small', onclick: async () => {
            if (!await confirmBox({ title: `Revoke ${c.identity}?`, body: c.current ? 'This is the certificate this browser uses — you will lose access until you get a new one.' : 'That certificate stops working immediately.' }, 'Revoke', true)) return;
            try { await POST('/v1/pki/revoke', { serial: c.serial }); loadCerts(); } catch (e) { toast(e.message, true); }
          },
        }, 'Revoke') : null))))));
  } catch (e) { set(box, e.message); }
}
async function loadRequests() {
  const box = $('#reqList');
  if (!box) return;
  try {
    const j = await GET('/v1/pki/requests');
    const live = j.requests.filter(r => r.status === 'pending' || r.status === 'approved');
    const old = j.requests.filter(r => !live.includes(r)).slice(0, 10);
    if (!live.length && !old.length) { set(box, h('p', { class: 'muted' }, 'None. Another agent can ask with POST /v1/pki/requests; you decide here.')); return; }
    const tag = { pending: 'acc', approved: 'ok', issued: 'ok', denied: 'bad', expired: 'bad' };
    const decide = (r, verb) => async () => {
      if (verb === 'approve' && !await confirmBox({ title: `Let ${r.identity} in?`, body: 'It gets a normal-trust certificate for this web API the next time it checks. You can revoke it later.' }, 'Approve')) return;
      try { await POST(`/v1/pki/requests/${r.id}/${verb}`, {}); loadRequests(); loadCerts(); } catch (e) { toast(e.message, true); }
    };
    set(box, h('table', { class: 'certs' },
      h('thead', null, h('tr', null, h('th', null, 'Identity'), h('th', null, 'Asked'), h('th', null, 'Status'), h('th'))),
      h('tbody', null, [...live, ...old].map(r => h('tr', null,
        h('td', null, h('b', null, r.identity), ` · ${r.role}`, h('div', { class: 'muted mono', style: 'font-size:11px' }, `${r.id.slice(0, 8)} · ${r.source || ''}`),
          r.note ? h('div', { class: 'muted', style: 'font-size:12px' }, `“${r.note}”`) : null),
        h('td', null, new Date(r.created * 1000).toLocaleString()),
        h('td', null, h('span', { class: 'tag ' + (tag[r.status] || '') }, r.status)),
        h('td', null, r.status === 'pending' ? h('div', { class: 'row' },
          h('button', { class: 'btn small', onclick: decide(r, 'approve') }, 'Approve'),
          h('button', { class: 'btn flat danger small', onclick: decide(r, 'deny') }, 'Deny')) : null))))));
  } catch (e) { set(box, e.message); }
}
async function renewCert() {
  if (!await confirmBox({ title: 'Renew certificate?', body: 'This browser makes a new key, sends bia only a signing request, and saves a new .p12 (password 12345678). The key never leaves this computer. The current certificate is revoked right away, so import the new one before reloading.' }, 'Renew')) return;
  try {
    const { csr, key } = await BiaPki.makeCsr(S.me.identity);
    const r = await http('POST', '/v1/pki/renew', { csr }, true);
    if (!r.ok) throw new Error(await r.text());
    const der = await BiaPki.p12(key, await r.text(), `${S.me.identity} (bia)`);
    saveBlob(new Blob([der], { type: 'application/x-pkcs12' }), `${S.me.identity}-bia.p12`);
    modal({ title: 'New certificate downloaded', body: h('p', null, `Import the .p12 into your browser or OS (password ${BiaPki.PASSWORD}), remove the old one, then reload this page.`), buttons: [{ text: 'Reload now', primary: true, onClick: () => location.reload() }, { text: 'Later', value: 0 }] });
  } catch (e) { toast(e.message, true); }
}

// global shortcuts
document.addEventListener('keydown', e => {
  if ((e.ctrlKey || e.metaKey) && e.key === 'k') { e.preventDefault(); const s = $('#gsearch'); if (s) { s.focus(); s.select(); } }
  if (e.key === 'Escape' && S.selecting && !document.querySelector('.modal-back')) setSelecting(false);
});

// ---- live calls (WebSocket /v1/call: 16 kHz s16 mono up, bia's speech down) ----
const CALL = { ws: null };
const CALL_WORKLETS = `
class Cap extends AudioWorkletProcessor {
  constructor() { super(); this.step = sampleRate / 16000; this.next = this.step; this.pos = 0; this.sum = 0; this.cnt = 0; this.buf = new Int16Array(320); this.n = 0; }
  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (!ch) return true;
    for (let k = 0; k < ch.length; k++) {
      this.sum += ch[k]; this.cnt++; this.pos++;
      if (this.pos >= this.next) {
        this.next += this.step;
        const v = this.sum / this.cnt; this.sum = 0; this.cnt = 0;
        this.buf[this.n++] = Math.max(-32768, Math.min(32767, Math.round(v * 32767)));
        if (this.n === 320) { this.port.postMessage(this.buf.buffer, [this.buf.buffer]); this.buf = new Int16Array(320); this.n = 0; }
      }
    }
    return true;
  }
}
registerProcessor('bia-cap', Cap);
class Play extends AudioWorkletProcessor {
  constructor() {
    super(); this.q = []; this.off = 0; this.busy = false;
    this.port.onmessage = e => {
      if (e.data === 'clear') { this.q = []; this.off = 0; this.busy = false; }
      else if (e.data === 'check') { if (!this.q.length) this.port.postMessage('drained'); }
      else { this.q.push(e.data); this.busy = true; }
    };
  }
  process(_, outputs) {
    const out = outputs[0][0]; let i = 0;
    while (i < out.length && this.q.length) {
      const b = this.q[0]; const n = Math.min(out.length - i, b.length - this.off);
      out.set(b.subarray(this.off, this.off + n), i); i += n; this.off += n;
      if (this.off >= b.length) { this.q.shift(); this.off = 0; }
    }
    for (; i < out.length; i++) out[i] = 0;
    if (this.busy && !this.q.length) { this.busy = false; this.port.postMessage('drained'); }
    return true;
  }
}
registerProcessor('bia-play', Play);`;

function pcmTo(rate, pcm, to) {
  const n = Math.max(1, Math.floor(pcm.length * to / rate));
  const out = new Float32Array(n);
  const r = rate / to;
  for (let i = 0; i < n; i++) {
    const p = i * r, j = Math.floor(p), f = p - j;
    const a = pcm[j] || 0, b = pcm[j + 1] !== undefined ? pcm[j + 1] : a;
    out[i] = (a + (b - a) * f) / 32768;
  }
  return out;
}

async function startCall(chat, ringId) {
  if (CALL.ws) { toast('Already in a call', true); return; }
  if (!navigator.mediaDevices || !window.AudioWorkletNode) { toast('Calls are not supported in this browser', true); return; }
  let stream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 } });
  } catch (e) {
    toast('Microphone: ' + (e.message || e.name), true);
    if (ringId) act('call_decline', { chat, id: ringId }).catch(() => {});
    return;
  }
  const ctx = new AudioContext();
  try {
    await ctx.audioWorklet.addModule(URL.createObjectURL(new Blob([CALL_WORKLETS], { type: 'text/javascript' })));
  } catch (e) {
    stream.getTracks().forEach(t => t.stop()); ctx.close();
    toast('Audio: ' + e.message, true); return;
  }
  const cap = new AudioWorkletNode(ctx, 'bia-cap');
  const play = new AudioWorkletNode(ctx, 'bia-play', { outputChannelCount: [1] });
  const sink = ctx.createGain(); sink.gain.value = 0;   // keeps the capture worklet running
  ctx.createMediaStreamSource(stream).connect(cap);
  cap.connect(sink).connect(ctx.destination);
  play.connect(ctx.destination);
  const proto = location.protocol === 'https:' ? 'wss' : 'ws';
  const ws = new WebSocket(`${proto}://${location.host}/v1/call?chat=${encodeURIComponent(chat)}${ringId ? '&ring=' + encodeURIComponent(ringId) : ''}`);
  ws.binaryType = 'arraybuffer';
  Object.assign(CALL, { ws, ctx, stream, cap, play, chat, gen: 0, sayEnd: -1, muted: false, t0: 0, state: 'connecting', lines: [] });
  cap.port.onmessage = e => {
    if (ws.readyState !== 1) return;
    // muted: keep the stream alive with silence
    ws.send(CALL.muted ? new Int16Array(320).buffer : e.data);
  };
  play.port.onmessage = e => {
    if (e.data === 'drained' && CALL.sayEnd === CALL.gen && CALL.ws === ws) {
      ws.send(JSON.stringify({ t: 'played', gen: CALL.gen }));
      CALL.sayEnd = -1; setCallState('listening');
    }
  };
  ws.onmessage = e => {
    if (typeof e.data !== 'string') {
      const dv = new DataView(e.data);
      const gen = dv.getUint32(0, true), rate = dv.getUint32(4, true);
      if (gen !== (CALL.gen >>> 0)) return;
      play.port.postMessage(pcmTo(rate, new Int16Array(e.data, 8), ctx.sampleRate));
      setCallState('speaking');
      return;
    }
    let v; try { v = JSON.parse(e.data); } catch { return; }
    switch (v.t) {
      case 'ready': CALL.t0 = Date.now(); setCallState('listening'); break;
      case 'say': CALL.gen = v.gen; CALL.sayEnd = -1; play.port.postMessage('clear'); addCaption('bia', v.text); break;
      case 'say_end': if (v.gen === CALL.gen) { CALL.sayEnd = v.gen; play.port.postMessage('check'); } break;
      case 'stop': CALL.gen = v.gen; CALL.sayEnd = -1; play.port.postMessage('clear'); setCallState('listening'); break;
      case 'caption': addCaption('you', v.text); break;
      case 'end': endCall(v.reason); break;
    }
  };
  ws.onclose = () => endCall();
  ws.onerror = () => endCall('connection failed');
  showCallUI(chat);
}

function endCall(reason) {
  if (!CALL.ws) return;
  const { ws, ctx, stream } = CALL;
  CALL.ws = null;
  try { if (ws.readyState === 1) ws.send(JSON.stringify({ t: 'hangup' })); ws.close(); } catch {}
  stream.getTracks().forEach(t => t.stop());
  ctx.close().catch(() => {});
  clearInterval(CALL.timer);
  if (CALL.el) CALL.el.remove();
  CALL.el = null;
  if (reason && reason !== 'they hung up') toast('Call ended: ' + reason);
}

function setCallState(s) {
  CALL.state = s;
  const el = CALL.el && CALL.el.querySelector('.call-state');
  if (el) el.textContent = { connecting: 'connecting…', listening: 'listening', speaking: 'bia is speaking' }[s] || s;
  if (CALL.el) CALL.el.classList.toggle('speaking', s === 'speaking');
}

function addCaption(who, text) {
  if (!CALL.el || !text) return;
  const box = CALL.el.querySelector('.call-caps');
  box.appendChild(h('div', { class: 'cap ' + (who === 'bia' ? 'bia' : 'me') }, h('b', {}, who === 'bia' ? 'bia' : 'You'), ' ', text));
  while (box.children.length > 8) box.firstChild.remove();
  box.scrollTop = box.scrollHeight;
}

function showCallUI(chat) {
  const c = S.chats.get(chat) || { title: chatTitle(chat) };
  const time = h('span', { class: 'call-time' }, '0:00');
  const muteBtn = h('button', { class: 'call-btn', title: 'Mute' }, ic('mic'));
  muteBtn.onclick = () => {
    CALL.muted = !CALL.muted;
    set(muteBtn, ic(CALL.muted ? 'micOff' : 'mic'));
    muteBtn.classList.toggle('on', CALL.muted);
    if (CALL.ws && CALL.ws.readyState === 1) CALL.ws.send(JSON.stringify({ t: 'mute', on: CALL.muted }));
  };
  const hang = h('button', { class: 'call-btn hang', title: 'Hang up', onclick: () => endCall() }, ic('phoneOff'));
  CALL.el = h('div', { class: 'callbox' },
    h('div', { class: 'call-head' },
      h('img', { class: 'call-ava', src: '/icon.svg' }),
      h('div', { class: 'grow' }, h('div', { class: 'call-title ellipsis' }, 'bia · ' + c.title), h('div', { class: 'call-sub' }, h('span', { class: 'call-state' }, 'connecting…'), ' · ', time))),
    h('div', { class: 'call-caps' }),
    h('div', { class: 'call-ctl' }, muteBtn, hang));
  document.body.appendChild(CALL.el);
  CALL.timer = setInterval(() => { if (CALL.t0) time.textContent = fmtDur((Date.now() - CALL.t0) / 1000); }, 500);
}

// bia calls you: incoming-call screen with a ringtone
const RING = { el: null, ctx: null, timer: null };
function ringTone() {
  try {
    const ctx = new AudioContext(); RING.ctx = ctx;
    const beep = () => {
      if (RING.ctx !== ctx) return;
      [440, 480].forEach(f => {
        const o = ctx.createOscillator(), g = ctx.createGain();
        o.frequency.value = f; g.gain.value = 0.06;
        o.connect(g).connect(ctx.destination); o.start(); o.stop(ctx.currentTime + 1.6);
      });
    };
    beep(); RING.timer = setInterval(beep, 4000);
  } catch {}
}
function stopRing() {
  clearInterval(RING.timer);
  if (RING.ctx) { RING.ctx.close().catch(() => {}); RING.ctx = null; }
  if (RING.el) { RING.el.remove(); RING.el = null; }
}
function showRing(v) {
  stopRing();
  if (CALL.ws) return;
  const c = S.chats.get(v.chat) || { title: chatTitle(v.chat) };
  RING.chat = v.chat;
  RING.el = h('div', { class: 'callbox ringing' },
    h('div', { class: 'call-head' },
      h('img', { class: 'call-ava', src: '/icon.svg' }),
      h('div', { class: 'grow' }, h('div', { class: 'call-title' }, 'bia is calling…'), h('div', { class: 'call-sub' }, c.title))),
    h('div', { class: 'call-ctl' },
      h('button', { class: 'call-btn hang', title: 'Decline', onclick: () => { stopRing(); act('call_decline', { chat: v.chat, id: v.id }).catch(() => {}); } }, ic('phoneOff')),
      h('button', { class: 'call-btn accept', title: 'Answer', onclick: () => { stopRing(); go('chat/' + v.chat); startCall(v.chat, v.id); } }, ic('phone'))));
  document.body.appendChild(RING.el);
  ringTone();
}
