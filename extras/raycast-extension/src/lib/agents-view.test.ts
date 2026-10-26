// `npm test`: the Agents view (agents-view.ts) and the cards it draws with (cards.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { AgentInfo, AgentsHost, AgentsReport, Host, Session, UsageRow } from "./cli";
import { chartTheme } from "./charts";
import { checks, itemRows, notice, wrap } from "./cards";
import {
  agentBadges,
  agentMarkdown,
  agentsSummary,
  compareVersions,
  costText,
  groupAgents,
  installIssues,
  limitMeters,
  missingMarkdown,
  missingSummary,
  missingTag,
  newest,
  openNow,
  openText,
  tilde,
  usageOf,
} from "./agents-view";

const dark = chartTheme(true);
const light = chartTheme(false);
const PNG = "data:image/png;base64,iVBORw0KGgo=";
const logo = (kind: string, id: string | null | undefined) => (id ? `${PNG}#${kind}/${id}` : undefined);
const svgsOf = (md: string) =>
  [...md.matchAll(/base64,([^)]+)\)/g)].map((m) => Buffer.from(m[1], "base64").toString("utf8"));

const agent = (id: string, over: Partial<AgentInfo> = {}): AgentInfo => ({
  id,
  name: id === "claude" ? "Claude Code" : id === "codex" ? "Codex" : id,
  kind: id,
  glyph: "*",
  color: "#d97757",
  command: id,
  installed: true,
  path: `/home/me/.local/bin/${id}`,
  version: "1.0.0",
  check: "verified",
  on_path: true,
  shadowed_by: null,
  integration: id,
  integration_state: "current",
  persistence: true,
  resume: ["--continue"],
  session_logs: `~/.${id}/projects`,
  install_method: "npm",
  install_command: `npm i -g ${id}`,
  install_ready: true,
  install_blocker: null,
  prerequisites: [],
  ...over,
});

const machine = (alias: string, agents: AgentInfo[], over: Partial<AgentsHost> = {}): AgentsHost => ({
  alias,
  reachable: true,
  error: null,
  os: "Linux x86_64",
  os_name: "Ubuntu 24.04.3 LTS",
  agents,
  ...over,
});

const report = (): AgentsReport => ({
  hosts: [
    machine("b31", [
      agent("claude", { version: "2.1.309" }),
      agent("codex", { installed: false, version: null, path: null }),
      agent("opencode", { version: "1.18.15", integration_state: "not_installed" }),
    ]),
    machine(
      "local",
      [
        agent("claude", { version: "2.1.309", integration_state: "outdated", path: "/Users/me/.local/bin/claude" }),
        agent("codex", { version: "0.163.0" }),
        agent("opencode", { version: "2.1.2", on_path: false }),
      ],
      { os: "Darwin arm64", os_name: "macOS 15.1" },
    ),
    machine("gpu2", [], { reachable: false, error: "ssh: connect to host gpu2 port 22: Operation timed out" }),
  ],
});

const sess = (host: string, name: string, agentId: string, status: string): Session => ({
  host,
  name,
  label: name,
  display: name,
  pane_id: `w1:${name}`,
  tab_id: "",
  workspace_id: "",
  terminal_id: "",
  agent: agentId,
  status,
  status_since: 1000,
  cwd: "/home/me/work/px",
  title: "",
  mark: "",
  hue: "",
  glyph: "",
  color: "",
  focused: false,
});

const fleet = (): Host[] => [
  {
    alias: "b31",
    machine: "b31",
    color: "#89b4fa",
    address: "me@b31",
    reachable: true,
    error: null,
    stale: false,
    fetched_at: 1000,
    latency_ms: 40,
    sessions: [sess("b31", "temper", "claude", "blocked"), sess("b31", "ignite", "claude", "working")],
  },
  {
    alias: "local",
    machine: "local",
    color: "#fab387",
    address: "this computer",
    reachable: true,
    error: null,
    stale: false,
    fetched_at: 1000,
    latency_ms: 10,
    sessions: [sess("local", "quill", "claude", "idle"), sess("local", "shell", "codex", "idle")],
  },
];

