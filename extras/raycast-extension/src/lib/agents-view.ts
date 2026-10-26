// Agents, drawn: each agent (Claude Code, Codex, …) across the fleet — where
// it's installed and at which version, its sessions open now, its usage and
// cost today and over 30 days — and each agent a machine is missing, with
// what installing it takes. `agents --json --local`, the fleet (`ls --json`)
// and `usage --by agent`, as list badges and Markdown with SVG cards. No
// Raycast imports: plain node tests it.
import type { AgentInfo, AgentsHost, AgentsReport, Host, Session, UsageRow } from "./cli";
import { ChartTheme, Tile, tiles } from "./charts";
import { Check, checks, ItemRow, itemRows, notice } from "./cards";
import { hero, Meter, meters, PANE, Part, Pill, pills, sessionList, SessionRow, severity } from "./dash";
import { since, STATE_ORDER, stateOf, STATUS_LABEL } from "./fmt";
import { osId } from "./logo-ids";
import { stateColor } from "./machines-view";
import { explainError } from "./model";
import { img, LogoFn } from "./stats-view";
import { compact, money } from "./usage";

// ---------------------------------------------------------------- versions and paths

/** `/home/me/.local/bin/claude` → `~/.local/bin/claude` (any home, and root's). */
export const tilde = (p: string) => p.replace(/^\/(home|Users)\/[^/]+(?=\/|$)/, "~").replace(/^\/root(?=\/|$)/, "~");

