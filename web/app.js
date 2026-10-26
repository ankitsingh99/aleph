/* Aleph web console. No framework and no build step: it talks to the
   daemon's HTTP API (/api/v1/<method> — the same methods as `aleph api`)
   with this device's token, and follows the fleet live over Server-Sent
   Events (/api/v1/events). Everything shown is built with textContent, never
   innerHTML: session titles and screens are other programs' output. */
'use strict';

// ------------------------------------------------------------------ DOM helpers

const $ = (sel, el = document) => el.querySelector(sel);

function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k === 'text') el.textContent = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'style') for (const [p, x] of Object.entries(v)) { if (x != null) el.style.setProperty(p, x); }
    else if (k === 'value') el.value = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const c of kids.flat(Infinity)) {
    if (c == null || c === false) continue;
    el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return el;
}

// replaceChildren, skipping null/false and flattening arrays like h() does
// (replaceChildren itself would print them as "null" or "[object …]").
function fill(el, ...kids) {
  el.replaceChildren(...kids.flat(Infinity).filter((c) => c != null && c !== false));
  return el;
}

const SVG = 'http://www.w3.org/2000/svg';
function mark(cls = 'mark') {
  const s = document.createElementNS(SVG, 'svg');
  s.setAttribute('class', cls);
  s.setAttribute('viewBox', '0 0 100 100');
  s.setAttribute('aria-hidden', 'true');
  const u = document.createElementNS(SVG, 'use');
  u.setAttribute('href', '#mark');
  s.append(u);
  return s;
}

/** The aleph-pulse spinner, with what's happening. */
function spinner(text, small) {
  const s = h('span', { class: 'spin' + (small ? ' small' : ''), 'aria-hidden': 'true' }, mark());
  return text ? h('div', { class: 'loading' }, s, h('span', { text })) : s;
}

// ------------------------------------------------------------------ formatting

function ago(t) {
  if (!t) return '';
  const s = Math.max(0, Math.floor(Date.now() / 1000 - t));
  if (s < 10) return 'now';
  if (s < 60) return s + 's';
  if (s < 3600) return Math.floor(s / 60) + 'm';
  if (s < 86400) return Math.floor(s / 3600) + 'h';
  return Math.floor(s / 86400) + 'd';
}
function tokens(n) {
  n = Number(n) || 0;
  if (n >= 1e9) return (n / 1e9).toFixed(1) + 'B';
  if (n >= 1e6) return (n / 1e6).toFixed(1) + 'M';
  if (n >= 1e4) return Math.round(n / 1e3) + 'k';
  if (n >= 1e3) return (n / 1e3).toFixed(1) + 'k';
  return String(n);
}
function money(x) {
  x = Number(x) || 0;
  return x >= 100 ? '$' + Math.round(x) : '$' + x.toFixed(2);
}
function dur(s) {
  s = Math.round(Number(s) || 0);
  if (s < 60) return s + 's';
  if (s < 3600) return Math.round(s / 60) + 'm';
  const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60);
  return h + 'h' + (m ? ' ' + m + 'm' : '');
}
function shortPath(p) {
  if (!p) return '';
  return p.replace(/^\/(home|Users)\/[^/]+/, '~');
}
const enc = (id) => encodeURIComponent(id).replace(/%3A/gi, ':');

// ------------------------------------------------------------------ state

const S = {
  token: null,
  me: null,
  snap: null,
  inbox: [],
  es: null,
  poll: null,
  view: null,
  questions: new Map(),
  answered: new Map(),
  range: 'today',
  install: null,
};

const ls = {
  get(k) { try { return localStorage.getItem('aleph.' + k); } catch (_) { return null; } },
  set(k, v) {
    try { if (v == null) localStorage.removeItem('aleph.' + k); else localStorage.setItem('aleph.' + k, v); } catch (_) { /* private mode */ }
  },
};

