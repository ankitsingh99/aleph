// Usage, shaped for the charts: numbers the way `aleph usage` prints them,
// the days of a range, a stacked series per agent (the rest as "Other"),
// bars per model, machine and project. No Raycast imports (plain node tests it).
import type { Bar, Series } from "./charts";
import type { UsageRow } from "./cli";
import { moneyText } from "./showcase";

export type Metric = "cost" | "tokens";

/** The periods Usage offers (⌘1–9): each a `range` the daemon and `aleph usage` take. */
export const RANGES = [
  { id: "24h", title: "Last 24 Hours" },
  { id: "3d", title: "Last 3 Days" },
  { id: "7d", title: "Last 7 Days" },
  { id: "30d", title: "Last 30 Days" },
  { id: "6mo", title: "Last 6 Months" },
  { id: "today", title: "Today" },
  { id: "this-week", title: "This Week" },
  { id: "this-month", title: "This Month" },
  { id: "all", title: "All Time" },
] as const;
/** A preset's id, any `Nd`, or your own `2026-09-01..2026-10-15` (`..` to today). */
export type RangeId = string;

/** A range's name and the local days it covers, oldest first (`all`: from `first`, else 90 days). */
export interface Span {
  title: string;
  days: string[];
}

const noon = (k: string) => {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(k);
  return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]), 12) : undefined;
};

/** Every day from `a` to `b` (both `YYYY-MM-DD`), at most 4000. */
export function daysBetween(a: string, b: string): string[] {
  const [x, y] = [noon(a), noon(b)];
  if (!x || !y || x > y) return [];
  const out: string[] = [];
  for (const d = new Date(x); d <= y && out.length < 4000; d.setDate(d.getDate() + 1)) out.push(dayKey(d));
  return out;
}

export function span(id: RangeId, now = new Date(), first?: string): Span {
  const preset = RANGES.find((r) => r.id === id);
  const today = dayKey(now);
  const title = preset?.title ?? "";
  if (id === "today") return { title, days: [today] };
  if (id === "24h") return { title, days: lastDays(2, now) };
  if (id === "6mo") return { title, days: lastDays(183, now) };
  if (id === "this-week") return { title, days: lastDays(((now.getDay() + 6) % 7) + 1, now) };
  if (id === "this-month") return { title, days: lastDays(now.getDate(), now) };
  if (id === "all") return { title, days: first && first < today ? daysBetween(first, today) : lastDays(90, now) };
  const n = /^(\d+)d$/.exec(id);
  if (n) return { title: title || `Last ${n[1]} Days`, days: lastDays(Math.max(1, Number(n[1])), now) };
  const c = /^(\d{4}-\d{2}-\d{2})?\.\.(\d{4}-\d{2}-\d{2})?$/.exec(id);
  if (c) {
    const [a, b] = [c[1] ?? first ?? today, c[2] ?? today];
    return { title: `${dayLabel(a)} – ${c[2] ? dayLabel(b) : "today"}`, days: daysBetween(a, b) };
  }
  return { title: id, days: lastDays(30, now) };
}

/** `999`, `12.4k`, `3.1M`, `1.2B` (usage/cmd.rs `compact`). */
export function compact(n: number): string {
  const v = Math.max(0, n);
  const trim = (x: number, u: string) => `${x >= 100 ? x.toFixed(0) : x.toFixed(1)}${u}`;
  if (v < 1000) return String(Math.round(v));
  if (v < 1e6) return trim(v / 1e3, "k");
  if (v < 1e9) return trim(v / 1e6, "M");
  return trim(v / 1e9, "B");
}

/** `$142.10`, `$0.03`, `<$0.01`, `–` (`?` when nothing had a price). */
export function money(cost: number, unpriced = 0): string {
  // showcase: hidden amounts read $— (the daemon sends them as 0)
  const hidden = moneyText();
  if (hidden) return hidden;
  if (!(cost > 0)) return unpriced > 0 ? "?" : "–";
  if (cost < 0.01) return "<$0.01";
  return cost < 1000 ? `$${cost.toFixed(2)}` : `$${Math.round(cost)}`;
}