const row = (key: string, cost: number, tokens: number, estimated = true): UsageRow => ({
  key,
  label: key,
  input: 0,
  output: 0,
  cache_read: 0,
  cache_write: 0,
  reasoning: 0,
  tokens,
  requests: 10,
  sessions: 3,
  active_seconds: 600,
  cost,
  estimated,
  unpriced_requests: 0,
  first: 0,
  last: 0,
  hosts: [],
  agents: [key],
  models: [],
  projects: [],
});

test("versions and paths", () => {
  assert.ok(compareVersions("2.1.2", "1.18.15") > 0);
  assert.ok(compareVersions("1.18.15", "2.1.2") < 0);
  assert.equal(compareVersions("v0.6.3", "0.6.3"), 0);
  assert.ok(compareVersions("0.6.3.post1", "0.6.3") > 0);
  assert.ok(compareVersions("2.10.0", "2.9.9") > 0);
  assert.equal(newest(["1.18.15", null, "2.1.2", "2.0.0"]), "2.1.2");
  assert.equal(newest([]), undefined);
  assert.equal(tilde("/home/selene/.local/bin/claude"), "~/.local/bin/claude");
  assert.equal(tilde("/Users/me"), "~");
  assert.equal(tilde("/root/.cargo/bin/x"), "~/.cargo/bin/x");
  assert.equal(tilde("/opt/homebrew/bin/pi"), "/opt/homebrew/bin/pi");
});

test("one agent across the fleet", () => {
  const groups = groupAgents(report());
  assert.deepEqual(
    groups.map((g) => g.id),
    ["claude", "codex", "opencode"],
    "every agent installed somewhere, registry order",
  );
  const claude = groups[0];
  assert.deepEqual(
    claude.installs.map((i) => i.host.alias),
    ["b31", "local"],
  );
  assert.deepEqual(
    claude.offline.map((h) => h.alias),
    ["gpu2"],
  );
  const codex = groups[1];
  assert.deepEqual(
    codex.missing.map((i) => i.host.alias),
    ["b31"],
  );
  const opencode = groups[2];
  assert.equal(opencode.newest, "2.1.2");
  assert.deepEqual(installIssues(opencode.installs[0].a, opencode.newest), ["1.18.15, behind 2.1.2"]);
  assert.deepEqual(installIssues(opencode.installs[1].a, opencode.newest), ["not on herdr's PATH"]);
  assert.deepEqual(installIssues(claude.installs[1].a, claude.newest), ["integration outdated"]);
  assert.equal(groupAgents(report(), true).length, 3);
  assert.equal(groupAgents(undefined).length, 0);
});

test("sessions open now, usage, badges", () => {
  const groups = groupAgents(report());
  const o = openNow(fleet(), ["claude", "claude"]);
  assert.equal(o.total, 3);
  assert.deepEqual(
    o.sessions.map((s) => s.name),
    ["temper", "ignite", "quill"],
    "needs you first",
  );
  assert.equal(openText(o), "1 needs you · 1 working · 1 idle");
  assert.equal(openNow(fleet(), ["claude"], "local").total, 1, "one machine's");
  assert.equal(openText(openNow([], ["x"])), "none open");

  const today = [row("claude", 38.39, 63_700_000), row("codex", 0, 0, false)];
  assert.equal(usageOf(today, ["claude", null])?.cost, 38.39);
  assert.equal(costText(usageOf(today, ["claude"])), "$38.39*");
  assert.equal(costText(row("x", 2, 1, false)), "$2.00");
  assert.equal(costText(undefined), "–");

  const b = agentBadges(groups[0], { open: o, today: today[0], month: row("claude", 1447.4, 4.4e9) });
  assert.deepEqual(
    b.map((x) => [x.kind, x.text]),
    [
      ["warn", ""],
      ["needs", "1"],
      ["cost", "$38.39*"],
    ],
  );
  assert.match(b[0].tooltip, /local: integration outdated/);
  assert.match(b[2].tooltip, /30 days \$1447\* · 4\.4B tokens/);
  const wide = agentBadges(groups[1], { open: openNow(fleet(), ["codex"]), wide: true });
  assert.deepEqual(
    wide.map((x) => [x.kind, x.text]),
    [
      ["open", "1"],
      ["machines", "local"],
    ],
  );
  assert.equal(agentsSummary(groups, 4, today), "3 installed · 4 open · $38.39* today");
  assert.equal(agentsSummary(groups, 0, []), "3 installed");
});

