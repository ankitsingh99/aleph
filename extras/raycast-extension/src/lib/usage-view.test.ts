// `npm test`: Usage as a list (usage-view.ts): project names, shares, the
// overview and one row's detail, the tables.
import assert from "node:assert/strict";
import { test } from "node:test";
import { chartTheme } from "./charts";
import type { UsageRow } from "./cli";
import { keyColors, lastDays } from "./usage";
import {
  figure,
  itemFacts,
  itemMarkdown,
  limitsCard,
  limitsMarkdown,
  limitSummary,
  Loaded,
  overviewMarkdown,
  projectLabel,
  projectName,
  rowColor,
  shareOf,
  shownDays,
  sourcesMarkdown,
  tablesMarkdown,
} from "./usage-view";

const dark = chartTheme(true);
const light = chartTheme(false);
const logo = (kind: string, id: string | null | undefined) =>
  id ? `data:image/png;base64,AA#${kind}/${id}` : undefined;
const brand = (id: string) => ({
  name: id === "claude" ? "Claude Code" : id,
  color: id === "claude" ? "#d97757" : "#10a37f",
});
const opts = (t = dark) => ({
  m: "cost" as const,
  t,
  logo,
  brand,
  hostColor: (h: string) => (h === "b31" ? "#89b4fa" : undefined),
});

const row = (key: string, tokens: number, cost: number, extra: Partial<UsageRow> = {}): UsageRow => ({
  key,
  label: key,
  input: tokens / 2,
  output: tokens / 10,
  cache_read: tokens / 4,
  cache_write: 0,
  reasoning: 0,
  tokens,
  requests: 3,
  sessions: 2,
  active_seconds: 120,
  cost,
  estimated: false,
  unpriced_requests: 0,
  first: 1792600000,
  last: 1792660000,
  hosts: ["b31"],
  agents: ["claude"],
  models: ["claude-opus-5-5"],
  projects: ["/home/u/site"],
  ...extra,
});

const svgs = (md: string) =>
  [...md.matchAll(/base64,([^)]+)\)/g)].map((m) => Buffer.from(m[1], "base64").toString("utf8"));

test("projects named the way a person would", () => {
  assert.deepEqual(projectName("/home/u/site"), { name: "site", parent: "~" });
  assert.deepEqual(projectName("/Users/me/work/Projects/aleph/"), { name: "aleph", parent: "~/work/Projects" });
  assert.deepEqual(projectName("/Users/me/work/Projects/aleph/.claude/worktrees/agent-1"), {
    name: "aleph",
    parent: "~/work/Projects",
    worktree: "agent-1",
  });
  assert.deepEqual(projectName("/srv/train/.worktrees/feat-x"), { name: "train", parent: "/srv", worktree: "feat-x" });
  assert.deepEqual(projectName("/home/u"), { name: "~", parent: "home" });
  assert.deepEqual(projectName(""), { name: "(no folder)", parent: "" });
  // a project's id (a repo wherever it was worked on)
  assert.deepEqual(projectName("r:github.com/acme/app"), { name: "app", parent: "github.com/acme" });
  assert.deepEqual(projectName("n:work"), { name: "work", parent: "" });
  assert.deepEqual(projectName("c:a1b2c3d4e5f6"), { name: "repo a1b2c3d4", parent: "" });
  assert.deepEqual(projectName("d:b31:/home/u/site"), { name: "site", parent: "b31:~" });
  assert.equal(projectName("/srv").parent, "/");
  assert.equal(projectLabel("/Users/me/x/.claude/worktrees/w"), "x · worktree w · ~");
});

test("shares and figures", () => {
  const total = row("total", 1000, 4);
  assert.equal(shareOf(row("a", 250, 1), total, "cost"), 0.25);
  assert.equal(shareOf(row("a", 250, 1), total, "tokens"), 0.25);
  assert.equal(shareOf(row("a", 250, 1), undefined, "cost"), 0, "nothing to share");
  assert.equal(figure(row("a", 1500, 1.234, { estimated: true }), "cost"), "$1.23*");
  assert.equal(figure(row("a", 1500, 0), "tokens"), "1.5k");
  assert.equal(rowColor("project", "/x", opts()), dark.muted, "projects in the theme's ink, not gold");
  assert.equal(rowColor("host", "b31", opts()), "#89b4fa");
  assert.equal(rowColor("model", "claude-opus-5-5", opts()), "#d97757", "a model in its maker's colour");
});