/** Compare two versions part by part, numbers as numbers (`2.1.2` > `1.18.15`, `v0.6.3.post1` > `0.6.3`). */
export function compareVersions(a: string, b: string): number {
  const parts = (v: string) => v.replace(/^v/i, "").split(/[.\-+_ ]/);
  const [x, y] = [parts(a), parts(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const [p, q] = [x[i] ?? "", y[i] ?? ""];
    const [m, n] = [/^\d+$/.test(p), /^\d+$/.test(q)];
    if (m && n && Number(p) !== Number(q)) return Number(p) - Number(q);
    if (m !== n) return p === "" ? -1 : q === "" ? 1 : m ? 1 : -1;
    if (!m && p !== q) return p < q ? -1 : 1;
  }
  return 0;
}

/** The newest of these versions. */
export function newest(versions: (string | null | undefined)[]): string | undefined {
  return versions
    .filter((v): v is string => !!v)
    .reduce<string | undefined>((b, v) => (!b || compareVersions(v, b) > 0 ? v : b), undefined);
}

// ---------------------------------------------------------------- one agent across the fleet

export interface Install {
  host: AgentsHost;
  a: AgentInfo;
}

/** One agent across the machines: where it's installed, where it isn't, where nobody could look. */
export interface AgentGroup {
  id: string;
  name: string;
  color: string;
  kind: string | null;
  /** its registry entry (the first machine's): resume flags, logs, install method */
  info: AgentInfo;
  installs: Install[];
  /** reachable machines without it */
  missing: Install[];
  /** machines that didn't answer */
  offline: AgentsHost[];
  /** the newest version installed anywhere */
  newest?: string;
}

/** Every agent installed on at least one machine (`all`: every agent), registry order. */
export function groupAgents(r: AgentsReport | undefined, all = false): AgentGroup[] {
  const hosts = r?.hosts ?? [];
  const order: string[] = [];
  const byId = new Map<string, AgentGroup>();
  for (const h of hosts) {
    for (const a of h.agents ?? []) {
      let g = byId.get(a.id);
      if (!g) {
        g = { id: a.id, name: a.name, color: a.color, kind: a.kind, info: a, installs: [], missing: [], offline: [] };
        byId.set(a.id, g);
        order.push(a.id);
      }
      (a.installed ? g.installs : g.missing).push({ host: h, a });
    }
  }
  const offline = hosts.filter((h) => !h.reachable);
  return order
    .map((id) => byId.get(id)!)
    .filter((g) => all || g.installs.length > 0)
    .map((g) => ({ ...g, offline, newest: newest(g.installs.map((i) => i.a.version)) }));
}

/** Older than the newest version on another machine. */
export const behind = (a: AgentInfo, newestVersion?: string) =>
  !!(a.version && newestVersion && compareVersions(a.version, newestVersion) < 0);

/** What's off about one install, in a few words each (empty: nothing). */
export function installIssues(a: AgentInfo, newestVersion?: string): string[] {
  const out: string[] = [];
  if (a.on_path === false) out.push("not on herdr's PATH");
  if (a.shadowed_by) out.push(`shadowed by ${tilde(a.shadowed_by)}`);
  if (a.integration_state === "outdated") out.push("integration outdated");
  if (behind(a, newestVersion)) out.push(`${a.version}, behind ${newestVersion}`);
  return out;
}

/** herdr's integration for it, in a word or two ("" when the agent has none). */
export function integrationText(a: AgentInfo): string {
  switch (a.integration_state) {
    case "current":
      return "integrated";
    case "outdated":
      return "integration outdated";
    case "not_installed":
      return a.integration ? "not integrated" : "";
    default:
      return "";
  }
}

/** Every machine's issues with this agent: `b31: not on herdr's PATH`. */
export function groupIssues(g: AgentGroup): string[] {
  return g.installs.flatMap((i) => installIssues(i.a, g.newest).map((x) => `${i.host.alias}: ${x}`));
}

// ---------------------------------------------------------------- sessions open now

export interface OpenNow {
  total: number;
  /** per state (blocked · working · limit · done · idle) */
  by: Record<string, number>;
  sessions: Session[];
}

/** Does a session run this agent (its id, or herdr's kind for it)? */
const runs = (s: Pick<Session, "agent">, ids: (string | null | undefined)[]) => ids.includes(s.agent);

/** The sessions open now running this agent, on every machine (or `only` one), what needs you first. */
export function openNow(hosts: Host[] | undefined, ids: (string | null | undefined)[], only?: string): OpenNow {
  const sessions = (hosts ?? [])
    .filter((h) => !only || h.alias === only)
    .flatMap((h) => h.sessions.map((s) => ({ ...s, host: s.host || h.alias })))
    .filter((s) => runs(s, ids))
    .sort((a, b) => STATE_ORDER.indexOf(stateOf(a)) - STATE_ORDER.indexOf(stateOf(b)));
  const by: Record<string, number> = {};
  for (const s of sessions) by[stateOf(s)] = (by[stateOf(s)] ?? 0) + 1;
  return { total: sessions.length, by, sessions };
}

/** `1 needs you · 2 working · 1 idle` (what's there, in the order it matters). */
export function openText(o: OpenNow): string {
  if (!o.total) return "none open";
  return STATE_ORDER.filter((s) => o.by[s])
    .map((s) => `${o.by[s]} ${STATUS_LABEL[s] ?? s}`)
    .join(" · ");
}

// ---------------------------------------------------------------- usage

/** This agent's usage row (keyed by its id or herdr's kind). */
export function usageOf(rows: UsageRow[] | undefined, ids: (string | null | undefined)[]): UsageRow | undefined {
  return rows?.find((r) => ids.includes(r.key));
}

/** `$38.39*` (`*`: part of it priced from Aleph's table), `?` without prices, `–` for nothing. */
export const costText = (r: UsageRow | undefined) =>
  r ? `${money(r.cost, r.unpriced_requests)}${r.estimated && r.cost > 0 ? "*" : ""}` : "–";

/** `$38.39* · 72.1M tokens` */
export const usageText = (r: UsageRow | undefined) =>
  r && (r.cost > 0 || r.tokens > 0) ? `${costText(r)} · ${compact(r.tokens)} tokens` : "nothing yet";

/** Plan windows from the agents' own logs: machine → agent → account → its latest snapshot (usage-view's `Limits`). */
export type PlanLimits = Record<
  string,
  Record<
    string,
    Record<
      string,
      {
        ts: number;
        plan?: string | null;
        windows: { used_percent: number; minutes?: number | null; resets_at?: number | null }[];
      }
    >
  >
>;

/** `5h`, `week`, `2 days`. */
export function windowText(min?: number | null): string {
  if (!min) return "window";
  if (min % 10080 === 0) return min === 10080 ? "week" : `${min / 10080} weeks`;
  if (min % 1440 === 0) return min === 1440 ? "day" : `${min / 1440} days`;
  if (min % 60 === 0) return `${min / 60}h`;
  return `${min}m`;
}

/** `5d 4h`, `3h 10m`, `12m`. */
function left(s: number): string {
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  return d ? `${d}d ${h}h` : h ? `${h}h ${m}m` : `${m}m`;
}

/** This agent's plan windows on every machine, as meters: `b31 · week` 69 %, `resets 5d 4h`. */
export function limitMeters(
  l: PlanLimits | undefined,
  ids: (string | null | undefined)[],
  now: number,
  t: ChartTheme,
): { plan?: string; rows: Meter[] } {
  const rows: Meter[] = [];
  let plan: string | undefined;
  for (const [host, agents] of Object.entries(l ?? {})) {
    for (const [agent, accounts] of Object.entries(agents ?? {})) {
      if (!ids.includes(agent)) continue;
      for (const snap of Object.values(accounts ?? {})) {
        plan ??= snap.plan ?? undefined;
        for (const w of snap.windows ?? []) {
          const f = Math.min(1, Math.max(0, w.used_percent / 100));
          const resets = w.resets_at ? (w.resets_at > now ? `resets ${left(w.resets_at - now)}` : "reset") : "";
          rows.push({
            label: `${host} · ${windowText(w.minutes)}`,
            frac: f,
            value: [`${Math.round(w.used_percent)}%`, resets].filter(Boolean).join(" · "),
            color: severity(f, t),
          });
        }
      }
    }
  }
  return { plan, rows };
}

function usageTile(label: string, r: UsageRow | undefined): Tile {
  const sessions = r?.sessions ? `${r.sessions} session${r.sessions === 1 ? "" : "s"}` : "";
  return {
    label,
    value: r ? costText(r) : "–",
    sub: [r?.tokens ? `${compact(r.tokens)} tokens` : "no tokens", sessions].filter(Boolean),
  };
}

// ---------------------------------------------------------------- row badges

/** A row's accessory before it becomes Raycast's. */
export interface AgentBadge {
  kind: "warn" | "needs" | "working" | "open" | "cost" | "version" | "machines";
  text: string;
  tooltip: string;
}

/**
 * An agent's row: a warning when an install needs a look, its sessions open
 * now (who needs you first), today's cost; with the detail hidden, also the
 * newest version and how many machines have it.
 */
export function agentBadges(
  g: AgentGroup,
  o: { open: OpenNow; today?: UsageRow; month?: UsageRow; wide?: boolean },
): AgentBadge[] {
  const out: AgentBadge[] = [];
  const issues = groupIssues(g);
  if (issues.length) out.push({ kind: "warn", text: "", tooltip: issues.join("\n") });
  const { open } = o;
  const tip = `${open.total} open: ${openText(open)}`;
  if (open.by.blocked) out.push({ kind: "needs", text: String(open.by.blocked), tooltip: tip });
  else if (open.by.working) out.push({ kind: "working", text: String(open.by.working), tooltip: tip });
  else if (open.total) out.push({ kind: "open", text: String(open.total), tooltip: tip });
  if (o.today && o.today.cost > 0)
    out.push({
      kind: "cost",
      text: costText(o.today),
      tooltip: [
        `Today ${usageText(o.today)}`,
        o.month ? `30 days ${usageText(o.month)}` : "",
        o.today.estimated ? "* priced from Aleph's table" : "",
      ]
        .filter(Boolean)
        .join("\n"),
    });
  // versions and paths are the Doctor's: here, where it runs
  if (o.wide) {
    out.push({
      kind: "machines",
      text: g.installs.map((i) => i.host.alias).join(" "),
      tooltip: `on ${g.installs.map((i) => i.host.alias).join(", ")}`,
    });
  }
  return out;
}

/** The list's summary line: `3 installed · 5 open · $38.39* today`. */
export function agentsSummary(groups: AgentGroup[], open: number, today?: UsageRow[]): string {
  const cost = (today ?? []).reduce((s, r) => s + r.cost, 0);
  const est = (today ?? []).some((r) => r.estimated && r.cost > 0);
  return [
    `${groups.length} installed`,
    open ? `${open} open` : "",
    cost > 0 ? `${money(cost)}${est ? "*" : ""} today` : "",
  ]
    .filter(Boolean)
    .join(" · ");
}

// ---------------------------------------------------------------- the agent's detail

export interface AgentView {
  g: AgentGroup;
  open: OpenNow;
  today?: UsageRow;
  month?: UsageRow;
  /** usage has been read (false: still loading, or analytics is off) */
  usage: boolean;
  /** plan windows (today's usage answer's `limits`) */
  limits?: PlanLimits;
  t: ChartTheme;
  logo: LogoFn;
  /** a session's status in words (cli.ts `statusText`) */
  statusText?: (s: Session) => string;
  now?: number;
  width?: number;
}

/**
 * One machine's row in an agent's detail: ready or what needs a look, and its
 * sessions there (versions and paths are the Doctor's); missing or offline, muted.
 */
function machineRow(i: Install, g: AgentGroup, t: ChartTheme, logo: LogoFn, open?: OpenNow): ItemRow {
  const issues = installIssues(i.a, g.newest);
  const os = logo("os", osId(i.host.os_name, i.host.os)?.id);
  const here = open?.sessions.filter((s) => s.host === i.host.alias).length ?? 0;
  return {
    name: i.host.alias,
    logo: os,
    letter: os ? undefined : i.host.alias,
    letterColor: t.other,
    mark: issues.length ? { kind: "warn", color: t.state.warn } : { kind: "check", color: t.state.done },
    sub: issues.length ? issues[0] : "ready",
    value: here ? `${here} open` : "–",
    valueColor: here ? undefined : t.muted,
  };
}

/** The machines: where it is (ready or not, its sessions), then where it isn't, then who didn't answer. */
export function machineRows(g: AgentGroup, t: ChartTheme, logo: LogoFn, only?: string, open?: OpenNow): ItemRow[] {
  const keep = (h: AgentsHost) => !only || h.alias === only;
  const missing = (i: Install): ItemRow => {
    const os = logo("os", osId(i.host.os_name, i.host.os)?.id);
    const tag = missingTag(i.a);
    return {
      name: i.host.alias,
      logo: os,
      letter: os ? undefined : i.host.alias,
      letterColor: t.other,
      mark: { kind: "ring", color: t.other },
      muted: true,
      value: "–",
      valueColor: t.muted,
      sub: `not installed · ${tag.text}`,
    };
  };
  return [
    ...g.installs.filter((i) => keep(i.host)).map((i) => machineRow(i, g, t, logo, open)),
    ...g.missing.filter((i) => keep(i.host)).map(missing),
    ...g.offline.filter(keep).map((h) => ({
      name: h.alias,
      mark: { kind: "down" as const, color: t.state.down },
      muted: true,
      sub: explainError(h.error).head,
      value: "offline",
      valueColor: t.state.down,
    })),
  ];
}

/** What the agent is, wherever it runs: how it resumes, where its logs are, whether it survives a herdr restart. */
export function aboutChecks(g: AgentGroup): Check[] {
  const a = g.installs[0]?.a ?? g.info;
  const out: Check[] = [];
  if (a.resume?.length) out.push({ label: "Resume", state: "info", value: a.resume.join(" ") });
  if (a.session_logs) out.push({ label: "Session logs", state: "info", value: a.session_logs });
  const kept = g.installs.filter((i) => i.a.persistence).map((i) => i.host.alias);
  out.push({
    label: "Survives a herdr restart",
    state: kept.length ? "ok" : "off",
    value:
      kept.length === g.installs.length ? (kept.length ? "yes" : "no") : kept.length ? `on ${kept.join(", ")}` : "no",
  });
  return out;
}

/** An agent's whole detail: the card, today and 30 days, its sessions open now, its machines, how it resumes. */
export function agentMarkdown(o: AgentView): string {
  const { g, t } = o;
  const w = o.width ?? PANE;
  const now = o.now ?? Date.now() / 1000;
  const issues = groupIssues(g);
  const tags: Part[] = [];
  if (issues.length) tags.push({ text: issues.length === 1 ? issues[0] : `${issues.length} issues`, warn: true });
  const machines = g.installs.length + g.missing.length + g.offline.length;
  // each machine's OS logo, for its sessions
  const osOf = new Map([...g.installs, ...g.missing].map((i) => [i.host.alias, osId(i.host.os_name, i.host.os)?.id]));
  const out = [
    img(
      `${g.name}: ${g.installs.length} machines`,
      hero(
        {
          title: g.name,
          color: g.color,
          logo: o.logo("agent", g.id),
          letter: g.name,
          subtitle: [g.info.command, `on ${g.installs.length} of ${machines} machine${machines === 1 ? "" : "s"}`]
            .filter(Boolean)
            .join(" · "),
          tags,
          aside: o.open.total ? { value: String(o.open.total), label: "open now" } : undefined,
        },
        t,
        w,
      ),
    ),
  ];
  if (o.usage && (o.today || o.month))
    out.push(
      img("Usage today and over 30 days", tiles([usageTile("Today", o.today), usageTile("30 days", o.month)], t, w)),
    );
  const limits = limitMeters(o.limits, [g.id, g.kind], now, t);
  if (limits.rows.length)
    out.push(
      img(
        "Plan limits",
        meters(limits.rows, t, w, { title: limits.plan ? `Plan · ${limits.plan}` : "Plan", labelW: 100 }),
      ),
    );
  if (o.open.total) {
    const pillList: Pill[] = STATE_ORDER.filter((s) => o.open.by[s]).map((s) => ({
      n: o.open.by[s],
      label: STATUS_LABEL[s] ?? s,
      state: s,
      color: stateColor(s, t),
    }));
    out.push(img("Open now", pills(pillList, t, w)));
    const shown = o.open.sessions.slice(0, 6);
    const rows: SessionRow[] = shown.map((s) => {
      const st = stateOf(s);
      const folder = s.cwd && s.cwd !== "-" ? tilde(s.cwd).split("/").pop() || "~" : "";
      return {
        name: s.name || s.display || s.pane_id,
        title: [s.host, folder].filter(Boolean).join(" · "),
        state: st,
        color: stateColor(st, t),
        status: o.statusText
          ? o.statusText(s)
          : [STATUS_LABEL[st] ?? st, since(s.status_since, now)].filter(Boolean).join(" · "),
        // where it runs (the agent is the same on every row)
        logo: o.logo("os", osOf.get(s.host)),
        agentLetter: s.host,
        agentColor: t.other,
      };
    });
    out.push(img("Sessions", sessionList(rows, t, w)));
    if (o.open.total > shown.length) out.push(`+${o.open.total - shown.length} more`);
  }
  out.push(img("Machines", itemRows(machineRows(g, t, o.logo, undefined, o.open), t, w, { title: "Machines" })));
  out.push(img("About", checks(aboutChecks(g), t, w)));
  return out.join("\n\n");
}

// ---------------------------------------------------------------- an agent a machine is missing

/** How it would install, in a word: the method (ready), what's missing first, or manual. */
export function missingTag(a: AgentInfo): { text: string; tone: "ready" | "blocked" | "manual"; tooltip: string } {
  if (a.install_ready || (!a.install_blocker && a.install_command && !(a.prerequisites ?? []).some((p) => !p.ok)))
    return { text: a.install_method ?? "installable", tone: "ready", tooltip: a.install_command ?? "" };
  const need = (a.prerequisites ?? []).find((p) => !p.ok);
  if (need)
    return {
      text: `needs ${need.what.replace(/\s*[≥>=].*$/, "")}`,
      tone: "blocked",
      tooltip: a.install_blocker ?? need.fix ?? "",
    };
  return { text: "manual", tone: "manual", tooltip: a.install_blocker ?? "no install command aleph can run" };
}

/** The site in a blocker (`… — see https://github.com/x/y`). */
export const blockerDocs = (a: Pick<AgentInfo, "install_blocker">) =>
  /https?:\/\/\S+/.exec(a.install_blocker ?? "")?.[0]?.replace(/[).,]+$/, "");

