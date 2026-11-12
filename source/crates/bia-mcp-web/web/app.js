// bia web app (the web MCP): Telegram-style chats with the agent; for root also
// a root shell, a file manager, human-in-the-loop tasks, the display and certificate admin.
'use strict';
// bia's picture, in the page itself: it shows even when a phone's WebView drops an image request
// or will not draw the SVG's filters. web/icon-192.png at 112x112.
const BIA_ICON = 'data:image/png;base64,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';

// ======================================================================
// utilities
// ======================================================================
const $ = (s, el = document) => el.querySelector(s);

// iOS does not resize the page for its keyboard: it scrolls the page and leaves the layout taller
// than what can be seen, so the sidebar shows under the chat. The app is pinned to what is visible.
(function keepToVisibleArea() {
  const vv = window.visualViewport;
  if (!vv) return;
  let stick = true;
  document.addEventListener('scroll', e => {
    if (e.target && e.target.id === 'msgs') { const m = e.target; stick = m.scrollTop + m.clientHeight >= m.scrollHeight - 80; }
  }, true);
  let shown = 0;
  const fit = () => {
    if (vv.scale > 1.01) return; // someone's own pinch zoom pans freely
    const height = Math.round(vv.height);
    if (height !== shown) {
      shown = height;
      document.documentElement.style.setProperty('--app-h', height + 'px');
      const m = document.getElementById('msgs');
      if (m && stick) requestAnimationFrame(() => { m.scrollTop = m.scrollHeight; });
    }
    if (window.scrollX || window.scrollY) window.scrollTo(0, 0);
  };
  vv.addEventListener('resize', fit);
  vv.addEventListener('scroll', fit);
  window.addEventListener('scroll', fit);
  fit();
})();
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
// bia's own icon (web_icon), when it chose one: the page comes with its rev in the icon link, and
// hello / a `self` frame bring changes. '' = the built-in picture above.
let ICON_REV = ((document.querySelector('link[rel="icon"]') || {}).href || '').split('?v=')[1] || '';
const biaIconSrc = () => ICON_REV ? '/icon-192.png?v=' + ICON_REV : BIA_ICON;
function biaImg(attrs) {
  const img = h('img', Object.assign({ src: biaIconSrc(), alt: '' }, attrs || {}));
  img.classList.add('bia-icon');
  img.onerror = () => { if (img.src !== BIA_ICON) img.src = BIA_ICON; };
  return img;
}
function setIcon(rev) {
  if (typeof rev !== 'string' || rev === ICON_REV) return; // an older bia does not say
  ICON_REV = rev;
  document.querySelectorAll('img.bia-icon').forEach(i => { i.src = biaIconSrc(); });
  const v = rev ? '?v=' + rev : '';
  const tab = document.querySelector('link[rel="icon"]');
  if (tab) { tab.type = rev ? 'image/png' : 'image/svg+xml'; tab.href = rev ? '/icon-192.png' + v : '/icon.svg'; }
  const apple = document.querySelector('link[rel="apple-touch-icon"]');
  if (apple) apple.href = '/apple-touch-icon.png' + v;
  // the manifest is read again when the browser installs the app
  const man = document.querySelector('link[rel="manifest"]');
  if (man) man.href = '/manifest.webmanifest' + v;
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
  // one chat with bia ('main'); its summary is worked out here from the loaded messages
  chats: new Map(),        // 'main' → {id, title, muted, unread, last, pinned}
  order: [],
  cache: new Map(),        // id → {list, byId, moreBefore, moreAfter, loading}
  drafts: new Map(),       // chat → {id, text}
  typing: new Set(),
  pending: new Map(),      // chat → [{id, text, files, progress, reply_to, kind}] (not in the chat yet)
  upProg: new Map(),       // fid → fraction of one of our files bia has pulled
  cur: null,
  route: '',
  link: null, wsOk: false, linkState: 'connecting', hellos: 0,
  ready: null, readyResolve: null, loadedAt: Date.now(),
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

  // "bia-5a3f2c agent CA" since 2.1.0; older bias all share "bia agent CA"
  const caName = info.ca_name || 'bia agent CA';
  // The browser presented a certificate this machine refused (another bia's, expired, revoked).
  const bad = info.cert_problem;
  const why = bad && ({
    other_ca: 'was issued by another bia, not by this machine',
    expired: 'has expired',
    revoked: 'was revoked on this machine',
    wrong_source: 'only works from another address than this browser’s',
  }[bad.kind] || 'is not valid here');
  const problemStep = bad ? h('div', { class: 'step problem' },
    h('h2', null, ic('shield'), 'Wrong certificate selected'),
    h('p', null, `The certificate “${bad.identity || 'unnamed'}” you picked ${why}.`),
    h('p', null, 'Your browser remembers that choice for this site until it is fully restarted. Quit it completely (Chrome on macOS: ⌘Q; elsewhere close every window) or open this page in a private / Incognito window, then pick a certificate issued by this bia — its CA fingerprint is in step 2.'),
    bad.kind === 'other_ca' ? h('p', null, `This bia's CA is “${caName}”. Bias set up before 2.1.0 all name theirs “bia agent CA”, so certificates of other machines are offered too; newer certificates also name the bia that issued them (“Identity: root · Fred”). Removing the ones you no longer use from your keychain or certificate store avoids picking them again.`) : null) : null;

  const certStep = h('div', { class: 'step' },
    h('h2', null, h('span', { class: 'num' }, '1'), 'Get your certificate'),
    info.root_issued
      ? h('p', null, 'Access needs a personal client certificate. Ask root (or bia on the machine: /pki issue <name>) for a one-time pickup link, valid 15 minutes.')
      : h('p', null, 'First-time setup: no root certificate exists yet. Issue it here — the file stays only in this tab until you download it. Its password is 12345678.'),
    !info.root_issued ? h('div', { class: 'actions' }, issueBtn, dlBtn) : null,
    h('div', { class: 'row', style: 'margin-top:10px' }, pickIn, pickBtn));

  const importStep = h('div', { class: 'step' },
    h('h2', null, h('span', { class: 'num' }, '2'), 'Import it, trust bia, reload'),
    h('p', null, `Import the .p12 file into your browser or OS (password 12345678). It also holds this machine’s certificate authority, “${caName}”: trust it, so your browser recognises this machine without warnings. Check first that its fingerprint matches the one shown on the machine (/web on its console). Then reload this page and pick your certificate when asked.`),
    fp ? h('div', { class: 'fp' }, 'SHA-256 ' + fp.join(':')) : null,
    howto('How to import and trust', [
      ['macOS', `Double-click the .p12 → login keychain → password 12345678. In Keychain Access double-click “${caName}” → Trust → “Always Trust”. Chrome and Safari use both.`],
      ['Windows', `Double-click the .p12 → Current User → password 12345678 → automatic store; confirm the security warning that installs “${caName}”.`],
      ['Linux (Chrome)', `chrome://settings/certificates → Your certificates → Import the .p12; then Authorities → “${caName}” → Edit → trust it for websites.`],
      ['Firefox', `Settings → Privacy & Security → Certificates → View Certificates → Your Certificates → Import; then Authorities → “${caName}” → Edit Trust → identify websites.`],
      ['iPhone / iPad', `Open the .p12 in Safari → Settings → Profile Downloaded → Install; then Settings → General → About → Certificate Trust Settings → enable “${caName}”.`],
      ['Android', `Settings → Security → Encryption & credentials → Install a certificate → VPN & app user certificate → the .p12 (it installs “${caName}” too).`],
    ]),
    h('div', { class: 'actions', style: 'margin-top:10px' }, h('button', { class: 'btn', onclick: () => location.reload() }, ic('refresh'), 'Reload')));

  set(box, h('div', { class: 'wrap' },
    biaImg({ class: 'logo' }),
    h('h1', null, 'bia'),
    h('div', { class: 'sub' }, 'This machine talks to you over HTTPS with client certificates. Two steps to get in.'),
    problemStep, certStep, importStep, err,
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
  if (S.root) { pollTasks(); setInterval(pollTasks, 4000); pollStatus(); setInterval(pollStatus, 5000); pollQr(); setInterval(pollQr, 3000); }
  if (who.days_left !== undefined && who.days_left < 30) toast(`Your certificate expires in ${who.days_left} days — renew it in Settings`, true);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) maybeRead(); updateTitle(); sendActive(); });
  window.addEventListener('focus', () => { maybeRead(); sendActive(); });
  // focus moving into an iframe (display, task pages) blurs the window but keeps the tab in front
  window.addEventListener('blur', () => setTimeout(sendActive, 0));
  notifInit();
}

