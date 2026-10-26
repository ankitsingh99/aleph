// `npm test`: Presets, drawn (presets-view.ts) — which panes go where, the
// picture of the tab, the detail.
import assert from "node:assert/strict";
import { test } from "node:test";
import { chartTheme } from "./charts";
import type { PresetPane } from "./cli";
import {
  cells,
  gridSplit,
  layoutCard,
  layoutWord,
  paneLooks,
  panesOf,
  parseShape,
  Preset,
  presetAgents,
  presetHero,
  presetHosts,
  presetMarkdown,
  presetSection,
  presetSplit,
  Split,
  tilde,
} from "./presets-view";

const dark = chartTheme(true);
const light = chartTheme(false);
const PNG = "data:image/png;base64,iVBORw0KGgo=";
const logo = (kind: string, id: string | null | undefined) => (id ? `${PNG}#${kind}/${id}` : undefined);
const brand = (id: string) => ({
  id,
  name: id === "claude" ? "Claude Code" : id === "codex" ? "Codex" : id,
  color: id === "codex" ? "#10a37f" : "#d97757",
});
const ctx = {
  fallback: "b31",
  defaultAgent: (h: string) => (h === "gpu2" ? "codex" : "claude"),
  color: (h: string) => ({ b31: "#89b4fa", gpu2: "#cba6f7" })[h],
};

const one: Preset = {
  name: "px",
  host: "b31",
  dir: "/home/me/px",
  args: "--model opus",
  agent: "claude",
  worktree: true,
  prompt: "run the tests",
  source: "/home/me/.config/aleph/aleph.toml",
};
const pane = (
  name: string,
  host: string | null,
  agent: string | null,
  extra: Partial<PresetPane> = {},
): PresetPane => ({
  name,
  host,
  agent,
  dir: "/srv/hunt",
  worktree: null,
  args: [],
  prompt: null,
  ...extra,
});
const ml: Preset = {
  name: "ml",
  host: "b31",
  dir: "/srv/hunt",
  args: null,
  agent: "claude",
  worktree: false,
  prompt: null,
  layout: "grid",
  focus: "review",
  tags: ["gpu"],
  panes: [
    pane("train", "b31", "claude", { prompt: "train it" }),
    pane("review", null, null),
    pane("eval", "gpu2", null),
  ],
};

/** The panes a split holds, in order. */
const leaves = (s: Split): number[] => ("pane" in s ? [s.pane] : s.parts.flatMap((p) => leaves(p.node)));

test("panes, machines, agents, sections", () => {
  assert.equal(panesOf(one).length, 1);
  assert.deepEqual(panesOf(one)[0], {
    name: null,
    host: "b31",
    agent: "claude",
    dir: "/home/me/px",
    worktree: "auto",
    args: ["--model", "opus"],
    prompt: "run the tests",
  });
  assert.deepEqual(presetHosts(ml, ctx), ["b31", "gpu2"]);
  // a pane without an agent: its machine's default
  assert.deepEqual(presetAgents(ml, ctx), ["claude", "codex"]);
  assert.equal(presetSection(one, ctx), "b31");
  assert.equal(presetSection(ml, ctx), "Several machines");
  assert.equal(presetSection({ ...one, host: null }, ctx), "b31");
  assert.equal(tilde("/home/selene/x/y"), "~/x/y");
  assert.equal(tilde("/srv/x"), "/srv/x");
  assert.equal(layoutWord(one), "one session");
  assert.equal(layoutWord(ml), "grid");
  assert.equal(layoutWord({ ...ml, layout: "columns" }), "3 columns");
  assert.equal(layoutWord({ ...ml, layout: "tabs" }), "3 tabs");
  assert.equal(layoutWord({ ...ml, shape: "(train | (review / eval))" }), "exact layout");
});

