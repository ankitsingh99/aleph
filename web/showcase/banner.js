/* Showcase mode's ribbon (showcase builds only; the daemon adds this file's
   tag to index.html as it serves it). While the console shows masked data
   or the demo world, a gold "ℵ showcase" / "ℵ demo" chip sits beside the
   brand with a rotating tagline. The mode comes from the "showcase" marker
   the daemon puts on the live state (app.js keeps it in S.snap) and on
   /api/v1/me (S.me). Text only, never markup. */
'use strict';

(() => {
  const LINES = [
    'the point that contains all points',
    "Don't Panic",
    'mostly harmless',
    'Make it so',
    'never tell me the odds',
    'ℵ₀ sessions and counting',
  ];
  const NAMES = { masked: 'showcase', demo: 'demo' };
  const EVERY = 8000;
  // how often /api/v1/me is asked again for the watermark (it can change)
  const MARK_EVERY = 60000;
  let el = null;
  let word = null;
  let tag = null;
  let mark = null;
  let shown = -1;
  // your watermark (showcase.watermark, shown here while watermark_on has
  // web): /api/v1/me says it; null until asked again
  let fresh = null;
  let asked = 0;

  /** app.js's state, when it's loaded. */
  function app() {
    try {
      // eslint-disable-next-line no-undef
      return typeof S === 'object' && S ? S : null;
    } catch (_) {
      return null;
    }
  }

  /** real | masked | demo: the live state says (it always carries the
      marker while on), else what /api/v1/me said. */
  function mode() {
    const s = app();
    if (!s) return 'real';
    if (s.snap && typeof s.snap === 'object') return s.snap.showcase || 'real';
    return (s.me && s.me.showcase) || 'real';
  }

  /** The watermark: the latest answer, else what /api/v1/me said at start. */
  function watermark() {
    if (typeof fresh === 'string') return fresh;
    const s = app();
    const w = s && s.me && s.me.watermark;
    return typeof w === 'string' ? w : '';
  }

  /** Ask /api/v1/me again now and then (app.js's req, with its token). */
  function askAgain() {
    const now = Date.now();
    if (now - asked < MARK_EVERY) return;
    asked = now;
    let ask = null;
    try {
      // eslint-disable-next-line no-undef
      ask = typeof req === 'function' ? req : null;
    } catch (_) {
      ask = null;
    }
    if (!ask) return;
    Promise.resolve(ask('/api/v1/me', { quiet: true }))
      .then((me) => {
        if (me && typeof me === 'object') fresh = typeof me.watermark === 'string' ? me.watermark : '';
      })
      .catch(() => {});
  }

  function build() {
    const brand = document.querySelector('.top .brand');
    if (!brand) return false;
    el = document.createElement('span');
    el.className = 'sc-ribbon';
    el.setAttribute('role', 'status');
    word = document.createElement('b');
    tag = document.createElement('i');
    mark = document.createElement('span');
    mark.className = 'sc-mark';
    el.append(word, tag, mark);
    brand.after(el);
    return true;
  }

  function tick() {
    const m = mode();
    const on = m === 'masked' || m === 'demo';
    document.body.classList.toggle('sc-on', on);
    if (!on) {
      if (el) el.hidden = true;
      return;
    }
    if (!el && !build()) return;
    el.hidden = false;
    el.dataset.mode = m;
    word.textContent = 'ℵ ' + NAMES[m];
    el.title = m === 'demo' ? 'Showcase: the demo world, nothing real' : 'Showcase: names, folders and addresses are space references';
    // your own words, as you wrote them (text only)
    askAgain();
    const w = watermark();
    mark.hidden = !w;
    if (mark.textContent !== (w ? '✦ ' + w : '')) mark.textContent = w ? '✦ ' + w : '';
    const i = Math.floor(Date.now() / EVERY) % LINES.length;
    if (i === shown) return;
    if (shown < 0) {
      tag.textContent = LINES[i];
    } else {
      tag.classList.add('out');
      setTimeout(() => {
        tag.textContent = LINES[i];
        tag.classList.remove('out');
      }, 350);
    }
    shown = i;
  }

  tick();
  setInterval(tick, 1000);
})();

/* ---- safety nets: what the daemon's "showcase_safety" marker says (on the
   live state and /api/v1/me), followed here:
   * paused (the panic key, `aleph showcase panic`): a starfield and the ℵ
     over the whole console ("be right back"); the data underneath is blank
     anyway. When it ends, the view is drawn again with real data.
   * money: hidden → every $ amount reads $— (scaled ones come scaled).
   * clipboard: off → the Copy buttons copy nothing and say so. */
