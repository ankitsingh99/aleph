// The Daemon command, drawn: the daemon's state at a glance as SVG cards for
// a Detail's full width — a status card (running or not in its state colour,
// uptime, version, the hub, the service's health, and the one thing to do
// when something needs doing), the machines it watches with their links, what
// it did today (events, auto-approvals, the queue, when usage was last read)
// and remote access as chips (the HTTP API, Tailscale, Cloudflare, paired
// devices) with the keys that change them. From `daemon.status`, `aleph
// daemon status --json`, the fleet snapshot, the inbox, the queue, `usage
// today --json`, `daemon token ls --json` and the settings — nothing guessed.
// No Raycast imports: plain node tests it.
import type { DaemonStatus, Device, Host, InboxEvent, QueueTask, UsageReport } from "./cli";
import { ChartTheme, clip, esc, fit, svgDoc, text, textWidth, Tile, tiles } from "./charts";
import { ChipGroup, chips, logoMark, signalBars, sparkline, stateMark } from "./dash";
import { since } from "./fmt";
import { osId } from "./logo-ids";
import { explainError, fleetLink, hostLink, Tone } from "./model";
import { bars, LinkSample, seriesOf } from "./samples";
import { img, LogoFn } from "./stats-view";

/** The Detail's markdown width without a metadata column, about (Raycast scales a wider image down). */
export const WIDE = 680;

const r1 = (v: number) => Math.round(v * 10) / 10;

// ---------------------------------------------------------------- what the answers say

export type DaemonState = "running" | "standin" | "stopped" | "checking" | "missing";

/** What the daemon is doing, from both answers (the socket's wins where they overlap). */
export interface DaemonView {
  /** running · a dashboard standing in · stopped · no answer yet · this aleph has no daemon */
  state: DaemonState;
  running: boolean;
  s: Partial<DaemonStatus>;
  /** a service manager runs it (launchd / systemd) */
  installed: boolean;
  /** where the HTTP API listens now (null: it doesn't) */
  listening: string | null;
  /** `api.http` is on in the settings */
  httpOn: boolean;
}

const settingOf = (s: Record<string, unknown> | undefined, key: string): string => {
  const v = s?.[key];
  return typeof v === "string" ? v : v == null ? "" : String(v);
};
const truthy = (v: string) => v === "on" || v === "true" || v === "1" || v === "yes";

export function daemonView(
  live: DaemonStatus | undefined,
  details: DaemonStatus | undefined,
  settings?: Record<string, unknown>,
  o: { loading?: boolean; unsupported?: boolean } = {},
): DaemonView {
  const s: Partial<DaemonStatus> = { ...(details ?? {}), ...(live ?? {}) };
  const running = !!live || !!details?.running;
  const state: DaemonState = o.unsupported
    ? "missing"
    : running
      ? s.kind === "dashboard"
        ? "standin"
        : "running"
      : !live && !details && o.loading
        ? "checking"
        : "stopped";
  return {
    state,
    running,
    s,
    installed: !!details?.service?.installed,
    listening: live?.http ?? (details?.running ? (details.http ?? null) : null),
    httpOn: details?.api?.http ?? truthy(settingOf(settings, "api.http")),
  };
}

/** How long it's been up: `40s`, `13m`, `1h 13m`, `3d 4h`. */
export function uptimeText(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs)) return "–";
  const d = Math.max(0, Math.floor(secs));
  if (d < 60) return `${d}s`;
  if (d < 3600) return `${Math.floor(d / 60)}m`;
  if (d < 86400) {
    const m = Math.floor((d % 3600) / 60);
    return `${Math.floor(d / 3600)}h${m ? ` ${m}m` : ""}`;
  }
  const h = Math.floor((d % 86400) / 3600);
  return `${Math.floor(d / 86400)}d${h ? ` ${h}h` : ""}`;
}

const SIGNALS: Record<number, [string, string, boolean]> = {
  1: ["SIGHUP", "hung up", true],
  2: ["SIGINT", "interrupted", true],
  4: ["SIGILL", "crashed", false],
  6: ["SIGABRT", "crashed", false],
  8: ["SIGFPE", "crashed", false],
  9: ["SIGKILL", "was killed", false],
  10: ["SIGBUS", "crashed", false],
  11: ["SIGSEGV", "crashed", false],
  15: ["SIGTERM", "was stopped", true],
};

/** systemd's `Result=` words. */
const RESULTS: Record<string, string> = {
  "exit-code": "last run exited with an error",
  signal: "last run was killed by a signal",
  "core-dump": "last run crashed (core dump)",
  timeout: "last run timed out",
  watchdog: "last run hung (watchdog)",
  "start-limit-hit": "restarted too often: systemd gave up",
  resources: "couldn't start (out of resources)",
  "oom-kill": "last run ran out of memory",
};