/** `40s`, `16m`, `2h 13m`, `–`. */
export function activeText(secs: number): string {
  const s = Math.round(secs);
  if (s <= 0) return "–";
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  const m = Math.floor((s % 3600) / 60);
  return m ? `${Math.floor(s / 3600)}h ${m}m` : `${Math.floor(s / 3600)}h`;
}

export const metricOf = (r: Pick<UsageRow, "cost" | "tokens"> | undefined, m: Metric) =>
  (m === "cost" ? r?.cost : r?.tokens) ?? 0;

/** A metric's value as text (axis ticks: a plain 0). */
export const fmtMetric =
  (m: Metric) =>
  (v: number): string =>
    v === 0
      ? "0"
      : m === "cost"
        ? v < 0.01
          ? "<$0.01"
          : v < 10
            ? `$${v.toFixed(2)}`
            : `$${Math.round(v)}`
        : compact(v);

/** Local `YYYY-MM-DD` (the day keys of `--by day`). */
export function dayKey(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** The last `n` local days, today included, oldest first. */
export function lastDays(n: number, now = new Date()): string[] {
  const out: string[] = [];
  for (let i = n - 1; i >= 0; i--) {
    const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() - i, 12);
    out.push(dayKey(d));
  }
  return out;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** `2026-10-22` → `Oct 22`. */
export function dayLabel(key: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(key);
  return m ? `${MONTHS[Number(m[2]) - 1] ?? m[2]} ${Number(m[3])}` : key;
}

/** Whose model it is, as the agent whose colour stands for that maker (models have no colour of their own). */
export function vendorAgent(model: string): string | undefined {
  const m = model.toLowerCase();
  if (/claude|opus|sonnet|haiku|anthropic/.test(m)) return "claude";
  if (/gpt|codex|openai|\bo[134](-|$)/.test(m)) return "codex";
  if (/gemini|gemma/.test(m)) return "gemini";
  if (/grok/.test(m)) return "grok";
  if (/qwen/.test(m)) return "qwen";
  if (/kimi|moonshot/.test(m)) return "kimi";
  return undefined;
}

/** `/home/me/x` → `~/x`. */
export const shortPath = (p: string) =>
  p.replace(/^\/(home|Users)\/[^/]+(?=\/|$)/, "~").replace(/^\/root(?=\/|$)/, "~");

/** The other figure, for a note: tokens beside a cost, the cost beside tokens (`*`: estimated, as in the list). */
function otherFigure(r: UsageRow, m: Metric): string {
  if (m === "cost") return `${compact(r.tokens)} tok`;
  const c = money(r.cost, r.unpriced_requests);
  return c === "–" ? "" : `${c}${r.estimated ? "*" : ""}`;
}

/**
 * The stacked series over `days`: the top agents by `m` (each its own
 * colour), then the rest of each day's total as "Other". `perAgent` holds
 * each agent's `--by day` rows; `dayTotals` everyone's.
 */
export function agentSeries(o: {
  days: string[];
  dayTotals: UsageRow[];
  byAgent: UsageRow[];
  perAgent: Record<string, UsageRow[]>;
  metric: Metric;
  colorOf: (agent: string) => string;
  nameOf: (agent: string) => string;
  otherColor: string;
  top?: number;
}): Series[] {
  const m = o.metric;
  const fmt = fmtMetric(m);
  const at = (rows: UsageRow[] | undefined) => {
    const by = new Map((rows ?? []).map((r) => [r.key, metricOf(r, m)]));
    return o.days.map((d) => by.get(d) ?? 0);
  };
  const ranked = [...o.byAgent]
    .filter((r) => metricOf(r, m) > 0 && o.perAgent[r.key])
    .sort((a, b) => metricOf(b, m) - metricOf(a, m))
    .slice(0, o.top ?? 5);
  const series: Series[] = ranked.map((r) => ({
    label: o.nameOf(r.key),
    color: o.colorOf(r.key),
    values: at(o.perAgent[r.key]),
    note: fmt(metricOf(r, m)),
  }));
  const totals = at(o.dayTotals);
  const rest = totals.map((v, i) => Math.max(0, v - series.reduce((s, x) => s + x.values[i], 0)));
  const restSum = rest.reduce((s, v) => s + v, 0);
  // more than rounding: someone else's share
  if (restSum > Math.max(1e-6, totals.reduce((s, v) => s + v, 0) * 1e-6)) {
    series.push({ label: "Other", color: o.otherColor, values: rest, note: fmt(restSum) });
  }
  return series;
}

/** Bars for a breakdown, largest first; past `max`, the rest fold into one "n more" bar. */
export function breakdown(o: {
  rows: UsageRow[];
  metric: Metric;
  colorOf: (r: UsageRow) => string;
  labelOf?: (r: UsageRow) => string;
  otherColor: string;
  max?: number;
}): Bar[] {
  const m = o.metric;
  const max = o.max ?? 8;
  const sorted = [...o.rows].filter((r) => metricOf(r, m) > 0).sort((a, b) => metricOf(b, m) - metricOf(a, m));
  const shown = sorted.length > max ? sorted.slice(0, max - 1) : sorted;
  const bars: Bar[] = shown.map((r) => ({
    label: o.labelOf ? o.labelOf(r) : r.label || r.key,
    value: metricOf(r, m),
    color: o.colorOf(r),
    note: otherFigure(r, m),
  }));
  const more = sorted.slice(shown.length);
  if (more.length) {
    bars.push({
      label: `${more.length} more`,
      value: more.reduce((s, r) => s + metricOf(r, m), 0),
      color: o.otherColor,
    });
  }
  return bars;
}

/** A Markdown table of a breakdown (the charts' numbers, for the table view and the clipboard); largest first unless `keepOrder`. */
export function table(
  title: string,
  rows: UsageRow[],
  labelOf: (r: UsageRow) => string,
  m: Metric,
  keepOrder = false,
): string {
  const esc = (s: string) => s.replace(/\|/g, "\\|");
  const sorted = keepOrder ? rows : [...rows].sort((a, b) => metricOf(b, m) - metricOf(a, m));
  if (!sorted.length) return `| ${title} |\n|---|\n| nothing in this range |`;
  return [
    `| ${title} | Cost | Tokens | Requests | Sessions | Active |`,
    "|---|--:|--:|--:|--:|--:|",
    ...sorted.map(
      (r) =>
        `| ${esc(labelOf(r))} | ${money(r.cost, r.unpriced_requests)}${r.estimated ? " est." : ""} | ${compact(r.tokens)} | ${r.requests} | ${r.sessions} | ${activeText(r.active_seconds)} |`,
    ),
  ].join("\n");
}

/**
 * The categorical palette, in its fixed order (the dataviz reference, checked
 * for colour-blind separation): [dark, light] steps of blue, orange, aqua,
 * yellow, magenta, green, violet, red.
 */
const SERIES: [string, string][] = [
  ["#3987e5", "#2a78d6"],
  ["#d95926", "#eb6834"],
  ["#199e70", "#1baf7a"],
  ["#c98500", "#eda100"],
  ["#d55181", "#e87ba4"],
  ["#008300", "#008300"],
  ["#9085e9", "#6250d6"],
  ["#e66767", "#e34948"],
];

/** FNV-1a: a slot from a name that doesn't change when other names come and go. */
function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193) >>> 0;
  return h;
}

/**
 * Each name's own colour (a model's, a project's). Up to eight names take the
 * palette's slots in its order, alphabetically (so `claude-opus-5` and
 * `claude-opus-5-5` sit side by side as blue and orange); more than that, each
 * takes the slot its name picks, the next free one while any is free, then
 * shared (the chips always carry the name too).
 */
export function keyColors(keys: string[], dark: boolean): Map<string, string> {
  const names = [...new Set(keys)].sort();
  const out = new Map<string, string>();
  const used = new Set<number>();
  names.forEach((k, i) => {
    let slot = i;
    if (names.length > SERIES.length) {
      slot = hash(k) % SERIES.length;
      if (used.size < SERIES.length) while (used.has(slot)) slot = (slot + 1) % SERIES.length;
    }
    used.add(slot);
    out.set(k, SERIES[slot][dark ? 0 : 1]);
  });
  return out;
}