// The service worker answers notification buttons with this device's token:
// it reads it from IndexedDB (it can't see localStorage).
function idb() {
  return new Promise((res, rej) => {
    const r = indexedDB.open('aleph', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('kv');
    r.onsuccess = () => res(r.result);
    r.onerror = () => rej(r.error);
  });
}
async function idbPut(k, v) {
  try {
    const db = await idb();
    await new Promise((res, rej) => {
      const t = db.transaction('kv', 'readwrite');
      if (v == null) t.objectStore('kv').delete(k); else t.objectStore('kv').put(v, k);
      t.oncomplete = res;
      t.onerror = () => rej(t.error);
    });
  } catch (_) { /* no IndexedDB: notification buttons fall back to the cookie */ }
}

const RANK = { read: 0, reply: 1, control: 2 };
const rank = () => RANK[S.me && S.me.scope] ?? -1;

const hosts = () => (S.snap && S.snap.hosts) || [];
const sessions = () => hosts().flatMap((x) => (x.sessions || []).map((s) => Object.assign({ _host: x }, s)));
const sid = (s) => s.host + ':' + s.name;
function findSession(id) {
  const [host, ...rest] = id.split(':');
  const name = rest.join(':');
  return sessions().find((s) => s.host === host && (s.name === name || s.pane_id === name));
}
function stateOf(s) {
  if (s._host && !s._host.reachable) return 'down';
  if (s.limit) return 'limit';
  return ['working', 'blocked', 'done', 'idle'].includes(s.status) ? s.status : 'unknown';
}
const STATE_LABEL = { working: 'working', blocked: 'needs you', done: 'done', idle: 'idle', unknown: 'unknown', down: 'unreachable', limit: 'at its limit' };

// ------------------------------------------------------------------ the API

class ApiError extends Error {
  constructor(status, code, message) { super(message); this.status = status; this.code = code; }
}

async function req(path, opt = {}) {
  const method = opt.method || 'GET';
  const headers = { Accept: 'application/json' };
  if (S.token && !opt.anon) headers.Authorization = 'Bearer ' + S.token;
  if (method !== 'GET') { headers['X-Aleph'] = '1'; headers['Content-Type'] = 'application/json'; }
  let r;
  try {
    r = await fetch(path, {
      method,
      headers,
      body: method === 'GET' ? undefined : JSON.stringify(opt.body || {}),
      credentials: 'same-origin',
      cache: 'no-store',
    });
  } catch (_) {
    setLive('offline');
    throw new ApiError(0, 'offline', "Can't reach Aleph — is the computer awake and online?");
  }
  let j = null;
  try { j = await r.json(); } catch (_) { /* not JSON */ }
  if (!r.ok) {
    const e = (j && j.error) || {};
    const err = new ApiError(r.status, e.code || 'http', e.message || 'HTTP ' + r.status);
    if (r.status === 401 && !opt.quiet) signedOut();
    throw err;
  }
  return j ? j.result : null;
}
const api = (method, params) => req('/api/v1/' + method, { method: 'POST', body: params || {} });
const get = (path, q) => req('/api/v1/' + path + (q ? '?' + new URLSearchParams(q) : ''));

/** What went wrong, for a person: what to do about it, with the setting. */
function explain(e) {
  const m = (e && e.message) || String(e);
  const ra = m.match(/remote control isn't (?:on|allowed) for (\S+) \(remote\.allow\)/);
  if (ra) {
    return { title: `Remote control isn't on for ${ra[1]}`, body: 'Phones and the web only act on sessions that remote.allow covers. On the computer:', code: allowCmd(ra[1]) };
  }
  const sc = m.match(/needs the (\w+) scope/);
  if (e.status === 403 && sc) {
    return { title: `This device has the ${(S.me && S.me.scope) || 'read'} scope`, body: `That needs ${sc[1]}. Pair it again on the computer:`, code: `aleph daemon pair --scope ${sc[1]}` };
  }
  if (e.status === 429) return { title: 'Slow down a little', body: m };
  return { title: m };
}
function allowCmd(hostAgent) {
  const cur = ((S.me && S.me.remote && S.me.remote.allow) || '').trim();
  const list = cur ? cur.split(',').map((x) => x.trim()).filter(Boolean) : [];
  if (!list.includes(hostAgent)) list.push(hostAgent);
  return `aleph settings set remote.allow "${list.join(',')}"`;
}
function fail(e) {
  const x = explain(e);
  toast(x.body ? x.title + '. ' + x.body : x.title, 'err', { code: x.code, sticky: !!x.code });
}

/** Run `fn` with the button showing a spinner (and not clickable twice). */
async function busy(btn, fn) {
  if (!btn) return fn();
  const kids = [...btn.childNodes];
  btn.disabled = true;
  fill(btn, spinner(null, true), ...kids);
  try { return await fn(); } finally { btn.disabled = false; fill(btn, ...kids); }
}

// ------------------------------------------------------------------ toasts

function toast(text, kind = 'info', opt = {}) {
  const gl = { ok: '✓', err: '✕', needs: '▲', info: 'ℵ' }[kind] || 'ℵ';
  const el = h('div', { class: 'toast ' + kind, role: kind === 'err' ? 'alert' : 'status' },
    h('span', { class: 'gl', text: gl }),
    h('div', { class: 'body' }, h('div', { text }), opt.code ? h('code', { text: opt.code }) : null),
    opt.open ? h('a', { class: 'btn small', href: opt.open, onclick: () => el.remove() }, 'Open') : null,
    opt.code && navigator.clipboard ? h('button', { class: 'small', onclick: () => navigator.clipboard.writeText(opt.code).then(() => toast('Copied', 'ok')) }, 'Copy') : null,
    h('button', { class: 'ghost small', 'aria-label': 'Dismiss', onclick: () => el.remove() }, '✕'));
  const box = $('#toasts');
  box.append(el);
  while (box.children.length > 4) box.firstChild.remove();
  setTimeout(() => el.remove(), opt.sticky ? 14000 : 4500);
}

// ------------------------------------------------------------------ live state

function setLive(st) {
  const el = $('#live');
  el.dataset.state = st;
  el.hidden = st === 'off';
  $('b', el).textContent = { live: 'live', connecting: 'connecting', reconnecting: 'reconnecting', polling: 'polling', offline: 'offline' }[st] || st;
  el.title = {
    live: 'Updates arrive as they happen',
    reconnecting: 'The live stream dropped; trying again',
    polling: 'Checking every few seconds (the live stream is unavailable)',
    offline: "Can't reach Aleph",
  }[st] || '';
}

function connect() {
  if (S.es) S.es.close();
  if (!('EventSource' in window)) { startPolling(); return; }
  setLive('connecting');
  const es = new EventSource('/api/v1/events');
  S.es = es;
  es.addEventListener('open', () => { setLive('live'); stopPolling(); });
  es.addEventListener('state', (ev) => {
    try { S.snap = JSON.parse(ev.data); } catch (_) { return; }
    onState();
  });
  es.addEventListener('event', (ev) => {
    try { onEvent(JSON.parse(ev.data)); } catch (_) { /* a bad line */ }
  });
  es.addEventListener('error', () => {
    if (es.readyState === EventSource.CLOSED) {
      // refused (no cookie yet, too many streams): check by hand meanwhile
      setLive('polling');
      startPolling();
      setTimeout(async () => {
        if (S.es !== es || !S.me) return;
        await ensureCookie();
        connect();
      }, 20000);
    } else {
      setLive('reconnecting');
    }
  });
}

function startPolling() {
  if (S.poll) return;
  S.poll = setInterval(refreshState, 5000);
  refreshState();
}
function stopPolling() {
  if (S.poll) clearInterval(S.poll);
  S.poll = null;
}
async function refreshState() {
  try {
    S.snap = await get('state.snapshot');
    if ($('#live').dataset.state === 'offline') setLive(S.es && S.es.readyState === 1 ? 'live' : 'polling');
    onState();
  } catch (_) { /* said by req */ }
}

async function refreshInbox() {
  try {
    S.inbox = (await api('inbox.list', { n: 120 })) || [];
    updateBadges();
    if (S.view && S.view.inbox) S.view.inbox();
  } catch (_) { /* said */ }
}

function onState() {
  updateBadges();
  if (S.view && S.view.update) S.view.update();
}

function onEvent(e) {
  if (!S.inbox.some((x) => x.id === e.id)) {
    S.inbox.unshift(Object.assign({ unread: true }, e));
    S.inbox = S.inbox.slice(0, 200);
  }
  updateBadges();
  if (e.kind === 'blocked' && e.name) {
    toast(`${e.name} on ${e.host} needs you${e.title ? ': ' + e.title : ''}`, 'needs', { open: '#/s/' + enc(e.host + ':' + e.name) });
  }
  if (S.view && S.view.inbox) S.view.inbox();
}

function updateBadges() {
  const ss = sessions();
  const needs = ss.filter((s) => s.status === 'blocked').length;
  const unread = S.inbox.filter((e) => e.unread).length;
  $('#c-needs').textContent = needs || '';
  $('#c-inbox').textContent = unread ? (unread > 99 ? '99+' : unread) : '';
  $('#c-fleet').textContent = ss.length || '';
  document.title = needs ? `(${needs}) Aleph` : 'Aleph';
}

// ------------------------------------------------------------------ routing

const VIEWS = { needs: Needs, fleet: Fleet, s: Session, inbox: Inbox, presets: Presets, usage: Usage, device: Device };

function route() {
  const parts = location.hash.replace(/^#\/?/, '').split('/');
  let name = parts[0] || 'needs';
  const arg = decodeURIComponent(parts.slice(1).join('/'));
  if (name === 'pair') return mount(Pair, arg, 'pair');
  if (!S.me) return mount(Pair, '', 'pair');
  if (!VIEWS[name]) name = 'needs';
  mount(VIEWS[name], arg, name);
}

function mount(View, arg, name) {
  if (S.view && S.view.unmount) S.view.unmount();
  const root = $('#view');
  root.replaceChildren();
  S.view = View(root, arg) || {};
  const tab = name === 's' ? 'fleet' : name;
  for (const a of document.querySelectorAll('#tabs a')) {
    if (a.dataset.view === tab) a.setAttribute('aria-current', 'page'); else a.removeAttribute('aria-current');
  }
  $('#device-link').classList.toggle('on', name === 'device');
  if (S.view.update) S.view.update();
  window.scrollTo(0, 0);
}

function showChrome(on) {
  $('#tabs').hidden = !on;
  $('#device-link').hidden = !on;
  if (!on) { $('#where').textContent = ''; setLive('off'); }
}

/** Reuse list items whose content didn't change (no flicker, no lost focus). */
function keyed(box, items, key, sig, make) {
  const old = box._keyed || new Map();
  const next = new Map();
  const els = items.map((it) => {
    const k = key(it), s = sig(it);
    let e = old.get(k);
    if (!e || e.sig !== s) e = { el: make(it), sig: s };
    next.set(k, e);
    return e.el;
  });
  box._keyed = next;
  const same = els.length === box.children.length && els.every((el, i) => box.children[i] === el);
  if (!same) fill(box, ...els);
}

// ------------------------------------------------------------------ shared bits

function hostChip(alias) {
  const x = hosts().find((y) => y.alias === alias);
  return h('span', { class: 'chip', title: x && !x.reachable ? 'unreachable' : null, style: { '--c': x && x.color } }, h('span', { class: 'sw' }), alias);
}

function canAct(s) {
  if (rank() < 1) return { ok: false, why: 'This device can only look (read scope). To answer from here, pair it with:', code: 'aleph daemon pair --scope reply' };
  if (!s.remote) return { ok: false, why: `Remote control isn't on for ${s.host}:${s.agent}. On the computer:`, code: allowCmd(`${s.host}:${s.agent}`) };
  return { ok: true };
}

function cantNote(c) {
  if (c.ok) return null;
  return h('div', { class: 'note' }, c.why, ' ', h('code', { text: c.code }));
}

async function loadQuestion(s, el) {
  const k = sid(s) + '|' + (s.status_since || '');
  let q = S.questions.get(k);
  if (q === undefined) {
    S.questions.set(k, null);
    try {
      const r = await api('screen.read', { session: sid(s), lines: 1 });
      q = (r && r.question) || '';
    } catch (_) { q = false; }
    S.questions.set(k, q);
  } else if (q === null) {
    // another card is reading it: look again shortly
    setTimeout(() => loadQuestion(s, el), 600);
    return;
  }
  if (q) { el.classList.remove('wait'); el.textContent = q; } else if (q === '') el.remove(); else el.textContent = "Couldn't read the question: open the session to see it.";
}

/** Close a session: after Aleph's grace, with an Undo snackbar until then. */
async function closeSession(btn, s) {
  if (!confirm(`Close ${s.name} on ${s.host}? The agent stops and its pane closes after a few seconds — Undo keeps it until then.`)) return;
  await busy(btn, async () => {
    try {
      const r = (await api('session.close', { session: sid(s) })) || {};
      if (r.pending) closingSnack(s, r);
      else toast(`Closed ${s.name} on ${s.host}`, 'ok');
    } catch (e) { fail(e); }
  });
}

/** The snackbar of a close waiting out its grace: a countdown, and Undo. */
function closingSnack(s, r) {
  const end = r.deadline_ms || (r.deadline || 0) * 1000;
  const msg = h('div');
  let timer = null;
  const done = () => { clearInterval(timer); el.remove(); };
  const undoB = h('button', { class: 'small primary', onclick: (e) => undo(e.currentTarget) }, 'Undo');
  const el = h('div', { class: 'toast snack', role: 'status' },
    h('span', { class: 'gl', text: '✕' }),
    h('div', { class: 'body' }, msg),
    undoB,
    h('button', { class: 'ghost small', 'aria-label': 'Dismiss', onclick: done }, '✕'));
  function tick() {
    const left = Math.max(0, Math.ceil((end - Date.now()) / 1000));
    if (left > 0) {
      msg.textContent = `Closing ${s.name} on ${s.host} in ${left}s — the agent runs until then`;
      return;
    }
    clearInterval(timer);
    undoB.remove();
    el.classList.add('ok');
    msg.textContent = `Closing ${s.name}…`;
    setTimeout(() => el.remove(), 2500);
  }
  async function undo(b) {
    await busy(b, async () => {
      try {
        await api('session.close.undo', { session: sid(s) });
        done();
        toast(`Kept ${s.name} — it won't close`, 'ok');
      } catch (e) { done(); fail(e); }
    });
  }
  $('#toasts').append(el);
  tick();
  timer = setInterval(tick, 250);
}

/** Approve / deny (and anything else that answers a session). */
async function answer(btn, method, s, done) {
  await busy(btn, async () => {
    try {
      await api(method, { session: sid(s) });
      S.answered.set(sid(s) + '|' + (s.status_since || ''), done);
      toast(`${done}: ${s.name} on ${s.host}`, 'ok');
      if (S.view && S.view.update) S.view.update();
    } catch (e) { fail(e); }
  });
}

// ------------------------------------------------------------------ Needs you

function Needs(root) {
  const sub = h('span', { class: 'sub' });
  const cards = h('div', { class: 'cards' });
  const extra = h('div');
  root.append(h('div', { class: 'view-head' }, h('h1', { text: 'Needs you' }), sub), cards, extra);

  function card(s) {
    const answered = S.answered.get(sid(s) + '|' + (s.status_since || ''));
    const c = canAct(s);
    const q = h('div', { class: 'question wait' }, 'Reading the question…');
    loadQuestion(s, q);
    const ok = c.ok && !answered;
    return h('article', { class: 'card st-blocked', style: { '--sc': s.color } },
      h('div', { class: 'card-top' },
        h('span', { class: 'dot' }),
        h('a', { class: 'card-name', href: '#/s/' + enc(sid(s)), text: s.name || s.display }),
        hostChip(s.host),
        h('span', { class: 'chip', text: s.agent || 'agent' }),
        s.remote ? h('span', { class: 'remote-mark', title: 'Remote control is on for it', text: '⇄' }) : null,
        h('span', { class: 'since', title: 'waiting', text: ago(s.status_since) })),
      s.title ? h('p', { class: 'card-title', text: s.title }) : null,
      q,
      h('div', { class: 'actions' },
        h('button', { class: 'approve', disabled: !ok, onclick: (e) => answer(e.currentTarget, 'session.approve', s, 'Approved') }, '✓ Approve'),
        h('button', { class: 'deny', disabled: !ok, onclick: (e) => answer(e.currentTarget, 'session.deny', s, 'Denied') }, '✕ Deny'),
        h('a', { class: 'btn', href: '#/s/' + enc(sid(s)) }, 'Open ›')),
      answered ? h('div', { class: 'note', text: `✓ ${answered} — waiting for the agent to carry on` }) : cantNote(c));
  }

  function update() {
    if (!S.snap) { fill(cards, spinner('Looking at every machine…')); return; }
    const all = sessions();
    const blocked = all.filter((s) => s.status === 'blocked').sort((a, b) => (a.status_since || 0) - (b.status_since || 0));
    sub.textContent = blocked.length ? `${blocked.length} waiting` : '';
    if (!blocked.length) {
      cards._keyed = null;
      const working = all.filter((s) => s.status === 'working').length;
      fill(cards, h('div', { class: 'panel empty' },
        h('div', { class: 'big', text: '✓' }),
        h('h2', { text: 'Nothing needs you' }),
        h('div', { text: `${all.length} session${all.length === 1 ? '' : 's'} on ${hosts().length} machine${hosts().length === 1 ? '' : 's'}${working ? ` · ${working} working` : ''}` }),
        h('p', null, h('a', { href: '#/fleet', text: 'See the fleet ›' }))));
    } else {
      keyed(cards, blocked, sid,
        (s) => [s.status_since, s.title, s.remote, rank(), S.answered.get(sid(s) + '|' + (s.status_since || '')) || ''].join('|'),
        card);
    }
    // sessions at their usage limit, machines down
    const lim = all.filter((s) => s.limit && s.status !== 'blocked');
    const down = hosts().filter((x) => !x.reachable);
    fill(extra,
      lim.length ? h('section', { class: 'panel flush' },
        h('div', { class: 'panel-head' }, h('h2', { class: 'grow', text: 'At their limit' }), h('span', { class: 'badge st-limit', text: lim.length })),
        h('ul', { class: 'sessions' }, lim.map((s) => h('li', null, sessRow(s))))) : null,
      down.map((x) => h('div', { class: 'callout bad' }, h('b', { text: `${x.alias} isn't answering` }), h('span', { class: 'muted', text: x.error || 'no answer from its herdr' }))));
  }
  return { update };
}

// ------------------------------------------------------------------ Fleet

function sessRow(s) {
  const st = stateOf(s);
  return h('a', { class: 'sess st-' + st, href: '#/s/' + enc(sid(s)), style: { '--sc': s.color } },
    h('span', { class: 'dot', title: STATE_LABEL[st] }),
    h('span', { class: 'nm' }, s.name || s.display || s.pane_id, h('span', { class: 'ag', text: s.agent })),
    h('span', { class: 'rt' },
      s.remote ? h('span', { class: 'remote-mark', title: 'Remote control is on', text: '⇄' }) : null,
      h('span', { class: 'badge st-' + st, text: STATE_LABEL[st] }),
      s.status_since ? h('span', { text: ago(s.status_since) }) : null),
    h('span', { class: 'ti', text: s.title || shortPath(s.cwd) || '' }));
}

function Fleet(root) {
  const sub = h('span', { class: 'sub' });
  const sum = h('div', { class: 'summary' });
  const list = h('div');
  root.append(h('div', { class: 'view-head' }, h('h1', { text: 'Fleet' }), sub), sum, list);
  function update() {
    if (!S.snap) { fill(list, spinner('Looking at every machine…')); return; }
    const hs = hosts();
    const ss = sessions();
    const n = (st) => ss.filter((s) => s.status === st).length;
    sub.textContent = `${hs.filter((x) => x.reachable).length} of ${hs.length} machines up`;
    fill(sum, 
      h('span', { class: 'badge st-working' }, h('span', { class: 'dot' }), `${n('working')} working`),
      h('span', { class: 'badge st-blocked' }, h('span', { class: 'dot' }), `${n('blocked')} need you`),
      h('span', { class: 'badge st-done' }, h('span', { class: 'dot' }), `${n('done')} done`),
      h('span', { class: 'badge st-idle' }, h('span', { class: 'dot' }), `${n('idle')} idle`));
    keyed(list, hs, (x) => x.alias, (x) => JSON.stringify([x.reachable, x.error, x.latency_ms, x.herdr_version, x.stale, x.sessions, rank()]), (x) => {
      const up = x.reachable;
      const meta = [up ? (x.latency_ms != null ? x.latency_ms + ' ms' : '') : 'unreachable', x.herdr_version ? 'herdr ' + x.herdr_version : '', x.stale ? 'stale' : ''].filter(Boolean).join(' · ');
      const ss2 = (x.sessions || []).map((s) => Object.assign({ _host: x }, s));
      return h('section', { class: 'panel flush' },
        h('div', { class: 'panel-head host-head', style: { '--c': x.color } },
          h('span', { class: 'sw' }),
          h('span', { class: 'name grow', text: x.alias }),
          h('span', { class: 'badge ' + (up ? 'st-done' : 'st-down') }, h('span', { class: 'dot' }), up ? 'up' : 'down'),
          h('span', { class: 'meta', text: meta })),
        !up && x.error ? h('div', { class: 'down-note', text: x.error }) : null,
        ss2.length ? h('ul', { class: 'sessions' }, ss2.map((s) => h('li', null, sessRow(s))))
          : h('div', { class: 'down-note muted', text: up ? 'No sessions.' : '' }));
    });
  }
  return { update };
}

// ------------------------------------------------------------------ one session

const KEYS = [['1', '1'], ['2', '2'], ['3', '3'], ['enter', '⏎ enter'], ['esc', 'esc'], ['up', '↑'], ['down', '↓'], ['tab', 'tab'], ['ctrl+c', 'ctrl+c']];

function Session(root, id) {
  let s = findSession(id);
  let timer = null;
  let seenQ = '';
  const title = h('h1');
  const badge = h('span', { class: 'badge' });
  const chips = h('span', { class: 'row' });
  const titleLine = h('p', { class: 'sess-title' });
  const callout = h('div');
  const screen = h('pre', { class: 'screen', tabindex: '0', 'aria-label': 'The session\'s screen' });
  const meta = h('span', { class: 'screen-meta' });
  const qPanel = h('section', { class: 'panel', hidden: true });
  const qText = h('div', { class: 'question' });
  const approveB = h('button', { class: 'approve', onclick: (e) => s && answer(e.currentTarget, 'session.approve', s, 'Approved').then(poll) }, '✓ Approve');
  const denyB = h('button', { class: 'deny', onclick: (e) => s && answer(e.currentTarget, 'session.deny', s, 'Denied').then(poll) }, '✕ Deny');
  // closing waits Aleph's grace (close.grace): the snackbar's Undo keeps it
  const closeB = h('button', { class: 'deny small close-s', title: 'Stop the agent and close its pane, after a few seconds (Undo keeps it until then)', onclick: (e) => s && closeSession(e.currentTarget, s) }, '✕ Close…');
  qPanel.append(h('h2', { text: 'It asks' }), qText, h('div', { class: 'actions' }, approveB, denyB));
  const ta = h('textarea', {
    placeholder: 'Write to the agent…', rows: '3', 'aria-label': 'Message',
    onkeydown: (e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); send(sendB); } },
  });
  const sendB = h('button', { class: 'primary', onclick: (e) => send(e.currentTarget) }, 'Send');
  const queueB = h('button', { onclick: (e) => queue(e.currentTarget), title: 'Sent when the session is free' }, 'Queue for later');
  const keysBox = h('div', { class: 'keys' }, KEYS.map(([k, label]) => h('button', { class: 'key small' + (k === 'ctrl+c' ? ' deny' : ''), title: 'send ' + k, onclick: (e) => keys(e.currentTarget, k) }, label)));
  const qList = h('ul', { class: 'queue-list' });
  const replyPanel = h('section', { class: 'panel' }, h('h2', { text: 'Reply' }), h('div', { class: 'note', text: 'Ctrl/⌘ + Enter sends.' }), ta, h('div', { class: 'actions' }, sendB, queueB));
  const keysPanel = h('section', { class: 'panel' }, h('h2', { text: 'Keys' }), h('div', { class: 'note', text: 'Answer a menu or move in it.' }), keysBox);
  const queuePanel = h('section', { class: 'panel', hidden: true }, h('h2', { text: 'Queued for it' }), qList);

  root.append(
    h('a', { class: 'back', href: '#/fleet' }, '‹ Fleet'),
    h('div', { class: 'sess-head' }, title, badge, chips, closeB),
    titleLine,
    callout,
    h('div', { class: 'grid two' },
      h('section', { class: 'panel flush screen-wrap' },
        h('div', { class: 'panel-head' }, h('h2', { class: 'grow', text: 'Screen' }), meta,
          h('button', { class: 'ghost small', 'aria-label': 'Read it again', onclick: () => poll() }, '↻')),
        screen),
      h('div', null, qPanel, replyPanel, keysPanel, queuePanel)));

  let last = 0;
  async function poll() {
    if (!s) return;
    fill(meta, spinner(null, true), 'reading');
    try {
      const r = await api('screen.read', { session: sid(s), lines: 120 });
      const atEnd = screen.scrollTop + screen.clientHeight >= screen.scrollHeight - 24;
      screen.textContent = (r && r.text) || '';
      if (atEnd) screen.scrollTop = screen.scrollHeight;
      seenQ = (r && r.question) || '';
      last = Date.now();
      showQuestion();
    } catch (e) {
      if (e.status === 404) screen.textContent = 'That session is gone.';
      else if (e.code !== 'offline') screen.textContent = e.message;
    }
    meta.textContent = last ? 'updated ' + ago(last / 1000) : '';
  }
  function showQuestion() {
    const blocked = s && s.status === 'blocked';
    qPanel.hidden = !blocked;
    qText.textContent = seenQ || 'Reading the question…';
  }
  async function send(btn) {
    const text = ta.value.trim();
    if (!text || !s) return;
    await busy(btn, async () => {
      try {
        await api('session.send', { session: sid(s), text });
        ta.value = '';
        toast(`Sent to ${s.name}`, 'ok');
        setTimeout(poll, 800);
      } catch (e) {
        if (/waiting/.test(e.message) && s.status === 'blocked') toast(`${s.name} is waiting on a question: answer it first (Approve, Deny or a key)`, 'err');
        else fail(e);
      }
    });
  }
  async function queue(btn) {
    const text = ta.value.trim();
    if (!text || !s) return;
    await busy(btn, async () => {
      try {
        const r = await api('queue.add', { host: s.host, prompt: text, to: s.name, agent: s.agent || undefined });
        ta.value = '';
        toast(`Queued (#${r && r.id}): ${s.name} gets it when it's free`, 'ok');
        loadQueue();
      } catch (e) { fail(e); }
    });
  }
  async function keys(btn, k) {
    if (!s) return;
    if (k === 'ctrl+c' && !confirm(`Interrupt ${s.name} (ctrl+c)?`)) return;
    await busy(btn, async () => {
      try {
        await api('session.keys', { session: sid(s), keys: [k] });
        setTimeout(poll, 500);
      } catch (e) { fail(e); }
    });
  }
  async function loadQueue() {
    if (!s) return;
    try {
      const all = (await api('queue.list')) || [];
      const mine = all.filter((t) => t.host === s.host && t.to === s.name && t.state === 'queued');
      queuePanel.hidden = !mine.length;
      fill(qList, ...mine.map((t) => h('li', null,
        h('span', { text: t.prompt }),
        rank() >= 1 ? h('button', { class: 'ghost small', 'aria-label': 'Drop it', onclick: async (e) => {
          await busy(e.currentTarget, async () => {
            try { await api('queue.rm', { id: t.id }); loadQueue(); } catch (err) { fail(err); }
          });
        } }, '✕') : null)));
    } catch (_) { /* said */ }
  }

  function update() {
    s = findSession(id) || s;
    if (!s) {
      title.textContent = id;
      titleLine.textContent = S.snap ? 'No such session right now (it may have ended).' : '';
      return;
    }
    const st = stateOf(s);
    title.textContent = s.name || s.display;
    title.parentElement.style.setProperty('--sc', s.color || null);
    badge.className = 'badge st-' + st;
    fill(badge, h('span', { class: 'dot' }), STATE_LABEL[st] + (s.status_since ? ' · ' + ago(s.status_since) : ''));
    fill(chips, hostChip(s.host), h('span', { class: 'chip', text: s.agent || 'agent' }),
      s.remote ? h('span', { class: 'remote-mark', title: 'Remote control is on', text: '⇄ remote' }) : null);
    titleLine.textContent = [s.title, shortPath(s.cwd)].filter(Boolean).join(' · ');
    const c = canAct(s);
    fill(callout, c.ok ? null : h('div', { class: 'callout' }, h('b', { text: rank() < 1 ? 'Look only' : 'Remote control is off for it' }), c.why, h('div', { class: 'cmd' }, h('code', { text: c.code }),
      navigator.clipboard ? h('button', { class: 'small', onclick: () => navigator.clipboard.writeText(c.code).then(() => toast('Copied', 'ok')) }, 'Copy') : null)));
    for (const b of [sendB, queueB, approveB, denyB, closeB, ...keysBox.children]) b.disabled = !c.ok;
    // (closing needs the control scope)
    closeB.hidden = rank() < 2;
    showQuestion();
  }

  update();
  poll();
  loadQueue();
  const tick = () => { if (document.visibilityState === 'visible') poll(); };
  timer = setInterval(tick, 2500);
  return { update, unmount() { clearInterval(timer); } };
}

// ------------------------------------------------------------------ Inbox

const KIND = {
  blocked: ['▲', 'blocked', 'needed you'], done: ['✓', 'done', 'finished'], new: ['+', 'done', 'started'],
  gone: ['–', 'idle', 'ended'], error: ['✕', 'down', 'unreachable'], auto: ['↯', 'aleph', 'auto-approved'],
  task: ['≡', 'working', 'task sent'], heal: ['↻', 'done', 'herdr restarted'], limit: ['◷', 'limit', 'at its limit'],
  up: ['●', 'done', 'back up'], remote: ['⇄', 'aleph', 'remote action'],
};

function Inbox(root) {
  const sub = h('span', { class: 'sub' });
  const readB = h('button', { class: 'small', onclick: (e) => markRead(e.currentTarget) }, 'Mark all read');
  const list = h('ul', { class: 'events' });
  root.append(h('div', { class: 'view-head' }, h('h1', { text: 'Inbox' }), sub, h('span', { class: 'grow' }), rank() >= 1 ? readB : null),
    h('section', { class: 'panel flush' }, list));
  async function markRead(btn) {
    await busy(btn, async () => {
      try {
        await api('inbox.read', {});
        S.inbox.forEach((e) => { e.unread = false; });
        render();
        updateBadges();
      } catch (e) { fail(e); }
    });
  }
  async function dismiss(btn, e) {
    await busy(btn, async () => {
      try {
        await api('inbox.dismiss', { ids: [e.id] });
        S.inbox = S.inbox.filter((x) => x.id !== e.id);
        render();
        updateBadges();
      } catch (err) { fail(err); }
    });
  }
  function row(e) {
    const [gl, st, what] = KIND[e.kind] || ['•', 'idle', e.kind];
    const who = e.name ? `${e.name} on ${e.host}` : e.host;
    const text = e.title || what;
    const link = e.name ? '#/s/' + enc(e.host + ':' + e.name) : null;
    return h('li', null, h('div', { class: 'ev st-' + st + (e.unread ? ' unread' : '') },
      h('span', { class: 'gl', text: gl, title: what }),
      h('div', null,
        link ? h('a', { class: 'tt', href: link, text }) : h('div', { class: 'tt', text }),
        h('div', { class: 'who', text: [who, e.agent, e.source === 'agent' ? 'in its own words' : ''].filter(Boolean).join(' · ') }),
        e.body && e.body !== e.title ? h('div', { class: 'small muted', text: e.body }) : null),
      h('div', { class: 'side' },
        h('span', null, e.unread ? h('span', { class: 'unread-dot', title: 'unread' }) : null, ' ', ago(e.t)),
        rank() >= 1 ? h('button', { class: 'ghost x', 'aria-label': 'Dismiss', onclick: (ev) => dismiss(ev.currentTarget, e) }, '✕') : null)));
  }
  function render() {
    const unread = S.inbox.filter((e) => e.unread).length;
    sub.textContent = unread ? `${unread} unread` : '';
    if (!S.inbox.length) {
      list._keyed = null;
      fill(list, h('li', null, h('div', { class: 'empty' }, h('h2', { text: 'Nothing yet' }), 'Questions, finished turns, machines going down and remote actions land here.')));
      return;
    }
    keyed(list, S.inbox, (e) => e.id, (e) => [e.unread, rank()].join('|'), row);
  }
  list.append(h('li', null, spinner('Reading the inbox…')));
  refreshInbox().then(render);
  return { inbox: render };
}

// ------------------------------------------------------------------ Presets

function Presets(root) {
  const list = h('div', { class: 'cards' }, spinner('Reading presets…'));
  root.append(h('div', { class: 'view-head' }, h('h1', { text: 'Presets' }), h('span', { class: 'sub', text: 'saved starting points' })),
    rank() < 2 ? h('div', { class: 'callout' }, h('b', { text: 'Starting presets needs the control scope' }), 'This device can see them. To start one from here, pair it with ', h('code', { text: 'aleph daemon pair --scope control' })) : null,
    list);
  async function launch(btn, p) {
    await busy(btn, async () => {
      try {
        const r = await api('preset.launch', { name: p.name });
        const n = (r && r.started && r.started.length) || 0;
        if (r && r.errors && r.errors.length) toast(`@${p.name}: ${n} started, ${r.errors.length} problem(s): ${r.errors[0]}`, 'err', { sticky: true });
        else toast(`@${p.name}: ${n} session${n === 1 ? '' : 's'} running`, 'ok', { open: '#/fleet' });
      } catch (e) { fail(e); }
    });
  }
  (async () => {
    try {
      const ps = (await api('preset.list')) || [];
      if (!ps.length) {
        fill(list, h('div', { class: 'panel empty' }, h('h2', { text: 'No presets yet' }), 'On the computer: ', h('code', { text: 'aleph preset add <name> --host <host> --dir <folder>' })));
        return;
      }
      fill(list, ...ps.map((p) => h('article', { class: 'card preset' },
        h('div', { class: 'card-top' }, h('span', { class: 'card-name', text: '@' + p.name }),
          p.autostart ? h('span', { class: 'chip', text: 'autostart' }) : null,
          (p.tags || []).map((t) => h('span', { class: 'chip', text: '#' + t }))),
        p.description ? h('p', { class: 'card-title', text: p.description }) : null,
        h('div', { class: 'panes' }, (p.panes || []).map((x) => h('span', { class: 'chip', text: [x.name, x.host, x.agent, shortPath(x.dir)].filter(Boolean).join(' · ') || 'a session' }))),
        h('div', { class: 'actions' }, h('button', { class: 'primary', disabled: rank() < 2, onclick: (e) => launch(e.currentTarget, p) }, '▶ Launch')))));
    } catch (e) {
      fill(list, h('div', { class: 'callout bad' }, h('b', { text: "Couldn't read the presets" }), e.message));
    }
  })();
  return {};
}

// ------------------------------------------------------------------ Usage

function Usage(root) {
  const seg = h('div', { class: 'seg', role: 'group', 'aria-label': 'Range' });
  const body = h('div');
  for (const [r, label] of [['today', 'Today'], ['7d', '7 days'], ['30d', '30 days']]) {
    seg.append(h('button', {
      'aria-pressed': String(S.range === r),
      onclick: (ev) => {
        S.range = r;
        for (const b of seg.children) b.setAttribute('aria-pressed', String(b === ev.currentTarget));
        load();
      },
    }, label));
  }
  root.append(h('div', { class: 'view-head' }, h('h1', { text: 'Usage' }), h('span', { class: 'sub', text: 'tokens, time and cost from the agents\' own logs' }), h('span', { class: 'grow' }), seg), body);

  function bars(title, rows) {
    const top = rows.filter((r) => r.tokens > 0 || r.cost > 0).sort((a, b) => b.tokens - a.tokens).slice(0, 8);
    if (!top.length) return null;
    const max = Math.max(...top.map((r) => r.tokens), 1);
    return h('section', { class: 'panel' }, h('h2', { text: title }), h('ul', { class: 'bars' }, top.map((r) => h('li', null,
      h('span', { text: r.label || r.key || '—' }),
      h('span', { class: 'num', text: `${tokens(r.tokens)} · ${money(r.cost)}${r.estimated ? ' est.' : ''}` }),
      h('span', { class: 'bar' }, h('i', { style: { '--w': Math.max(2, Math.round((r.tokens / max) * 100)) + '%' } }))))));
  }
  async function load() {
    fill(body, spinner('Adding it up…'));
    try {
      const [tot, agent, host, model] = await Promise.all([
        api('usage.query', { range: S.range }),
        api('usage.query', { range: S.range, by: 'agent' }),
        api('usage.query', { range: S.range, by: 'host' }),
        api('usage.query', { range: S.range, by: 'model' }),
      ]);
      const t = (tot && tot.rows && tot.rows[0]) || {};
      if (!t.tokens && !t.requests) {
        fill(body, h('div', { class: 'panel empty' }, h('h2', { text: 'No usage in this range' }), 'The daemon reads the agents\' logs now and then (analytics.interval).'));
        return;
      }
      fill(body, 
        h('div', { class: 'stats' },
          stat('Tokens', tokens(t.tokens), `${tokens(t.input)} in · ${tokens(t.output)} out`),
          stat('Cost', money(t.cost), t.estimated ? 'est.' : ''),
          stat('Requests', tokens(t.requests), `${t.sessions || 0} sessions`),
          stat('Active time', dur(t.active_seconds), '')),
        h('div', { class: 'grid two' },
          h('div', null, bars('By agent', (agent && agent.rows) || []), bars('By model', (model && model.rows) || [])),
          h('div', null, bars('By machine', (host && host.rows) || []))));
    } catch (e) {
      fill(body, h('div', { class: 'callout bad' }, h('b', { text: "Couldn't read usage" }), e.message));
    }
  }
  function stat(k, v, note) {
    return h('div', { class: 'stat' }, h('div', { class: 'k', text: k }), h('div', { class: 'v' }, v, note ? h('small', { text: note }) : null));
  }
  load();
  return {};
}

// ------------------------------------------------------------------ this device

const isIOS = () => /iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
const isStandalone = () => (window.matchMedia && matchMedia('(display-mode: standalone)').matches) || navigator.standalone === true;

function b64bytes(s) {
  const pad = '='.repeat((4 - (s.length % 4)) % 4);
  const raw = atob((s + pad).replace(/-/g, '+').replace(/_/g, '/'));
  return Uint8Array.from(raw, (c) => c.charCodeAt(0));
}
function sameKey(buf, b64) {
  if (!buf) return false;
  const a = new Uint8Array(buf), b = b64bytes(b64);
  return a.length === b.length && a.every((x, i) => x === b[i]);
}

/** Why notifications can't work here (null: they can). */
function pushBlocker() {
  if (isIOS() && !isStandalone()) return { title: 'Add Aleph to your Home Screen first', body: 'On iPhone and iPad, web notifications only work from the Home Screen: tap Share, then Add to Home Screen, and open Aleph from there.' };
  if (!window.isSecureContext) return { title: 'Notifications need https', body: 'Open the console through tailscale serve (https://…ts.net) or on this computer (http://127.0.0.1).' };
  if (!('serviceWorker' in navigator) || !('PushManager' in window) || !('Notification' in window)) return { title: "This browser can't do web notifications", body: 'Try Chrome, Edge, Firefox, or Safari 16.4+.' };
  if (Notification.permission === 'denied') return { title: 'Notifications are blocked for this site', body: "Allow them in the browser's site settings, then come back." };
  return null;
}

/** The service worker's registration, active (registered now if need be). */
async function swReg() {
  let reg = await navigator.serviceWorker.getRegistration('/');
  if (!reg) reg = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
  if (!reg.active) {
    await new Promise((res) => {
      const w = reg.installing || reg.waiting;
      if (!w) { res(); return; }
      w.addEventListener('statechange', () => { if (w.state === 'activated') res(); });
      setTimeout(res, 5000);
    });
  }
  return reg;
}

async function pushState() {
  if (pushBlocker()) return 'blocked';
  try {
    const reg = await navigator.serviceWorker.getRegistration('/');
    const sub = reg && (await reg.pushManager.getSubscription());
    return sub ? 'on' : 'off';
  } catch (_) { return 'off'; }
}

async function enablePush() {
  const b = pushBlocker();
  if (b) { toast(b.title + '. ' + b.body, 'err', { sticky: true }); return false; }
  const perm = await Notification.requestPermission();
  if (perm !== 'granted') { toast("Notifications weren't allowed", 'err'); return false; }
  const reg = await swReg();
  const { key } = await get('push/key');
  let sub = await reg.pushManager.getSubscription();
  if (sub && !sameKey(sub.options && sub.options.applicationServerKey, key)) { await sub.unsubscribe(); sub = null; }
  if (!sub) sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64bytes(key) });
  await req('/api/v1/push/subscribe', { method: 'POST', body: sub.toJSON() });
  toast('Notifications are on for this device', 'ok');
  return true;
}