// ======================================================================
// the web bus (web/link.js): one WebSocket for the chat, its files and acks
// ======================================================================
// The latest messages of the chat, loaded once after connecting (the list's summary and
// unread count come from them, also before the chat is opened).
function loadMain() {
  const have = S.cache.get('main');
  if (have) return have.ready;
  const c = { list: [], byId: new Map(), moreBefore: false, moreAfter: false, loading: true };
  S.cache.set('main', c);
  c.ready = (async () => {
    try {
      const j = await hist({ limit: 60 });
      // what arrived meanwhile (live frames) is newer than the page
      const live = c.list;
      c.list = j.messages; c.byId = new Map(j.messages.map(m => [m.id, m]));
      for (const m of live) if (!c.byId.has(m.id)) { c.list.push(m); c.byId.set(m.id, m); }
      c.list.sort((a, b) => a.ts - b.ts);
      c.moreBefore = j.more_before; c.moreAfter = false;
    } catch (e) {
      toast('Could not load messages: ' + e.message, true);
      if (S.cache.get('main') === c) S.cache.delete('main');
      c.loading = false;
      return null;
    }
    c.loading = false;
    summarize();
    return c;
  })();
  return c.ready;
}

function connect() {
  S.ready = new Promise(r => { S.readyResolve = r; });
  S.chats.set('main', { id: 'main', title: 'bia', muted: false, unread: 0, last: null, pinned: [] });
  S.order = ['main'];
  S.link = BiaLink.create({
    identity: S.me.identity,
    onHello, onFrame: onEvent, onReset, onUpload, onOutbox,
    onState: st => { S.linkState = st; S.wsOk = st === 'online'; setConn(); },
    onMoved: (url, port) => {
      S.movedTo = url;
      // inside a bia deploy app: it keeps the new address and opens it by itself
      try {
        if (window.BiaApp && BiaApp.moved) { BiaApp.moved(port); toast('bia moved to port ' + port + '; opening it there…'); return; }
        const ios = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.biaMoved;
        if (ios) { ios.postMessage(port); toast('bia moved to port ' + port + '; opening it there…'); return; }
      } catch {}
      // on every page of the app (console and shell too): nothing here works any more
      modal({
        title: 'bia moved its web app',
        body: h('div', {}, h('p', {}, 'It now answers on another port, so this address stopped working:'),
          h('p', {}, h('a', { href: url }, url.replace(/^(https:\/\/[^/]+).*/, '$1'))),
          h('p', { class: 'muted' }, 'It is ready there in a few seconds. The browser asks for your certificate again; a saved bookmark or home-screen app needs the new address.')),
        buttons: [{ text: 'Later', value: false }, { text: 'Open it', primary: true, cls: 'primary', onClick: () => { location.href = url; } }],
      });
    },
  });
  S.link.start();
}

// Actions go over the link; changes to the chat wait in its outbox while it is down.
function act(t, payload = {}) {
  const p = Object.assign({}, payload);
  delete p.chat;
  return S.link.act(t, p);
}
// History and search need the link (they wait for the first hello).
async function hist(params) {
  await S.ready;
  const r = await act('hist', params);
  if (r.rev > (S.link.rx || 0)) S.link.rx = r.rev;
  return r;
}

function onHello(v) {
  S.wsOk = true;
  setCalled(v.called);
  setIcon(v.icon);
  const main = S.chats.get('main');
  main.muted = !!v.muted;
  if (v.typing) S.typing.add('main'); else S.typing.delete('main');
  setConn();
  S.readyResolve();
  // the first hello loads the chat; a reconnect without a known revision reloads it
  if (!S.hellos++) loadMain();
  else if (S.link.rx === null) { S.cache.delete('main'); loadMain(); if (S.cur) openChat('main', { keepScroll: false }); }
  if (S.cur && S.route.startsWith('chat')) { renderChatHead(); renderPinbar(); }
  renderChatList();
  updateTitle();
  sendActive();
}

function onReset() {
  S.cache.delete('main');
  loadMain();
  if (S.cur) openChat('main', { keepScroll: false });
}

function onEvent(v) {
  switch (v.t) {
    case 'm': upsertMsg(v.m); return;
    case 'gone': removeMsgs(v.ids); return;
    case 'self': setCalled(v.called); setIcon(v.icon); renderInstallAsk(); return;
    case 'ring': showRing(Object.assign({}, v, { chat: 'main' })); return;
    case 'ring_end': if (RING.chat) stopRing(); return;
    case 'call': return;
    case 'typing':
      if (v.on) S.typing.add('main'); else S.typing.delete('main');
      renderChatList();
      if (S.cur) { renderChatHead(); renderTail(); updateSendBtn(); }
      return;
    case 'draft': {
      const d = S.drafts.get('main');
      if (d && d.id === v.id) d.text += v.d;
      else S.drafts.set('main', { id: v.id, text: v.d });
      if (S.cur) scheduleDraft();
      return;
    }
    case 'draft_end':
      S.drafts.delete('main');
      if (S.cur) renderTail();
      return;
  }
}

// A message's new state: a new one, an edit, a reaction, a read mark, a file arriving.
function upsertMsg(m) {
  const d = S.drafts.get('main');
  if (d && d.id === m.id) S.drafts.delete('main');
  const pend = S.pending.get('main') || [];
  const pi = pend.findIndex(p => p.id === m.id);
  if (pi >= 0) pend.splice(pi, 1);
  const c = S.cache.get('main');
  if (c && c.loading) {
    // merged into the history when it arrives
    if (!c.byId.has(m.id)) c.list.push(m);
    c.byId.set(m.id, m);
  } else if (c) {
    const old = c.byId.get(m.id);
    if (old) {
      if ((m.rev || 0) >= (old.rev || 0)) {
        c.list[c.list.indexOf(old)] = m; c.byId.set(m.id, m);
        if (S.cur) rerenderRow(m.id);
      }
    } else if (!c.moreAfter) {
      const first = c.list[0];
      if (c.moreBefore && first && m.ts < first.ts) { /* older than what is loaded */ }
      else {
        c.byId.set(m.id, m);
        const last = c.list[c.list.length - 1];
        if (!last || last.ts <= m.ts) { c.list.push(m); if (S.cur) appendRow(m); }
        else { c.list.push(m); c.list.sort((a, b) => a.ts - b.ts); if (S.cur) renderMessages({ keep: true }); }
        if (!isMine(m) && !m.service && !m.read && m.ts > S.loadedAt - 60e3) notifyMsg('main', m);
      }
    } else if (S.cur && !isMine(m)) { S.jumpUnread = (S.jumpUnread || 0) + 1; renderJump(); }
  }
  summarize();
  if (S.cur) { renderTail(); renderPinbar(); }
}

function removeMsgs(ids) {
  const c = S.cache.get('main');
  if (c) {
    c.list = c.list.filter(m => !ids.includes(m.id));
    ids.forEach(id => c.byId.delete(id));
    if (S.cur) { ids.forEach(id => S.sel.delete(id)); renderMessages({ keep: true }); }
  }
  summarize();
}

// The chat's summary for the list and the title, from the loaded messages.
function summarize() {
  const c = S.cache.get('main'), sum = S.chats.get('main');
  if (!c || !sum) return;
  sum.unread = c.list.filter(m => m.from === 'bia' && !m.read && !m.service).length;
  sum.last = c.list[c.list.length - 1] || null;
  sum.pinned = c.list.filter(m => m.pinned).map(m => m.id);
  renderChatList();
  updateTitle();
}

// bia pulled (part of) a file of ours: show how far it got.
function onUpload(fid, done, total) {
  if (done === -1) S.upProg.delete(fid);
  else if (done === -2) { S.upProg.delete(fid); toast('A file was not sent: ' + total, true); }
  else S.upProg.set(fid, total ? done / total : 0);
  if (onUpload.raf) return;
  onUpload.raf = requestAnimationFrame(() => {
    onUpload.raf = 0;
    const c = S.cache.get('main');
    if (!c || !S.cur) return;
    for (const m of c.list) if ((m.files || []).some(f => f.pending)) rerenderRow(m.id);
  });
}

// Messages written while offline (also before a reload) wait in the outbox: show them.
function onOutbox(list) {
  const pend = S.pending.get('main') || [];
  S.pending.set('main', pend);
  const c = S.cache.get('main');
  for (const a of list) {
    if (a.t !== 'send' || !a.id || pend.some(p => p.id === a.id) || (c && c.byId.has(a.id))) continue;
    pend.push({ id: a.id, chat: 'main', text: a.text || '', files: (a.files || []).map(f => ({ name: f.name, size: f.size })), progress: {}, reply_to: a.reply_to, ts: Date.now() });
  }
  if (S.cur) renderTail();
}