const loaded = (): Loaded => {
  const days = lastDays(30);
  return {
    range: "30d",
    today: row("today", 100, 1),
    week: row("7d", 700, 7),
    month: row("30d", 3000, 30),
    total: row("total", 3000, 30),
    days: days.map((d) => row(d, 100, 1)),
    agents: [row("claude", 2000, 20), row("codex", 1000, 10)],
    models: [row("claude-opus-5-5", 3000, 30)],
    hosts: [row("b31", 2000, 20), row("gpu2", 1000, 10)],
    projects: Array.from({ length: 93 }, (_, i) => row(`/home/u/p${i}`, 30, 0.3)),
    perAgent: { claude: days.map((d) => row(d, 60, 0.6)) },
    fromDaemon: true,
    unpriced: [],
    errors: [],
  };
};

test("the overview: periods, days by agent, agents and machines, never an 'n more'", () => {
  const md = overviewMarkdown(loaded(), opts());
  const s = svgs(md);
  assert.equal(s.length, 4, "tiles, days, agents, machines");
  assert.ok(s[0].includes(">Today<") && s[0].includes(">$30.00<"));
  assert.ok(!/\d+ more/.test(md + s.join("")), "nothing folded away");
  assert.ok(s.every((x) => !/NaN|undefined/.test(x)));
  const filtered = overviewMarkdown(loaded(), { ...opts(), filter: "Claude Code" });
  assert.match(filtered, /last 30 days · Claude Code/);
});

test("one row's detail", () => {
  const d = loaded();
  const p = row("/Users/me/work/aleph/.claude/worktrees/agent-1", 600, 6, { estimated: true });
  const md = itemMarkdown({ ...opts(light), kind: "project", row: p, total: d.total, range: "30d", days: d.days });
  const s = svgs(md);
  assert.equal(s.length, 4, "hero, figures, days, where");
  assert.ok(s[0].includes(">aleph<") && s[0].includes("worktree agent-1") && s[0].includes(">20% of cost<"));
  assert.ok(s[1].includes(">$6.00<"));
  assert.match(s[1], />est\.( · [^<]*)?</, "no room beside the value: the estimate goes under it");
  assert.ok(s[3].includes(">Claude Code<") && s[3].includes("#agent/claude") && s[3].includes(">cache read<"));
  const waiting = itemMarkdown({
    ...opts(),
    kind: "model",
    row: d.models[0],
    total: d.total,
    range: "30d",
    days: "loading",
  });
  assert.match(waiting, /Reading its days…/);
  const agent = itemMarkdown({
    ...opts(),
    kind: "agent",
    row: d.agents[0],
    total: d.total,
    range: "7d",
    days: d.perAgent.claude,
    logoId: { kind: "agent", id: "claude" },
  });
  assert.ok(svgs(agent)[0].includes("#agent/claude") && svgs(agent)[0].includes(">Claude Code<"));
  const f = itemFacts("agent", d.agents[0], opts());
  assert.deepEqual(
    f.map((x) => x.label),
    ["Tokens", "Machines", "Models", "Projects", "Seen"],
    "an agent's detail doesn't list itself",
  );
});