test("plan limits", () => {
  const l = {
    b31: {
      codex: {
        codex: {
          ts: 1,
          plan: "pro",
          windows: [{ minutes: 10080, resets_at: 1000 + 5 * 86400 + 3600, used_percent: 69 }],
        },
      },
    },
    local: { codex: { codex: { ts: 1, plan: "pro", windows: [{ minutes: 300, resets_at: 500, used_percent: 95 }] } } },
    gpu2: { claude: { x: { ts: 1, windows: [{ minutes: 300, used_percent: 10 }] } } },
  };
  const m = limitMeters(l, ["codex"], 1000, dark);
  assert.equal(m.plan, "pro");
  assert.deepEqual(
    m.rows.map((r) => [r.label, r.value]),
    [
      ["b31 · week", "69% · resets 5d 1h"],
      ["local · 5h", "95% · reset"],
    ],
  );
  assert.equal(m.rows[1].color, dark.state.down, "nearly used up is red");
  assert.equal(limitMeters(undefined, ["codex"], 0, dark).rows.length, 0);
});

test("the agent's detail", () => {
  const groups = groupAgents(report());
  const md = agentMarkdown({
    g: groups[0],
    open: openNow(fleet(), ["claude"]),
    today: row("claude", 38.39, 63_700_000),
    month: row("claude", 1447.4, 4.4e9),
    usage: true,
    t: dark,
    logo,
    now: 1240,
  });
  const svgs = svgsOf(md);
  assert.equal(svgs.length, 6, "hero, tiles, pills, sessions, machines, about");
  assert.ok(
    svgs[0].includes("#agent/claude") && !svgs[0].includes("2.1.309") && svgs[0].includes("on 2 of 3 machines"),
  );
  assert.ok(svgs[0].includes("local: integration outdated"));
  assert.ok(svgs[1].includes(">$38.39*<") && svgs[1].includes(">$1447*<") && svgs[1].includes("4.4B tokens"));
  assert.ok(svgs[2].includes(">needs you<") && svgs[2].includes(">working<"));
  assert.ok(svgs[3].includes(">temper<") && svgs[3].includes("b31 · px") && svgs[3].includes("#os/ubuntu"));
  assert.ok(
    svgs[4].includes(">b31<") && !svgs[4].includes(".local/bin") && !svgs[4].includes("2.1.309"),
    "versions and paths are the Doctor's",
  );
  assert.ok(svgs[4].includes(">offline<"), "a machine that didn't answer is listed");
  assert.ok(svgs[5].includes(">--continue<") && svgs[5].includes("Survives a herdr restart"));
  // nothing open, no usage read: just the card, the machines and what it is
  const bare = svgsOf(agentMarkdown({ g: groups[2], open: openNow([], ["opencode"]), usage: false, t: light, logo }));
  assert.equal(bare.length, 3);
  assert.ok(bare[1].includes(light.state.warn), "an install that needs a look is marked");
});