/**
 * The service's last exit in words: launchd's code (`143` is 128 + SIGTERM,
 * a normal stop), or systemd's result. `normal`: a stop, not a failure.
 * Undefined when it never exited.
 */
export function lastExit(code: string | number | null | undefined): { text: string; normal: boolean } | undefined {
  if (code == null) return undefined;
  const s = String(code).trim();
  if (!s || /never/i.test(s) || s === "success") return undefined;
  const m = /^-?\d+/.exec(s);
  if (!m) return { text: RESULTS[s] ?? `last run: ${s}`, normal: false };
  const n = Number(m[0]);
  if (n === 0) return { text: "last run exited cleanly", normal: true };
  const sig = n < 0 ? -n : n > 128 && n < 160 ? n - 128 : 0;
  const known = SIGNALS[sig];
  if (known) return { text: `last run ${known[1]} (${known[0]})`, normal: known[2] };
  if (sig) return { text: `last run ended by signal ${sig}`, normal: false };
  return { text: `last run failed (exit ${n})`, normal: false };
}

/** What to do next, when something needs doing: the one line under the status, and the primary action. */
export interface NextStep {
  text: string;
  /** the primary action (↵) */
  op?: "start" | "install" | "restart";
  verb?: string;
}

export function nextStep(v: DaemonView): NextStep | undefined {
  const s = v.s;
  if (v.state === "missing") return { text: "This aleph has no daemon: aleph self-update brings it" };
  if (v.state === "checking") return undefined;
  if (v.state === "stopped")
    return v.installed
      ? { text: "Nothing watches your machines: no alerts, queue or auto-approve", op: "start", verb: "start" }
      : { text: "Nothing watches your machines: no alerts, queue or auto-approve", op: "install", verb: "install" };
  if (v.state === "standin")
    return { text: "An open dashboard serves until you close it", op: "install", verb: "install the daemon" };
  if (s.version_match === false && s.binary)
    return {
      text: `It still runs Aleph ${s.version}; this aleph is ${s.binary.version}`,
      // started by hand, no service restarts it: the line only
      op: v.installed ? "restart" : undefined,
      verb: "restart",
    };
  if (v.httpOn && !v.listening)
    return {
      text: "api.http is on, but the API isn't listening yet",
      op: v.installed ? "restart" : undefined,
      verb: "restart",
    };
  if (s.service?.linger === false) return { text: "Linger is off: it stops when you log out (loginctl enable-linger)" };
  return undefined;
}

// ---------------------------------------------------------------- small pieces

/** A keycap (`⇧⌘P`) at (x, baseline y): the SVG and its width. */
function keycap(x: number, y: number, key: string, t: ChartTheme): [string, number] {
  const w = Math.max(20, textWidth(key, 11) + 12);
  return [
    `<rect x="${r1(x)}" y="${r1(y - 14)}" width="${r1(w)}" height="19" rx="5" fill="${t.grid}" fill-opacity="${t.dark ? 0.7 : 0.75}"/>` +
      `<rect x="${r1(x)}" y="${r1(y + 3)}" width="${r1(w)}" height="2" rx="1" fill="${t.grid}"/>` +
      text(x + w / 2, y, key, t.text, 11, ' text-anchor="middle" font-weight="600"'),
    w,
  ];
}

export interface KeyHint {
  key: string;
  label: string;
}

/** Keycap hints in a row (`⇧⌘P pair a device`), like the dashboard's footer. */
export function keyHints(list: KeyHint[], t: ChartTheme, width = WIDE): string {
  const parts: string[] = [];
  let x = 0;
  let y = 16;
  for (const h of list) {
    const w = Math.max(20, textWidth(h.key, 11) + 12) + 6 + textWidth(h.label, 12);
    if (x > 0 && x + w > width) {
      x = 0;
      y += 26;
    }
    const [cap, cw] = keycap(x, y, h.key, t);
    parts.push(cap, text(x + cw + 6, y, h.label, t.muted, 12));
    x += w + 18;
  }
  return svgDoc(width, list.length ? y + 7 : 1, parts.join(""), list.map((h) => `${h.key} ${h.label}`).join(", "));
}