function setConn() {
  const el = $('#connState');
  if (el) {
    el.hidden = S.wsOk;
    const st = S.linkState;
    if (st === 'blocked') set(el, 'Another session of yours is still open. Waiting about 30 s, then this page connects by itself.');
    else if (st === 'replaced') set(el, 'This chat is now open in another tab or window. ', h('a', { href: '#', onclick: e => { e.preventDefault(); S.link.retry(); } }, 'Use it here'));
    else if (st === 'moved') set(el, 'bia moved its web app to another address. Open ', h('a', { href: S.movedTo }, S.movedTo.replace(/^(https:\/\/[^/]+).*/, '$1')), ' (it answers there in a few seconds; your browser asks for your certificate again).');
    else if (st === 'banned') set(el, 'This identity is banned on this bia.');
    else set(el, 'Connecting…');
  }
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
  if (isActive()) { blip(); return; }
  // subscribed: bia pushes it (it saw no tab of ours in front), with the system's sound
  if (NOTIF.sub) return;
  blip();
  showNote(chatTitle(chat), snippet(m), 'bia-' + chat, 'chat/' + chat);
}

// Browser notifications. A service worker (/sw.js) shows them; with Web Push bia wakes it
// even when no tab is open. bia only pushes while no tab of this identity is in front, so
// each tab reports {t:'active'} over its WebSocket.
const NOTIF = { reg: null, sub: null, id: '', err: '' };
const canNotify = () => 'Notification' in window;
const isIOS = () => /iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
const isStandalone = () => navigator.standalone === true || matchMedia('(display-mode: standalone)').matches;
function isActive() { return document.visibilityState === 'visible' && document.hasFocus(); }
function sendActive() { if (S.link) S.link.sendJson({ t: 'active', on: isActive() }); }
async function notifInit() {
  if ('serviceWorker' in navigator) {
    navigator.serviceWorker.addEventListener('message', e => { if (e.data && e.data.t === 'open') go(e.data.url || ''); });
    try { NOTIF.reg = await navigator.serviceWorker.register('/sw.js'); }
    catch (e) { NOTIF.err = 'service worker: ' + e.message; }
  }
  renderNotifAsk(); renderInstallAsk();
  if (S.prefs.notify && canNotify() && Notification.permission === 'granted') await pushOn();
}
function b64uBytes(s) {
  const b = atob(s.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((s.length + 3) % 4));
  return Uint8Array.from(b, c => c.charCodeAt(0));
}
function sameBytes(a, b) {
  if (!a || !b) return false;
  a = new Uint8Array(a); b = new Uint8Array(b);
  return a.length === b.length && a.every((x, i) => x === b[i]);
}
// subscribe this browser with its push service and hand the subscription to bia
async function pushOn() {
  NOTIF.err = '';
  if (!NOTIF.reg || !('PushManager' in window)) { NOTIF.err = NOTIF.err || 'this browser has no Web Push'; return false; }
  try {
    const reg = await navigator.serviceWorker.ready;
    const key = b64uBytes((await GET('/v1/push/key')).key);
    let sub = await reg.pushManager.getSubscription();
    if (sub && !sameBytes(sub.options.applicationServerKey, key)) { await sub.unsubscribe(); sub = null; }
    if (!sub) sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key });
    NOTIF.id = (await POST('/v1/push/subscribe', sub.toJSON())).id || '';
    NOTIF.sub = sub;
    return true;
  } catch (e) {
    NOTIF.sub = null; NOTIF.err = e.message || String(e);
    return false;
  }
}
async function pushOff() {
  let sub = NOTIF.sub;
  if (!sub && NOTIF.reg) { try { sub = await NOTIF.reg.pushManager.getSubscription(); } catch {} }
  NOTIF.sub = null; NOTIF.id = '';
  if (!sub) return;
  try { await POST('/v1/push/unsubscribe', { endpoint: sub.endpoint }); } catch {}
  try { await sub.unsubscribe(); } catch {}
}
// tell bia what the person decided about notifications, so it can acknowledge it (a system message)
function notifDecided(state) {
  POST('/v1/push/permission', { state, push: !!NOTIF.sub, error: NOTIF.err || '' }).catch(() => {});
}
// from a click: browsers only ask for permission on a user gesture
async function enableNotifications() {
  S.prefs.notify = true; S.prefs.notifAsked = true; savePrefs();
  const before = canNotify() ? Notification.permission : '';
  if (canNotify() && Notification.permission === 'default') await Notification.requestPermission();
  if (canNotify() && Notification.permission === 'granted') {
    if (await pushOn()) toast('Notifications are on'); else toast('Notifications are on while this tab is open (' + NOTIF.err + ')', true);
    notifDecided('granted');
  } else if (canNotify() && Notification.permission === 'denied' && before !== 'denied') {
    notifDecided('denied'); // the browser's prompt was rejected
  }
  renderNotifAsk(); renderSettings();
}
async function showNote(title, body, tag, url, urgent) {
  if (!S.prefs.notify || !canNotify() || Notification.permission !== 'granted') return;
  const opts = { body, tag, renotify: true, requireInteraction: !!urgent, icon: '/icon-192.png' + (ICON_REV ? '?v=' + ICON_REV : ''), badge: '/icon-192.png', data: { url } };
  if (NOTIF.reg) { try { await NOTIF.reg.showNotification(title, opts); return; } catch {} }
  try { const n = new Notification(title, opts); n.onclick = () => { window.focus(); go(url); n.close(); }; } catch {}
}
function ago(ms) {
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}
// Settings: every browser of this identity that bia pushes to, with how delivery went
async function loadPushSubs() {
  const el = $('#pushSubs');
  if (!el) return;
  let subs;
  try { subs = (await GET('/v1/push/subs')).subs || []; } catch (e) { set(el, h('div', { class: 'err-line' }, e.message)); return; }
  if (!subs.length) { set(el, h('div', { class: 'muted', style: 'font-size:13px' }, 'No browser gets push notifications yet.')); return; }
  const remove = async (ids, self) => {
    try { await POST('/v1/push/remove', { ids }); } catch (e) { toast(e.message, true); return; }
    // our own entry gone: stop this browser's subscription too
    if (self) { S.prefs.notify = false; savePrefs(); await pushOff(); renderSettings(); notifDecided('stopped'); }
    else { loadPushSubs(); notifDecided('removed'); }
  };
  const bad = subs.filter(x => x.expired || x.failing);
  set(el, ...subs.map(x => {
    const self = x.id === NOTIF.id;
    const state = x.expired ? h('span', { class: 'tag bad' }, 'expired')
      : x.failing ? h('span', { class: 'tag bad', title: x.last_err }, 'failing')
      : x.last_ok ? h('span', { class: 'tag ok' }, 'delivered ' + ago(x.last_ok))
      : h('span', { class: 'tag' }, 'nothing sent yet');
    const detail = [x.service, 'added ' + new Date(x.created).toLocaleDateString()];
    if (x.expires && !x.expired) detail.push('expires ' + new Date(x.expires).toLocaleDateString());
    if (x.failing && x.last_err) detail.push(`${ago(x.last_err_at)}: ${x.last_err}`);
    return h('div', { class: 'setrow' },
      h('div', { class: 'grow' }, x.device, ' ', self ? h('span', { class: 'tag acc' }, 'this browser') : null, ' ', state,
        h('div', { class: 'muted', style: 'font-size:13px;word-break:break-word' }, detail.join(' · '))),
      iconBtn('trash', self ? 'Stop notifications in this browser' : 'Remove this browser', () => remove([x.id], self)));
  }), bad.length > 1 ? h('div', { class: 'setrow' }, h('div', { class: 'grow' }),
    h('button', { class: 'btn ghost small', onclick: () => remove(bad.map(x => x.id), bad.some(x => x.id === NOTIF.id)) }, `Remove ${bad.length} expired or failing`)) : null);
}
function renderNotifAsk() {
  const el = $('#notifAsk');
  if (!el) return;
  const ask = canNotify() && Notification.permission === 'default' && S.prefs.notify && !S.prefs.notifAsked;
  el.hidden = !ask;
  if (!ask) { set(el); return; }
  set(el, ic('bell'), h('div', { class: 'grow' }, 'Get notified when bia writes'),
    h('button', { class: 'btn small', onclick: enableNotifications }, 'Turn on'),
    iconBtn('close', 'Not now', () => { S.prefs.notifAsked = true; savePrefs(); renderNotifAsk(); notifDecided('dismissed'); }));
}
// ---- installing the web app on a phone's home screen ----
// Only there does a phone deliver push notifications properly: iOS gives a web page none at all
// until it is on the Home Screen, Android stops a browser tab's ones when the browser sleeps.
// Not inside the bia deploy apps (their WebView is already an app) and not once installed.
const isAndroid = () => /Android/i.test(navigator.userAgent);
const inBiaApp = () => !!(window.BiaApp || (window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.biaMoved));
const calledName = () => BASE_TITLE.replace(/ · bia$/, '') || 'bia';
// Chrome and its kin on Android (and desktop) hand over their own install dialog. The event is
// left alone otherwise, so the browser may also show its own banner.
const INSTALL = { ev: null, done: false };
window.addEventListener('beforeinstallprompt', e => { INSTALL.ev = e; renderInstallAsk(); if (S.route === 'settings') renderSettings(); });
window.addEventListener('appinstalled', () => { INSTALL.ev = null; INSTALL.done = true; renderInstallAsk(); if (S.route === 'settings') renderSettings(); toast(calledName() + ' is installed'); });
// 'ios' | 'android' on a phone's browser where installing makes sense, 'prompt' elsewhere when the
// browser offers its dialog (a desktop Chrome), '' when there is nothing to offer.
function installKind() {
  if (inBiaApp() || isStandalone() || INSTALL.done) return '';
  if (isIOS()) return 'ios';
  if (isAndroid()) return 'android';
  return INSTALL.ev ? 'prompt' : '';
}
// the browser's own install dialog; false when it has none to show
async function installNow() {
  const ev = INSTALL.ev;
  if (!ev) return false;
  try {
    ev.prompt();
    const r = await ev.userChoice;
    INSTALL.ev = null; // one prompt per event
    if (r && r.outcome === 'accepted') INSTALL.done = true;
  } catch { return false; }
  renderInstallAsk(); if (S.route === 'settings') renderSettings();
  return true;
}
function installSteps(kind) {
  const name = calledName();
  const ua = navigator.userAgent;
  if (kind === 'ios') {
    const safari = !/CriOS|FxiOS|EdgiOS|OPiOS/.test(ua);
    return [
      safari ? 'Tap the Share button (the square with an arrow, in the bar at the bottom; on newer iOS it is behind the ••• button).'
        : 'Tap the Share button of this browser (the square with an arrow; in Chrome it is at the top right).',
      'Scroll down and tap “Add to Home Screen”, then “Add”.',
      `Close this page and open ${name} from the Home Screen.`,
      'There, open Settings and turn on “Browser notifications”. iPhone and iPad deliver them only to the app on the Home Screen, never to a browser tab.',
    ];
  }
  if (kind === 'android') {
    const menu = /SamsungBrowser/.test(ua) ? 'Tap the ☰ menu at the bottom, then “Add to” → “Home screen”.'
      : /Firefox/.test(ua) ? 'Tap the ⋮ menu, then “Add app to Home screen”.'
      : 'Tap the ⋮ menu at the top right, then “Add to Home screen” → “Install”.';
    return [
      INSTALL.ev ? `Tap “Install” below and confirm. (The same is in the browser’s menu: “Add to Home screen” → “Install”.)` : menu,
      `Open ${name} from the home screen or the app list.`,
      'There, open Settings and turn on “Browser notifications”: an installed app keeps getting them when the browser is closed.',
    ];
  }
  return ['Click “Install” below and confirm in the browser’s own dialog.', `${name} then opens in its own window, with its own icon.`];
}
function installHelp() {
  const kind = installKind();
  if (!kind) return;
  const body = h('div', { class: 'install-steps' },
    h('p', { class: 'muted', style: 'margin-top:0' }, kind === 'prompt' ? `${calledName()} as an app of its own on this computer.`
      : `On the home screen ${calledName()} opens like an app, full screen, and its notifications reach this phone properly.`),
    h('ol', null, installSteps(kind).map(s => h('li', null, s))),
    kind === 'android' && !INSTALL.ev ? h('p', { class: 'muted' }, 'If the browser only offers a shortcut, or nothing, this phone does not trust this bia’s CA yet: download it (Settings → Agent CA here), install it in Android’s Settings → Security → Encryption & credentials → Install a certificate → CA certificate, then reload this page.') : null);
  modal({ title: `Install ${calledName()}`, body, buttons: [
    INSTALL.ev ? { text: 'Install', primary: true, onClick: () => { installNow(); } } : null,
    { text: INSTALL.ev ? 'Later' : 'Close', value: 0 }].filter(Boolean) });
}
// the banner over the chat list, on phones only: one tap to the browser's dialog or to the steps
function renderInstallAsk() {
  const el = $('#installAsk');
  if (!el) return;
  const kind = installKind();
  const ask = (kind === 'ios' || kind === 'android') && !S.prefs.installAsked;
  el.hidden = !ask;
  if (!ask) { set(el); return; }
  set(el, biaImg({ class: 'install-ico' }),
    h('div', { class: 'grow' }, `Install ${calledName()} on this phone`, h('div', { class: 'muted', style: 'font-size:12px' }, 'for notifications that arrive')),
    h('button', { class: 'btn small', id: 'installBtn', onclick: async () => { if (!(await installNow())) installHelp(); } }, INSTALL.ev ? 'Install' : 'How'),
    iconBtn('close', 'Not now', () => { S.prefs.installAsked = true; savePrefs(); renderInstallAsk(); }));
}
// the server names the page after this bia ("Fred · bia")
let BASE_TITLE = document.title || 'bia';
// bia's name as it is now (hello, and a `self` frame when root renames it): the same title the
// server gives a page it serves
function setCalled(name) {
  if (typeof name !== 'string') return; // an older bia does not say
  BASE_TITLE = BASE_TITLE.replace(/^(?:.* · )?bia/, (name.trim() ? name.trim() + ' · ' : '') + 'bia');
  updateTitle();
}
function updateTitle() {
  let n = 0;
  for (const c of S.chats.values()) if (!c.muted) n += c.unread || 0;
  const t = S.tasks.length;
  document.title = (n || t ? `(${n + t}) ` : '') + BASE_TITLE;
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
    case 'chat': openChat('main'); break;
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
    h('div', { id: 'notifAsk', class: 'notif-ask', hidden: true }),
    h('div', { id: 'installAsk', class: 'notif-ask', hidden: true }),
    h('div', { class: 'side-scroll', id: 'sideScroll' },
      h('div', { id: 'searchRes', class: 'search-res', hidden: true }),
      h('div', { id: 'chatList' }),
      S.root ? h('div', { id: 'toolList' }) : null));
}

