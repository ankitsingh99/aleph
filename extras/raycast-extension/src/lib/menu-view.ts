// The menu bar, decided without Raycast: its title, the one status line, the
// sessions each section lists and in what order, the Inbox messages still
// worth a line (what the sections above already say, or what has since
// been put right, goes), and every line's words. No Raycast imports: plain
// node tests it.
import type { Host, Session } from "./cli";
import { since, STATE_ORDER, stateOf, STATUS_LABEL } from "./fmt";
import { bySource, EventLike, explainError, FleetLink, fold, hostLink, timeline, TimelineItem } from "./model";
import { moneyText } from "./showcase";

/** How many sessions are in each state. */
export function stateCounts(sessions: Pick<Session, "status" | "limit">[]): Record<string, number> {
  const c: Record<string, number> = {};
  for (const s of sessions) c[stateOf(s)] = (c[stateOf(s)] ?? 0) + 1;
  return c;
}

/** The menu bar's title: `▲2` while sessions wait, then `◐` / `○` for a weak or lost link, `⊘` in do not disturb. */
export function menuTitle(blocked: number, link: Pick<FleetLink, "state" | "glyph">, dnd: boolean): string | undefined {
  const degraded = link.state === "weak" || link.state === "offline";
  return (
    [blocked ? `▲${blocked}` : "", degraded ? link.glyph : "", dnd ? "⊘" : ""].filter(Boolean).join(" ") || undefined
  );
}

/** The status line: `2 need you · 3 working · 1 done` (the states there are, needs-you first). */
export function countsLine(c: Record<string, number>): string {
  const word = (s: string) => (s === "blocked" && c[s] > 1 ? "need you" : (STATUS_LABEL[s] ?? s));
  const parts = STATE_ORDER.filter((s) => c[s]).map((s) => `${c[s]} ${word(s)}`);
  return parts.length ? parts.join(" · ") : "No sessions running";
}

/** Sessions in `state`: waiting the longest first (needs you, at limit), else the latest first. */
export function inState<T extends Pick<Session, "status" | "limit" | "status_since">>(
  sessions: T[],
  state: string,
  max: number,
): T[] {
  const oldestFirst = state === "blocked" || state === "limit";
  return sessions
    .filter((s) => stateOf(s) === state)
    .sort((a, b) => ((a.status_since ?? 0) - (b.status_since ?? 0)) * (oldestFirst ? 1 : -1))
    .slice(0, max);
}

/** A session's line: its name, then where and for how long (`b31 · 4m`). */
export function sessionLine(
  s: Pick<Session, "host" | "name" | "display" | "pane_id" | "status_since"> & { cwd?: string | null },
  now = Date.now() / 1000,
  agentName?: string,
): { title: string; subtitle: string } {
  return {
    title: sessionName(s, agentName),
    subtitle: [s.host, since(s.status_since, now)].filter(Boolean).join(" · "),
  };
}

/**
 * A session's name as a person would say it: its own name, but a bare
 * number (herdr's default tab name) gives way to its folder (`test_folder`),
 * else the agent's name.
 */
export function sessionName(
  s: Pick<Session, "name" | "display" | "pane_id"> & { cwd?: string | null },
  agentName?: string,
): string {
  const own = (s.display || s.name || "").trim();
  if (own && !/^\d+$/.test(own)) return own;
  const folder = s.cwd && s.cwd !== "-" ? s.cwd.replace(/\/+$/, "").split("/").pop() : "";
  return folder || agentName || own || s.pane_id;
}

/** The same on one line, for a submenu (it has no subtitle). */
export const sessionTitle = (s: Parameters<typeof sessionLine>[0], now?: number) => {
  const l = sessionLine(s, now);
  return `${l.title}  ·  ${l.subtitle}`;
};

/** Does this inbox say which events are read (the daemon's `inbox.list`, newer `inbox --json`)? */
export const tracksRead = (events: Pick<EventLike, "unread">[]) => events.some((e) => typeof e.unread === "boolean");

/**
 * The events the menu's Inbox may list: only the unread ones once the inbox
 * says which are — so after Mark All Read, none. An inbox that says nothing
 * about read (an old build) keeps them all.
 */
export function unreadOnly<T extends Pick<EventLike, "unread">>(events: T[]): T[] {
  return tracksRead(events) ? events.filter((e) => e.unread === true) : events;
}

/**
 * The Inbox messages still worth a line in the menu: not a question while
 * the session's machine answers (Needs You shows what waits now), not a
 * machine's trouble once it answers again, not a usage limit its session is
 * no longer at. Machines we can't reach keep their messages.
 */
