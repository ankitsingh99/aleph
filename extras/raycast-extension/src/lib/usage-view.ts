// Usage, drawn for a list's detail: the overview (today, 7 and 30 days as
// tiles, the range per day stacked by agent, by agent and by machine) and one
// detail per agent, model, machine and project (its figures, its share, its
// days, where it ran and with what). Projects get a name a person would use
// (the folder, its repo for a worktree) with where it lives beside it. No
// Raycast imports: plain node tests it.
import type { UsageRow } from "./cli";
import { barList, ChartTheme, clip, fit, runs, stackedColumns, text, textWidth, Tile, tiles, svgDoc } from "./charts";
import { Fact, facts, hero, logoMark, PANE, Part, severity, stateMark } from "./dash";
import type { LogoFn } from "./stats-view";
import { img } from "./stats-view";
import {
  activeText,
  agentSeries,
  breakdown,
  compact,
  dayLabel,
  fmtMetric,
  Metric,
  metricOf,
  money,
  RangeId,
  shortPath,
  span as rangeSpan,
  Span,
  table,
  vendorAgent,
} from "./usage";

/** What one view of Usage read (usage.tsx `load`). */
export interface Loaded {
  range: RangeId;
  today?: UsageRow;
  week?: UsageRow;
  month?: UsageRow;
  /** the range's total */
  total?: UsageRow;
  days: UsageRow[];
  agents: UsageRow[];
  models: UsageRow[];
  hosts: UsageRow[];
  projects: UsageRow[];
  /** by account (older builds: none) */
  accounts?: UsageRow[];
  perAgent: Record<string, UsageRow[]>;
  /** the daemon answered (else the CLI, which also says what it couldn't read) */
  fromDaemon: boolean;
  unpriced: string[];
  errors: { host: string; error: string | null }[];
  /** this aleph has no `usage` */
  missing?: boolean;
  /** plan windows from the logs: host → agent → id → snapshot */
  limits?: Limits;
  /** `aleph usage sources --json` */
  sources?: Sources;
}

export interface LimitWindow {
  used_percent: number;
  minutes?: number | null;
  resets_at?: number | null;
}
export type Limits = Record<string, Record<string, Record<string, LimitSnapshot>>>;
export interface LimitSnapshot {
  ts: number;
  plan?: string | null;
  windows: LimitWindow[];
  /** "" or missing: a log's last snapshot; `live`: asked of the agent; `cache`: the agent's own */
  source?: string;
  /** a hash of whose account it is (two different ones of an agent are told apart) */
  account?: string;
  /** every machine on this account, this one first */
  hosts?: string[];
}

export interface SourceRun {
  at?: number;
  error?: string | null;
  took_ms?: number;
  records?: number;
  sqlite3?: boolean;
  agents?: { agent: string; listed: number; to_read: number; unchanged: number; roots: string[] }[];
}
export interface Sources {
  hosts: {
    host: string;
    enabled: boolean;
    files_tracked: number;
    bytes_tracked: number;
    ok_at?: number | null;
    last_run?: SourceRun | null;
  }[];
  no_reader?: { agent: string; why: string }[];
  store?: { dir: string; months: string[] };
}

/** `5h`, `7 days`: a plan window's length in words. */
export function windowName(min?: number | null): string {
  if (!min) return "window";
  if (min % 10080 === 0) return min === 10080 ? "week" : `${min / 10080} weeks`;
  if (min % 1440 === 0) return min === 1440 ? "day" : `${min / 1440} days`;
  if (min % 60 === 0) return min === 60 ? "hour" : `${min / 60} hours`;
  return `${min} min`;
}

/**
 * A plan's name as its maker sells it: ChatGPT's (Codex's) plan types —
 * `prolite` → `Pro $100`, `pro` → `Pro $200` — and anyone else's as given
 * (Claude's already read `Max 5x`, `Pro`).
 */
export function planName(p?: string | null, agent?: string): string {
  const plan = (p ?? "").trim();
  if (agent && agent !== "codex") return plan;
  const chatgpt: Record<string, string> = {
    free: "Free",
    plus: "Plus",
    pro: "Pro $200",
    prolite: "Pro $100",
    team: "Team",
    business: "Business",
    enterprise: "Enterprise",
    edu: "Edu",
  };
  return chatgpt[plan.toLowerCase()] ?? plan;
}

export interface CardOpts {
  t: ChartTheme;
  logo: LogoFn;
  brand: (id: string) => { name: string; color: string };
  hostColor: (alias: string) => string | undefined;
  width?: number;
  now?: number;
}