test("where herdr puts the panes", () => {
  // herdr's grid: ⌈√n⌉ columns, the first ones a row taller, numbered down each column
  const g3 = gridSplit(3) as Extract<Split, { dir: string }>;
  assert.equal(g3.dir, "right");
  assert.deepEqual(leaves(g3.parts[0].node), [0, 1]);
  assert.deepEqual(leaves(g3.parts[1].node), [2]);
  assert.deepEqual(leaves(gridSplit(5)), [0, 1, 2, 3, 4]);
  assert.equal(presetSplit({ ...ml, layout: "tabs" }), undefined);
  assert.equal(presetSplit(one), undefined);
  const cols = presetSplit({ ...ml, layout: "columns" }) as Extract<Split, { dir: string }>;
  assert.equal(cols.dir, "right");
  assert.equal(cols.parts.length, 3);
  // an exact layout: the names say which pane is where, the percentage sizes the first part
  const ex = parseShape("(eval 70% | (train / review))", ["train", "review", "eval"]) as Extract<
    Split,
    { dir: string }
  >;
  assert.deepEqual(leaves(ex), [2, 0, 1]);
  assert.equal(ex.parts[0].w, 0.7);
  assert.equal(parseShape("(a | b", ["a", "b"]), undefined);
  assert.equal(parseShape("(a ? b)", ["a", "b"]), undefined);
  // a broken shape falls back to the layout
  assert.deepEqual(leaves(presetSplit({ ...ml, shape: "((" })!), [0, 1, 2]);
  // cells fill the box, a gap between neighbours
  const c = cells(gridSplit(4), 0, 0, 400, 100, 6);
  assert.equal(c.length, 4);
  assert.deepEqual(
    c.map((x) => [x.x, x.y, x.w, x.h]),
    [
      [0, 0, 197, 47],
      [0, 53, 197, 47],
      [203, 0, 197, 47],
      [203, 53, 197, 47],
    ],
  );
});

test("the picture of the tab, the hero", () => {
  const looks = paneLooks(ml, ctx, logo, brand);
  assert.deepEqual(
    looks.map((l) => [l.name, l.agent, l.host, l.defaultHost]),
    [
      ["train", "claude", "b31", false],
      ["review", "claude", "b31", true],
      ["eval", "codex", "gpu2", false],
    ],
  );
  for (const t of [dark, light]) {
    const grid = layoutCard(ml, looks, t);
    assert.match(grid, /One tab · grid/);
    // the pane opened in Focus is ringed in gold
    assert.match(grid, /opens review in Focus/);
    assert.match(grid, new RegExp(`stroke="${t.accent}"`));
    assert.match(grid, /#agent\/codex/);
    assert.match(grid, />gpu2</);
    const tabs = layoutCard({ ...ml, layout: "tabs", focus: null }, looks, t);
    assert.match(tabs, />3 tabs</);
    assert.doesNotMatch(tabs, /in Focus/);
    // several agents: their logos together in the hero's tile, how many sessions on the right
    const h = presetHero(ml, looks, t);
    assert.match(h, /#agent\/claude/);
    assert.match(h, /#agent\/codex/);
    assert.match(h, />3</);
    assert.match(h, />sessions</);
    assert.match(h, />Claude Code, Codex</);
    assert.match(h, />#gpu</);
  }
});

test("the detail", () => {
  const single = presetMarkdown({ p: one, ctx, t: dark, logo, brand });
  const imgs = single.match(/!\[/g) ?? [];
  assert.equal(imgs.length, 2);
  assert.match(single, /^> run the tests$/m);
  const svg = (md: string, i: number) =>
    Buffer.from(/base64,([^)]+)\)/g.exec(md.split("![")[i + 1])![1], "base64").toString("utf8");
  // the hero says the folder; the facts the agent, machine, worktree, args — not where it's saved (the main config)
  assert.match(svg(single, 0), />~\/px</);
  const f = svg(single, 1);
  assert.match(f, />Claude Code</);
  assert.match(f, />b31</);
  assert.match(f, />new worktree</);
  assert.match(f, />--model opus</);
  assert.doesNotMatch(f, /Saved in/);
  assert.match(
    svg(presetMarkdown({ p: { ...one, source: "/home/me/team/presets.toml" }, ctx, t: dark, logo, brand }), 1),
    /~\/team\/presets\.toml/,
  );
  // several sessions: the picture of the tab, and each first prompt with its pane's name
  const multi = presetMarkdown({ p: ml, ctx, t: light, logo, brand });
  assert.match(multi, /^> \*\*train\*\* · train it$/m);
  assert.match(svg(multi, 1), /One tab · grid/);
});
