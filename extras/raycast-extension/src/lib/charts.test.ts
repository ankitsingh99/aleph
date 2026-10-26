// `npm test`: the Usage command's charts (charts.ts) and numbers (usage.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  barList,
  chartTheme,
  clip,
  contrast,
  deltaE,
  distinct,
  fit,
  niceScale,
  stackedColumns,
  tiles,
  shareImages,
  cardUri,
  cardFilesIn,
} from "./charts";
import type { UsageRow } from "./cli";
import {
  activeText,
  agentSeries,
  breakdown,
  compact,
  dayLabel,
  fmtMetric,
  lastDays,
  money,
  table,
  vendorAgent,
} from "./usage";

const dark = chartTheme(true);
const light = chartTheme(false);

const row = (key: string, tokens: number, cost: number, extra: Partial<UsageRow> = {}): UsageRow => ({
  key,
  label: key,
  input: 0,
  output: 0,
  cache_read: 0,
  cache_write: 0,
  reasoning: 0,
  tokens,
  requests: 1,
  sessions: 1,
  active_seconds: 0,
  cost,
  estimated: false,
  unpriced_requests: 0,
  first: 0,
  last: 0,
  hosts: [],
  agents: [],
  models: [],
  projects: [],
  ...extra,
});

test("numbers read like aleph usage", () => {
  assert.equal(compact(999), "999");
  assert.equal(compact(12_400), "12.4k");
  assert.equal(compact(3_100_000), "3.1M");
  assert.equal(compact(191_000), "191k");
  assert.equal(money(142.1), "$142.10");
  assert.equal(money(0.004), "<$0.01");
  assert.equal(money(0), "–");
  assert.equal(money(0, 3), "?", "nothing had a price");
  assert.equal(activeText(0), "–");
  assert.equal(activeText(40), "40s");
  assert.equal(activeText(7980), "2h 13m");
  assert.equal(fmtMetric("cost")(0), "0");
  assert.equal(fmtMetric("cost")(0.25), "$0.25");
  assert.equal(fmtMetric("tokens")(2500), "2.5k");
});

test("days and their labels", () => {
  const d = lastDays(7, new Date(2026, 9, 22, 9));
  assert.equal(d.length, 7);
  assert.equal(d[6], "2026-10-22");
  assert.equal(d[0], "2026-10-16");
  assert.deepEqual(lastDays(3, new Date(2026, 2, 1, 1)), ["2026-02-27", "2026-02-28", "2026-03-01"]);
  assert.equal(dayLabel("2026-10-02"), "Oct 2");
});

test("models take their maker's colour", () => {
  assert.equal(vendorAgent("claude-opus-5-5"), "claude");
  assert.equal(vendorAgent("gpt-5.1-codex"), "codex");
  assert.equal(vendorAgent("o3-mini"), "codex");
  assert.equal(vendorAgent("gemini-2.5-pro"), "gemini");
  assert.equal(vendorAgent("deepseek-v3"), undefined);
});

test("colours readable on both themes", () => {
  assert.ok(contrast("#ffffff", "#000000") > 20);
  for (const c of ["#dcdcdc", "#fab283", "#ffffff", "#000000", "#E2B451", "#4285f4"]) {
    assert.ok(contrast(fit(c, dark), dark.surface) >= 3, `${c} on dark`);
    assert.ok(contrast(fit(c, light), light.surface) >= 3, `${c} on light`);
  }
  // a colour that's already fine stays as it is
  assert.equal(fit("#10a37f", dark), "#10a37f");
  assert.equal(dark.accent.toLowerCase(), "#e2b451", "Aleph's gold, as is, on dark");
  assert.equal(fit("nope", dark), dark.other);
});

test("two series in one colour are told apart, by name rather than rank", () => {
  const [a, b] = distinct(["#4285f4", "#4285f4"], dark, ["Gemini CLI", "Antigravity CLI"]);
  assert.ok(deltaE(a, b) >= 12);
  const [b2, a2] = distinct(["#4285f4", "#4285f4"], dark, ["Antigravity CLI", "Gemini CLI"]);
  assert.equal(a, a2, "Gemini looks the same whichever is bigger");
  assert.equal(b, b2);
  assert.equal(b.toLowerCase(), "#4285f4", "the first by name keeps its brand colour");
});

test("clean axes", () => {
  assert.deepEqual(niceScale(0.38), { max: 0.4, step: 0.1 });
  assert.deepEqual(niceScale(0), { max: 1, step: 0.25 });
  const s = niceScale(22);
  assert.ok(s.max >= 22 && s.max <= 30);
});

test("labels are cut to fit; a path keeps its end", () => {
  assert.equal(clip("short", 200, 12), "short");
  const p = clip("~/work/Projects/a-very-long-project-name/with/more", 120, 12);
  assert.ok(p.startsWith("…") && p.endsWith("/more"), p);
  const w = clip("claude-sonnet-4-20250514-and-then-some", 100, 12);
  assert.ok(w.endsWith("…") && w.startsWith("claude"), w);
});

