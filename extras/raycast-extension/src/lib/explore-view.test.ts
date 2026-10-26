// `npm test`: Usage beyond the totals (explore-view.ts), its new charts
// (charts.ts) and every period's days (usage.ts `span`).
import assert from "node:assert/strict";
import { test } from "node:test";
import { calendarHeat, chartTheme, heatWeek, histogram, lineCompare, shareBar } from "./charts";
import { Dist, Explore, exploreMarkdown, insightsMarkdown, splitName } from "./explore-view";
import { daysBetween, span } from "./usage";

const dark = chartTheme(true);
const light = chartTheme(false);
const clean = (svg: string) => {
  assert.ok(svg.startsWith("<svg"), svg.slice(0, 40));
  assert.ok(!/NaN|undefined|Infinity/.test(svg), svg);
};

const dist = (n: number, p50: number, p90: number, buckets: [number, number, number][] = []): Dist => ({
  n,
  sum: p50 * n,
  mean: p50,
  p50,
  p90,
  p99: p90,
  max: p90 * 2,
  buckets,
});

const sample = (): Explore => ({
  range: "last 7 days",
  split: "account",
  breakup: [
    { key: "a1", label: "work@acme · max 20×", cost: 30, tokens: 4_000_000 },
    { key: "b2", label: "me@home · pro", cost: 10, tokens: 1_200_000 },
    { key: "", label: "account unknown", cost: 0.5, tokens: 10_000 },
  ],
  series: [
    { key: "a1", label: "work@acme · max 20×" },
    { key: "b2", label: "me@home · pro" },
  ],
  daily: ["2026-10-20", "2026-10-21", "2026-10-22"].map((day, i) => ({
    day,
    label: day.slice(5),
    cost: [10 * (i + 1), i === 1 ? null : 3],
  })),
  cumulative: { now: [13, 33, 40.5], before: [5, 15, 30] },
  per_day: { tokens: [1, 2, 3], requests: [4, 5, 6] },
  models: ["claude-opus-4-5"],
  model_mix: [{ label: "10/20", shares: [1] }],
  heat: {
    cost: Array.from({ length: 7 }, (_, d) =>
      Array.from({ length: 24 }, (_, h) => (d === 1 && h === 14 ? 9 : h > 8 && h < 18 ? 1 : 0)),
    ),
    tokens: Array.from({ length: 7 }, () => Array(24).fill(0)),
  },
  calendar: daysBetween("2026-08-03", "2026-10-25").map((day, i) => ({ day, cost: i % 3 ? i / 10 : 0 })),
  tools: [
    { name: "Read", calls: 120, failed: 0 },
    { name: "Bash", calls: 40, failed: 6 },
  ],
  parallel: 3,
  insights: [
    { label: "streak", value: "4 days now · 9 the longest" },
    { label: "busiest hour", value: "14:00–15:00" },
  ],
  plans: [{ account: "work@acme", plan: "max 20×", monthly: 200, value: 1840 }],
  stats: {
    prompt_cost: dist(40, 0.31, 1.9, [
      [0, 0.1, 6],
      [0.1, 0.5, 20],
      [0.5, 2, 12],
      [2, 8, 2],
    ]),
    prompt_seconds: dist(40, 95, 600),
    prompt_tokens: dist(40, 90_000, 400_000),
    prompt_requests: dist(40, 6, 20),
    session_cost: dist(5, 4.2, 12),
    session_seconds: dist(5, 3600, 9000),
    top_prompts: [
      {
        ts: 1792659000,
        seconds: 1260,
        host: "b31",
        agent: "claude",
        session: "s1",
        account: "a1",
        account_label: "work@acme",
        guessed: false,
        project: "github.com/acme/shop",
        model: "claude-opus-4-5",
        tokens: 2_100_000,
        requests: 38,
        cost: 6.4,
        estimated: true,
      },
    ],
    top_sessions: [],
    outside_prompts: { requests: 12, cost: 0.4 },
  },
});

