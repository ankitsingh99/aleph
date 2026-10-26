// Pure logic the commands share, with no Raycast imports (so plain node can
// test it): what an ssh / herdr error means, how each machine's link is doing
// and the fleet's overall connectivity, and the Inbox timeline (events and the
// sessions you started, merged the way the dashboard's Inbox does).

/** How a value should look: the UI maps tones to Aleph's palette. */
export type Tone = "good" | "warn" | "bad" | "muted";

// ---------------------------------------------------------------- errors

const NOT_RUNNING = /server_not_running|no herdr server|not running|os \{ code:.*(connectionrefused|notfound)/;

/** The machine answers, but herdr isn't running there. */
export const herdrStopped = (text: string | null | undefined) => NOT_RUNNING.test((text ?? "").toLowerCase());

/**
 * ssh / herdr error text → a plain explanation and the detail (`host:port · first line`),
 * like the dashboard's inbox: "ssh: connect to host … Operation timed out" → "unreachable — ssh timed out".
 */
export function explainError(text: string | null | undefined): { head: string; detail: string } {
  const t = (text ?? "").trim();
  const l = t.toLowerCase();
  const m = /connect to host (\S+) port (\d+)/.exec(t);
  const target = m ? `${m[1]}:${m[2]}` : "";
  const head = !t
    ? "unreachable"
    : l.includes("timed out")
      ? "unreachable — ssh timed out"
      : l.includes("could not resolve hostname") || l.includes("name or service not known")
        ? "unknown host — the name doesn't resolve"
        : l.includes("no route to host") || l.includes("network is unreachable")
          ? "unreachable — no route (VPN off? asleep?)"
          : l.includes("permission denied")
            ? "key login refused"
            : l.includes("host key verification failed") || l.includes("identification has changed")
              ? "host key isn't trusted"
              : l.startsWith("ssh") && l.includes("connection refused")
                ? "ssh refused — sshd isn't listening"
                : NOT_RUNNING.test(l)
                  ? "herdr stopped there"
                  : l.includes("unknown machine")
                    ? "not saved in herdr"
                    : "error";
  const first = (t.split("\n")[0] ?? "").trim().replace(/^ssh: /, "");
  return { head, detail: target ? `${target} · ${first}` : first };
}

// ---------------------------------------------------------------- links

/** A poll slower than this is a slow link (fleet.rs `SLOW_MS`, the dashboard's ◐). */
export const SLOW_MS = 800;

/** What `ls --json` says about a machine. */
export interface HostLike {
  alias: string;
  reachable: boolean;
  error: string | null;
  stale?: boolean;
  latency_ms?: number | null;
}

export interface LinkInfo {
  state: "up" | "slow" | "down" | "stopped" | "unknown";
  /** ● up · ◐ slow · ○ down · ◍ herdr stopped · ◌ not known yet */
  glyph: string;
  /** `32 ms`, `950 ms — slow`, `offline — ssh timed out`, … */
  text: string;
  tone: Tone;
}

/** How one machine's link is doing, from its last poll (the dashboard's per-machine bars). */
export function hostLink(h: HostLike): LinkInfo {
  if (!h.reachable) {
    if (herdrStopped(h.error)) return { state: "stopped", glyph: "◍", text: "herdr stopped there", tone: "warn" };
    return { state: "down", glyph: "○", text: `offline — ${explainError(h.error).head}`, tone: "bad" };
  }
  if (h.stale) return { state: "unknown", glyph: "◌", text: "not polled yet (cached)", tone: "muted" };
  const ms = h.latency_ms;
  if (ms == null) return { state: "up", glyph: "●", text: "online", tone: "good" };
  if (ms > SLOW_MS) return { state: "slow", glyph: "◐", text: `${ms} ms — slow`, tone: "warn" };
  return { state: "up", glyph: "●", text: `${ms} ms`, tone: "good" };
}

export interface FleetLink {
  state: "online" | "weak" | "offline" | "unknown";
  /** the dashboard header's ● online · ◐ weak · ○ offline */
  glyph: string;
  /** `Online · 32 ms`, `Weak — gpu2 is offline`, `Offline — no machine answers` */
  text: string;
  tone: Tone;
}

const median = (v: number[]) => {
  const s = [...v].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
};

/** The fleet's connectivity: every machine answering quickly, some down or slow, or none answering. */
export function fleetLink(hosts: HostLike[] | undefined): FleetLink {
  if (!hosts) return { state: "unknown", glyph: "◌", text: "Checking your machines…", tone: "muted" };
  if (!hosts.length) return { state: "unknown", glyph: "◌", text: "No machines yet — aleph setup", tone: "muted" };
  const links = hosts.map((h) => ({ h, l: hostLink(h) }));
  // a machine whose herdr stopped still answers: its link is fine
  const answering = links.filter(({ l }) => l.state !== "down");
  if (!answering.length) return { state: "offline", glyph: "○", text: "Offline — no machine answers", tone: "bad" };
  const down = links.filter(({ l }) => l.state === "down").map(({ h }) => h.alias);
  const slow = links.filter(({ l }) => l.state === "slow").map(({ h }) => h.alias);
  if (down.length) {
    const who = down.length === 1 ? `${down[0]} is offline` : `${down.length} machines are offline`;
    return { state: "weak", glyph: "◐", text: `Weak — ${who}`, tone: "warn" };
  }
  if (slow.length) {
    const who = slow.length === 1 ? `${slow[0]} is slow` : `${slow.length} machines are slow`;
    return { state: "weak", glyph: "◐", text: `Weak — ${who}`, tone: "warn" };
  }
  const ms = answering.map(({ h }) => h.latency_ms).filter((x): x is number => x != null);
  return {
    state: "online",
    glyph: "●",
    text: ms.length ? `Online · ${median(ms)} ms` : "Online",
    tone: "good",
  };
}

// ---------------------------------------------------------------- the inbox timeline

/** An `inbox --json` event. `name` is empty for machine events (error, heal). */
export interface EventLike {
  t: number;
  host: string;
  name: string;
  kind: string;
  title: string;
  agent?: string;
  /** a stable id (newer builds) */
  id?: string;
  /** who said so: empty (herdr's view), `agent` (its own hook), `aleph` */
  source?: string;
  /** more than the title, when there is more: the agent's own words */
  body?: string;
  /** newer than the inbox's read mark (the daemon's `inbox.list` only) */
  unread?: boolean;
}

/** A `history --json` entry: a session you started. */
export interface LaunchLike {
  t: number;
  host: string;
  dir: string;
  name: string;
  agent: string;
  args: string[];
  prompt: string | null;
  worktree: boolean;
}

export interface TimelineItem {
  /** stable across refreshes: `t|host|name|kind` of the newest event it stands for */
  key: string;
  /** the event (a launch without one gets a `new` event of its own) */
  e: EventLike;
  /** how many identical events this row stands for (newest first) */
  n: number;
  /** the oldest of them */
  first: number;
  /** how the session was started (a launch from history) */
  launch?: LaunchLike;
  /** it's in the event log (a launch on its own isn't) */
  logged: boolean;
  /** the ids of the events it stands for (set by `fold`; dismissing hides them all) */
  ids?: string[];
}

export const eventKey = (e: EventLike) => e.id || `${e.t}|${e.host}|${e.name}|${e.kind}|${e.source ?? ""}`;

/** Does launch `l` belong to `started` event `e`: the same session, seen from 5 s before the launch to 120 s after. */
export function joins(l: LaunchLike, e: EventLike): boolean {
  return e.kind === "new" && l.host === e.host && l.name === e.name && e.t + 5 >= l.t && e.t <= l.t + 120;
}

/**
 * Events and launches as one timeline, newest first: a launch joins its
 * `started` event; launches whose event isn't in the log (older, or started
 * elsewhere) stand on their own (tui/inbox.rs `merged`).
 */
export function timeline(events: EventLike[], launches: LaunchLike[]): TimelineItem[] {
  const used = new Array<boolean>(launches.length).fill(false);
  const by = new Map<string, number[]>();
  launches.forEach((l, i) => {
    const k = `${l.host}|${l.name}`;
    by.set(k, [...(by.get(k) ?? []), i]);
  });
  const all: TimelineItem[] = events.map((e) => {
    const ix = e.kind === "new" ? (by.get(`${e.host}|${e.name}`) ?? []) : [];
    const i = ix.find((j) => !used[j] && joins(launches[j], e));
    if (i !== undefined) used[i] = true;
    return { key: eventKey(e), e, n: 1, first: e.t, launch: i !== undefined ? launches[i] : undefined, logged: true };
  });
  launches.forEach((l, i) => {
    if (used[i]) return;
    const e: EventLike = { t: l.t, host: l.host, name: l.name, kind: "new", title: l.dir, agent: l.agent };
    all.push({ key: `launch|${eventKey(e)}`, e, n: 1, first: l.t, launch: l, logged: false });
  });
  return all.sort((a, b) => b.e.t - a.e.t);
}

const same = (a: EventLike, b: EventLike) =>
  a.host === b.host &&
  a.name === b.name &&
  a.kind === b.kind &&
  a.title === b.title &&
  (a.body ?? "") === (b.body ?? "");

/** A run of identical events (same machine, session, kind and text) folds into its newest: `×n`. */
export function fold(items: TimelineItem[]): TimelineItem[] {
  const out: TimelineItem[] = [];
  const idOf = (it: TimelineItem) => (it.logged && it.e.id ? [it.e.id] : []);
  for (const it of items) {
    const p = out[out.length - 1];
    if (p && !p.launch && !it.launch && same(p.e, it.e)) {
      p.n += 1;
      p.first = it.e.t;
      p.ids = [...(p.ids ?? []), ...idOf(it)];
    } else {
      out.push({ ...it, ids: idOf(it) });
    }
  }
  return out;
}

/**
 * Where an Inbox launch context `{host, name?, kind?, t?}` lands (a desktop
 * banner's click with `notify.desktop.click = raycast`, the menu bar's Show
 * in Inbox): the row holding that event — inside a folded run too — else
 * that session's newest row of that kind, else its newest row.
 */
export function landing(
  items: TimelineItem[],
  ctx: { host?: string; name?: string; kind?: string; t?: number | string } | undefined,
): TimelineItem | undefined {
  if (!ctx?.host) return undefined;
  const mine = items.filter((it) => it.e.host === ctx.host && (!ctx.name || it.e.name === ctx.name));
  const ofKind = mine.filter((it) => !ctx.kind || it.e.kind === ctx.kind);
  const t = Number(ctx.t);
  const exact = t > 0 ? ofKind.find((it) => it.first <= t && t <= it.e.t) : undefined;
  return exact ?? ofKind.find((it) => it.logged) ?? ofKind[0] ?? mine[0];
}

/** The Inbox's kind filters (the dashboard's `f`). */
export const INBOX_FILTERS: { id: string; title: string; kinds?: string[] }[] = [
  { id: "attention", title: "Needs You", kinds: ["blocked"] },
  { id: "finished", title: "Finished", kinds: ["done", "gone"] },
  { id: "problems", title: "Problems", kinds: ["error", "limit"] },
  { id: "automation", title: "Automation", kinds: ["auto", "task", "heal"] },
  { id: "launches", title: "Launched by You" },
];

/** Does `it` pass filter `id` (`all`, a kind filter, `launches`, or `host:<alias>`)? */
export function keeps(id: string, it: TimelineItem): boolean {
  if (id === "all") return true;
  if (id === "launches") return !!it.launch;
  if (id.startsWith("host:")) return it.e.host === id.slice(5);
  const f = INBOX_FILTERS.find((x) => x.id === id);
  return f?.kinds ? f.kinds.includes(it.e.kind) : true;
}

/** The kinds the menu bar shows by default (`notify.raycast.kinds`): the dashboard's unread kinds. */
export const RAYCAST_KINDS = ["blocked", "done", "error", "limit"];

/** Every event kind, with the words Settings use. */
export const ALL_KINDS: [string, string][] = [
  ["blocked", "Needs you"],
  ["done", "Finished"],
  ["limit", "At its usage limit"],
  ["error", "Machine unreachable"],
  ["up", "Machine back up"],
  ["heal", "herdr restarted"],
  ["auto", "Auto-approved"],
  ["task", "Queued task sent"],
  ["remote", "Remote action (phone, Raycast, web)"],
  ["new", "Session started"],
  ["gone", "Session ended"],
];

/**
 * `notify.source` (both | herdr | agents), like the router's `source_ok`:
 * `herdr` drops what the agents' own hooks said, `agents` drops herdr's
 * questions and finished turns, `both` keeps whichever said it — and when
 * both said the same within 15 s, the agent's words. Newest first in, newest
 * first out.
 */
export function bySource<T extends EventLike>(events: T[], mode: string): T[] {
  const said = (e: EventLike) => e.kind === "blocked" || e.kind === "done";
  const fromAgent = (e: EventLike) => e.source === "agent";
  if (mode === "herdr") return events.filter((e) => !fromAgent(e));
  if (mode === "agents") return events.filter((e) => !said(e) || (e.source ?? "") !== "");
  return events.filter(
    (e) =>
      !said(e) ||
      fromAgent(e) ||
      !events.some(
        (o) =>
          o !== e &&
          fromAgent(o) &&
          o.host === e.host &&
          o.name === e.name &&
          o.kind === e.kind &&
          Math.abs(o.t - e.t) <= 15,
      ),
  );
}

/**
 * Do not disturb (`notify.dnd`): off (empty, `off`, `0`, or a time that has
 * passed), on until turned off (`on`), or on until a unix time.
 */
export function dndState(v: string | null | undefined, now = Date.now() / 1000): { on: boolean; until?: number } {
  const t = (v ?? "").trim();
  if (!t || t === "off" || t === "0") return { on: false };
  if (t === "on" || t === "true") return { on: true };
  const u = Number(t);
  return Number.isFinite(u) && u > now ? { on: true, until: u } : { on: false };
}

/** `notify.raycast.kinds` → the kinds (`all`, a comma list, or empty for the default). */
export function parseKinds(v: string | null | undefined): string[] {
  const t = (v ?? "").trim();
  if (!t) return RAYCAST_KINDS;
  if (t === "all") return ALL_KINDS.map(([k]) => k);
  if (t === "none") return [];
  return t
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);
}