(() => {
  const SVGNS = 'http://www.w3.org/2000/svg';
  const STARS = ['·', '✦', '˚', '⋆', '∙'];
  let card = null;
  let was = false;

  /** The marker: the live state's, else /me's (null: nothing to follow). */
  function safety() {
    let s = null;
    try {
      // eslint-disable-next-line no-undef
      s = typeof S === 'object' && S ? S : null;
    } catch (_) {
      s = null;
    }
    if (!s) return null;
    if (s.snap && typeof s.snap === 'object' && s.snap.showcase_safety) return s.snap.showcase_safety;
    return (s.me && s.me.showcase_safety) || null;
  }

  /** A star's place, the same on every visit (a small hash). */
  function spot(i, k) {
    let z = (i + 1) * 2654435761 + k * 40503;
    z = (z ^ (z >>> 13)) * 1274126177;
    return ((z ^ (z >>> 16)) >>> 0) / 4294967296;
  }

  function build() {
    card = document.createElement('div');
    card.className = 'sc-pause';
    card.setAttribute('role', 'status');
    card.setAttribute('aria-live', 'polite');
    const sky = document.createElement('div');
    sky.className = 'sc-sky';
    sky.setAttribute('aria-hidden', 'true');
    for (let i = 0; i < 90; i++) {
      const star = document.createElement('i');
      star.textContent = STARS[i % STARS.length];
      star.style.left = (spot(i, 1) * 100).toFixed(2) + '%';
      star.style.top = (spot(i, 2) * 100).toFixed(2) + '%';
      star.style.animationDelay = (spot(i, 3) * 6).toFixed(2) + 's';
      if (i % 11 === 0) star.className = 'gold';
      sky.append(star);
    }
    const mark = document.createElementNS(SVGNS, 'svg');
    mark.setAttribute('viewBox', '0 0 100 100');
    mark.setAttribute('class', 'sc-mark');
    mark.setAttribute('aria-hidden', 'true');
    const use = document.createElementNS(SVGNS, 'use');
    use.setAttribute('href', '#mark');
    mark.append(use);
    const words = document.createElement('p');
    words.textContent = 'be right back';
    card.append(sky, mark, words);
    document.body.append(card);
  }

  // money: app.js's formatter, wrapped (a hidden amount reads $—), asked
  // at every call so a view drawn before the marker came is right too
  const realMoney = typeof window.money === 'function' ? window.money : null;
  if (realMoney) {
    window.money = (x) => {
      const s = safety();
      return s && s.money === 'hidden' ? '$—' : realMoney(x);
    };
  }

  // the clipboard: Copy buttons copy nothing while copies are off
  const clip = navigator.clipboard;
  const realWrite = clip && typeof clip.writeText === 'function' ? clip.writeText.bind(clip) : null;
  if (realWrite) {
    try {
      Object.defineProperty(clip, 'writeText', {
        configurable: true,
        writable: true,
        value: (text) => {
          const s = safety();
          if (s && (s.clipboard === 'off' || s.paused)) {
            // eslint-disable-next-line no-undef
            if (typeof toast === 'function') toast('Copying is off for the demo (showcase.clipboard)', 'err');
            return new Promise(() => {});
          }
          return realWrite(text);
        },
      });
    } catch (_) {
      /* a browser that won't: copies go as they are (the text is masked) */
    }
  }

  function tick() {
    const s = safety();
    const paused = !!(s && s.paused);
    if (paused && !card) build();
    if (card) card.hidden = !paused;
    document.body.classList.toggle('sc-paused', paused);
    if (was && !paused) {
      // back: the view drawn again from real data
      // eslint-disable-next-line no-undef
      if (typeof route === 'function') route();
    }
    was = paused;
  }

  tick();
  setInterval(tick, 500);
})();

/* ---- presenter sync (showcase.sync): the console follows the dashboard you
   present from. The daemon says what that dashboard shows (presenter.get,
   and the "presenter" topic on an event stream of its own); this page goes
   to the same place — Focus or Tile: the session's page; Monitor › Usage:
   Usage; the Inbox; anything else: the Fleet — and marks the presenter's
   machine and session there. Names come masked, as everything else here.
   "▶ following" in the ribbon says so; a click on it stops (and starts)
   following on this page. Text only, never markup. */