/** A section's title (`Watching`) with a muted note after it. */
export function heading(title: string, note: string | undefined, t: ChartTheme, width = WIDE): string {
  const room = width - textWidth(title, 13) * 1.05 - 8;
  return svgDoc(
    width,
    20,
    `<text x="0" y="14"><tspan fill="${t.text}" font-size="13" font-weight="700">${esc(title)}</tspan>${
      note ? `<tspan dx="8" fill="${t.muted}" font-size="12">${esc(clip(note, room, 12))}</tspan>` : ""
    }</text>`,
    note ? `${title}: ${note}` : title,
  );
}

/** SVG documents one under the other, as one (each keeps its own drawing; `gap` px between). */
export function stack(docs: string[], width = WIDE, gap = 10): string {
  let y = 0;
  const body: string[] = [];
  const labels: string[] = [];
  for (const d of docs.filter(Boolean)) {
    const h = Number(/^<svg [^>]*?\bheight="(\d+(?:\.\d+)?)"/.exec(d)?.[1] ?? 0);
    const label = /aria-label="([^"]*)"/.exec(d)?.[1];
    if (label) labels.push(label);
    if (y > 0) y += gap;
    body.push(d.replace(/^<svg /, `<svg x="0" y="${r1(y)}" `));
    y += h;
  }
  // the labels are escaped already: put them back before svgDoc escapes them again
  const unesc = (s: string) =>
    s.replace(
      /&(amp|lt|gt|quot|#39);/g,
      (_, e) => ({ amp: "&", lt: "<", gt: ">", quot: '"', "#39": "'" })[e as string] ?? _,
    );
  return svgDoc(width, Math.max(1, y), body.join(""), labels.map(unesc).join("; "));
}

// ---------------------------------------------------------------- the status card

export interface Badge {
  text: string;
  /** muted, after it */
  note?: string;
  color: string;
  /** a state mark (dash.ts `stateMark`): done · warn · down · idle · … */
  mark?: string;
  /** or an embedded logo (launchd: Apple's, systemd: Linux's) */
  logo?: string;
  /** or a glyph (the hub's ℵ) */
  glyph?: string;
}

export interface StatusCard {
  state: DaemonState;
  title: string;
  subtitle?: string;
  /** a figure on the right: `1h 13m` · `uptime` */
  aside?: { value: string; label: string };
  badges: Badge[];
  /** the one thing to do, with its key */
  step?: NextStep;
}

/** The state's colour: running green, a dashboard standing in yellow, stopped red, the rest grey. */
export function stateColor(state: DaemonState, t: ChartTheme): string {
  return state === "running"
    ? t.state.done
    : state === "standin"
      ? t.state.warn
      : state === "stopped"
        ? t.state.down
        : t.other;
}

/** Pills in a row from (0, y), wrapping: [mark · text · note], each tinted with its colour. */
function badgeRow(list: Badge[], y0: number, t: ChartTheme, width: number): [string, number] {
  const h = 26;
  const parts: string[] = [];
  let x = 0;
  let y = y0;
  for (const b of list) {
    const lead = b.logo || b.mark || b.glyph ? 22 : 0;
    const w = 12 + lead + textWidth(b.text, 12) * 1.05 + (b.note ? 6 + textWidth(b.note, 12) : 0) + 12;
    if (x > 0 && x + w > width) {
      x = 0;
      y += h + 8;
    }
    parts.push(
      `<rect x="${r1(x)}" y="${y}" width="${r1(w)}" height="${h}" rx="${h / 2}" fill="${b.color}" fill-opacity="${t.dark ? 0.16 : 0.12}"/>`,
    );
    const cx = x + 12;
    if (b.logo) parts.push(logoMark(cx, y + 5, 16, b.logo, {}, t));
    else if (b.glyph)
      parts.push(text(cx + 7, y + 18.5, b.glyph, b.color, 15, ' text-anchor="middle" font-weight="700"'));
    else if (b.mark) parts.push(stateMark(b.mark, cx + 7, y + h / 2, b.color));
    parts.push(
      `<text x="${r1(cx + lead)}" y="${y + 17.5}"><tspan fill="${t.text}" font-size="12" font-weight="600">${esc(b.text)}</tspan>${
        b.note ? `<tspan dx="6" fill="${t.muted}" font-size="12">${esc(b.note)}</tspan>` : ""
      }</text>`,
    );
    x += w + 8;
  }
  return [parts.join(""), list.length ? y + h : y0 - 12];
}