// ---------------------------------------------------------------- the doctor

/** One check in `doctor --json`. */
export interface DoctorCheck {
  /** ok · warn · fail · info */
  status: string;
  label: string;
  text: string;
  fix: string[];
  /** what `doctor --fix` would do for it (it can carry it out itself) */
  action?: string | null;
}

export interface DoctorSection {
  /** `This Mac`, `Host · b31`, `On b31`, `herdr · b31`, `Shortcuts` */
  section: string;
  checks: DoctorCheck[];
}

export interface DoctorReport {
  ok: number;
  warn: number;
  fail: number;
  sections: DoctorSection[];
  /** the daemon's: when it ran (unix seconds), and whether a new run is under way */
  at?: number;
  running?: boolean;
}

const RANK: Record<string, number> = { fail: 0, warn: 1, info: 2, ok: 3 };
const rank = (status: string) => RANK[status] ?? 2;

/** The worst status in a section. */
export const worst = (s: DoctorSection) => s.checks.reduce((w, c) => (rank(c.status) < rank(w) ? c.status : w), "ok");

/** Failures first: sections by their worst check, checks by status (each kept in order otherwise). */
export function doctorSorted(r: DoctorReport): DoctorSection[] {
  return r.sections
    .map((s, i) => ({ s, i }))
    .sort((a, b) => rank(worst(a.s)) - rank(worst(b.s)) || a.i - b.i)
    .map(({ s }) => ({
      ...s,
      checks: s.checks
        .map((c, i) => ({ c, i }))
        .sort((a, b) => rank(a.c.status) - rank(b.c.status) || a.i - b.i)
        .map(({ c }) => c),
    }));
}

