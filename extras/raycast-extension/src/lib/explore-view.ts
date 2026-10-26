// Usage beyond the totals (the daemon's `usage.explore`, or `aleph usage
// --explore --json`): the cost broken up by account (or agent, machine,
// model, project) under the total, this period against the one before, the
// days stacked by that split, the week's hours and the last 12 weeks — and
// the insights: per prompt and per session, the costliest prompts, the
// tools, plan value, the streaks. No Raycast imports: plain node tests it.
import type { UsageRow } from "./cli";
import {
  calendarHeat,
  ChartTheme,
  heatWeek,
  histogram,
  lineCompare,
  shareBar,
  stackedColumns,
  Tile,
  tiles,
} from "./charts";
import { PANE } from "./dash";
import { img } from "./stats-view";
import { activeText, compact, dayKey, dayLabel, daysBetween, money } from "./usage";

/** A spread (`usage::Dist`): how many, their sum and mean, percentiles, the biggest, buckets [from, to, count]. */
export interface Dist {
  n: number;
  sum: number | null;
  mean: number | null;
  p50: number | null;
  p90: number | null;
  p99: number | null;
  max: number | null;
  buckets: [number, number, number][];
}

export interface TopPrompt {
  ts: number;
  seconds: number;
  host: string;
  agent: string;
  session: string;
  account: string;
  account_label: string;
  guessed: boolean;
  project: string;
  model: string;
  tokens: number;
  requests: number;
  cost: number | null;
  estimated: boolean;
}

export interface Stats {
  prompt_cost: Dist;
  prompt_seconds: Dist;
  prompt_tokens: Dist;
  prompt_requests: Dist;
  session_cost: Dist;
  session_seconds: Dist;
  top_prompts: TopPrompt[];
  top_sessions: (UsageRow & { prompts?: number })[];
  outside_prompts: { requests: number; cost: number | null };
}

export interface Explore {
  range: string;
  /** account · agent · host · model · project */
  split: string;
  breakup: { key: string; label: string; cost: number | null; tokens: number }[];
  series: { key: string; label: string }[];
  daily: { day: string; label: string; cost: (number | null)[] }[];
  cumulative: { now: (number | null)[]; before: (number | null)[] };
  /** tokens · requests · prompts · active (hours) · cache (0–1), a value a day */
  per_day: Record<string, number[]>;
  models: string[];
  model_mix: { label: string; shares: number[] }[];
  /** Monday first × 24 hours */
  heat: { cost: (number | null)[][]; tokens: number[][] };
  calendar: { day: string; cost: number | null }[];
  tools: { name: string; calls: number; failed: number }[];
  parallel: number;
  insights: { label: string; value: string }[];
  plans: { account: string; plan: string; monthly: number | null; value: number | null }[];
  stats: Stats;
}

export interface ExploreOpts {
  t: ChartTheme;
  /** a part's colour: an account's agent, an agent's brand, a machine's own */
  colorOf: (key: string, label: string) => string;
  width?: number;
}

const n0 = (v: number | null | undefined) => v ?? 0;
const usd = (v: number | null | undefined) => (v === null || v === undefined ? "–" : money(v));

/** What the split is called in a heading. */
export function splitName(split: string): string {
  return { account: "account", agent: "agent", host: "machine", model: "model", project: "project" }[split] ?? split;
}

/** The breakup, the period against the one before, the days by the split, the week's hours, 12 weeks. */
export function exploreMarkdown(x: Explore, o: ExploreOpts): string {
  const { t } = o;
  const w = o.width ?? PANE;
  const by = splitName(x.split);
  const out: string[] = [];
  const total = x.breakup.reduce((s, p) => s + n0(p.cost), 0);
  out.push(`## ${usd(total)} · ${x.range}`);
  if (x.breakup.length) {
    out.push(
      `**The cost by ${by}**`,
      img(
        `the cost by ${by}`,
        shareBar(
          {
            parts: x.breakup.slice(0, 8).map((p) => ({
              label: p.label,
              value: n0(p.cost),
              color: o.colorOf(p.key, p.label),
              note: usd(p.cost),
            })),
            label: `the cost by ${by}`,
          },
          t,
          w,
        ),
      ),
    );
    // the rest, as numbers: every part with its tokens
    const rows = x.breakup.map(
      (p) =>
        `| ${p.label.replace(/\|/g, "/")} | ${usd(p.cost)} | ${total > 0 ? Math.round((n0(p.cost) / total) * 100) : 0}% | ${compact(p.tokens)} |`,
    );
    out.push(
      [`| ${by[0].toUpperCase()}${by.slice(1)} | Cost | Share | Tokens |`, "|---|--:|--:|--:|", ...rows].join("\n"),
    );
  }
  if (x.cumulative.now.length > 1) {
    const now = x.cumulative.now.map(n0);
    const before = x.cumulative.before.map(n0);
    const [a, b] = [now[now.length - 1] ?? 0, before[before.length - 1] ?? 0];
    // a percentage says something only against a period that had some use
    const delta =
      b > 0 && b >= a / 20
        ? ` · ${a >= b ? "▲" : "▼"} ${Math.abs(Math.round(((a - b) / b) * 100))}% on the period before`
        : ` · the period before: ${money(b)}`;
    out.push(
      `**This period against the one before** · so far${delta}`,
      img(
        "this period against the one before",
        lineCompare(
          {
            columns: x.daily.map((d) => d.label),
            now,
            before,
            fmt: (v) => money(v),
            label: "cost so far: this period, the one before",
          },
          t,
          w,
        ),
      ),
    );
  }
  if (x.daily.length && x.series.length) {
    out.push(
      `**A day at a time, by ${by}**`,
      img(
        `a day at a time by ${by}`,
        stackedColumns(
          {
            columns: x.daily.map((d) => dayLabel(d.day)),
            series: x.series.map((s, i) => ({
              label: s.label,
              color: s.key === "other" ? t.other : o.colorOf(s.key, s.label),
              values: x.daily.map((d) => n0(d.cost[i])),
            })),
            fmt: (v) => money(v),
            label: `cost a day by ${by}`,
          },
          t,
          w,
        ),
      ),
    );
  }
  if (x.heat.cost.some((r) => r.some((v) => n0(v) > 0))) {
    out.push(
      "**The week's hours**",
      img(
        "the week's hours",
        heatWeek(
          { cells: x.heat.cost.map((r) => r.map(n0)), fmt: (v) => money(v), label: "cost by weekday and hour" },
          t,
          w,
        ),
      ),
    );
  }
  if (x.calendar.length) {
    // every day of the 12 weeks, the quiet ones too
    const by = new Map(x.calendar.map((d) => [d.day, n0(d.cost)]));
    const last = x.calendar.reduce((m, d) => (d.day > m ? d.day : m), x.calendar[0].day);
    const end = new Date(`${last}T12:00:00`);
    const first = new Date(end.getFullYear(), end.getMonth(), end.getDate() - 83, 12);
    const days = daysBetween(dayKey(first), last).map((day) => ({ day, value: by.get(day) ?? 0 }));
    out.push(
      "**The last 12 weeks**",
      img("the last 12 weeks", calendarHeat({ days, fmt: (v) => money(v), label: "cost a day" }, t, w)),
    );
  }
  return out.join("\n\n");
}