/** The card the view opens with: ℵ in the state's colour, the state, what runs, uptime on the right, the badges, the next step. */
export function statusCard(c: StatusCard, t: ChartTheme, width = WIDE): string {
  const color = stateColor(c.state, t);
  const parts: string[] = [];
  const tile = 56;
  parts.push(
    `<rect x="0" y="0" width="${tile}" height="${tile}" rx="14" fill="${color}" fill-opacity="${t.dark ? 0.18 : 0.14}"/>`,
    `<rect x="16" y="${tile - 4}" width="${tile - 32}" height="3" rx="1.5" fill="${color}"/>`,
    text(tile / 2, 38, "ℵ", color, 30, ' text-anchor="middle" font-weight="700"'),
  );
  // a live dot on the tile's corner while it runs
  if (c.state === "running" || c.state === "standin")
    parts.push(
      `<circle cx="${tile - 3}" cy="5" r="6.5" fill="${color}" fill-opacity="0.25"/><circle cx="${tile - 3}" cy="5" r="4" fill="${color}"/>`,
    );
  const x0 = tile + 16;
  const right = c.aside ? width - Math.max(90, textWidth(c.aside.value, 22) + 16) : width;
  parts.push(text(x0, 26, clip(c.title, right - x0, 22), color, 22, ' font-weight="700"'));
  if (c.subtitle) parts.push(text(x0, 47, clip(c.subtitle, right - x0, 12), t.muted, 12));
  if (c.aside) {
    parts.push(text(width, 26, c.aside.value, t.text, 22, ' text-anchor="end" font-weight="600"'));
    parts.push(text(width, 45, c.aside.label, t.muted, 11, ' text-anchor="end"'));
  }
  const [row, rowEnd] = badgeRow(c.badges, tile + 14, t, width);
  parts.push(row);
  let y = rowEnd;
  if (c.step) {
    const by = y + 26;
    parts.push(stateMark("warn", 7, by - 4.5, t.state.warn));
    const msg = c.step.text;
    const keyW = c.step.op ? 26 + 6 + textWidth(c.step.verb ?? "", 12) + 16 : 0;
    const s = clip(msg, width - 20 - keyW, 12);
    parts.push(text(20, by, s, t.text, 12));
    if (c.step.op) {
      const kx = 20 + textWidth(s, 12) + 14;
      const [cap, cw] = keycap(kx, by, "↵", t);
      parts.push(cap, text(kx + cw + 6, by, c.step.verb ?? "", t.muted, 12));
    }
    y = by + 7;
  }
  return svgDoc(
    width,
    Math.max(tile + 2, y + 2),
    parts.join(""),
    [
      c.title,
      c.subtitle,
      c.aside ? `${c.aside.label} ${c.aside.value}` : "",
      ...c.badges.map((b) => b.text),
      c.step?.text,
    ]
      .filter(Boolean)
      .join(", "),
  );
}

/** The status card's contents from what the daemon says. `logo`: launchd and systemd as Apple's and Linux's. */
export function statusOf(v: DaemonView, logo?: LogoFn): StatusCard {
  const s = v.s;
  const svc = s.service;
  const step = nextStep(v);
  const title = {
    running: "Running",
    standin: "A dashboard stands in",
    stopped: "Not running",
    checking: "Checking…",
    missing: "No daemon here",
  }[v.state];
  const version = s.version ?? s.binary?.version;
  const subtitle =
    v.state === "running" || v.state === "standin"
      ? [version ? `Aleph ${version}` : "", s.pid ? `pid ${s.pid}` : ""]
      : v.state === "stopped"
        ? [version ? `Aleph ${version}` : "", svc?.installed ? `${svc.name}` : "not installed as a service"]
        : [];
  return {
    state: v.state,
    title,
    subtitle: subtitle.filter(Boolean).join(" · ") || undefined,
    aside: v.running && s.uptime_s != null ? { value: uptimeText(s.uptime_s), label: "uptime" } : undefined,
    badges: badgesOf(v, logo),
    step,
  };
}

/** The service, the hub, a version that differs, a last run that failed: what the badges say. */
export function badgesOf(v: DaemonView, logo?: LogoFn): Badge[] {
  // the colours are filled in by `themed` (here: names)
  const s = v.s;
  const out: Badge[] = [];
  const svc = s.service;
  if (svc && svc.manager && svc.manager !== "none") {
    const mgr = svc.manager.split(" ")[0];
    const os = logo?.("os", mgr === "launchd" ? "apple" : "linux");
    if (!svc.installed)
      out.push({
        text: mgr,
        note: v.running ? "not installed · started by hand" : "not installed",
        color: "other",
        ...(os ? { logo: os } : { mark: "idle" }),
      });
    else
      out.push({
        text: mgr,
        note: svc.running ? "running · at login" : svc.loaded ? "loaded, not running" : "not loaded",
        color: svc.running ? "done" : "warn",
        ...(os ? { logo: os } : { mark: svc.running ? "done" : "warn" }),
      });
  }
  if (v.running && s.hub !== undefined) {
    const where = s.hub_machine && !s.hub_machine.here ? s.hub_machine.setting : "";
    out.push(
      s.hub
        ? { text: "hub", note: "runs here", color: "accent", glyph: "ℵ" }
        : {
            text: "hub",
            note: where ? `on ${where}` : s.hub_holder ? `pid ${s.hub_holder}` : "elsewhere",
            color: "other",
            glyph: "ℵ",
          },
    );
  }
  // a version that differs is the next step's line, not a badge too
  if (v.running && s.version_match === true) out.push({ text: "up to date", color: "done", mark: "done" });
  const exit = lastExit(svc?.last_exit);
  if (exit && !exit.normal) out.push({ text: exit.text, color: "warn", mark: "warn" });
  if (svc?.linger === false) out.push({ text: "linger off", note: "stops at logout", color: "warn", mark: "warn" });
  return out;
}