async function disablePush() {
  try {
    const reg = await navigator.serviceWorker.getRegistration('/');
    const sub = reg && (await reg.pushManager.getSubscription());
    if (sub) {
      await req('/api/v1/push/unsubscribe', { method: 'POST', body: { endpoint: sub.endpoint } }).catch(() => {});
      await sub.unsubscribe();
    }
    toast('Notifications are off for this device', 'ok');
  } catch (e) { fail(e); }
}

function Device(root) {
  const me = S.me || {};
  const scopeHelp = { read: 'looks: the fleet, screens, the inbox, usage', reply: 'answers: approve, deny, keys, replies, the queue (where remote.allow says)', control: 'everything: also starts and stops sessions, presets, settings' }[me.scope] || '';
  const push = h('div', null, spinner('Checking notifications…'));
  const installB = h('button', { hidden: !S.install, onclick: async () => { const p = S.install; S.install = null; installB.hidden = true; if (p) { p.prompt(); await p.userChoice; } } }, 'Install Aleph');
  root.append(
    h('div', { class: 'view-head' }, h('h1', { text: 'This device' })),
    h('section', { class: 'panel' },
      h('dl', { class: 'kv' },
        h('dt', { text: 'Name' }), h('dd', { text: me.name || '' }),
        h('dt', { text: 'Scope' }), h('dd', null, h('b', { text: me.scope || '' }), scopeHelp ? ' — ' + scopeHelp : ''),
        h('dt', { text: 'Computer' }), h('dd', { text: me.computer || '' }),
        h('dt', { text: 'Address' }), h('dd', { text: location.origin }),
        h('dt', { text: 'Remote control' }), h('dd', null, me.remote && me.remote.allow ? h('code', { text: me.remote.allow }) : 'off (remote.allow is empty: notifications only)',
          me.remote && me.remote.deny ? [' except ', h('code', { text: me.remote.deny })] : null),
        h('dt', { text: 'Version' }), h('dd', { text: `${me.version || ''} · console ${(document.querySelector('meta[name="aleph-build"]') || {}).content || ''}` }))),
    h('section', { class: 'panel' }, h('h2', { text: 'Notifications' }), push),
    h('section', { class: 'panel' }, h('h2', { text: 'On this device' }),
      h('div', { class: 'actions' }, installB,
        h('button', { class: 'deny', onclick: signOut }, 'Sign out')),
      h('div', { class: 'note' }, 'Signing out forgets the token here. To cut a lost device off, on the computer: ', h('code', { text: `aleph daemon token revoke ${me.name || '<name>'}` }))));

  async function renderPush() {
    const b = pushBlocker();
    const st = await pushState();
    const testB = h('button', { onclick: async (e) => busy(e.currentTarget, async () => {
      try { const r = await api('push/test'); toast(`Sent a test to ${r && r.sent} subscription(s)`, 'ok'); } catch (err) { fail(err); }
    }) }, 'Send a test');
    if (b) {
      fill(push, h('div', { class: 'callout' }, h('b', { text: b.title }), b.body));
      return;
    }
    if (st === 'on') {
      fill(push, h('p', null, h('span', { class: 'badge st-done' }, h('span', { class: 'dot' }), 'On'), ' Questions, finished turns and problems arrive here', rank() >= 1 ? ', with Approve and Deny on questions remote control allows.' : '.'),
        h('div', { class: 'actions' }, testB, h('button', { class: 'ghost', onclick: async (e) => { await busy(e.currentTarget, disablePush); renderPush(); } }, 'Turn off')));
    } else {
      fill(push, h('p', { class: 'muted', text: 'Get a notification when a session needs you, even with the console closed.' }),
        h('div', { class: 'actions' }, h('button', { class: 'primary', onclick: async (e) => { await busy(e.currentTarget, async () => { try { await enablePush(); } catch (err) { fail(err); } }); renderPush(); } }, 'Turn on notifications')));
    }
  }
  renderPush();
  return {};
}

