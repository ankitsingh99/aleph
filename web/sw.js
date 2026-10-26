/* Aleph's service worker: the console's shell offline, Web Push
   notifications, and their Approve / Deny / Reply buttons. The build id
   changes with every new console, so an update replaces the cache. API
   calls are never cached. */
'use strict';

const BUILD = '__ALEPH_BUILD__';
const CACHE = 'aleph-' + BUILD;
const SHELL = ['/', '/app.js', '/style.css', '/manifest.webmanifest', '/icons/aleph-192.png', '/icons/aleph-32.png'];
const ICON = '/icons/aleph-192.png';

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys()
    .then((ks) => Promise.all(ks.filter((k) => k.startsWith('aleph-') && k !== CACHE).map((k) => caches.delete(k))))
    .then(() => self.clients.claim()));
});

self.addEventListener('fetch', (e) => {
  const u = new URL(e.request.url);
  if (u.origin !== self.location.origin || e.request.method !== 'GET') return;
  const p = u.pathname;
  if (p.startsWith('/api/') || p === '/metrics' || p === '/a' || p.startsWith('/a/')) return;
  // the network first (a fresh console when online), the cache when not
  e.respondWith(fetch(e.request)
    .then((r) => {
      if (r.ok) {
        const copy = r.clone();
        caches.open(CACHE).then((c) => c.put(e.request, copy));
      }
      return r;
    })
    .catch(() => caches.match(e.request).then((r) => r || caches.match('/'))));
});

// ------------------------------------------------------------------ push

self.addEventListener('push', (e) => {
  let d = {};
  try { d = e.data ? e.data.json() : {}; } catch (_) { d = { title: 'Aleph', body: e.data ? e.data.text() : '' }; }
  const opts = {
    body: d.body || '',
    tag: d.tag || undefined,
    renotify: !!d.tag,
    requireInteraction: !!d.urgent,
    icon: ICON,
    timestamp: d.t ? d.t * 1000 : Date.now(),
    data: d,
    actions: (d.actions || []).slice(0, 3),
  };
  e.waitUntil(self.registration.showNotification(d.title || 'Aleph', opts));
});

/** This device's token (the page keeps it in IndexedDB for us). */
function token() {
  return new Promise((res) => {
    let r;
    try { r = indexedDB.open('aleph', 1); } catch (_) { res(null); return; }
    r.onupgradeneeded = () => r.result.createObjectStore('kv');
    r.onerror = () => res(null);
    r.onsuccess = () => {
      try {
        const g = r.result.transaction('kv').objectStore('kv').get('token');
        g.onsuccess = () => res(g.result || null);
        g.onerror = () => res(null);
      } catch (_) { res(null); }
    };
  });
}

async function call(path, body, type) {
  const t = await token();
  const headers = { 'X-Aleph': '1', 'Content-Type': type || 'application/json' };
  if (t) headers.Authorization = 'Bearer ' + t;
  const r = await fetch(path, { method: 'POST', headers, body, credentials: 'same-origin' });
  let j = {};
  try { j = await r.json(); } catch (_) { /* not JSON */ }
  return { ok: r.ok, j };
}

function said(title, body, d) {
  return self.registration.showNotification(title, {
    body,
    tag: (d.tag || 'aleph') + '|done',
    icon: ICON,
    data: { url: d.url || '/' },
  });
}

/** Approve or Deny: the signed token from the push, posted back once. */
async function act(which, d) {
  const tok = d.tokens && d.tokens[which];
  if (!tok) return open(d.url);
  try {
    const { ok, j } = await call('/a', tok, 'text/plain');
    const who = d.name ? `${d.name} on ${d.host}` : d.host || 'Aleph';
    if (ok) return said(`${which === 'approve' ? '✓ Approved' : '✕ Denied'}: ${who}`, (j.result && j.result.done) || '', d);
    return said(`Not done: ${who}`, (j.error && j.error.message) || 'Aleph refused it', d);
  } catch (_) {
    return said("Couldn't reach Aleph", 'Open the console to answer it.', d);
  }
}

/** Reply typed into the notification (where the browser offers it). */
async function reply(d, text) {
  if (!d.session || !text) return open(d.url);
  try {
    const { ok, j } = await call('/api/v1/session.send', JSON.stringify({ session: d.session, text }));
    return said(ok ? `Sent to ${d.name}` : `Not sent to ${d.name}`, ok ? text : ((j.error && j.error.message) || ''), d);
  } catch (_) {
    return said("Couldn't reach Aleph", 'Open the console to reply.', d);
  }
}

/** The console, at this notification's session (a window already open first). */
async function open(url) {
  const to = new URL(url || '/', self.location.origin).href;
  const all = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
  for (const c of all) {
    if (new URL(c.url).origin === self.location.origin) {
      c.postMessage({ go: to });
      return c.focus();
    }
  }
  return self.clients.openWindow(to);
}

self.addEventListener('notificationclick', (e) => {
  const d = (e.notification && e.notification.data) || {};
  e.notification.close();
  if (e.action === 'approve' || e.action === 'deny') { e.waitUntil(act(e.action, d)); return; }
  if (e.action === 'reply' && e.reply) { e.waitUntil(reply(d, e.reply)); return; }
  e.waitUntil(open(d.url));
});