/** Colour names (done · warn · down · accent · other) → this theme's colours. */
export function themed(list: Badge[], t: ChartTheme): Badge[] {
  const pick = (c: string) =>
    c === "accent"
      ? t.accent
      : c === "other"
        ? t.other
        : c in t.state
          ? t.state[c as keyof ChartTheme["state"]]
          : fit(c, t);
  return list.map((b) => ({ ...b, color: pick(b.color) }));
}

// ---------------------------------------------------------------- the machines it watches

export interface Watched {
  alias: string;
  color?: string;
  logo?: string;
  /** its link: up · slow · down · stopped · unknown */
  state: string;
  tone: Tone;
  /** 0..4 */
  bars: number;
  /** `199 ms`, `offline`, `herdr stopped` */
  link: string;
  /** why it's offline: `ssh timed out` */
  why?: string;
  sessions: number;
  needs: number;
  /** latency readings, oldest first */
  history?: [number, number | null][];
}

const toneColor = (tone: Tone, t: ChartTheme) =>
  tone === "good" ? t.state.done : tone === "warn" ? t.state.warn : tone === "bad" ? t.state.down : t.other;

/** Each machine the daemon watches, from the fleet snapshot (and the link history kept by Machines). */
export function watchedOf(
  hosts: Host[] | undefined,
  names: string[] | undefined,
  o: { hist?: Record<string, LinkSample[]>; logo?: LogoFn; osText?: (alias: string) => string | undefined } = {},
): Watched[] {
  const byName = new Map((hosts ?? []).map((h) => [h.alias, h]));
  const order = hosts?.length ? hosts.map((h) => h.alias) : (names ?? []);
  return order.map((alias) => {
    const h = byName.get(alias);
    const os = osId(o.osText?.(alias));
    const logo = os ? o.logo?.("os", os.id) : undefined;
    if (!h)
      return { alias, logo, state: "unknown", tone: "muted", bars: 0, link: "not polled yet", sessions: 0, needs: 0 };
    const l = hostLink(h);
    const history = seriesOf(o.hist?.[alias] ?? [], "ms");
    return {
      alias,
      color: h.color,
      logo,
      state: l.state,
      tone: l.tone,
      bars: l.state === "down" ? 0 : l.state === "stopped" ? 1 : l.state === "unknown" ? 0 : bars(h.latency_ms, true),
      link:
        l.state === "down"
          ? "offline"
          : l.state === "stopped"
            ? "herdr stopped"
            : l.state === "unknown"
              ? "not polled yet"
              : h.latency_ms != null
                ? `${h.latency_ms} ms`
                : "online",
      why: l.state === "down" ? explainError(h.error).head.replace(/^unreachable — /, "") : undefined,
      sessions: h.sessions.length,
      needs: h.sessions.filter((x) => x.status === "blocked").length,
      history: history.filter((p) => p[1] != null).length >= 2 ? history : undefined,
    };
  });
}