test("what a machine is missing", () => {
  const ready = agent("cursor", { installed: false, install_method: "script", install_ready: true });
  assert.deepEqual(missingTag(ready).text, "script");
  const blocked = agent("pi", {
    installed: false,
    install_ready: false,
    install_blocker: "Pi installs with npm, and b31 needs Node.js and npm first",
    prerequisites: [
      { what: "Node.js ≥ 18", ok: false, detail: "not found on b31", fix: "install Node.js 18 or newer" },
      { what: "npm", ok: false, detail: "not found", fix: "npm comes with Node.js" },
    ],
  });
  assert.deepEqual([missingTag(blocked).text, missingTag(blocked).tone], ["needs Node.js", "blocked"]);
  const manual = agent("omp", {
    installed: false,
    install_ready: false,
    install_command: null,
    install_method: "manual",
    install_blocker: "OMP has no install command we can run — see https://github.com/can1357/oh-my-pi.",
  });
  assert.deepEqual([missingTag(manual).text, missingTag(manual).tone], ["manual", "manual"]);
  const h = machine("b31", [agent("claude"), ready, blocked, manual]);
  assert.equal(missingSummary(h), "3 not installed · 1 can install");
  assert.equal(
    missingSummary(machine("x", [], { reachable: false, error: "ssh: Could not resolve hostname x" })),
    "unknown host — the name doesn't resolve",
  );

  const all = groupAgents({ hosts: [h, machine("local", [agent("pi", { version: "0.83.0" })])] }, true);
  const md = missingMarkdown({ h, a: blocked, g: all.find((g) => g.id === "pi"), t: dark, logo });
  const svgs = svgsOf(md);
  assert.equal(svgs.length, 3, "card, what's missing, installed on");
  assert.ok(svgs[0].includes("not on b31") && svgs[0].includes("needs Node.js"));
  assert.ok(svgs[1].includes("Needs Node.js ≥ 18 and npm") && svgs[1].includes("install Node.js 18 or newer"));
  assert.ok(svgs[2].includes(">local<") && !svgs[2].includes("0.83.0"), "where it is installed, not which version");
  assert.match(md, /```\nnpm i -g pi\n```/);
  const man = missingMarkdown({ h, a: manual, t: dark, logo });
  assert.ok(svgsOf(man)[1].includes("github.com/can1357/oh-my-pi<"), "the docs' site, no trailing dot");
});

test("cards: checklist, item rows, notice", () => {
  const c = checks(
    [
      { label: "Resume", state: "info", value: "--continue" },
      { label: "On PATH", state: "warn", value: "no" },
      { label: "curl", state: "ok", value: "/usr/bin/curl" },
      { label: "npm", state: "bad", value: "missing" },
    ],
    dark,
    400,
    { title: "Needs" },
  );
  assert.ok(
    c.includes(">Needs<") && c.includes(dark.state.warn) && c.includes(dark.state.down) && c.includes(dark.state.done),
  );
  assert.match(c, /height="118"/, "a title and four rows");
  const r = itemRows(
    [
      { name: "llama3.1:8b", sub: "8.0B · Q4_K_M", value: "6.2G", mark: { kind: "dot", color: dark.state.done } },
      { name: "qwen2.5-coder:7b", muted: true, mark: { kind: "ring", color: dark.other } },
    ],
    dark,
    400,
    { title: "Models", note: "1 of 2 loaded" },
  );
  assert.ok(r.includes(">llama3.1:8b<") && r.includes(">8.0B · Q4_K_M<") && r.includes(">6.2G<"));
  assert.match(r, /height="86"/, "a title, a two-line row, a one-line row");
  const n = notice(
    { state: "down", color: dark.state.down, title: "ssh timed out", detail: "The machine didn't answer." },
    dark,
  );
  assert.ok(n.includes(">ssh timed out<") && n.includes("didn&#39;t answer"));
  assert.deepEqual(wrap("one two three four", 40, 12, 2).length, 2);
  assert.ok(wrap("one two three four five six seven", 40, 12, 2)[1].endsWith("…"));
  assert.deepEqual(wrap("short", 400, 12), ["short"]);
});