export function freshMessages<T extends TimelineItem>(items: T[], hosts: Host[] | undefined): T[] {
  if (!hosts) return items;
  return items.filter(({ e }) => {
    const h = hosts.find((x) => x.alias === e.host);
    if (!h?.reachable) return true;
    if (e.kind === "blocked") return false;
    if (e.kind === "error") return false;
    if (e.kind === "limit")
      return h.sessions.some((s) => (s.name === e.name || s.display === e.name) && s.limit != null);
    return true;
  });
}

/**
 * The menu's Inbox rows, newest first: the events of the kinds it lists
 * (notify.raycast.kinds), as notify.source picks them, unread only, runs of
 * the same folded into one, minus what no longer counts (`freshMessages`).
 * Empty after Mark All Read; what needs you now is the live Needs You section.
 */
export function inboxRows<T extends EventLike>(
  events: T[],
  o: { kinds: string[]; source: string; hosts?: Host[] },
): TimelineItem[] {
  const picked = bySource(
    events.filter((e) => o.kinds.includes(e.kind)),
    o.source,
  );
  return freshMessages(fold(timeline(unreadOnly(picked), [])), o.hosts);
}

/** An Inbox message on one line: `temper · finished ×2 · 4m · new` (a machine's: its name). */
export function messageTitle(
  it: Pick<TimelineItem, "n"> & { e: Pick<EventLike, "t" | "host" | "name" | "unread"> },
  word: string,
  now = Date.now() / 1000,
): string {
  const e = it.e;
  return [e.name || e.host, `${word}${it.n > 1 ? ` ×${it.n}` : ""}`, since(e.t, now) || "now", e.unread ? "new" : ""]
    .filter(Boolean)
    .join("  ·  ");
}

/** A machine's line: `b31  ·  3 sessions · 85 ms`, `gpu2  ·  offline — ssh timed out`. */
export function machineTitle(h: Host): string {
  const l = hostLink(h);
  const dot = stateDot(h);
  // down: its circle and just why
  if (l.state === "down")
    return `${dot} ${h.alias}   ${explainError(h.error).head.replace(/^(unreachable|offline) — /, "")}`;
  // herdr stopped there: its red circle says enough
  if (l.state === "stopped") return `${dot} ${h.alias}`;
  const n = h.sessions.length;
  const asks = h.sessions.filter((x) => x.status === "blocked").length;
  // only when it has sessions: how many (who needs you first); the ping only when slow
  const said = [
    asks ? `▲${asks} needs you` : "",
    n ? `${n} session${n === 1 ? "" : "s"}` : "",
    l.state === "slow" ? l.text : "",
  ]
    .filter(Boolean)
    .join(" · ");
  return said ? `${dot} ${h.alias}   ${said}` : `${dot} ${h.alias}`;
}

/** A machine's state for its dot: up, busy (an agent working there), down. */
export function machineState(h: Host): "up" | "busy" | "down" {
  const l = hostLink(h);
  if (l.state === "down" || l.state === "stopped") return "down";
  return h.sessions.some((s) => s.status === "working") ? "busy" : "up";
}

/** The Machines line: `4 machines running`, `3 of 4 machines running`. */
export function runningLine(hosts: Host[] | undefined): string {
  if (!hosts) return "Checking your machines…";
  if (!hosts.length) return "No machines yet";
  const up = hosts.filter((h) => hostLink(h).state !== "down").length;
  const n = hosts.length;
  return up === n ? `${n} machine${n === 1 ? "" : "s"} running` : `${up} of ${n} machines running`;
}

/** A machine's state as a coloured dot (where an icon can't carry it): 🟢 online, 🟡 an agent working, 🔴 offline. */
export function stateDot(h: Host): string {
  return { up: "🟢", busy: "🟡", down: "🔴" }[machineState(h)];
}

/** More than a section shows: `+3 more`. */
export const more = (total: number, shown: number) => (total > shown ? `+${total - shown} more` : "");

/** The sessions line: how many, then each state there is (`1 needs you · 2 working · 3 idle`); its colour, the first's. */
export function sessionsLine(c: Record<string, number>): { title: string; subtitle: string; state: string } {
  const states = STATE_ORDER.filter((s) => c[s]);
  const total = states.reduce((n, s) => n + c[s], 0);
  if (!total) return { title: "No sessions running", subtitle: "", state: "idle" };
  return {
    title: `${total} session${total === 1 ? "" : "s"}`,
    subtitle: countsLine(c),
    state: states[0],
  };
}