test("the new charts draw on both themes, empty or not", () => {
  for (const t of [dark, light]) {
    clean(
      shareBar(
        {
          parts: [
            { label: "a", value: 3, color: "#d97757", note: "$3" },
            { label: "b", value: 1, color: "#10a37f" },
          ],
          label: "x",
        },
        t,
      ),
    );
    clean(shareBar({ parts: [], label: "x" }, t));
    clean(lineCompare({ columns: ["a", "b", "c"], now: [1, 2, 3], before: [1, 1, 2], fmt: String, label: "x" }, t));
    clean(lineCompare({ columns: ["a"], now: [0], before: [], fmt: String, label: "x" }, t));
    clean(
      histogram(
        {
          buckets: [
            [0, 1, 3],
            [1, 2, 0],
            [2, 4, 7],
          ],
          fmt: String,
          label: "x",
        },
        t,
      ),
    );
    clean(histogram({ buckets: [], fmt: String, label: "x" }, t));
    clean(heatWeek({ cells: [[1, 2], [], [0]], fmt: String, label: "x" }, t));
    clean(
      calendarHeat(
        {
          days: [
            { day: "2026-10-01", value: 2 },
            { day: "2026-10-02", value: 0 },
          ],
          fmt: String,
          label: "x",
        },
        t,
      ),
    );
    clean(calendarHeat({ days: [], fmt: String, label: "x" }, t));
  }
  // the share bar's legend says each part's share
  const s = shareBar(
    {
      parts: [
        { label: "work", value: 3, color: "#d97757" },
        { label: "home", value: 1, color: "#10a37f" },
      ],
      label: "x",
    },
    dark,
  );
  assert.match(s, />75%</);
  assert.match(s, />25%</);
  // the week's busiest hour is the deepest gold, named in its title
  const h = heatWeek({ cells: [[0, 5]], fmt: (v) => `$${v}`, label: "x" }, dark);
  assert.match(h, /Mon 01:00: \$5/);
});

test("the breakup, the comparison, the days, the week and 12 weeks", () => {
  const md = exploreMarkdown(sample(), { t: dark, colorOf: () => "#d97757" });
  assert.match(md, /^## \$40\.50 · last 7 days/);
  assert.match(md, /\*\*The cost by account\*\*/);
  // the table: every part, its share and tokens
  assert.match(md, /\| work@acme · max 20× \| \$30\.00 \| 74% \| 4\.0M \|/);
  assert.match(md, /\| account unknown \| \$0\.50 \| 1% \|/);
  // 40.50 against 30: up 35%
  assert.match(md, /▲ 35% on the period before/);
  assert.match(md, /A day at a time, by account/);
  assert.match(md, /The week's hours/);
  assert.match(md, /The last 12 weeks/);
  assert.equal(splitName("host"), "machine");
  // nothing yet: no charts, no crash
  const none: Explore = {
    ...sample(),
    breakup: [],
    series: [],
    daily: [],
    cumulative: { now: [], before: [] },
    calendar: [],
  };
  none.heat = { cost: [], tokens: [] };
  const e = exploreMarkdown(none, { t: light, colorOf: () => "#000000" });
  assert.ok(e.startsWith("## ") && !e.includes("!["), e);
});

test("insights: per prompt, the costliest, insights, plans, tools", () => {
  const md = insightsMarkdown(sample(), {
    t: dark,
    colorOf: () => "#d97757",
    brandName: (id) => (id === "claude" ? "Claude Code" : id),
  });
  assert.match(md, /What a prompt cost/);
  assert.match(md, /\| \$6\.40\* \| 21m \| 2\.1M \| Claude Code · b31 · shop \|/);
  assert.match(md, /- \*\*streak\*\* · 4 days now/);
  assert.match(md, /of API value in 30 days on .* a month · 9\.2×/);
  assert.match(md, /\| Bash \| 40 \| 6 \(15%\) \|/);
  assert.match(md, /At most 3 sessions at work in one hour/);
  assert.match(md, /12 requests \(\$0\.40\) fall outside any prompt/);
  // no prompts: says why
  const x = sample();
  x.stats.prompt_cost = dist(0, 0, 0);
  assert.match(
    insightsMarkdown(x, { t: light, colorOf: () => "#000", brandName: (id) => id }),
    /only Claude Code and Codex/,
  );
});

test("every period's days", () => {
  const now = new Date(2026, 9, 22, 15); // Thursday 22 October
  assert.deepEqual(span("today", now).days, ["2026-10-22"]);
  assert.deepEqual(span("24h", now).days, ["2026-10-21", "2026-10-22"]);
  assert.equal(span("7d", now).days.length, 7);
  assert.equal(span("90d", now).title, "Last 90 Days");
  assert.equal(span("6mo", now).days.length, 183);
  // Monday on
  assert.deepEqual(span("this-week", now).days, ["2026-10-19", "2026-10-20", "2026-10-21", "2026-10-22"]);
  assert.equal(span("this-month", now).days[0], "2026-10-01");
  assert.equal(span("this-month", now).days.length, 22);
  // your own dates
  const c = span("2026-09-28..2026-10-02", now);
  assert.deepEqual(c.days, ["2026-09-28", "2026-09-29", "2026-09-30", "2026-10-01", "2026-10-02"]);
  assert.equal(c.title, "Sep 28 – Oct 2");
  assert.equal(span("2026-10-20..", now).title, "Oct 20 – today");
  assert.equal(span("2026-10-20..", now).days.length, 3);
  // all time: from the first day with data
  assert.equal(span("all", now, "2026-10-01").days.length, 22);
  assert.equal(span("all", now).days.length, 90);
  assert.deepEqual(daysBetween("2026-10-02", "2026-10-01"), []);
});