/** A machine's section line: `24 not installed · 8 can install`. */
export function missingSummary(h: AgentsHost): string {
  if (!h.reachable) return explainError(h.error).head;
  const missing = h.agents.filter((a) => !a.installed);
  const ready = missing.filter((a) => missingTag(a).tone === "ready").length;
  return [`${missing.length} not installed`, ready ? `${ready} can install` : ""].filter(Boolean).join(" · ");
}

/** A missing agent's detail: the card, what's in the way, what it needs, the command, where it is installed. */
export function missingMarkdown(o: {
  h: AgentsHost;
  a: AgentInfo;
  /** the same agent on every machine (`groupAgents(r, true)`) */
  g?: AgentGroup;
  t: ChartTheme;
  logo: LogoFn;
  width?: number;
}): string {
  const { h, a, t } = o;
  const w = o.width ?? PANE;
  const tag = missingTag(a);
  const tone = tag.tone === "ready" ? t.state.working : tag.tone === "blocked" ? t.state.warn : t.other;
  const out = [
    img(
      `${a.name} isn't on ${h.alias}`,
      hero(
        {
          title: a.name,
          color: a.color,
          logo: o.logo("agent", a.id),
          letter: a.name,
          subtitle: `not on ${h.alias}`,
          tags: [{ text: tag.text, dot: tone }],
        },
        t,
        w,
      ),
    ),
  ];
  const need = (a.prerequisites ?? []).filter((p) => !p.ok);
  if (tag.tone === "blocked" && need.length)
    out.push(
      img(
        "What's missing",
        notice(
          {
            state: "warn",
            color: t.state.warn,
            title: `Needs ${need.map((p) => p.what).join(" and ")}`,
            detail: need[0].fix ?? undefined,
          },
          t,
          w,
        ),
      ),
    );
  else if (tag.tone === "manual") {
    const docs = blockerDocs(a);
    out.push(
      img(
        "Manual install",
        notice(
          {
            state: "warn",
            color: t.other,
            title: "No install command aleph can run",
            detail: docs ? docs.replace(/^https?:\/\//, "") : undefined,
          },
          t,
          w,
        ),
      ),
    );
  }
  // the checklist when it says more than the notice: some of it is there already
  if (a.prerequisites?.length && need.length < a.prerequisites.length)
    out.push(
      img(
        "Needs",
        checks(
          a.prerequisites.map((p) => ({
            label: p.what,
            state: p.ok ? "ok" : "bad",
            value: p.ok ? tilde(p.detail ?? "") : "missing",
          })),
          t,
          w,
          { title: "Needs" },
        ),
      ),
    );
  if (a.install_command) out.push("```\n" + a.install_command + "\n```");
  if (o.g && o.g.installs.length)
    out.push(
      img(
        "Installed on",
        itemRows(
          machineRows(o.g, t, o.logo).filter((r) => !r.muted),
          t,
          w,
          { title: "Installed on" },
        ),
      ),
    );
  return out.join("\n\n");
}