/** The machines as tiles: OS logo, name, signal bars; the link, sessions, who needs you and a latency sparkline. */
export function watchTiles(rows: Watched[], t: ChartTheme, width = WIDE): string {
  const n = rows.length;
  if (!n) return svgDoc(width, 20, text(0, 14, "No machines yet: aleph setup adds them", t.muted, 12), "no machines");
  const cols = n <= 4 ? Math.max(3, n) : 3;
  const gap = 10;
  const w = (width - gap * (cols - 1)) / cols;
  const h = 60;
  const parts: string[] = [];
  rows.forEach((r, i) => {
    const x = (i % cols) * (w + gap);
    const y = Math.floor(i / cols) * (h + gap);
    const tone = toneColor(r.tone, t);
    const own = r.color ? fit(r.color, t) : t.other;
    parts.push(
      `<rect x="${r1(x + 0.5)}" y="${y + 0.5}" width="${r1(w - 1)}" height="${h - 1}" rx="10" fill="${r.state === "down" ? t.state.down : "none"}" fill-opacity="${r.state === "down" ? 0.08 : 0}" stroke="${r.state === "down" ? t.state.down : t.grid}" stroke-opacity="${r.state === "down" ? 0.6 : 1}"/>`,
      `<rect x="${r1(x + 10)}" y="${y + 13}" width="3" height="${h - 26}" rx="1.5" fill="${own}"/>`,
    );
    parts.push(logoMark(x + 21, y + 11, 18, r.logo, { letter: r.alias, color: r.color ?? t.other }, t));
    const markW = r.bars > 0 ? 22 : 12;
    // who needs you: a pill by the name (▲ and the count, in needs-you orange)
    const count = r.state !== "down" && r.needs ? String(r.needs) : "";
    const pillW = count ? 26 + textWidth(count, 12) * 1.05 : 0;
    const alias = clip(r.alias, w - 46 - markW - 18 - (count ? pillW + 8 : 0), 14);
    if (count) {
      const px = x + 46 + textWidth(alias, 14) * 1.05 + 8;
      parts.push(
        `<g><title>${esc(`${r.needs} need${r.needs === 1 ? "s" : ""} you`)}</title><rect x="${r1(px)}" y="${y + 11}" width="${r1(pillW)}" height="19" rx="9.5" fill="${t.state.blocked}" fill-opacity="${t.dark ? 0.2 : 0.14}"/>` +
          stateMark("blocked", px + 11, y + 21, t.state.blocked) +
          text(px + 20, y + 25, count, t.state.blocked, 12, ' font-weight="700"') +
          "</g>",
      );
    }
    parts.push(
      text(x + 46, y + 25, alias, t.text, 14, ' font-weight="700"'),
      r.bars > 0
        ? signalBars(x + w - 12 - 22, y + 26, r.bars, tone, t)
        : stateMark(
            r.state === "unknown" ? "idle" : "down",
            x + w - 18,
            y + 19,
            r.state === "unknown" ? t.other : tone,
          ),
    );
    // the second line: the link in its colour, the sessions (or why it's offline), the latency's sparkline
    const spark = r.history && w >= 190 ? 64 : 0;
    const room = w - 21 - 12 - (spark ? spark + 8 : 0);
    const bits: string[] = [];
    let used = 0;
    const add = (s: string, fill: string, weight = "") => {
      const piece = clip(s, room - used, 12);
      if (used >= room - 8 || !piece) return;
      bits.push(`<tspan${bits.length ? ' dx="6"' : ""} fill="${fill}" font-size="12"${weight}>${esc(piece)}</tspan>`);
      used += textWidth(piece, 12) + 6;
    };
    add(r.link, r.state === "up" ? t.text : tone, ' font-weight="600"');
    if (r.state === "down" && r.why) add(r.why, t.muted);
    if (r.state !== "down") add(`${r.sessions} session${r.sessions === 1 ? "" : "s"}`, t.muted);
    parts.push(`<text x="${r1(x + 21)}" y="${y + 46}">${bits.join("")}</text>`);
    if (spark && r.history) {
      const pts = r.history;
      const from = pts[0][0];
      const to = pts[pts.length - 1][0];
      if (to > from)
        parts.push(
          sparkline(pts, { x: x + w - 12 - spark, y: y + 34, w: spark, h: 14, from, to }, tone, t, { dot: false }),
        );
    }
  });
  const rowsN = Math.ceil(n / cols);
  return svgDoc(
    width,
    rowsN * h + (rowsN - 1) * gap,
    parts.join(""),
    rows.map((r) => `${r.alias}: ${r.link}`).join(", "),
  );
}

// ---------------------------------------------------------------- today

/** `usage today --by total --json`: what it sums, and when each machine's logs were last read. */
export type UsageToday = UsageReport & { total?: { cost?: number; tokens?: number } };

/** Local midnight before `now` (unix seconds). */
export function midnight(now: number): number {
  const d = new Date(now * 1000);
  d.setHours(0, 0, 0, 0);
  return d.getTime() / 1000;
}