/** A small rounded tag at the right edge ending at `right`; returns its left edge. */
function tag(right: number, y: number, s: string, color: string, t: ChartTheme): [string, number] {
  const w = textWidth(s, 11) + 16;
  const x = right - w;
  return [
    `<rect x="${x.toFixed(1)}" y="${y - 13}" width="${w.toFixed(1)}" height="18" rx="9" fill="${color}" fill-opacity="${t.dark ? 0.18 : 0.13}"/>` +
      text(x + w / 2, y, s, color, 11, ' text-anchor="middle" font-weight="600"'),
    x,
  ];
}

/**
 * The plan windows the agents' logs report, fullest first: per machine and
 * agent its logo, name, machine and plan, then one meter per window (its
 * length, how full, when it resets).
 */
export function limitsCard(l: Limits | undefined, o: CardOpts): string {
  const { t } = o;
  const w = o.width ?? PANE;
  const now = o.now ?? Date.now() / 1000;
  const groups = Object.entries(l ?? {}).flatMap(([host, agents]) =>
    Object.entries(agents ?? {}).flatMap(([agent, ids]) =>
      // a snapshot without windows still says whose account and plan (no reading there yet)
      Object.values(ids ?? {})
        .filter((snap) => snap.windows?.length || snap.account)
        .map((snap) => ({ host, agent, snap, top: Math.max(-1, ...(snap.windows ?? []).map((x) => x.used_percent)) })),
    ),
  );
  if (!groups.length) return svgDoc(w, 20, text(0, 14, "No plan limits in the logs yet", t.muted, 13), "no limits");
  groups.sort((a, b) => b.top - a.top || a.host.localeCompare(b.host));
  const parts: string[] = [];
  // columns shared by every row: the window, its bar, how full (right-aligned), when it resets
  // a window that reset after the reading (or a reading older than the
  // window itself) starts again from nothing: 0 %
  const lapsedAt = (win: LimitWindow, ts: number) =>
    win.resets_at ? win.resets_at <= now : !!win.minutes && ts + win.minutes * 60 <= now;
  const resetsOf = (win: LimitWindow, ts = now) =>
    lapsedAt(win, ts) ? "reset since" : win.resets_at ? `resets in ${fmtLeft(win.resets_at - now)}` : "";
  // an agent signed in to different accounts on different machines: say which is which
  const accountsOf = new Map<string, Set<string>>();
  for (const g of groups)
    if (g.snap.account) accountsOf.set(g.agent, (accountsOf.get(g.agent) ?? new Set()).add(g.snap.account));
  const labelW = 70;
  const resetW = Math.max(
    0,
    ...groups.flatMap((g) => g.snap.windows.map((x) => textWidth(resetsOf(x, g.snap.ts), 11) * 1.08)),
  );
  const pctW = 40;
  const pctX = w - (resetW ? resetW + 16 : 0);
  const barW = pctX - pctW - 14 - labelW;
  let y = 0;
  groups.forEach((g, gi) => {
    if (gi) {
      parts.push(`<line x1="0" x2="${w}" y1="${y + 0.5}" y2="${y + 0.5}" stroke="${t.grid}" stroke-width="1"/>`);
      y += 14;
    }
    const b = o.brand(g.agent);
    const cy = y + 13;
    parts.push(logoMark(0, cy - 13, 18, o.logo("agent", g.agent), { letter: b.name, color: b.color }, t));
    // the tags: account (when it has several), plan, how old a log's reading
    // is (as old as the last turn that wrote it: said past an hour) — all on
    // the name's line: shortened when they don't fit there (`beef` for
    // `account beef`, the age moved under the name)
    const age = now - g.snap.ts;
    const old = age > (g.snap.source ? 86400 : 3600);
    const acct = g.snap.account && (accountsOf.get(g.agent)?.size ?? 0) > 1 ? g.snap.account.slice(0, 4) : "";
    const plan = g.snap.plan ? planName(g.snap.plan, g.agent) : "";
    // stage 0: every tag; 1: the age under the name; 2: and the account's hash alone
    const tagsOf = (stage: number): [string, string][] => [
      ...(acct ? [[stage > 1 ? acct : `account ${acct}`, t.state.working] as [string, string]] : []),
      ...(plan ? [[plan, t.accent] as [string, string]] : []),
      ...(old && stage === 0 ? [[`as of ${fmtLeft(age)} ago`, t.muted] as [string, string]] : []),
    ];
    const width = (v: [string, string][]) => v.reduce((a, [x]) => a + textWidth(x, 11) + 16 + 6, 0);
    const name = clip(b.name, w - 26, 13);
    const hosts = g.snap.hosts?.length ? g.snap.hosts : [g.host];
    const nameEnd = 26 + textWidth(name, 13) * 1.06 + 12;
    const hostsW = hosts.reduce((a, h) => a + textWidth(h, 12) + 18, 0);
    const stage = [0, 1, 2].find((n) => nameEnd + hostsW + width(tagsOf(n)) <= w) ?? 2;
    const short = stage > 0;
    const tags = tagsOf(stage);
    parts.push(runs(26, cy + 1, [[name, t.text, 13, 600]]));
    // the machines, as many as fit before the tags
    let hx = nameEnd;
    const room = w - width(tags);
    for (const h of hosts) {
      const hw = textWidth(h, 12) + 18;
      if (hx + hw > room) break;
      parts.push(
        `<circle cx="${(hx + 4).toFixed(1)}" cy="${cy - 3.5}" r="4" fill="${fit(o.hostColor(h) ?? t.other, t)}"/>`,
      );
      parts.push(text(hx + 12, cy + 1, h, t.muted, 12));
      hx += hw;
    }
    let right = w;
    for (const [x, c] of [...tags].reverse()) {
      const [svg, nx] = tag(right, cy + 1, x, c, t);
      parts.push(svg);
      right = nx - 6;
    }
    y += 26;
    const wins = g.snap.windows ?? [];
    if (!wins.length) {
      parts.push(text(0, y + 13, "No usage reading here yet", t.muted, 11));
      y += 22;
    } else if (old && short) {
      parts.push(text(0, y + 11, `as of ${fmtLeft(age)} ago`, t.muted, 11));
      y += 18;
    }
    for (const win of wins) {
      const lapsed = lapsedAt(win, g.snap.ts);
      const f = lapsed ? 0 : Math.min(1, Math.max(0, win.used_percent / 100));
      const c = severity(f, t);
      const wname = windowName(win.minutes);
      parts.push(text(0, y + 14, wname[0].toUpperCase() + wname.slice(1), t.muted, 12));
      parts.push(
        `<rect x="${labelW}" y="${y + 6}" width="${barW.toFixed(1)}" height="9" rx="4.5" fill="${c}" fill-opacity="0.2"/>`,
      );
      if (f > 0)
        parts.push(
          `<rect x="${labelW}" y="${y + 6}" width="${Math.max(5, barW * f).toFixed(1)}" height="9" rx="4.5" fill="${c}"/>`,
        );
      parts.push(
        text(
          pctX,
          y + 14.5,
          lapsed ? "0%" : `${Math.round(win.used_percent)}%`,
          t.text,
          13,
          ' text-anchor="end" font-weight="600"',
        ),
      );
      const resets = resetsOf(win, g.snap.ts);
      if (resets) parts.push(text(w - resetW, y + 14, resets, t.muted, 11));
      y += 24;
    }
    y += 4;
  });
  return svgDoc(
    w,
    y,
    parts.join(""),
    groups
      .map(
        (g) =>
          `${g.host} ${o.brand(g.agent).name}: ${g.snap.windows.map((x) => `${windowName(x.minutes)} ${Math.round(x.used_percent)}%`).join(", ")}`,
      )
      .join("; "),
  );
}