(() => {
  // a fallback look when the stream can't open (too many streams, an old daemon)
  const POLL = 5000;
  let es = null;
  let pres = null;
  let applied = null;
  let off = false;
  let polled = 0;
  let chip = null;

  function app() {
    try {
      // eslint-disable-next-line no-undef
      return typeof S === 'object' && S ? S : null;
    } catch (_) {
      return null;
    }
  }

  /** Is showcase mode on (the live state's marker, else /me's)? */
  function showcase() {
    const s = app();
    const m = s && ((s.snap && s.snap.showcase) || (s.me && s.me.showcase));
    return m === 'masked' || m === 'demo';
  }

  /** `host:name` as the console's routes write it. */
  function key(p) {
    const id = p.host + ':' + p.name;
    try {
      // eslint-disable-next-line no-undef
      return typeof enc === 'function' ? enc(id) : encodeURIComponent(id);
    } catch (_) {
      return encodeURIComponent(id);
    }
  }

  /** Where the presenter is, as this console's route. */
  function where(p) {
    const s = p.host && p.name ? '#/s/' + key(p) : null;
    if ((p.view === 'focus' || p.view === 'tile') && s) return s;
    if (p.view === 'usage') return '#/usage';
    if (p.view === 'inbox') return '#/inbox';
    return '#/fleet';
  }

  async function look() {
    let ask = null;
    try {
      // eslint-disable-next-line no-undef
      ask = typeof get === 'function' ? get : null;
    } catch (_) {
      ask = null;
    }
    if (!ask) return;
    try {
      pres = await ask('presenter.get');
    } catch (_) {
      pres = null;
    }
    follow();
  }

  function stream() {
    if (es || !('EventSource' in window)) return;
    const q = encodeURIComponent(JSON.stringify({ state: false, kinds: [], topics: ['presenter'] }));
    const s = new EventSource('/api/v1/events?params=' + q);
    es = s;
    s.addEventListener('open', look);
    s.addEventListener('changed', (ev) => {
      try {
        if (JSON.parse(ev.data).topic === 'presenter') look();
      } catch (_) {
        /* a line we don't understand */
      }
    });
    s.addEventListener('error', () => {
      if (s.readyState === EventSource.CLOSED && es === s) es = null;
    });
  }

  function close() {
    if (es) es.close();
    es = null;
  }

  /** Go where the presenter went (once per move: you may look around in between). */
  function follow() {
    const p = pres;
    if (!p || !p.on || off) {
      mark();
      return;
    }
    if (applied !== p.gen) {
      applied = p.gen;
      const to = where(p);
      if (location.hash !== to) location.hash = to;
      setTimeout(() => mark(true), 120);
    }
    mark();
  }

  /** The presenter's session and machine, marked where this view lists them. */
  function mark(scroll) {
    for (const el of document.querySelectorAll('.sc-presented')) el.classList.remove('sc-presented');
    const p = pres;
    if (!p || !p.on || off || !p.host) return;
    let first = null;
    if (p.name) {
      const href = '#/s/' + key(p);
      for (const a of document.querySelectorAll('a.sess, a.card-name')) {
        if (a.getAttribute('href') !== href) continue;
        a.classList.add('sc-presented');
        first = first || a;
      }
    }
    for (const n of document.querySelectorAll('.host-head .name')) {
      if (n.textContent !== p.host) continue;
      n.closest('.host-head').classList.add('sc-presented');
      first = first || n;
    }
    if (scroll && first && first.scrollIntoView) first.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }

  /** `▶ following` in the ribbon while there's a presenter (a click: stop / start). */
  function badge() {
    const ribbon = document.querySelector('.sc-ribbon');
    const p = pres;
    const show = !!(ribbon && p && p.on);
    if (!show) {
      if (chip) chip.hidden = true;
      return;
    }
    if (!chip || !ribbon.contains(chip)) {
      chip = document.createElement('button');
      chip.type = 'button';
      chip.className = 'sc-follow';
      chip.addEventListener('click', () => {
        off = !off;
        applied = null;
        follow();
        badge();
      });
      ribbon.append(chip);
    }
    chip.hidden = false;
    chip.classList.toggle('off', off);
    chip.textContent = off ? '▷ follow the presenter' : '▶ following the presenter';
    chip.title = off ? 'Go where the presenter goes again' : 'Stop following the presenter on this page';
  }

  function tick() {
    const s = app();
    if (!showcase() || !s || !s.me) {
      close();
      pres = null;
      applied = null;
      badge();
      mark();
      return;
    }
    stream();
    if (!es && Date.now() - polled > POLL) {
      polled = Date.now();
      look();
    }
    badge();
    mark();
  }

  setInterval(tick, 1000);
})();