/** The machines line: `All 4 machines online` / `3 of 4 online`, then who's down or slow, else the latency range. */
export function machinesLine(hosts: Host[] | undefined): { title: string; subtitle: string; tone: FleetLink["tone"] } {
  if (!hosts) return { title: "Checking your machines…", subtitle: "", tone: "muted" };
  if (!hosts.length) return { title: "No machines yet", subtitle: "aleph setup", tone: "muted" };
  const links = hosts.map((h) => ({ h, l: hostLink(h) }));
  const down = links.filter(({ l }) => l.state === "down").map(({ h }) => h.alias);
  const slow = links.filter(({ l }) => l.state === "slow").map(({ h }) => `${h.alias} ${h.latency_ms} ms`);
  const up = hosts.length - down.length;
  const n = hosts.length;
  const title = down.length
    ? up
      ? `${up} of ${n} machines online`
      : `${n === 1 ? "Your machine is" : `All ${n} machines are`} offline`
    : `${n === 1 ? "1 machine" : `All ${n} machines`} online`;
  const ms = links
    .filter(({ l }) => l.state !== "down")
    .map(({ h }) => h.latency_ms)
    .filter((x): x is number => x != null);
  const range = ms.length
    ? Math.min(...ms) === Math.max(...ms)
      ? `${ms[0]} ms`
      : `${Math.min(...ms)}–${Math.max(...ms)} ms`
    : "";
  const subtitle = down.length ? `${down.join(", ")} offline` : slow.length ? `slow: ${slow.join(", ")}` : range;
  return { title, subtitle, tone: !up ? "bad" : down.length || slow.length ? "warn" : "good" };
}

const compactN = (n: number) =>
  n < 1e3
    ? String(Math.round(n))
    : n < 1e6
      ? `${(n / 1e3).toFixed(1)}k`
      : n < 1e9
        ? `${(n / 1e6).toFixed(1)}M`
        : `${(n / 1e9).toFixed(1)}B`;

// ---------------------------------------------------------------- usage

/** What the usage rows need of `aleph usage` (cli.ts `UsageRow`). */
export interface UsageLike {
  key: string;
  cost: number;
  tokens: number;
  estimated?: boolean;
  unpriced_requests?: number;
}

/** A plan window as the menu shows it: `week`, how full, when it resets. */
export interface QuotaWindow {
  minutes: number;
  pct: number;
  resets_at?: number | null;
}

/** One account's plan: its name, its windows (the longest first), the machines it was read on. */
export interface Quota {
  plan?: string;
  windows: QuotaWindow[];
  hosts: string[];
}

export interface AgentUsage {
  agent: string;
  week: UsageLike | undefined;
  today: UsageLike | undefined;
  /** a plan per account it's signed in to (two accounts: two quotas), the fullest first */
  quotas: Quota[];
}

/** One bucket of the daemon's `usage.now`. */
export interface NowSum {
  tokens: number;
  cost: number;
  estimated: boolean;
  requests?: number;
  unpriced?: number;
}
type Spans = { today: NowSum; week: NowSum; month: NowSum };

/** `usage.now`: the daemon's summary, refreshed every minute. */
export interface UsageNow {
  generated_at: number;
  total: Spans;
  agents: (Spans & { agent: string; accounts: string[] })[];
  accounts: {
    agent: string;
    account: string;
    plan?: string | null;
    hosts: string[];
    usage: Spans;
    windows: { minutes: number; used: number; left: number; resets_at?: number | null }[];
    lapsed?: boolean;
    as_of?: number | null;
    billed?: { kind: "plan" | "api"; plan?: string; monthly?: number | null };
  }[];
  machines: {
    host: string;
    state: "ok" | "behind" | "offline" | "error" | "never";
    synced_at: number;
    behind_s?: number | null;
    error?: string | null;
    same_as?: string[];
  }[];
}

const asUsage = (key: string, x: NowSum | undefined): UsageLike | undefined =>
  x ? { key, cost: x.cost, tokens: x.tokens, estimated: x.estimated } : undefined;