test("the stacked series: the top agents, then Other", () => {
  const days = ["2026-10-21", "2026-10-22"];
  const s = agentSeries({
    days,
    dayTotals: [row("2026-10-21", 100, 1), row("2026-10-22", 300, 3)],
    byAgent: [row("claude", 250, 2.5), row("codex", 100, 1), row("pi", 50, 0.5)],
    perAgent: {
      claude: [row("2026-10-21", 50, 0.5), row("2026-10-22", 200, 2)],
      codex: [row("2026-10-22", 100, 1)],
      pi: [row("2026-10-21", 50, 0.5)],
    },
    metric: "tokens",
    colorOf: (a) => (a === "claude" ? "#d97757" : "#10a37f"),
    nameOf: (a) => a.toUpperCase(),
    otherColor: "#777777",
    top: 2,
  });
  assert.deepEqual(
    s.map((x) => x.label),
    ["CLAUDE", "CODEX", "Other"],
  );
  assert.deepEqual(s[0].values, [50, 200]);
  assert.deepEqual(s[1].values, [0, 100]);
  assert.deepEqual(s[2].values, [50, 0], "the rest of each day");
  assert.equal(s[0].note, "250");
  const all = agentSeries({
    days,
    dayTotals: [row("2026-10-22", 100, 1)],
    byAgent: [row("claude", 100, 1)],
    perAgent: { claude: [row("2026-10-22", 100, 1)] },
    metric: "cost",
    colorOf: () => "#d97757",
    nameOf: (a) => a,
    otherColor: "#777777",
  });
  assert.equal(all.length, 1, "no Other when one agent is everything");
});

test("breakdowns fold their tail", () => {
  const rows = Array.from({ length: 10 }, (_, i) => row(`m${i}`, 1000 - i, 10 - i));
  const bars = breakdown({ rows, metric: "cost", colorOf: () => "#888888", otherColor: "#777777", max: 4 });
  assert.equal(bars.length, 4);
  assert.equal(bars[3].label, "7 more");
  assert.equal(bars[3].value, 7 + 6 + 5 + 4 + 3 + 2 + 1);
  assert.match(bars[0].note ?? "", /tok/);
  const md = table("Model", [row("a|b", 10, 1, { estimated: true })], (r) => r.key, "cost");
  assert.match(md, /a\\\|b \| \$1\.00 est\./);
});

test("the SVGs", () => {
  const svg = stackedColumns(
    {
      columns: ["Oct 21", "Oct 22"],
      series: [
        { label: "Claude <Code>", color: "#d97757", values: [1, 2], note: "$3.00" },
        { label: "Codex", color: "#10a37f", values: [0.5, 0], note: "$0.50" },
      ],
      fmt: fmtMetric("cost"),
      label: "cost per day",
    },
    light,
  );
  assert.match(svg, /^<svg [^>]*width="680"/);
  assert.ok(svg.includes("Claude &lt;Code&gt;"), "labels are escaped");
  assert.ok(!/NaN|undefined/.test(svg));
  assert.ok(svg.includes(">$2.00<"), "the peak's total on its cap");
  assert.ok(!svg.includes('<rect width="100%"'), "no background: the view's own shows through");
  const empty = stackedColumns({ columns: ["Oct 22"], series: [], fmt: fmtMetric("cost"), label: "x" }, dark);
  assert.ok(empty.includes("Nothing in this range"));
  const bars = barList(
    { bars: [{ label: "b31", value: 2, color: "#89b4fa", note: "1k tok" }], fmt: fmtMetric("cost"), label: "x" },
    dark,
  );
  assert.ok(bars.includes(">b31<") && bars.includes(">$2.00<") && bars.includes(">1k tok<"));
  const t = tiles(
    [{ label: "Today", value: "$0.38", suffix: "est.", sub: ["191k tokens", "16 sessions", "16m active"] }],
    dark,
  );
  assert.ok(t.includes(">est.<") && t.includes("191k tokens"));
});

test("an image used more than once is kept once", () => {
  const img = (x: number) =>
    `<image href="data:image/png;base64,QUJD" xlink:href="data:image/png;base64,QUJD" x="${x}" y="2" width="16" height="16" preserveAspectRatio="xMidYMid meet"/>`;
  const once = `<image href="data:image/png;base64,WFla" xlink:href="data:image/png;base64,WFla" x="0" y="0" width="8" height="8"/>`;
  const out = shareImages(img(1) + img(30) + once);
  assert.equal((out.match(/QUJD/g) ?? []).length, 1, "the shared one once, in <defs>");
  assert.equal((out.match(/<use /g) ?? []).length, 2);
  assert.ok(out.includes('transform="translate(30 2) scale(16 16)"'));
  assert.ok(out.includes(once), "an image used once stays as it was");
  assert.equal(shareImages(once), once);
});

test("cards as files named by their content", async () => {
  const { mkdtempSync, readdirSync, readFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  assert.match(cardUri("<svg/>"), /^data:image\/svg\+xml;base64,/, "no folder set: a data URI");
  const dir = mkdtempSync(join(tmpdir(), "aleph-cards-"));
  cardFilesIn(dir);
  const a = cardUri("<svg>a</svg>");
  const again = cardUri("<svg>a</svg>");
  const b = cardUri("<svg>b</svg>");
  assert.equal(a, again, "the same card, the same address (Raycast reuses it)");
  assert.notEqual(a, b);
  assert.match(a, /^file:\/\/.+\/[0-9a-f]{16}\.svg$/);
  assert.equal(readdirSync(dir).length, 2);
  assert.equal(readFileSync(decodeURI(a.slice(7)), "utf8"), "<svg>a</svg>");
});