const secs = (v: number | null | undefined) => activeText(Math.round(n0(v)));

/** Per prompt and per session, the costliest prompts, the tools, the plans, and the insight lines. */
export function insightsMarkdown(x: Explore, o: ExploreOpts & { brandName: (id: string) => string }): string {
  const { t } = o;
  const w = o.width ?? PANE;
  const st = x.stats;
  const out: string[] = [];
  const pc = st.prompt_cost;
  if (pc.n > 0) {
    const list: Tile[] = [
      { label: "Per prompt", value: usd(pc.p50), suffix: "median", sub: [`p90 ${usd(pc.p90)}`, `${pc.n} prompts`] },
      {
        label: "Time per prompt",
        value: secs(st.prompt_seconds.p50),
        suffix: "median",
        sub: [`p90 ${secs(st.prompt_seconds.p90)}`, `longest ${secs(st.prompt_seconds.max)}`],
      },
    ];
    if (st.session_cost.n > 0)
      list.push({
        label: "Per session",
        value: usd(st.session_cost.p50),
        suffix: "median",
        sub: [`p90 ${usd(st.session_cost.p90)}`, `${st.session_cost.n} sessions`],
      });
    out.push(img("per prompt and per session", tiles(list, t, w)));
    if (pc.buckets.length)
      out.push(
        "**What a prompt cost**",
        img(
          "what a prompt cost",
          histogram(
            { buckets: pc.buckets, fmt: (v) => (v > 0 ? money(v) : "$0"), label: "prompts by cost", unit: "prompts" },
            t,
            w,
          ),
        ),
      );
  } else {
    out.push("_No prompts finished in this range: only Claude Code and Codex write turn records yet._");
  }
  if (st.top_prompts.length) {
    const rows = st.top_prompts.slice(0, 8).map((p) => {
      const where = [o.brandName(p.agent), p.host, p.project ? p.project.split("/").pop() : ""]
        .filter(Boolean)
        .join(" · ");
      return `| ${usd(p.cost)}${p.estimated ? "*" : ""} | ${secs(p.seconds)} | ${compact(p.tokens)} | ${where} | ${new Date(p.ts * 1000).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })} |`;
    });
    out.push(
      "**The costliest prompts**",
      ["| Cost | Took | Tokens | Where | When |", "|--:|--:|--:|---|---|", ...rows].join("\n"),
    );
  }
  if (x.insights.length) {
    out.push("**Insights**", x.insights.map((i) => `- **${i.label}** · ${i.value}`).join("\n"));
  }
  if (x.plans.length) {
    const rows = x.plans.map((p) => {
      const times = n0(p.monthly) > 0 ? ` · ${(n0(p.value) / n0(p.monthly)).toFixed(1)}×` : "";
      return `- **${p.account}** · ${p.plan}: ${usd(p.value)} of API value in 30 days on ${usd(p.monthly)} a month${times}`;
    });
    out.push("**Plans**", rows.join("\n"));
  }
  if (x.tools.length) {
    const rows = x.tools
      .slice(0, 10)
      .map(
        (tl) =>
          `| ${tl.name} | ${tl.calls} | ${tl.failed ? `${tl.failed} (${Math.round((tl.failed / Math.max(1, tl.calls)) * 100)}%)` : "–"} |`,
      );
    out.push("**Tools**", ["| Tool | Calls | Failed |", "|---|--:|--:|", ...rows].join("\n"));
  }
  if (x.parallel > 1) out.push(`_At most ${x.parallel} sessions at work in one hour._`);
  if (st.outside_prompts.requests > 0)
    out.push(
      `_${st.outside_prompts.requests} requests (${usd(st.outside_prompts.cost)}) fall outside any prompt: agents that don't write turn records._`,
    );
  return out.join("\n\n");
}