/** The menu's usage from `usage.now`: each agent's week, today and plans, the totals, what's late. */
export function usageFromNow(v: UsageNow): {
  perAgent: AgentUsage[];
  week: UsageLike;
  today: UsageLike;
  late: string;
} {
  const perAgent: AgentUsage[] = v.agents.map((a) => ({
    agent: a.agent,
    week: asUsage(a.agent, a.week),
    today: asUsage(a.agent, a.today),
    quotas: v.accounts
      .filter((x) => x.agent === a.agent && x.windows.length)
      .map((x) => ({
        plan: x.plan ?? undefined,
        hosts: x.hosts,
        windows: [...x.windows]
          .sort((p, q) => q.minutes - p.minutes)
          .map((w) => ({ minutes: w.minutes, pct: w.used, resets_at: w.resets_at })),
      })),
  }));
  // a plan read on a machine with no usage this week still shows
  for (const x of v.accounts)
    if (x.windows.length && !perAgent.some((a) => a.agent === x.agent))
      perAgent.push({
        agent: x.agent,
        week: undefined,
        today: undefined,
        quotas: [
          {
            plan: x.plan ?? undefined,
            hosts: x.hosts,
            windows: x.windows.map((w) => ({ minutes: w.minutes, pct: w.used, resets_at: w.resets_at })),
          },
        ],
      });
  const late = v.machines
    .filter((m) => m.state === "offline" || m.state === "behind" || m.state === "error")
    .map((m) =>
      m.state === "offline"
        ? `${m.host} offline`
        : m.state === "error"
          ? `${m.host} couldn't be read`
          : `${m.host} behind ${fmtAgo(m.behind_s ?? 0)}`,
    )
    .join(" · ");
  return {
    perAgent,
    week: asUsage("week", v.total.week) ?? { key: "week", cost: 0, tokens: 0 },
    today: asUsage("today", v.total.today) ?? { key: "today", cost: 0, tokens: 0 },
    late,
  };
}

const fmtAgo = (s: number) =>
  s < 90
    ? `${Math.round(s)}s`
    : s < 5400
      ? `${Math.round(s / 60)}m`
      : s < 172800
        ? `${Math.round(s / 3600)}h`
        : `${Math.round(s / 86400)}d`;

type Snap = {
  ts: number;
  plan?: string | null;
  account?: string;
  hosts?: string[];
  windows: { used_percent: number; minutes?: number | null; resets_at?: number | null }[];
};
type LimitsLike = Record<string, Record<string, Record<string, Snap>>>;

/** Each agent's week, today and plans (one per account), the costliest first (then the most tokens). */
export function agentUsage(
  week: UsageLike[],
  today: UsageLike[],
  limits: LimitsLike | undefined,
  now = Date.now() / 1000,
): AgentUsage[] {
  // the newest reading per agent and account (a reading without an account: its machine's)
  const newest = new Map<string, Map<string, { snap: Snap; host: string }>>();
  for (const [host, agents] of Object.entries(limits ?? {}))
    for (const [agent, ids] of Object.entries(agents ?? {}))
      for (const [id, snap] of Object.entries(ids ?? {})) {
        if (!snap.windows?.length) continue;
        const key = snap.account || `${host}/${id}`;
        const by = newest.get(agent) ?? new Map();
        const had = by.get(key);
        if (!had || snap.ts > had.snap.ts) by.set(key, { snap, host });
        newest.set(agent, by);
      }
  const top = (q: Quota) => Math.max(0, ...q.windows.map((w) => w.pct));
  const ids = [...new Set([...week.map((r) => r.key), ...newest.keys()])].filter(Boolean);
  return ids
    .map((agent) => ({
      agent,
      week: week.find((r) => r.key === agent),
      today: today.find((r) => r.key === agent),
      quotas: [...(newest.get(agent)?.values() ?? [])]
        .map(({ snap, host }) => ({
          plan: snap.plan ?? undefined,
          hosts: snap.hosts?.length ? snap.hosts : [host],
          windows: [...snap.windows]
            .sort((x, y) => (y.minutes ?? 0) - (x.minutes ?? 0))
            // a window that reset after the reading (or a reading older than
            // the window) starts again from nothing: 0 %
            .map((w) => {
              const reset = w.resets_at ? w.resets_at <= now : !!w.minutes && snap.ts + w.minutes * 60 <= now;
              return {
                minutes: w.minutes ?? 0,
                pct: reset ? 0 : w.used_percent,
                resets_at: reset ? null : w.resets_at,
              };
            }),
        }))
        .sort((x, y) => top(y) - top(x)),
    }))
    .sort((a, b) => (b.week?.cost ?? 0) - (a.week?.cost ?? 0) || (b.week?.tokens ?? 0) - (a.week?.tokens ?? 0));
}

/** `$612.10*`, `$1447*`, `–`. */
export function dollars(r: Pick<UsageLike, "cost" | "estimated"> | undefined): string {
  // showcase: hidden amounts read $— (the daemon sends them as 0)
  const hidden = moneyText();
  if (hidden) return hidden;
  if (!r || !(r.cost > 0)) return "–";
  const v = r.cost < 1000 ? r.cost.toFixed(2) : String(Math.round(r.cost));
  return `$${v}${r.estimated ? "*" : ""}`;
}

/** `1.2B tok`. */
export const tokensText = (n: number) => `${compactN(n)} tok`;

/** The window the row's bar shows: the week when there is one, else the longest. */
export function headlineWindow(ws: QuotaWindow[]): QuotaWindow | undefined {
  return ws.find((w) => w.minutes === 10080) ?? ws[0];
}