async function signOut() {
  await req('/api/v1/logout', { method: 'POST', quiet: true }).catch(() => {});
  forget();
  showChrome(false);
  location.hash = '#/pair';
  route();
}

function forget() {
  S.me = null;
  S.token = null;
  S.snap = null;
  S.inbox = [];
  ls.set('token', null);
  idbPut('token', null);
  if (S.es) S.es.close();
  S.es = null;
  stopPolling();
}

function signedOut() {
  if (!S.me) return;
  forget();
  showChrome(false);
  toast("This device isn't signed in any more (its token was revoked?). Pair it again.", 'err', { sticky: true });
  route();
}

// ------------------------------------------------------------------ pairing

function fmtCode(c) {
  const x = (c || '').toUpperCase().replace(/[^0-9A-Z]/g, '');
  return x.length > 5 ? x.slice(0, 5) + '-' + x.slice(5, 10) : x;
}
function guessName() {
  const ua = navigator.userAgent.toLowerCase();
  if (/iphone/.test(ua)) return 'iphone';
  if (/ipad/.test(ua) || isIOS()) return 'ipad';
  if (/android/.test(ua)) return 'android';
  if (/macintosh/.test(ua)) return 'mac';
  if (/windows/.test(ua)) return 'windows';
  return 'browser';
}