test("tables for ⌘L and Copy", () => {
  const md = tablesMarkdown(loaded(), "cost", (id) => brand(id).name, "b31");
  assert.match(md, /## Usage · last 30 days · b31/);
  assert.match(md, /\| Claude Code \| \$20\.00 \|/);
  assert.equal((md.match(/\| p\d+ · ~ \|/g) ?? []).length, 93, "every project, by name and folder");
});

test("plan limits and sources as cards", () => {
  const now = 1_000_000;
  const cards = { ...opts(), now };
  const lim = svgs(
    limitsMarkdown(
      {
        b31: {
          codex: {
            codex: {
              ts: now,
              plan: "prolite",
              windows: [{ used_percent: 69, minutes: 10080, resets_at: now + 3 * 86400 + 3600 }],
            },
          },
        },
        local: { codex: { codex: { ts: now - 3 * 86400, plan: "pro", windows: [{ used_percent: 9, minutes: 300 }] } } },
      },
      cards,
    ),
  )[0];
  assert.ok(lim.includes(">Pro $100<") && lim.includes(">Week<") && lim.includes(">69%<"));
  assert.ok(lim.includes(">resets in 3d 1h<"));
  // a 5-hour window read 3 days ago has reset since: 0 %, not its old 9 %
  assert.ok(!lim.includes(">9%<") && lim.includes(">0%<"), "a window that reset starts from 0");
  assert.ok(!/<table|\|/.test(lim));
  assert.match(svgs(limitsMarkdown(undefined, cards))[0], /No plan limits in the logs yet/);
  const src = svgs(
    sourcesMarkdown(
      {
        hosts: [
          {
            host: "b31",
            enabled: true,
            files_tracked: 446,
            bytes_tracked: 1_230_456_899,
            last_run: {
              at: now - 600,
              took_ms: 1500,
              sqlite3: false,
              agents: [
                { agent: "claude", listed: 351, to_read: 1, unchanged: 350, roots: ["/home/n/.claude/projects"] },
                { agent: "opencode", listed: 1, to_read: 0, unchanged: 0, roots: ["/home/n/.local/share/opencode"] },
              ],
            },
          },
        ],
        no_reader: [{ agent: "cursor", why: "no token counts" }],
      },
      cards,
    ),
  )[0];
  assert.ok(src.includes(">b31<") && src.includes(">read 10m ago<"));
  assert.ok(src.includes(">446 files · 1.1 GB<"));
  assert.ok(src.includes(">Claude Code<") && src.includes(">351<") && src.includes(">1 to read<"));
  assert.ok(src.indexOf(">Claude Code<") < src.indexOf(">opencode<"), "most files first");
  assert.ok(src.includes("~/.claude/projects") && src.includes(">needs sqlite3<"));
  assert.ok(!src.includes("cursor") && !src.includes("No usage data"), "agents with nothing to read aren't listed");
});

test("a detail names a few projects, once each, busiest first", () => {
  const r = row("claude", 100, 1, {
    projects: [
      "/home/u/a",
      "/home/u/work/b",
      "/srv/b",
      ...Array.from({ length: 10 }, (_, i) => `/home/u/x${i}`),
      "/home/u/busy",
    ],
  });
  const f = itemFacts("agent", r, { ...opts(), projectOrder: ["/home/u/busy", "/srv/b"] });
  const p = f.find((x) => x.label === "Projects")!.parts.map((x) => x.text);
  assert.deepEqual(p.slice(0, 3), ["busy", "b", "a"]);
  assert.equal(p.length, 7);
  assert.equal(p[6], "+7 more", "13 names, two folders called b counted once");
  const seen = f.find((x) => x.label === "Seen")!.parts;
  assert.equal(seen.length, 1);
  assert.match(seen[0].text, / – /);
});

test("a chart starts at its first day with anything in it", () => {
  const days = lastDays(30);
  const v = (d: string) => (d >= days[25] ? 1 : 0);
  assert.equal(shownDays(days, v).length, 7, "at least a week");
  assert.equal(shownDays(days, (d) => (d >= days[10] ? 1 : 0)).length, 20);
  assert.equal(shownDays(days, () => 0).length, 30, "nothing: the whole range");
});

test("every model and project its own colour, as chips", () => {
  const c = keyColors(["claude-opus-5-5", "gpt-6-astra", "claude-opus-5", "codex-auto-review", "gpt-6.1-sol"], true);
  assert.equal(new Set(c.values()).size, 5, "five models, five colours");
  assert.notEqual(c.get("claude-opus-5"), c.get("claude-opus-5-5"));
  assert.deepEqual(keyColors(["b", "a"], true), keyColors(["a", "b", "a"], true), "the same names, the same colours");
  const many = keyColors(
    Array.from({ length: 30 }, (_, i) => `p${i}`),
    false,
  );
  assert.equal(new Set(many.values()).size, 8, "past eight they share");
  const r = row("claude", 100, 1, { models: ["claude-opus-5-5", "claude-opus-5"] });
  const f = itemFacts("agent", r, { ...opts(), modelColor: (m) => c.get(m), projectColor: () => "#3987e5" });
  const models = f.find((x) => x.label === "Models")!.parts;
  assert.ok(models.every((p) => p.chip) && models[0].chip !== models[1].chip);
  assert.ok(f.find((x) => x.label === "Projects")!.parts[0].chip);
  assert.equal(rowColor("model", "claude-opus-5", { ...opts(), modelColor: (m) => c.get(m) }), c.get("claude-opus-5"));
});

test("the list's limits: each agent's fullest window", () => {
  const s = limitSummary({
    b31: { codex: { codex: { ts: Date.now() / 1000, windows: [{ used_percent: 1, minutes: 10080 }] } } },
    local: {
      claude: {
        claude: {
          ts: Date.now() / 1000,
          windows: [
            { used_percent: 13, minutes: 300 },
            { used_percent: 44, minutes: 10080 },
          ],
        },
      },
    },
  });
  assert.deepEqual(
    s.map((x) => [x.agent, x.top]),
    [
      ["claude", 44],
      ["codex", 1],
    ],
  );
  assert.deepEqual(s[0].windows, ["5 hours 13%", "week 44%"]);
  // a window that reset after its reading counts as 0 (the old 96% isn't today's)
  const now = 2_000_000;
  const lapsed = limitSummary(
    {
      local: {
        claude: {
          claude: { ts: Date.now() / 1000, windows: [{ used_percent: 96, minutes: 10080, resets_at: now - 60 }] },
        },
      },
      b31: {
        claude: {
          claude: { ts: Date.now() / 1000, windows: [{ used_percent: 23, minutes: 10080, resets_at: now + 60 }] },
        },
      },
    },
    now,
  );
  assert.deepEqual(
    lapsed.map((x) => [x.agent, x.top]),
    [["claude", 23]],
  );
});

test("plan limits: the agent's name is never cut — the tags go under it when they don't fit", () => {
  const now = 2_000_000;
  const card = limitsCard(
    {
      local: {
        claude: {
          claude: {
            ts: now,
            plan: "Max 5x",
            account: "c0ffee00aaaa1111",
            windows: [{ used_percent: 4, minutes: 10080 }],
          },
        },
      },
      q7x: {
        claude: {
          claude: {
            ts: now - 10 * 86400,
            plan: "Max 5x",
            account: "beef0000bbbb2222",
            source: "log",
            windows: [{ used_percent: 0, minutes: 300, resets_at: now + 3600 }],
          },
        },
      },
    },
    { ...opts(), now, width: 400 },
  );
  assert.ok(!card.includes("Cla…") && card.includes(">Claude Code<"), card.slice(0, 400));
  // the tags on the name's line (the account's hash alone when even that's too wide), the age under it
  assert.ok(/>(account )?beef</.test(card) && card.includes(">as of 10d 0h ago<"), card);
});

test("plan limits: two accounts of one agent told apart; a reading older than its reset says so", () => {
  const now = 2_000_000;
  const card = limitsCard(
    {
      local: {
        claude: {
          claude: {
            ts: now - 3600,
            plan: "Max 5x",
            account: "c0ffee00aaaa1111",
            windows: [{ used_percent: 44, minutes: 10080, resets_at: now + 3600 }],
          },
        },
      },
      q7x: {
        claude: {
          claude: {
            ts: now - 9 * 86400,
            plan: "Max 5x",
            account: "beef0000bbbb2222",
            windows: [{ used_percent: 0, minutes: 10080, resets_at: now - 86400 }],
          },
        },
      },
    },
    { ...opts(), now },
  );
  assert.ok(card.includes(">account c0ff<") && card.includes(">account beef<"));
  // every window of q7x's reading has reset since: they start again from 0 %
  assert.ok(card.includes(">0%<") && card.includes(">reset since<") && !card.includes("Last read"), card);
});

test("plan limits: one account on machines without a reading of their own is listed with them", () => {
  const card = limitsCard(
    {
      b31: { claude: { claude: { ts: 0, plan: "Pro", account: "bbbb000000000000", windows: [] } } },
      local: {
        claude: {
          claude: {
            ts: 5,
            plan: "Max 5x",
            account: "aaaa000000000000",
            hosts: ["local", "dex42"],
            windows: [{ used_percent: 44, minutes: 10080 }],
          },
        },
      },
    },
    { ...opts(), now: 10 },
  );
  assert.ok(card.includes(">dex42<") && card.includes(">local<"), "both machines on the account");
  assert.ok(card.includes(">Pro<") && card.includes(">No usage reading here yet<"), "the other account, no reading");
  assert.ok(card.indexOf(">44%<") < card.indexOf(">No usage reading here yet<"), "readings first");
});