function sideMenu(e) {
  const r = e.currentTarget.getBoundingClientRect();
  const light = document.documentElement.classList.contains('light');
  menu(r.left, r.bottom + 4, [
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
      h('div', { class: 'avatar ' + (id === 'main' ? '' : colorOf(id)) }, id === 'main' ? biaImg({ style: 'width:100%;height:100%;border-radius:50%;object-fit:cover' }) : initials(c.title)),
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
    { icon: c.muted ? 'bell' : 'bellOff', text: c.muted ? 'Unmute' : 'Mute', onClick: () => setMute(!c.muted) },
    c.unread ? { icon: 'checks', text: 'Mark as read', onClick: () => markAllRead() } : null,
  ]);
}
function setMute(on) {
  const c = S.chats.get('main');
  c.muted = on; renderChatList(); renderChatHead();
  act('mute', { on }).catch(e => toast(e.message, true));
}
function markAllRead() {
  const c = S.cache.get('main');
  const last = c && [...c.list].reverse().find(m => m.from === 'bia');
  if (last) act('read', { upto: last.id }).catch(() => {});
}

async function globalSearch(q) {
  const res = $('#searchRes'), list = $('#chatList'), tools = $('#toolList');
  q = q.trim();
  if (!q) { res.hidden = true; list.hidden = false; if (tools) tools.hidden = false; return; }
  res.hidden = false; list.hidden = true; if (tools) tools.hidden = true;
  set(res, h('div', { class: 'side-label' }, 'Searching…'));
  let j = { results: [] };
  try { await S.ready; j = await act('search', { q }); } catch {}
  set(res,
    h('div', { class: 'side-label' }, j.results.length ? `Messages (${j.results.length})` : 'No messages found'),
    j.results.map(r => searchRow(r, q, () => jumpTo('main', r.m.id))));
}
function searchRow(r, q, onclick) {
  const text = snippet(r.m);
  const i = text.toLowerCase().indexOf(q.toLowerCase());
  const body = i < 0 ? text : [text.slice(Math.max(0, i - 30), i), h('mark', null, text.slice(i, i + q.length)), text.slice(i + q.length)];
  return h('div', { class: 'sr', onclick },
    h('div', { class: 'st' }, h('span', null, isMine(r.m) ? 'You' : 'bia'), h('span', null, fmtListTime(r.m.ts))),
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
  id = 'main';
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
  if (!S.cache.get(id) || S.cache.get(id).loading) {
    renderMessages();
    const c = await loadMain();
    if (S.cur !== id || !c) return;
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
      iconBtn('copy', 'Copy', copySelected),
      iconBtn('trash', 'Delete', () => deleteMsgs([...S.sel])));
    return;
  }
  const c = S.chats.get(S.cur) || { title: chatTitle(S.cur) };
  let sub, on = false;
  if (!S.wsOk) sub = S.linkState === 'blocked' ? 'waiting for your other session…' : S.linkState === 'replaced' ? 'open in another tab' : S.linkState === 'moved' ? 'moved to another address' : 'connecting…';
  else if (S.typing.has(S.cur)) { sub = 'typing…'; on = true; }
  else if (S.root && S.status && /^(thinking|tool|talking|reflecting)/.test(S.status.state || '')) { sub = S.status.state.split(' ')[0] + '…'; on = true; }
  else sub = S.root && S.status && S.status.model ? `online · ${S.status.model}` : 'online';
  set(head, 
    backBtn(),
    h('div', { class: 'avatar xs ' + (S.cur === 'main' ? '' : colorOf(S.cur)) }, S.cur === 'main' ? biaImg({ style: 'width:100%;height:100%;border-radius:50%;object-fit:cover' }) : initials(c.title)),
    h('div', { class: 'tbox' }, h('div', { class: 'title ellipsis' }, c.title), h('div', { class: 'subtitle' + (on ? ' on' : '') }, sub)),
    iconBtn('phone', 'Call bia', () => startCall(S.cur)),
    iconBtn('search', 'Search in chat', () => { S.chatSearch = S.chatSearch ? null : { q: '' }; renderChatSearch(); }),
    iconBtn('more', 'More', e => {
      const r = e.currentTarget.getBoundingClientRect();
      menu(r.right - 220, r.bottom + 4, [
        { icon: 'select', text: 'Select messages', onClick: () => setSelecting(true) },
        { icon: c.muted ? 'bell' : 'bellOff', text: c.muted ? 'Unmute' : 'Mute', onClick: () => setMute(!c.muted) },
        { icon: 'download', text: 'Export as text', onClick: () => exportChat(S.cur) },
      ]);
    }));
}