/** The plan limits, for the list's detail. */
export function limitsMarkdown(l: Limits | undefined, o: CardOpts): string {
  return img("Plan limits", limitsCard(l, o));
}

export function fmtLeft(s: number): string {
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  return d ? `${d}d ${h}h` : h ? `${h}h ${m}m` : `${m}m`;
}

function bytes(n: number): string {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 10 || i === 0 ? Math.round(v) : v.toFixed(1)} ${u[i]}`;
}

/** `just now`, `6m ago`, `2h 5m ago`. */
const ago = (secs: number) => (secs < 60 ? "just now" : `${fmtLeft(secs)} ago`);

/**
 * What each machine's logs gave, as a table: a band per machine (its name,
 * what it tracks, when it was read), then a row per agent in the same columns
 * everywhere: the agent, its folder, how many files, what's left to read (or
 * what it needs). Agents with the most files first.
 */
export function sourcesCard(s: Sources | undefined, o: CardOpts): string {
  const { t } = o;
  const w = o.width ?? PANE;
  const now = o.now ?? Date.now() / 1000;
  if (!s) return svgDoc(w, 20, text(0, 14, "Couldn't read aleph usage sources", t.muted, 13), "no sources");
  // the columns: agent · folder · files (right-aligned) · state
  const nameX = 32;
  const pathX = 130;
  const stateX = w - 76;
  const filesX = stateX - 10;
  const parts: string[] = [];
  let y = 0;
  s.hosts.forEach((h, hi) => {
    if (hi) y += 14;
    const r = h.last_run;
    // the machine's band
    parts.push(
      `<rect x="0" y="${y}" width="${w}" height="30" rx="8" fill="${t.grid}" fill-opacity="${t.dark ? 0.5 : 0.55}"/>`,
      `<circle cx="14" cy="${y + 15}" r="5" fill="${fit(o.hostColor(h.host) ?? t.other, t)}"/>`,
    );
    const files = `${h.files_tracked} file${h.files_tracked === 1 ? "" : "s"} · ${bytes(h.bytes_tracked)}`;
    parts.push(
      runs(
        26,
        y + 19.5,
        [
          [clip(h.host, 120, 13), t.text, 13, 700],
          [files, t.muted, 11],
        ],
        8,
      ),
    );
    const when = !h.enabled ? "off" : r?.at ? `read ${ago(Math.max(0, now - r.at))}` : "not read yet";
    parts.push(text(w - 10, y + 19, when, h.enabled ? t.muted : t.state.warn, 11, ' text-anchor="end"'));
    y += 30;
    if (r?.error) {
      parts.push(stateMark("warn", 18, y + 13, t.state.warn));
      parts.push(text(nameX, y + 17, clip(r.error, w - nameX, 11), t.state.warn, 11));
      y += 24;
    }
    const agents = (r?.agents ?? []).filter((x) => x.listed > 0).sort((a, b) => b.listed - a.listed);
    agents.forEach((a, i) => {
      const b = o.brand(a.agent);
      const cy = y + 17;
      if (i)
        parts.push(
          `<line x1="${nameX}" x2="${w}" y1="${y + 0.5}" y2="${y + 0.5}" stroke="${t.grid}" stroke-width="1"/>`,
        );
      parts.push(logoMark(10, cy - 12, 16, o.logo("agent", a.agent), { letter: b.name, color: b.color }, t));
      parts.push(text(nameX, cy, clip(b.name, pathX - nameX - 8, 12), t.text, 12, ' font-weight="600"'));
      parts.push(text(pathX, cy, clip(a.roots.map(shortPath).join("  "), filesX - 52 - pathX, 11, true), t.muted, 11));
      parts.push(text(filesX, cy, `${a.listed}`, t.text, 12, ' text-anchor="end"'));
      const needs = a.agent === "opencode" && r?.sqlite3 === false;
      const [state, color] = needs
        ? ["needs sqlite3", t.state.warn]
        : a.to_read
          ? [`${a.to_read} to read`, t.state.working]
          : ["up to date", t.muted];
      parts.push(text(stateX, cy, state, color, 11));
      y += 26;
    });
  });
  return svgDoc(
    w,
    Math.max(1, y),
    parts.join(""),
    s.hosts.map((h) => `${h.host}: ${h.files_tracked} files`).join("; "),
  );
}

/** The sources, for the list's detail. */
export function sourcesMarkdown(s: Sources | undefined, o: CardOpts): string {
  return img("Sources", sourcesCard(s, o));
}

export type Kind = "agent" | "model" | "host" | "project" | "account";

export interface Named {
  /** the folder's name (`site`), the repo for a worktree, `~` for a home */
  name: string;
  /** where it lives, short (`~/work`), "" for a home */
  parent: string;
  /** the worktree's own name when it is one */
  worktree?: string;
}

/**
 * A project folder as a person would name it: `/home/u/site` → `site` in
 * `~`; `/…/Projects/aleph/.claude/worktrees/agent-1` → `aleph`, worktree
 * `agent-1`, in `~/…/Projects`; "" → `(no folder)`.
 */
export function projectName(path: string): Named {
  const p = path.trim().replace(/\/+$/, "");
  if (!p) return { name: "(no folder)", parent: "" };
  // a project's id (usage by project): r:<remote>, n:<rule>, c:<first commit>, d:<machine>:<folder>
  const id = /^([rncd]):(.*)$/.exec(p);
  if (id) {
    const [, kind, rest] = id;
    if (kind === "r") {
      const i = rest.lastIndexOf("/");
      return i < 0 ? { name: rest, parent: "" } : { name: rest.slice(i + 1), parent: rest.slice(0, i) };
    }
    if (kind === "n") return { name: rest, parent: "" };
    if (kind === "c") return { name: `repo ${rest.slice(0, 8)}`, parent: "" };
    const j = rest.indexOf(":");
    const named = projectName(j < 0 ? rest : rest.slice(j + 1));
    return { ...named, parent: [j < 0 ? "" : rest.slice(0, j), named.parent].filter(Boolean).join(":") };
  }
  const wt = /^(.*?)\/(?:\.claude\/worktrees|\.worktrees|worktrees|\.git\/worktrees)\/([^/]+)$/.exec(p);
  if (wt) {
    const repo = projectName(wt[1]);
    return { name: repo.name, parent: repo.parent, worktree: wt[2] };
  }
  const short = shortPath(p);
  if (short === "~") return { name: "~", parent: "home" };
  const i = short.lastIndexOf("/");
  if (i < 0) return { name: short, parent: "" };
  return { name: short.slice(i + 1), parent: short.slice(0, i) || "/" };
}

/** `site · ~/work` / `aleph · worktree agent-1 · ~/work/Projects`, for search and copying. */
export const projectLabel = (path: string) => {
  const n = projectName(path);
  return [n.name, n.worktree ? `worktree ${n.worktree}` : "", n.parent].filter(Boolean).join(" · ");
};

/** `0.31` of the range's metric (0 when there's nothing). */
export function shareOf(r: UsageRow | undefined, total: UsageRow | undefined, m: Metric): number {
  const all = metricOf(total, m);
  return all > 0 ? Math.min(1, metricOf(r, m) / all) : 0;
}

/** The figure a row shows in the list: `$0.12` or `72.2k`, with its estimate mark. */
export function figure(r: UsageRow, m: Metric): string {
  return m === "cost" ? `${money(r.cost, r.unpriced_requests)}${r.estimated ? "*" : ""}` : compact(r.tokens);
}

/** A tile: the metric big, the other figures under it. */
export function periodTile(label: string, r: UsageRow | undefined, m: Metric): Tile {
  const cost = money(r?.cost ?? 0, r?.unpriced_requests ?? 0);
  const tokens = `${compact(r?.tokens ?? 0)} tokens`;
  const sessions = `${r?.sessions ?? 0} session${r?.sessions === 1 ? "" : "s"}`;
  const active = r?.active_seconds ? `${activeText(r.active_seconds)} active` : "";
  return m === "cost"
    ? { label, value: cost, suffix: r?.estimated ? "est." : undefined, sub: [tokens, sessions, active].filter(Boolean) }
    : {
        label,
        value: compact(r?.tokens ?? 0),
        suffix: "tokens",
        sub: [cost === "–" ? "no cost" : `${cost}${r?.estimated ? " est." : ""}`, sessions, active].filter(Boolean),
      };
}

/**
 * The days a chart shows: the range from its first day with anything in it
 * (an empty stretch before that says nothing), at least `min` days.
 */
export function shownDays(days: string[], valueOf: (day: string) => number, min = 7): string[] {
  const first = days.findIndex((d) => valueOf(d) > 0);
  if (first < 0) return days;
  return days.slice(Math.min(first, Math.max(0, days.length - min)));
}

/** `last 30 days`, or `since Oct 19` when the chart starts later than the range. */
function spanName(range: Span, shown: string[]): string {
  return shown.length < range.days.length && shown.length ? `since ${dayLabel(shown[0])}` : range.title.toLowerCase();
}

export interface ViewOpts {
  m: Metric;
  t: ChartTheme;
  logo: LogoFn;
  brand: (id: string) => { name: string; color: string };
  hostColor: (alias: string) => string | undefined;
  width?: number;
  /** the range's projects, busiest first (a detail names those first) */
  projectOrder?: string[];
  /** a model's own colour (`keyColors`); its maker's without one */
  modelColor?: (model: string) => string | undefined;
  /** a project's own colour, by its name; the theme's ink without one */
  projectColor?: (name: string) => string | undefined;
  /** an account's own colour (its agent's, told apart) */
  accountColor?: (id: string) => string | undefined;
}

const what = (m: Metric) => (m === "cost" ? "Cost" : "Tokens");

/** The overview: the periods, the range per day by agent, by agent, by machine. */
export function overviewMarkdown(d: Loaded, o: ViewOpts & { filter?: string }): string {
  const { m, t } = o;
  const w = o.width ?? PANE;
  const range = rangeSpan(d.range, new Date(), d.days[0]?.key);
  const fmt = fmtMetric(m);
  const totals = new Map(d.days.map((r) => [r.key, metricOf(r, m)]));
  const days = shownDays(range.days, (day) => totals.get(day) ?? 0);
  const series = agentSeries({
    days,
    dayTotals: d.days,
    byAgent: d.agents,
    perAgent: d.perAgent,
    metric: m,
    colorOf: (a) => o.brand(a).color,
    nameOf: (a) => o.brand(a).name,
    otherColor: t.other,
    top: 5,
  });
  const scope = o.filter ? ` · ${o.filter}` : "";
  const out = [
    img(
      "Today, 7 and 30 days",
      tiles(
        [periodTile("Today", d.today, m), periodTile("7 days", d.week, m), periodTile("30 days", d.month, m)],
        t,
        w,
      ),
    ),
    `**${what(m)} per day, by agent** · ${spanName(range, days)}${scope}`,
    img(
      `${what(m)} per day by agent`,
      stackedColumns({ columns: days.map(dayLabel), series, fmt, label: `${what(m)} per day by agent` }, t, w),
    ),
  ];
  const agents = breakdown({
    rows: d.agents,
    metric: m,
    colorOf: (r) => o.brand(r.key).color,
    labelOf: (r) => o.brand(r.key).name,
    otherColor: t.other,
    max: Math.max(1, d.agents.length),
  });
  if (agents.length > 1)
    out.push(`**By agent**`, img("By agent", barList({ bars: agents, fmt, label: "by agent" }, t, w)));
  const hosts = breakdown({
    rows: d.hosts,
    metric: m,
    colorOf: (r) => o.hostColor(r.key) ?? t.other,
    labelOf: (r) => r.key,
    otherColor: t.other,
    max: Math.max(1, d.hosts.length),
  });
  if (hosts.length > 1)
    out.push(`**By machine**`, img("By machine", barList({ bars: hosts, fmt, label: "by machine" }, t, w)));
  const models = breakdown({
    rows: d.models,
    metric: m,
    colorOf: (r) => rowColor("model", r.key, o),
    labelOf: (r) => r.label || r.key,
    otherColor: t.other,
    max: 8,
  });
  if (models.length > 1)
    out.push(`**By model**`, img("By model", barList({ bars: models, fmt, label: "by model" }, t, w)));
  return out.join("\n\n");
}

/** The colour a row is drawn in: an agent's brand, a model's and a project's own, a machine's own. */
export function rowColor(
  kind: Kind,
  key: string,
  o: Pick<ViewOpts, "brand" | "hostColor" | "t" | "modelColor" | "projectColor" | "accountColor">,
): string {
  if (kind === "agent") return o.brand(key).color;
  if (kind === "account") return o.accountColor?.(key) ?? o.t.muted;
  if (kind === "model") {
    const own = o.modelColor?.(key);
    if (own) return own;
    const a = vendorAgent(key);
    return a ? o.brand(a).color : o.t.other;
  }
  if (kind === "host") return o.hostColor(key) ?? o.t.other;
  // a project's own colour, else the theme's secondary ink (never Aleph's gold)
  return o.projectColor?.(projectName(key).name) ?? o.t.muted;
}

/**
 * One agent's, model's, machine's or project's detail: who it is and its share
 * of the range, its figures, its days (`"loading"` while they're read), the
 * tokens by kind, and where it ran and with what.
 */
export function itemMarkdown(
  o: ViewOpts & {
    kind: Kind;
    row: UsageRow;
    total?: UsageRow;
    range: RangeId;
    days?: UsageRow[] | "loading";
    /** the logo's (kind, id) for the hero: an agent, a maker's agent, a machine's OS */
    logoId?: { kind: "agent" | "os"; id: string };
  },
): string {
  const { m, t, row } = o;
  const w = o.width ?? PANE;
  const range = rangeSpan(o.range, new Date(), Array.isArray(o.days) ? o.days[0]?.key : undefined);
  const fmt = fmtMetric(m);
  const color = rowColor(o.kind, row.key, o);
  const share = shareOf(row, o.total, m);
  let title = row.label || row.key;
  let subtitle = "";
  if (o.kind === "agent") {
    title = o.brand(row.key).name;
    subtitle = `agent · ${row.key}`;
  } else if (o.kind === "model") {
    const a = vendorAgent(row.key);
    subtitle = a ? `model · as used by ${o.brand(a).name}` : "model";
  } else if (o.kind === "host") {
    title = row.key;
    subtitle = "machine";
  } else if (o.kind === "account") {
    title = row.label || "account unknown";
    subtitle = ["account", ...(row.agents ?? []).map((a) => o.brand(a).name)].join(" · ");
  } else {
    const n = projectName(row.key);
    title = n.name;
    subtitle = [n.parent || "project", n.worktree ? `worktree ${n.worktree}` : ""].filter(Boolean).join(" · ");
  }
  const out = [
    img(
      title,
      hero(
        {
          title,
          subtitle,
          color,
          logo: o.logoId ? o.logo(o.logoId.kind, o.logoId.id) : undefined,
          letter: o.kind === "project" ? title.replace(/^[~(]/, "") || "~" : undefined,
          aside: {
            value: figure(row, m).replace(/\*$/, ""),
            label: `${Math.round(share * 100)}% of ${what(m).toLowerCase()}`,
          },
        },
        t,
        w,
      ),
    ),
    img(
      "Figures",
      tiles(
        [
          {
            label: "Cost",
            value: money(row.cost, row.unpriced_requests),
            suffix: row.estimated ? "est." : undefined,
            sub: [
              row.unpriced_requests
                ? `${row.unpriced_requests} unpriced`
                : `${Math.round(shareOf(row, o.total, "cost") * 100)}% of all`,
            ],
          },
          {
            label: "Tokens",
            value: compact(row.tokens),
            sub: [`${compact(row.input)} in`, `${compact(row.output)} out`],
          },
          {
            label: "Sessions",
            value: String(row.sessions),
            sub: [`${row.requests} req`, row.active_seconds ? `${activeText(row.active_seconds)} active` : ""].filter(
              Boolean,
            ),
          },
        ],
        t,
        w,
      ),
    ),
  ];
  if (o.days === "loading" || o.days === undefined) {
    out.push(
      `**${what(m)} per day**`,
      o.days === "loading" ? "_Reading its days…_" : "_Its days aren't kept apart for this one._",
    );
  } else {
    const by = new Map(o.days.map((r) => [r.key, metricOf(r, m)]));
    const days = shownDays(range.days, (day) => by.get(day) ?? 0);
    out.push(
      `**${what(m)} per day** · ${spanName(range, days)}`,
      img(
        `${what(m)} per day`,
        stackedColumns(
          {
            columns: days.map(dayLabel),
            series: [{ label: title, color, values: days.map((dd) => by.get(dd) ?? 0) }],
            fmt,
            label: `${title}: ${what(m).toLowerCase()} per day`,
          },
          t,
          w,
        ),
      ),
    );
  }
  out.push(img("Where and with what", facts(itemFacts(o.kind, row, o), t, w, { title: "Where and with what" })));
  return out.join("\n\n");
}

/** The tokens by kind, then the agents, machines, models and projects it touched, and when. */
export function itemFacts(
  kind: Kind,
  r: UsageRow,
  o: Pick<ViewOpts, "brand" | "hostColor" | "logo" | "t" | "projectOrder" | "modelColor" | "projectColor">,
): Fact[] {
  const rows: Fact[] = [];
  const tok: Part[] = [
    { text: compact(r.input), note: "in" },
    { text: compact(r.output), note: "out" },
  ];
  if (r.cache_read) tok.push({ text: compact(r.cache_read), note: "cache read" });
  if (r.cache_write) tok.push({ text: compact(r.cache_write), note: "cache write" });
  if (r.reasoning) tok.push({ text: compact(r.reasoning), note: "reasoning" });
  rows.push({ label: "Tokens", parts: tok });
  if (kind !== "agent" && r.agents?.length)
    rows.push({
      label: "Agents",
      parts: r.agents.map((a) => ({
        text: o.brand(a).name,
        logo: o.logo("agent", a),
        letter: o.brand(a).name,
        letterColor: o.brand(a).color,
      })),
    });
  // machines, models and projects as chips in their own colours
  const chip = (c: string | undefined) => fit(c ?? o.t.other, o.t);
  if (kind !== "host" && r.hosts?.length)
    rows.push({ label: "Machines", parts: r.hosts.map((h) => ({ text: h, chip: chip(o.hostColor(h)) })) });
  if (kind !== "model" && r.models?.length)
    rows.push({
      label: "Models",
      parts: few(r.models, FEW).map((p) => (p.muted ? p : { ...p, chip: chip(rowColor("model", p.text, o)) })),
    });
  if (kind !== "project" && r.projects?.length) {
    // by name once (a repo's worktrees and the same folder on two machines are one project here), busiest first
    const rank = new Map((o.projectOrder ?? []).map((p, i) => [p, i]));
    const names = [...r.projects]
      .sort((a, b) => (rank.get(a) ?? 1e9) - (rank.get(b) ?? 1e9))
      .map((p) => projectName(p).name);
    rows.push({
      label: "Projects",
      parts: few([...new Set(names)], FEW).map((p) =>
        p.muted ? p : { ...p, chip: chip(o.projectColor?.(p.text) ?? o.t.other) },
      ),
    });
  }
  if (r.first && r.last) rows.push({ label: "Seen", parts: [{ text: span(r.first, r.last) }] });
  return rows;
}

/** How many models or projects a detail names before "+n more". */
const FEW = 6;

/** The first `n` as parts, then `+12 more`, muted. */
function few(list: string[], n: number): Part[] {
  const shown = list.length > n + 1 ? list.slice(0, n) : list;
  const parts: Part[] = shown.map((x) => ({ text: x }));
  if (list.length > shown.length) parts.push({ text: `+${list.length - shown.length} more`, muted: true });
  return parts;
}

/** `Oct 19 13:14 – Oct 22 18:59`, `Oct 22 09:10 – 18:59` within a day. */
function span(a: number, b: number): string {
  const [x, y] = [when(a), when(b)];
  const [dx, dy] = [x.slice(0, x.lastIndexOf(" ")), y.slice(0, y.lastIndexOf(" "))];
  return dx === dy ? `${x} – ${y.slice(y.lastIndexOf(" ") + 1)}` : `${x} – ${y}`;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** `Oct 22 14:05` (local time). */
export function when(t: number): string {
  const d = new Date(t * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${MONTHS[d.getMonth()]} ${d.getDate()} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** The same numbers as Markdown tables (⌘L, and Copy). */
export function tablesMarkdown(d: Loaded, m: Metric, brandName: (id: string) => string, filter?: string): string {
  const range = rangeSpan(d.range);
  const periods = [
    { key: "Today", r: d.today },
    { key: "Last 7 days", r: d.week },
    { key: "Last 30 days", r: d.month },
  ].flatMap((x) => (x.r ? [{ ...x.r, key: x.key }] : []));
  return [
    `## Usage · ${range.title.toLowerCase()}${filter ? ` · ${filter}` : ""}`,
    table("Period", periods, (r) => r.key, m, true),
    table("Agent", d.agents, (r) => brandName(r.key), m),
    table("Model", d.models, (r) => r.label || r.key || "(unknown)", m),
    table("Machine", d.hosts, (r) => r.key, m),
    table("Project", d.projects, (r) => projectLabel(r.key), m),
  ].join("\n\n");
}