/** `1 failing · 2 warnings · 23 ok` */
export function doctorTally(r: Pick<DoctorReport, "ok" | "warn" | "fail">): string {
  return [r.fail ? `${r.fail} failing` : "", r.warn ? `${r.warn} warning${r.warn === 1 ? "" : "s"}` : "", `${r.ok} ok`]
    .filter(Boolean)
    .join(" · ");
}

export const STATUS_GLYPH: Record<string, string> = { ok: "✓", warn: "!", fail: "✕", info: "·" };

/**
 * An Aleph command a fix suggests, ready to run in a terminal
 * (`aleph --host b31 agents setup codex   — why` → its words), or undefined
 * when the fix is advice or another tool's command.
 */
export function runnableFix(fix: string): string[] | undefined {
  const cmd = fix.split(/\s{2,}|\s—\s/)[0].trim();
  if (!/^aleph\s/.test(cmd) || /[|&;<>`$()]/.test(cmd)) return undefined;
  return cmd.split(/\s+/).slice(1);
}

/** The whole report as Markdown (failures first), for a Detail or the clipboard. */
export function doctorMarkdown(r: DoctorReport, title: string): string {
  const esc = (s: string) => s.replace(/[<>*_`#[\]]/g, (c) => `\\${c}`);
  const out = [`# ${title}`, `**${doctorTally(r)}**`];
  for (const s of doctorSorted(r)) {
    out.push(`## ${STATUS_GLYPH[worst(s)] ?? "·"} ${esc(s.section)}`);
    out.push(
      s.checks
        .map((c) => {
          const fixes = c.fix.map((f) => `  - fix: \`${f.replace(/`/g, "'")}\``).join("\n");
          return `- ${STATUS_GLYPH[c.status] ?? "·"} **${esc(c.label)}** — ${esc(c.text)}${fixes ? `\n${fixes}` : ""}`;
        })
        .join("\n"),
    );
  }
  return out.join("\n\n");
}

// ---------------------------------------------------------------- presets

/** The sessions `aleph preset launch` started: its `✓ <name> on <host>` lines. */
export function startedLines(out: string): { name: string; host: string }[] {
  return out
    .split("\n")
    .map((l) => /^\s*✓\s+(\S+)\s+on\s+(\S+)\s*$/.exec(l))
    .filter((m): m is RegExpExecArray => !!m)
    .map((m) => ({ name: m[1], host: m[2] }));
}