async function exportChat(id) {
  const all = [];
  let before = '';
  for (let i = 0; i < 100; i++) {
    const j = await hist(before ? { limit: 200, before } : { limit: 200 });
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
  iconBtn('unpin', 'Unpin', e => { e.stopPropagation(); act('pin', { id, on: false }); })));
  if (!m) {
    hist({ around: id, limit: 1 }).then(j => {
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
    const j = await act('search', { q }).catch(() => ({ results: [] }));
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
  chat = 'main';
  if (S.cur !== chat || !S.route.startsWith('chat')) { go('chat/' + chat); await new Promise(r => setTimeout(r, 50)); while (S.cache.get(chat) && S.cache.get(chat).loading) await new Promise(r => setTimeout(r, 50)); }
  if (scrollToId(id, true)) return;
  const c = S.cache.get(chat);
  try {
    const j = await hist({ around: id, limit: 80 });
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
    const j = await hist({ limit: 60 });
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
      const j = await hist({ before: c.list[0].id, limit: 60 });
      const fresh = j.messages.filter(m => !c.byId.has(m.id));
      c.list = fresh.concat(c.list); fresh.forEach(m => c.byId.set(m.id, m));
      c.moreBefore = j.more_before;
      c.paging = false;
      renderMessages({ keep: true });
    } catch { c.paging = false; }
  } else if (c.moreAfter && s.scrollTop + s.clientHeight > s.scrollHeight - 300) {
    c.paging = true;
    try {
      const j = await hist({ after: c.list[c.list.length - 1].id, limit: 60 });
      const fresh = j.messages.filter(m => !c.byId.has(m.id));
      c.list = c.list.concat(fresh); fresh.forEach(m => c.byId.set(m.id, m));
      c.moreAfter = j.more_after;
      c.paging = false;
      renderMessages({ keep: true });
    } catch { c.paging = false; }
  }
}

const maybeRead = debounce(() => {
  if (!S.cur || document.hidden || !S.route.startsWith('chat') || !nearBottom()) return;
  const sum = S.chats.get(S.cur);
  const c = S.cache.get(S.cur);
  if (!sum || !sum.unread || !c || c.moreAfter) return;
  const last = [...c.list].reverse().find(m => m.from === 'bia');
  sum.unread = 0;
  renderChatList(); updateTitle();
  if (last) act('read', { upto: last.id }).catch(() => {});
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

// A file of the chat as a blob URL: pulled over the link once (ours are here already).
function attSrc(f, onProg) { return S.link.url(f, onProg); }
function downloadAtt(f) { attSrc(f).then(u => download(u, f.name)).catch(e => toast(e.message, true)); }
const viewKind = f => f.mime.startsWith('text/') || f.mime.includes('json') ? 'text' : f.mime === 'application/pdf' ? 'pdf' : f.kind;

// A file still on its way (ours being pulled by bia, or bia's not arrived) or lost.
function fileState(f, mine) {
  const pct = mine ? (S.upProg.get(f.fid) || 0) : 0;
  const R = 21, C = 2 * Math.PI * R;
  const icon = h('div', { class: 'dicon', html: f.err ? '' : `<svg class="ring" viewBox="0 0 48 48"><circle cx="24" cy="24" r="${R}" stroke-dasharray="${C}" stroke-dashoffset="${C * (1 - pct)}"/></svg>` });
  icon.prepend(ic(f.err ? 'close' : mine ? 'upload' : 'download'));
  return h('div', { class: 'doc' }, icon,
    h('div', { class: 'grow' }, h('div', { class: 'dname' }, f.name),
      h('div', { class: 'dsize' }, f.err ? 'not received: ' + f.err : mine ? `${Math.round(pct * 100)}% of ${fmtBytes(f.size)}` : 'on its way')));
}

function buildMedia(m, f, i, all) {
  const box = h('div', { class: 'media', onclick: (e) => { e.stopPropagation(); if (!S.selecting && !f.pending) openViewer(all.map(x => ({ get: () => attSrc(x.f), name: x.f.name, kind: x.f.kind })), all.findIndex(x => x.i === i)); } });
  const mine = isMine(m);
  if (f.err || (f.pending && !mine) || !f.fid) { box.appendChild(fileState(f, mine)); return box; }
  if (f.kind === 'photo') {
    const img = h('img', { alt: f.name, decoding: 'async' });
    img.addEventListener('load', () => { if (nearBottomish()) scrollBottom(); }, { once: true });
    attSrc(f).then(u => { img.src = u; }).catch(() => {});
    box.appendChild(img);
  } else if (f.size < (24 << 20)) {
    const v = h('video', { preload: 'metadata', muted: true, playsinline: true });
    const dur = h('span', { class: 'dur' }, 'video');
    v.addEventListener('loadedmetadata', () => { dur.textContent = fmtDur(v.duration); });
    attSrc(f).then(u => { v.src = u + '#t=0.1'; }).catch(() => {});
    box.append(v, h('div', { class: 'play' }, ic('play')), dur);
  } else {
    // a big video is pulled only when opened
    box.append(h('div', { class: 'vtile' }), h('div', { class: 'play' }, ic('play')), h('span', { class: 'dur' }, fmtBytes(f.size)));
  }
  if (f.pending && mine) box.appendChild(h('span', { class: 'dur', style: 'top:auto;bottom:6px' }, `${Math.round((S.upProg.get(f.fid) || 0) * 100)}%`));
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
  if (f.pending || f.err || !f.fid) return fileState(f, isMine(m));
  if (f.kind === 'audio') return buildAudio(m, f, i);
  return h('div', { class: 'doc', onclick: (e) => { e.stopPropagation(); if (!S.selecting) openFileAtt(m, f, i); } },
    h('div', { class: 'dicon' }, ic(fileIcon(f.name, f.kind))),
    h('div', { class: 'grow' }, h('div', { class: 'dname' }, f.name), h('div', { class: 'dsize' }, fmtBytes(f.size) + ' · ' + (f.name.split('.').pop() || '').toUpperCase())));
}
function openFileAtt(m, f, i) {
  const viewable = /^(application\/pdf|text\/|image\/|video\/|audio\/|application\/json)/.test(f.mime);
  if (viewable) openViewer([{ get: () => attSrc(f), name: f.name, kind: viewKind(f) }], 0);
  else downloadAtt(f);
}

function buildAudio(m, f, i) {
  const audio = new Audio();
  audio.preload = 'none';
  const btn = h('button', { class: 'dicon', style: 'border:0;cursor:pointer' }, ic('play'));
  const sub = h('div', { class: 'dsize' }, fmtBytes(f.size));
  btn.onclick = (e) => {
    e.stopPropagation();
    if (!audio.src) { attSrc(f).then(u => { audio.src = u; stopOtherAudio(audio); audio.play(); }).catch(er => toast(er.message, true)); return; }
    if (audio.paused) { stopOtherAudio(audio); audio.play(); } else audio.pause();
  };
  audio.onplay = () => set(btn, ic('pause'));
  audio.onpause = audio.onended = () => set(btn, ic('play'));
  audio.ontimeupdate = () => { sub.textContent = `${fmtDur(audio.currentTime)} / ${fmtDur(audio.duration)}`; };
  return h('div', { class: 'doc' }, btn,
    h('div', { class: 'grow' }, h('div', { class: 'dname' }, f.name), sub),
    iconBtn('download', 'Download', (e) => { e.stopPropagation(); downloadAtt(f); }));
}
let playingAudio = null;
function stopOtherAudio(a) { if (playingAudio && playingAudio !== a) playingAudio.pause(); playingAudio = a; }

function buildVoice(m, f, i) {
  if (f.err || !f.fid || (f.pending && !isMine(m))) return fileState(f, isMine(m));
  const audio = new Audio();
  audio.preload = 'metadata';
  attSrc(f).then(u => { audio.src = u; }).catch(() => {});
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
  const voteBtn = h('button', { class: 'btn small pvote', hidden: true, onclick: (e) => { e.stopPropagation(); act('vote', { id: m.id, options: [...picks] }).catch(er => toast(er.message, true)); } }, 'Vote');
  p.options.forEach((o, i) => {
    const pct = total ? Math.round((o.voters.length / total) * 100) : 0;
    const mineOpt = o.voters.includes(me);
    const opt = h('div', { class: 'opt' + (mineOpt ? ' picked' : '') },
      showRes ? h('span', { class: 'pct' }, pct + '%') : h('span', { class: 'radio' }),
      h('div', null, o.text, showRes && mineOpt ? ' ✓' : ''),
      showRes ? h('div', { class: 'bar', style: `width:${Math.max(2, pct)}%` }) : null);
    if (!showRes) opt.onclick = (e) => {
      e.stopPropagation();
      if (!p.multi) { act('vote', { id: m.id, options: [i] }).catch(er => toast(er.message, true)); return; }
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
      onclick: (ev) => { ev.stopPropagation(); act('react', { id: m.id, emoji: e, on: !who.includes(me) }).catch(er => toast(er.message, true)); },
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
        try { await act('button', { id: m.id, row: r, col: c }); toast(S.wsOk ? `Sent: ${b.text}` : `${b.text}: goes when connected`); }
        catch (er) { toast(er.message, true); }
      },
    }, b.url ? [b.text, ' ↗'] : b.text)))));
}