/** `950`, `12.4K`, `51M`, `1.2B`. */
export function compact(n: number): string {
  const a = Math.abs(n);
  const f = (v: number, u: string) => `${v >= 10 ? Math.round(v) : Math.round(v * 10) / 10}${u}`;
  return a >= 1e9 ? f(n / 1e9, "B") : a >= 1e6 ? f(n / 1e6, "M") : a >= 1e3 ? f(n / 1e3, "K") : String(Math.round(n));
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** Where alerts go (the settings): `ntfy · Raycast · Moshi`, `off`, or `do not disturb`. */
export function alertSinks(settings: Record<string, unknown> | undefined): string {
  if (!settings) return "";
  const g = (k: string) => settingOf(settings, k);
  if (g("notify") === "false") return "alerts off";
  const out = [
    g("notify.ntfy") ? "ntfy" : "",
    g("notify.slack") ? "Slack" : "",
    g("notify.webhook") ? "webhook" : "",
    truthy(g("notify.raycast")) ? "Raycast" : "",
    g("moshi") === "on" ? "Moshi" : "",
  ].filter(Boolean);
  const dnd = g("notify.dnd") ? " · do not disturb" : "";
  return out.length ? `alerts go to ${out.join(" · ")}${dnd}` : dnd.slice(3);
}

/** What the hub did today: events, auto-approvals, the queue, when usage was last read. */
export function todayTiles(o: {
  events?: InboxEvent[];
  /** the inbox was asked for this many: when they're all from today there are more */
  cap?: number;
  queue?: QueueTask[];
  usage?: UsageToday;
  settings?: Record<string, unknown>;
  /** the machines it watches (usage's other machines don't count) */
  hosts?: string[];
  now: number;
}): Tile[] {
  const from = midnight(o.now);
  const today = (o.events ?? []).filter((e) => e.t >= from);
  const count = (k: string) => today.filter((e) => e.kind === k).length;
  const out: Tile[] = [];

  // events
  if (!o.events) out.push({ label: "Events", value: "–", sub: ["inbox not read yet"] });
  else {
    const more = o.cap && o.events.length >= o.cap && today.length === o.events.length ? "+" : "";
    const sub = (
      [
        ["blocked", "needed you"],
        ["done", "finished"],
        ["limit", "hit a limit"],
        ["new", "started"],
        ["error", "offline"],
        ["heal", "healed"],
      ] as const
    )
      .map(([k, w]) => [count(k), w] as const)
      .filter(([n]) => n > 0)
      .map(([n, w]) => `${n} ${w}`);
    out.push({ label: "Events", value: `${today.length}${more}`, sub: sub.length ? sub : ["quiet so far"] });
  }

  // auto-approve
  const auto = settingOf(o.settings, "auto_approve");
  const remote = count("remote");
  if (o.settings && auto && !truthy(auto)) out.push({ label: "Auto-approved", value: "off", sub: ["auto_approve"] });
  else
    out.push({
      label: "Auto-approved",
      value: o.events ? String(count("auto")) : "–",
      sub: remote ? [plural(remote, "remote action")] : ["by your rules"],
    });

  // the queue
  if (!o.queue) out.push({ label: "Queue", value: "–", sub: [] });
  else {
    const waiting = o.queue.filter((q) => q.state === "queued").length;
    const sent = o.queue.filter((q) => q.state === "sent" && (q.at ?? 0) >= from).length;
    const failed = o.queue.filter((q) => q.state === "failed").length;
    out.push({
      label: "Queue",
      value: String(waiting),
      suffix: "waiting",
      sub: [`${sent} sent today`, failed ? `${failed} failed` : ""].filter(Boolean),
    });
  }

  // usage: when the logs were last read
  const u = o.usage;
  if (!u) out.push({ label: "Usage read", value: "–", sub: [] });
  else if (u.enabled === false) out.push({ label: "Usage read", value: "off", sub: ["analytics.enabled"] });
  else {
    const synced = Object.entries(u.synced ?? {})
      .filter(([h, t]) => t > 0 && (!o.hosts?.length || o.hosts.includes(h)))
      .map(([, t]) => t);
    const last = synced.length ? Math.max(...synced) : 0;
    const cost = u.total?.cost ?? u.rows.reduce((s, r) => s + (r.cost ?? 0), 0);
    const tokens = u.total?.tokens ?? u.rows.reduce((s, r) => s + (r.tokens ?? 0), 0);
    out.push({
      label: "Usage read",
      value: last ? `${since(last, o.now) || "0s"} ago` : "not yet",
      sub: [cost > 0 ? `$${cost.toFixed(2)} today` : "", tokens > 0 ? `${compact(tokens)} tokens` : ""].filter(Boolean),
    });
  }
  return out;
}

// ---------------------------------------------------------------- remote access

/** Where the API can be reached from: `this computer only`, `your tailnet`, … */
function reach(v: DaemonView): string {
  const e = v.s.exposure;
  if (!v.listening) return v.httpOn ? "on after a restart" : "this computer only";
  const where: string[] = [];
  if (e?.tailscale?.some((x) => x.url)) where.push("your tailnet");
  if (e?.cloudflare?.hostname) where.push("the internet (Cloudflare)");
  if (where.length) return `reachable from ${where.join(" and ")}`;
  return v.s.api?.listen_any ? "every network this computer is on" : "this computer only";
}

/** The HTTP API, Tailscale, Cloudflare and the paired devices as chips. */
export function remoteGroup(v: DaemonView, devices: Device[] | undefined, t: ChartTheme): ChipGroup {
  const e = v.s.exposure;
  const on = t.state.done;
  const off = t.other;
  const ts = (e?.tailscale ?? []).find((x) => x.url)?.url;
  const cf = e?.cloudflare?.hostname;
  const out: ChipGroup["chips"] = [
    {
      text: "HTTP API",
      note: v.listening ?? (v.httpOn ? "restart to start" : "off"),
      dot: v.listening ? on : v.httpOn ? t.state.warn : off,
    },
    { text: "Tailscale", note: ts ? ts.replace(/^https?:\/\//, "") : "off", dot: ts ? on : off },
    {
      text: "Cloudflare",
      note: cf ?? "off",
      dot: cf ? (e?.cloudflare?.running === false ? t.state.warn : on) : off,
    },
  ];
  if (devices) {
    const push = devices.reduce((s, d) => s + (d.push ? 1 : 0), 0);
    out.push({
      text: "Devices",
      note: devices.length ? `${devices.length} paired${push ? ` · ${push} with push` : ""}` : "none paired",
      dot: devices.length ? on : off,
    });
  }
  return { title: "Remote access", note: reach(v), chips: out };
}

/** The keys that change remote access, for what's there now. */
export function remoteKeys(v: DaemonView): KeyHint[] {
  if (v.state === "missing") return [];
  const e = v.s.exposure;
  const exposed = !!(e?.tailscale?.some((x) => x.url) || e?.cloudflare?.hostname);
  if (!v.httpOn)
    return [{ key: "⇧⌘A", label: "turn the API on" }, ...(v.running ? [{ key: "⇧⌘P", label: "pair a device" }] : [])];
  if (!v.listening) return nextStep(v)?.op === "restart" ? [{ key: "↵", label: "restart to start it" }] : [];
  return [
    { key: "⇧⌘P", label: "pair a device" },
    { key: "⇧⌘W", label: "web console" },
    ...(exposed ? [] : [{ key: "⇧⌘T", label: "put it on your tailnet" }]),
  ];
}

// ---------------------------------------------------------------- the whole view

export function daemonMarkdown(o: {
  v: DaemonView;
  fleet?: Host[];
  hist?: Record<string, LinkSample[]>;
  events?: InboxEvent[];
  cap?: number;
  queue?: QueueTask[];
  usage?: UsageToday;
  devices?: Device[];
  settings?: Record<string, unknown>;
  t: ChartTheme;
  logo?: LogoFn;
  osText?: (alias: string) => string | undefined;
  now?: number;
  width?: number;
}): string {
  const { v, t } = o;
  const w = o.width ?? WIDE;
  const now = o.now ?? Date.now() / 1000;
  const card = statusOf(v, o.logo);
  const out = [img(`Daemon: ${card.title}`, statusCard({ ...card, badges: themed(card.badges, t) }, t, w))];
  if (v.state === "missing" || v.state === "checking") return out.join("\n\n");
  if (v.running) {
    const watched = watchedOf(o.fleet, v.s.hosts, { hist: o.hist, logo: o.logo, osText: o.osText });
    const link = fleetLink(o.fleet);
    const note = [plural(watched.length, "machine"), o.fleet?.length ? link.text : ""].filter(Boolean).join(" · ");
    out.push(img("Machines it watches", stack([heading("Watching", note, t, w), watchTiles(watched, t, w)], w, 8)));
    const list = todayTiles({
      events: o.events,
      cap: o.cap,
      queue: o.queue,
      usage: o.usage,
      settings: o.settings,
      hosts: v.s.hosts,
      now,
    });
    out.push(
      img(
        "Today",
        stack(
          [
            heading(v.s.hub === false ? "Today · the hub runs elsewhere" : "Today", alertSinks(o.settings), t, w),
            tiles(list, t, w),
          ],
          w,
          8,
        ),
      ),
    );
  }
  const keys = remoteKeys(v);
  out.push(
    img(
      "Remote access",
      stack([chips([remoteGroup(v, o.devices, t)], t, w), keys.length ? keyHints(keys, t, w) : ""], w, 10),
    ),
  );
  return out.join("\n\n");
}