/** What needs doing, under the overview: analytics off, machines not read, models without a price ("" when nothing). */
export function notes(d: Loaded, analyticsOn: boolean): string {
  const out: string[] = [];
  if (!analyticsOn) out.push("**Analytics is off**: nothing new is read. Turn it on with `aleph usage on`.");
  for (const e of d.errors)
    out.push(`**${e.host}**: ${e.error ?? "couldn't read its logs"}. Showing what was read before.`);
  if (d.unpriced.length) {
    out.push(
      `No price for ${d.unpriced.map((x) => `\`${x}\``).join(", ")}: \`aleph settings set analytics.prices.${d.unpriced[0]} <in>,<out>\` ($ per million tokens).`,
    );
  }
  return out.join("\n\n");
}

/** Per agent, its fullest plan window (an agent on several machines or accounts: the fullest of them), fullest first. */
export function limitSummary(
  l: Limits | undefined,
  now = Date.now() / 1000,
): { agent: string; top: number; windows: string[] }[] {
  const by = new Map<string, { top: number; windows: string[] }>();
  for (const agents of Object.values(l ?? {}))
    for (const [agent, ids] of Object.entries(agents ?? {}))
      for (const snap of Object.values(ids ?? {}))
        for (const win of snap.windows ?? []) {
          // a window that reset after the reading (or a reading older than the
          // window) starts again from nothing: 0 %
          const reset = win.resets_at ? win.resets_at <= now : !!win.minutes && snap.ts + win.minutes * 60 <= now;
          const used = reset ? 0 : win.used_percent;
          const e = by.get(agent) ?? { top: -1, windows: [] };
          e.top = Math.max(e.top, used);
          e.windows.push(`${windowName(win.minutes)} ${Math.round(used)}%`);
          by.set(agent, e);
        }
  return [...by.entries()].map(([agent, e]) => ({ agent, ...e })).sort((a, b) => b.top - a.top);
}
