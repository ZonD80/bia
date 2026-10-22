// bia web console service worker: notifications only (no fetch handler, no caching).
// Web Push from bia (see push.rs) and the page's own notifications both land here.
'use strict';

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', e => e.waitUntil(self.clients.claim()));

self.addEventListener('push', e => {
  let d = {};
  try { d = e.data ? e.data.json() : {}; } catch { d = { body: e.data ? e.data.text() : '' }; }
  e.waitUntil(show(d));
});

async function show(d) {
  // a tab in front already handles it (bia skips the push then; this covers the race)
  const wins = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
  if (d.tag !== 'bia-test' && wins.some(c => c.focused && c.visibilityState === 'visible')) return;
  return self.registration.showNotification(d.title || 'bia', {
    body: d.body || '',
    tag: d.tag || undefined,
    renotify: !!d.tag,
    requireInteraction: !!d.urgent,
    icon: '/icon-192.png',
    badge: '/icon-192.png',
    timestamp: d.ts || Date.now(),
    data: { url: d.url || '' },
  });
}

self.addEventListener('notificationclick', e => {
  e.notification.close();
  const url = (e.notification.data && e.notification.data.url) || '';
  e.waitUntil((async () => {
    const wins = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    const w = wins.find(c => new URL(c.url).origin === self.location.origin);
    if (w) {
      w.postMessage({ t: 'open', url });
      try { await w.focus(); } catch {}
      return;
    }
    await self.clients.openWindow('/' + (url ? '#' + url : ''));
  })());
});

// the browser rotated the subscription: hand the new one to bia
self.addEventListener('pushsubscriptionchange', e => {
  e.waitUntil((async () => {
    const old = e.oldSubscription;
    const key = old && old.options && old.options.applicationServerKey;
    const sub = e.newSubscription || (key && await self.registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key }));
    if (!sub) return;
    await fetch('/v1/push/subscribe', { method: 'POST', credentials: 'include', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(sub.toJSON()) });
  })());
});