function Pair(root, code) {
  const codeIn = h('input', { type: 'text', class: 'code', value: fmtCode(code), placeholder: 'ABCDE-FGHJK', autocomplete: 'one-time-code', autocapitalize: 'characters', spellcheck: 'false', maxlength: '14', 'aria-label': 'Pairing code', oninput: () => { codeIn.value = fmtCode(codeIn.value); } });
  const nameIn = h('input', { type: 'text', value: guessName(), autocapitalize: 'none', spellcheck: 'false', maxlength: '32', 'aria-label': 'Name this device' });
  const pairB = h('button', { class: 'primary', type: 'submit' }, 'Pair this device');
  const iosFirst = isIOS() && !isStandalone();
  const form = h('form', {
    onsubmit: async (ev) => {
      ev.preventDefault();
      await busy(pairB, async () => {
        try {
          const r = await req('/api/v1/pair', { method: 'POST', body: { code: codeIn.value, name: nameIn.value.trim().toLowerCase() }, anon: true, quiet: true });
          await signedIn(r.token);
          toast(`Paired as ${r.name} (${r.scope} scope)`, 'ok');
          location.hash = '#/';
        } catch (e) { fail(e); }
      });
    },
  },
  h('label', { class: 'field' }, 'Pairing code', codeIn),
  h('label', { class: 'field' }, 'Name this device', nameIn),
  h('div', { class: 'actions' }, pairB));
  const tokIn = h('input', { type: 'password', placeholder: 'aleph_…', autocomplete: 'off', 'aria-label': 'Device token' });
  const tokB = h('button', { type: 'submit' }, 'Sign in');
  const tokForm = h('form', {
    onsubmit: async (ev) => {
      ev.preventDefault();
      const t = tokIn.value.trim();
      if (!t) return;
      await busy(tokB, async () => {
        S.token = t;
        try {
          await req('/api/v1/login', { method: 'POST', quiet: true });
          await signedIn(t);
          location.hash = '#/';
        } catch (e) { S.token = null; fail(e); }
      });
    },
  }, h('label', { class: 'field' }, 'A token from aleph daemon token new', tokIn), h('div', { class: 'actions' }, tokB));
  root.append(h('div', { class: 'pair' },
    mark('mark hero'),
    h('h1', { text: S.me ? `Paired as ${S.me.name}` : 'Pair this device' }),
    S.me ? h('p', null, h('a', { href: '#/', text: 'Back to the console ›' })) : null,
    h('p', { class: 'muted', text: 'Aleph shows your coding agents on every machine and lets you answer them from here.' }),
    iosFirst ? h('div', { class: 'callout' }, h('b', { text: 'iPhone or iPad? Add to Home Screen first' }), 'Notifications only work from the Home Screen: tap Share → Add to Home Screen, open Aleph from there, and enter the code. It works for 5 minutes, once.') : null,
    h('section', { class: 'panel' }, form),
    code ? null : h('ol', { class: 'steps' },
      h('li', null, 'On the computer that runs Aleph: ', h('code', { text: 'aleph daemon pair' })),
      h('li', null, 'Scan its QR code, or type the code here.')),
    h('details', { class: 'panel' }, h('summary', { text: 'Sign in with a token instead' }), tokForm)));
  if (!code) codeIn.focus();
  return {};
}