function buildPendingRow(p) {
  const bubble = h('div', { class: 'bubble' });
  if (p.reply_to) bubble.appendChild(buildQuote(p.reply_to));
  for (const f of p.files || []) {
    // hashing here first, then bia pulls it (the real message shows that part)
    const pct = p.progress && p.progress[f.name] !== undefined ? p.progress[f.name] : 0;
    const R = 21, C = 2 * Math.PI * R;
    const icon = h('div', { class: 'dicon', html: `<svg class="ring" viewBox="0 0 48 48"><circle cx="24" cy="24" r="${R}" stroke-dasharray="${C}" stroke-dashoffset="${C * (1 - pct)}"/></svg>` });
    icon.prepend(ic(p.error ? 'close' : 'upload'));
    bubble.appendChild(h('div', { class: 'doc' }, icon,
      h('div', { class: 'grow' }, h('div', { class: 'dname' }, f.name), h('div', { class: 'dsize' }, p.error ? 'failed' : pct < 1 && p.progress[f.name] !== undefined ? `preparing ${Math.round(pct * 100)}%` : `${fmtBytes(f.size)} · waiting to go`))));
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

// ---- markdown (web/md.js) ----
const md = src => BiaMd.md(src, { paths: S.root });

// ---- context menu / actions ----
function msgMenu(e, m) {
  e.preventDefault();
  const mine = isMine(m);
  const mineOn = em => ((m.reactions || {})[em] || []).includes(S.me.identity);
  const reacts = h('div', { class: 'react-row' }, BIA_REACTIONS.slice(0, 7).map(em =>
    h('button', { onclick: () => { closeMenu(); act('react', { id: m.id, emoji: em, on: !mineOn(em) }).catch(er => toast(er.message, true)); } }, em)),
  h('button', { title: 'More', onclick: (ev) => { const r = ev.currentTarget.getBoundingClientRect(); closeMenu(); emojiPopup(r.left, r.top, em => act('react', { id: m.id, emoji: em, on: !mineOn(em) })); } }, '＋'));
  const files = m.files || [];
  const sel = String(window.getSelection() || '');
  menu(e.clientX, e.clientY, [
    { icon: 'reply', text: 'Reply', onClick: () => setReply(m) },
    mine && (m.text || files.length) && !m.poll ? { icon: 'edit', text: 'Edit', onClick: () => setEdit(m) } : null,
    m.text ? { icon: 'copy', text: sel ? 'Copy selected text' : 'Copy text', onClick: () => copyText(sel || m.text) } : null,
    { icon: m.pinned ? 'unpin' : 'pin', text: m.pinned ? 'Unpin' : 'Pin', onClick: () => act('pin', { id: m.id, on: !m.pinned }) },
    files.some(f => f.fid && !f.pending) ? { icon: 'download', text: files.length > 1 ? `Download all (${files.length})` : 'Download', onClick: () => files.filter(f => f.fid && !f.pending).forEach((f, i) => setTimeout(() => downloadAtt(f), i * 400)) } : null,
    S.root && files.length ? { icon: 'folder', text: 'Show in Files', onClick: () => openPathInFiles(files[0].path.startsWith('/') ? files[0].path : (S.me.files + '/' + files[0].path)) } : null,
    m.poll && m.poll.options.some(o => o.voters.includes(S.me.identity)) && !m.poll.closed ? { icon: 'poll', text: 'Retract vote', onClick: () => act('vote', { id: m.id, options: [] }) } : null,
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
  try { setSelecting(false); await act('delete', { ids }); }
  catch (e) { toast(e.message, true); }
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
  else { b.className = 'sendbtn'; set(b, ic('mic')); b.title = 'Record voice message'; }
}
function onSendBtn() {
  const ta = $('#composerInput');
  if (S.rec) { stopRecording(true); return; }
  if (ta.value.trim() || S.edit) { sendText(); return; }
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
    try { cancelCtx(); await act('edit', { id: m.id, text }); }
    catch (e) { toast(e.message, true); }
    return;
  }
  if (!text.trim()) return;
  ta.value = ''; autoGrow(ta); localStorage.removeItem(draftKey(S.cur));
  const reply = S.reply; S.reply = null; renderComposerCtx(); updateSendBtn();
  await sendMessage({ text, files: [], reply_to: reply });
}

// optimistic send: the message has our own id, so its bubble becomes the real one when bia
// stores it; files are hashed here, then bia pulls them over the link. While the link is
// down it waits in the outbox (also across a reload).
async function sendMessage({ text, files, reply_to, kind }) {
  const chat = 'main';
  const id = 'w' + [...crypto.getRandomValues(new Uint8Array(8))].map(x => x.toString(16).padStart(2, '0')).join('');
  const p = { id, chat, text: text || '', files: files.map(f => ({ name: f.name, size: f.size })), progress: {}, reply_to, ts: Date.now() };
  if (!S.pending.has(chat)) S.pending.set(chat, []);
  S.pending.get(chat).push(p);
  renderTail(); scrollBottom(true);
  try {
    await S.link.send({
      id, text: text || '', reply_to: reply_to || '', kind: kind || '', files,
      onHash: (f, done, total) => { p.progress[f.name] = total ? done / total : 1; if (S.cur === chat) scheduleDraft(); },
    });
    const list = S.pending.get(chat) || [];
    const i = list.indexOf(p);
    if (i >= 0) list.splice(i, 1);
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
  const dlBtn = iconBtn('download', 'Download', () => items[i].get().then(u => download(u, items[i].name)).catch(e => toast(e.message, true)));
  const v = h('div', { class: 'viewer' },
    h('div', { class: 'vtop' }, name, h('span', { class: 'muted', id: 'vcount' }), dlBtn,
      iconBtn('external', 'Open in new tab', () => items[i].get().then(u => window.open(u, '_blank', 'noopener'))),
      iconBtn('close', 'Close', () => close())),
    stage);
  const show = () => {
    const it = items[i];
    name.textContent = it.name;
    $('#vcount', v).textContent = items.length > 1 ? `${i + 1} of ${items.length}` : '';
    let el;
    const shown = i;
    const loading = h('div', { class: 'muted' }, 'Loading…');
    set(stage, loading);
    it.get().then(url => {
      if (shown !== i) return;
      if (it.kind === 'photo') el = h('img', { src: url, alt: it.name });
      else if (it.kind === 'video') el = h('video', { src: url, controls: true, autoplay: true, playsinline: true });
      else if (it.kind === 'audio' || it.kind === 'voice') el = h('audio', { src: url, controls: true, autoplay: true });
      else if (it.kind === 'pdf') el = h('iframe', { src: url });
      else if (it.kind === 'text') {
        el = h('pre', { class: 'editor', style: 'max-width:1000px;height:100%;margin:0;overflow:auto;white-space:pre-wrap' }, 'Loading…');
        fetch(url).then(r => r.text()).then(t => { el.textContent = t.length > 2e6 ? t.slice(0, 2e6) + '\n…' : t; });
      } else el = h('div', { class: 'muted' }, 'No preview');
      loading.replaceWith(el);
    }).catch(e => { loading.textContent = 'Not available: ' + e.message; });
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
  try { await act('send', { text: '', paths }); toast('Sent'); go('chat/main'); }
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
      if (!isActive() && !NOTIF.sub) showNote('bia needs you', t.title, 'hitl-' + t.id, 'task/' + t.id, true);
    }
  }
  const changed = JSON.stringify(tasks.map(t => t.id)) !== JSON.stringify(S.tasks.map(t => t.id));
  S.tasks = tasks;
  if (changed) { renderChatList(); renderTaskbar(); renderTasks(); updateTitle(); }
}
// ---------- login QRs (Telegram, WhatsApp): a popup while an MCP shows one ----------
// The same QR as the window on bia's screen; it follows the rotation and closes once the
// login is done (the MCP deletes its qr.png). Hide keeps it closed until the next login.
const QR = {};
async function pollQr() {
  let j;
  try { j = await GET('/v1/qr'); } catch { return; }
  const live = new Set();
  for (const q of j.qr || []) {
    live.add(q.bus);
    let p = QR[q.bus];
    if (!p) {
      const img = h('img', { class: 'qrimg', alt: q.title + ' login QR' });
      const back = h('div', { class: 'modal-back' }, h('div', { class: 'modal' },
        h('div', { class: 'mhead' }, ic('key'), q.title + ' login'),
        h('div', { class: 'mbody qrbody' }, img, h('p', null, q.hint), h('p', { class: 'muted' }, 'It changes every half a minute; this picture follows it.')),
        h('div', { class: 'mfoot' }, h('button', { class: 'btn flat', onclick: () => { p.hidden = true; back.remove(); } }, 'Hide'))));
      p = QR[q.bus] = { back, img, v: 0, hidden: false };
      document.body.appendChild(back);
      blip();
    }
    if (p.v !== q.v) { p.v = q.v; p.img.src = '/v1/qr/' + q.bus + '.png?v=' + q.v; }
  }
  for (const bus of Object.keys(QR)) {
    if (!live.has(bus)) { QR[bus].back.remove(); delete QR[bus]; }
  }
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
      h('div', { class: 'tbox' }, h('div', { class: 'title' }, 'Display'), h('div', { class: 'subtitle' }, 'bia\'s screen, live')),
      iconBtn('refresh', 'Reconnect', () => { const f = $('#displayFrame'); f.src = 'about:blank'; setTimeout(() => { f.src = '/gui'; }, 50); }),
      matchMedia('(pointer: coarse)').matches ? null : iconBtn('external', 'New tab', () => window.open('/gui', '_blank', 'noopener'))),
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
      h('button', { class: 'btn', onclick: renewCert }, ic('refresh'), 'Renew my certificate')));

  const appearance = h('div', { class: 'card' }, h('h3', null, ic('moon'), 'Appearance'),
    h('div', { class: 'setrow' }, h('div', { class: 'grow' }, 'Theme'), seg([['auto', 'Auto'], ['light', 'Day'], ['dark', 'Night']], S.prefs.theme, v => { S.prefs.theme = v; savePrefs(); renderSettings(); })),
    h('div', { class: 'setrow' }, h('div', { class: 'grow' }, `Message text size (${S.prefs.fs}px)`),
      h('input', { type: 'range', min: 12, max: 20, value: S.prefs.fs, oninput: e => { S.prefs.fs = +e.target.value; savePrefs(); e.target.parentElement.firstChild.textContent = `Message text size (${S.prefs.fs}px)`; } })),
    h('div', { class: 'setrow' }, h('div', { class: 'grow' }, 'Send with Enter (Shift+Enter for a new line)'), sw(S.prefs.enterSends, v => { S.prefs.enterSends = v; savePrefs(); })));

  const perm = canNotify() ? Notification.permission : '';
  const notifOn = S.prefs.notify && perm === 'granted';
  const notifState = !canNotify()
    ? (isIOS() && !isStandalone() && !inBiaApp() ? 'On iPhone and iPad notifications need the app on the Home Screen: see “Install” below, then turn this on there.' : 'not supported by this browser')
    : perm === 'denied' ? 'blocked for this site — allow notifications in the browser\'s site settings'
    : !notifOn ? 'off'
    : NOTIF.sub ? 'on — also when no bia tab is open'
    : `on while a bia tab is open${NOTIF.err ? ` (push: ${NOTIF.err})` : ''}`;
  const testPush = async () => {
    try { const r = await POST('/v1/push/test'); toast(`Sent to ${r.sent} browser${r.sent === 1 ? '' : 's'}`); }
    catch (e) { toast(e.message, true); }
    loadPushSubs();
  };
  const notif = h('div', { class: 'card' }, h('h3', null, ic('bell'), 'Notifications'),
    h('div', { class: 'setrow' }, h('div', { class: 'grow' }, 'Browser notifications', h('div', { class: 'muted', style: 'font-size:13px' }, notifState)),
      canNotify() && perm !== 'denied' ? sw(notifOn, async v => {
        if (v) return enableNotifications();
        S.prefs.notify = false; savePrefs(); await pushOff(); renderSettings(); notifDecided('off');
      }) : null),
    NOTIF.sub ? h('div', { class: 'setrow' }, h('div', { class: 'grow muted', style: 'font-size:13px' }, 'Notifications go through your browser\'s push service, end-to-end encrypted.'),
      h('button', { class: 'btn ghost small', onclick: testPush }, 'Send a test')) : null,
    h('div', { class: 'setrow' }, h('div', { class: 'grow' }, 'Sound'), sw(S.prefs.sound, v => { S.prefs.sound = v; savePrefs(); if (v) blip(); })),
    h('h3', { style: 'margin-top:18px' }, 'Browsers with push notifications'), h('div', { id: 'pushSubs' }, 'Loading…'));
  setTimeout(loadPushSubs, 0);

  // The agent CA is public (/v1/pki/ca): whoever connects to this bia trusts it, so anyone signed in may take it.
  const caFp = h('div', { class: 'copyline' }, h('code', null, 'Loading…'));
  const caBox = [
    h('p', { class: 'muted', style: 'margin-top:0' }, 'The certificate authority behind this bia\u2019s web address and every certificate it issues. Whoever connects trusts it to know they reached this bia; check the SHA-256 fingerprint after importing.'),
    h('div', { class: 'row' },
      h('button', { class: 'btn', id: 'caPem', onclick: () => download('/v1/pki/ca', 'bia-ca.crt') }, ic('download'), 'Download bia-ca.crt'),
      h('button', { class: 'btn ghost', id: 'caDer', title: 'The same certificate in binary (DER) form, for Windows and Android', onclick: () => download('/v1/pki/ca?format=der', 'bia-ca.cer') }, ic('download'), '.cer (DER)')),
    caFp];
  GET('/.well-known/bia.json').then(d => {
    const hex = d.ca_sha256 || '';
    const fp = (hex.toUpperCase().match(/.{1,2}/g) || []).join(':');
    set(caFp, h('code', { id: 'caSha' }, fp || 'unknown'), fp ? iconBtn('copy', 'Copy the fingerprint', () => copyText(hex, 'Fingerprint')) : null);
  }).catch(e => set(caFp, h('div', { class: 'err-line' }, e.message)));

  const kind = installKind();
  const install = !kind ? null : h('div', { class: 'card', id: 'installCard' }, h('h3', null, ic('download'), kind === 'prompt' ? `Install ${calledName()}` : `Install ${calledName()} on this phone`),
    h('p', { class: 'muted', style: 'margin-top:0' }, kind === 'prompt' ? 'As an app of its own on this computer, with its own window and icon.'
      : 'From the home screen it opens like an app, and notifications reach this phone properly. In a browser tab ' + (kind === 'ios' ? 'iPhone and iPad deliver none.' : 'they stop when the browser sleeps.')),
    h('ol', { class: 'install-steps' }, installSteps(kind).map(s => h('li', null, s))),
    INSTALL.ev ? h('button', { class: 'btn', onclick: installNow }, ic('download'), 'Install') : null,
    kind === 'android' && !INSTALL.ev ? h('p', { class: 'muted', style: 'margin-bottom:0' }, 'If the browser only offers a shortcut, or nothing, this phone does not trust this bia’s CA yet: download it under Agent CA below, install it in Android’s Settings → Security → Encryption & credentials → Install a certificate → CA certificate, then reload this page.') : null);

  const kids = [profile, appearance, notif, install].filter(Boolean);
  if (!S.root) kids.push(h('div', { class: 'card' }, h('h3', null, ic('shield'), 'Agent CA'), ...caBox));
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
    const confAddr = h('input', { class: 'input', value: location.host });
    const confName = h('input', { class: 'input', placeholder: 'optional, what to call this bia' });
    const confPass = h('input', { class: 'input', type: 'text', autocomplete: 'off', placeholder: 'leave empty for a strong one' });
    const confOut = h('div');
    const makeConfig = async () => {
      set(confOut);
      try {
        const r = await POST('/v1/pki/biaconfig', { identity: issueId.value.trim(), role: role.value, trust: trust.value, address: confAddr.value.trim(), name: confName.value.trim(), password: confPass.value });
        const bytes = Uint8Array.from(atob(r.file), c => c.charCodeAt(0));
        const blob = new Blob([bytes], { type: 'application/x-biaconfig' });
        saveBlob(blob, r.file_name);
        confPass.value = '';
        set(confOut, h('p', { class: 'muted' }, `${r.file_name} is downloaded, for ${r.identity} at ${r.addresses.join(', ')}. Send the file and the password by different channels: whoever has both is ${r.identity} on this machine. Revoke serial ${r.serial} below to end it.`),
          h('div', { class: 'copyline' }, h('code', null, r.password), iconBtn('copy', 'Copy the password', () => copyText(r.password, 'Copied'))),
          h('button', { class: 'btn', onclick: () => saveBlob(blob, r.file_name) }, ic('download'), 'Download again'));
        loadCerts();
      } catch (e) { set(confOut, h('div', { class: 'err-line' }, e.message)); }
    };
    const certs = h('div', { id: 'certList' }, 'Loading…');
    kids.push(h('div', { class: 'card' }, h('h3', null, ic('shield'), 'Certificates'),
      h('p', { class: 'muted', style: 'margin-top:0' }, 'Give someone (or another agent) access: issue a certificate and send them the pickup link. They also need the CA.'),
      h('div', { class: 'field' }, h('label', null, 'Identity'), issueId),
      h('div', { class: 'row' }, h('div', { class: 'field grow' }, h('label', null, 'Role'), role), h('div', { class: 'field grow' }, h('label', null, 'Trust'), trust)),
      h('button', { class: 'btn', onclick: issue }, ic('key'), 'Issue certificate'), out,
      h('h3', { style: 'margin-top:18px' }, 'Or as a .biaconfig file'),
      h('p', { class: 'muted', style: 'margin-top:0' }, 'One password-protected file with a new certificate for the identity above and this bia\u2019s address. The bia deploy apps open it and go straight to the chat.'),
      h('div', { class: 'field' }, h('label', null, 'Address'), confAddr),
      h('div', { class: 'row' }, h('div', { class: 'field grow' }, h('label', null, 'Name'), confName), h('div', { class: 'field grow' }, h('label', null, 'Password'), confPass)),
      h('button', { class: 'btn', onclick: makeConfig }, ic('download'), 'Make .biaconfig'), confOut,
      h('h3', { style: 'margin-top:18px' }, 'Agent CA'), ...caBox,
      h('h3', { style: 'margin-top:18px' }, 'Requests from other agents'), h('div', { id: 'reqList' }, 'Loading…'),
      h('h3', { style: 'margin-top:18px' }, 'Other bias'), h('div', { id: 'peerList' }, 'Loading…'),
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
    setTimeout(() => { loadCerts(); loadRequests(); loadPeers(); }, 0);
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
        h('td', null, h('b', null, r.identity), ` · ${r.role}`, h('div', { class: 'muted mono', style: 'font-size:11px' }, `${r.id.slice(0, 8)} · from IP ${r.source || '?'}`),
          r.peer_url ? h('div', { class: 'muted', style: 'font-size:12px' }, h('span', { class: 'tag acc' }, 'mutual'), ` a bia at ${r.peer_url}; approving opens a live link with it`) : null,
          r.note ? h('div', { class: 'muted', style: 'font-size:12px' }, `“${r.note}”`) : null),
        h('td', null, new Date(r.created * 1000).toLocaleString()),
        h('td', null, h('span', { class: 'tag ' + (tag[r.status] || '') }, r.status)),
        h('td', null, r.status === 'pending' ? h('div', { class: 'row' },
          h('button', { class: 'btn small', onclick: decide(r, 'approve') }, 'Approve'),
          h('button', { class: 'btn flat danger small', onclick: decide(r, 'deny') }, 'Deny')) : null))))));
  } catch (e) { set(box, e.message); }
}
async function loadPeers() {
  const box = $('#peerList');
  if (!box) return;
  try {
    const j = await GET('/v1/peers');
    if (!j.peers.length) { set(box, h('p', { class: 'muted' }, `None. This bia is ${j.me} to others; ask it to connect to another bia (web_peer_add).`)); return; }
    const act = (p, action) => async () => {
      if (action === 'ban' && !await confirmBox({ title: `Block ${p.name}?`, body: 'Its links and messages are refused until you unblock it. bia cannot lift a block you set.' }, 'Block', true)) return;
      try { await POST('/v1/peers/action', { action, peer: p.name }); loadPeers(); } catch (e) { toast(e.message, true); }
    };
    set(box, h('table', { class: 'certs' },
      h('thead', null, h('tr', null, h('th', null, 'bia'), h('th', null, 'Link'), h('th'))),
      h('tbody', null, j.peers.map(p => {
        const l = p.link || {};
        const tag = p.status !== 'active' ? h('span', { class: 'tag' }, p.status)
          : l.online ? h('span', { class: 'tag ok' }, 'online') : l.banned_by ? h('span', { class: 'tag bad' }, 'blocked') : h('span', { class: 'tag' }, 'offline');
        const btns = p.status !== 'active' ? null : l.banned_by
          ? h('button', { class: 'btn small', onclick: act(p, 'unban') }, 'Unblock')
          : h('div', { class: 'row' },
            l.online ? h('button', { class: 'btn flat small', onclick: act(p, 'disconnect') }, 'Disconnect') : h('button', { class: 'btn small', onclick: act(p, 'connect') }, 'Connect'),
            h('button', { class: 'btn flat danger small', onclick: act(p, 'ban') }, 'Block'));
        return h('tr', null,
          h('td', null, h('b', null, p.name), h('div', { class: 'muted mono', style: 'font-size:11px' }, p.url)),
          h('td', null, tag, h('div', { class: 'muted', style: 'font-size:12px' }, l.state || ''),
            ...(l.transfers || []).map(t => h('div', { class: 'muted', style: 'font-size:12px' }, t))),
          h('td', null, btns));
      }))));
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
      biaImg({ class: 'call-ava' }),
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
      biaImg({ class: 'call-ava' }),
      h('div', { class: 'grow' }, h('div', { class: 'call-title' }, 'bia is calling…'), h('div', { class: 'call-sub' }, c.title))),
    h('div', { class: 'call-ctl' },
      h('button', { class: 'call-btn hang', title: 'Decline', onclick: () => { stopRing(); act('call_decline', { chat: v.chat, id: v.id }).catch(() => {}); } }, ic('phoneOff')),
      h('button', { class: 'call-btn accept', title: 'Answer', onclick: () => { stopRing(); go('chat/' + v.chat); startCall(v.chat, v.id); } }, ic('phone'))));
  document.body.appendChild(RING.el);
  ringTone();
  if (!isActive() && !NOTIF.sub) showNote('bia is calling', 'Open the chat to answer', 'ring-' + v.chat, 'chat/' + v.chat, true);
}
