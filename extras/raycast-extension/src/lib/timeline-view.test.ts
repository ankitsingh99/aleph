// `npm test`: a conversation's tree as text (timeline-view.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import { fitFigures, folded, glyph, nextFold, TlRow, TlTree, took, treeLines, treeMarkdown } from "./timeline-view";

const sums = {
  input: 0,
  output: 0,
  reasoning: 0,
  cache: 0,
  tokens: 0,
  cost: 0,
  estimated: false,
  requests: 0,
  tools: 0,
  errors: 0,
};
const B = new Date(2026, 9, 22, 14, 20).getTime();
const row = (o: Partial<TlRow>): TlRow => ({
  id: "x",
  depth: 0,
  kind: "tool",
  label: "Read",
  start_ms: B,
  end_ms: B,
  sums,
  foldable: false,
  folded: false,
  bad: false,
  lane: "",
  key: "",
  ...o,
});

const rows: TlRow[] = [
  row({
    id: "p1",
    kind: "prompt",
    label: "prompt 1 · you",
    foldable: true,
    key: "pk",
    figures: "13.8s · 2.0k→400 · 2 req · $0.10",
  }),
  row({
    id: "t0",
    depth: 1,
    kind: "turn",
    label: "turn · sonnet-4-5",
    foldable: true,
    figures: "4.5s · 1.0k→200 · $0.05",
  }),
  row({ id: "g", depth: 2, kind: "group", label: "Read + 3 more", foldable: true, folded: true, figures: "3.5s" }),
  row({ id: "x1", depth: 2, kind: "tool", label: "Bash · failed after 0.8s", bad: true, key: "bk", figures: "0.8s" }),
  row({ id: "end", kind: "promptend", label: "done", figures: "2.0k→400 · 2 req · 5 tools · $0.10" }),
];

test("each row: its time, its mark, its label, the figures at the right edge", () => {
  const lines = treeLines(rows, 70);
  assert.equal(lines.length, 5);
  for (const l of lines) assert.ok([...l].length <= 70, l);
  assert.match(lines[0], /^14:20 ┌ prompt 1 · you +13\.8s · 2\.0k→400 · 2 req · \$0\.10$/);
  assert.match(lines[2], /^14:20 {5}▸ Read \+ 3 more +3\.5s$/);
  // the figures line up on the right
  assert.equal([...lines[0]].length, 70);
  assert.equal([...lines[1]].length, 70);
  // words after the label, cut before the figures are
  const w = treeLines(rows, 70, {
    pk: "fix the flaky perf test in the store module, then run the whole suite again",
    bk: "cargo test -q",
  });
  assert.match(w[0], /prompt 1 · you {2}fix the fl.*… +13\.8s/);
  assert.match(w[3], /Bash · failed after 0\.8s {2}cargo test -q +0\.8s$/);
  for (const l of w) assert.ok([...l].length <= 70, l);
  // a narrow width cuts the label, never the figures
  const n = treeLines([row({ label: "a very long tool name that goes on and on", figures: "$1.20" })], 30)[0];
  assert.ok(n.endsWith("$1.20") && [...n].length <= 30, n);
});

test("marks, folding, durations", () => {
  assert.equal(glyph({ kind: "prompt", folded: true, foldable: true }), "▸ ");
  assert.equal(glyph({ kind: "turn", folded: false, foldable: true }), "◆ ");
  assert.equal(glyph({ kind: "lane", folded: true, foldable: true }), "⑂▸");
  assert.equal(glyph({ kind: "error", folded: false, foldable: false }), "✗ ");
  assert.equal(nextFold("smart"), "open");
  assert.equal(nextFold("folded"), "smart");
  assert.deepEqual(folded(rows), ["g"]);
  assert.equal(took(12_000), "12s");
  assert.equal(took(4 * 60_000), "4m");
  assert.equal(took(190 * 60_000), "3h 10m");
  assert.equal(took(714 * 3_600_000), "29d 18h");
  // the counts give way first, then the tokens, then the time; the $ and failures stay
  const f = "13.8s · 2.0k→400 · 2 req · 5 tools · 1 failed · $0.10";
  assert.equal(fitFigures(f, 99), f);
  assert.equal(fitFigures(f, 40), "13.8s · 2.0k→400 · 1 failed · $0.10");
  assert.equal(fitFigures(f, 24), "13.8s · 1 failed · $0.10");
  assert.equal(fitFigures(f, 5), "1 failed · $0.10");
});

test("the detail: totals, then the tree; nothing read yet says so", () => {
  const t: TlTree = {
    host: "b31",
    agent: "claude",
    session: "s1",
    start_ms: B,
    end_ms: B + 13_800,
    sums: { ...sums, tokens: 2400, cost: 0.1, requests: 2, tools: 5, errors: 1 },
    lanes: [
      { id: "", kind: "", prompts: 1, sums },
      { id: "sub1", kind: "Explore", prompts: 1, sums },
    ],
    rows,
  };
  const md = treeMarkdown(t, { title: "shop", fold: "smart", agentName: "Claude Code" });
  assert.match(md, /^## shop/);
  assert.match(
    md,
    /Claude Code · b31 · 1 prompt · 2 requests · 5 tool calls · 1 subagent · 2\.4k tokens · \$0\.10 · 14s/,
  );
  assert.match(md, /_Folding: smart · 1 failed_/);
  assert.match(md, /```\n14:20 ┌ prompt 1/);
  const empty = treeMarkdown({ ...t, rows: [] }, { title: "shop", fold: "smart" });
  assert.match(empty, /Nothing read for it yet/);
});