async function signedIn(tok) {
  S.token = tok;
  ls.set('token', tok);
  await idbPut('token', tok);
  await boot();
}

/** The HttpOnly cookie the event stream signs in with (from our token). */
async function ensureCookie() {
  if (S.token) await req('/api/v1/login', { method: 'POST', quiet: true }).catch(() => {});
}

// ------------------------------------------------------------------ start

async function registerSW() {
  if (!('serviceWorker' in navigator) || !window.isSecureContext) return;
  try { await navigator.serviceWorker.register('/sw.js', { scope: '/' }); } catch (_) { /* no offline shell */ }
}

async function boot() {
  S.token = ls.get('token');
  try {
    S.me = await req('/api/v1/me', { quiet: true });
  } catch (e) {
    S.me = null;
    if (e.status && e.status !== 401) {
      fill($('#view'), h('div', { class: 'callout bad' }, h('b', { text: "Aleph didn't answer as expected" }), e.message, h('div', { class: 'actions' }, h('button', { onclick: boot }, 'Try again'))));
      return;
    }
    if (e.code === 'offline') {
      fill($('#view'), h('div', { class: 'callout bad' }, h('b', { text: "Can't reach Aleph" }), 'Is the computer awake, and the daemon running with api.http = on?', h('div', { class: 'actions' }, h('button', { onclick: boot }, 'Try again'))));
      return;
    }
  }
  if (!S.me) { showChrome(false); route(); return; }
  showChrome(true);
  $('#where').textContent = S.me.computer ? 'on ' + S.me.computer : '';
  if (S.token) { idbPut('token', S.token); ensureCookie(); }
  refreshState();
  refreshInbox();
  connect();
  route();
}

window.addEventListener('hashchange', route);
window.addEventListener('beforeinstallprompt', (e) => { e.preventDefault(); S.install = e; });
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible' && S.me) {
    if (!S.es || S.es.readyState === EventSource.CLOSED) connect();
    refreshState();
  }
});
document.addEventListener('keydown', (e) => {
  if (e.metaKey || e.ctrlKey || e.altKey || !S.me) return;
  const t = e.target;
  if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
  const to = { 1: '#/', 2: '#/fleet', 3: '#/inbox', 4: '#/presets', 5: '#/usage' }[e.key];
  if (to) { e.preventDefault(); location.hash = to; }
});
if ('serviceWorker' in navigator) {
  navigator.serviceWorker.addEventListener('message', (ev) => {
    if (ev.data && ev.data.go) location.hash = new URL(ev.data.go, location.origin).hash || '#/';
  });
}
registerSW();
boot();
