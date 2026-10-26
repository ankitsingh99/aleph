// `npm test`: the menu bar without Raycast (menu-view.ts) — its title, the
// status line, who each section lists, which Inbox messages still count,
// and each line's words.
import assert from "node:assert/strict";
import { test } from "node:test";
import type { Host, Session } from "./cli";
import {
  agentUsage,
  countsLine,
  dollars,
  freshMessages,
  headlineWindow,
  inState,
  inboxRows,
  machineTitle,
  machinesLine,
  menuTitle,
  messageTitle,
  more,
  sessionLine,
  sessionTitle,
  sessionsLine,
  stateCounts,
  tokensText,
  tracksRead,
  unreadOnly,
  sessionName,
  stateDot,
  machineState,
} from "./menu-view";
import { TimelineItem } from "./model";

const NOW = 1_800_000_000;

const sess = (name: string, status: string, since: number, extra: Partial<Session> = {}): Session => ({
  host: "b31",
  name,
  label: name,
  display: name,
  pane_id: `w1:${name}`,
  tab_id: "",
  workspace_id: "",
  terminal_id: "",
  agent: "claude",
  status,
  status_since: NOW - since,
  cwd: "/home/me/x",
  title: "",
  mark: "",
  hue: "",
  glyph: "",
  color: "",
  focused: false,
  ...extra,
});

const host = (alias: string, sessions: Session[], over: Partial<Host> = {}): Host => ({
  alias,
  machine: alias,
  color: "#89b4fa",
  address: "",
  reachable: true,
  error: null,
  stale: false,
  fetched_at: NOW,
  latency_ms: 85,
  sessions,
  ...over,
});

const item = (kind: string, name: string, h = "b31", n = 1, unread?: boolean): TimelineItem => ({
  key: `${kind}${name}${h}`,
  e: { t: NOW - 240, host: h, name, kind, title: "", unread },
  n,
  first: NOW - 600,
  logged: true,
});

test("title, status line, sections", () => {
  const all = [
    sess("temper", "blocked", 60),
    sess("ignite", "blocked", 600),
    sess("weave", "working", 30),
    sess("quill", "working", 300),
    sess("loom", "idle", 10, { limit: "2:40am" }),
    sess("shell", "done", 5),
  ];
  const c = stateCounts(all);
  assert.deepEqual(c, { blocked: 2, working: 2, limit: 1, done: 1 });
  assert.equal(countsLine(c), "2 need you · 2 working · 1 at limit · 1 done");
  assert.equal(countsLine({}), "No sessions running");
  assert.equal(menuTitle(2, { state: "online", glyph: "●" }, false), "▲2");
  assert.equal(menuTitle(2, { state: "weak", glyph: "◐" }, true), "▲2 ◐ ⊘");
  assert.equal(menuTitle(0, { state: "online", glyph: "●" }, false), undefined);
  assert.equal(menuTitle(0, { state: "offline", glyph: "○" }, false), "○");
  // waiting the longest first; at work the latest first
  assert.deepEqual(
    inState(all, "blocked", 8).map((s) => s.name),
    ["ignite", "temper"],
  );
  assert.deepEqual(
    inState(all, "working", 8).map((s) => s.name),
    ["weave", "quill"],
  );
  assert.deepEqual(
    inState(all, "working", 1).map((s) => s.name),
    ["weave"],
  );
  assert.deepEqual(
    inState(all, "limit", 3).map((s) => s.name),
    ["loom"],
  );
  assert.deepEqual(sessionLine(all[0], NOW), { title: "temper", subtitle: "b31 · 1m" });
  assert.equal(sessionTitle(all[1], NOW), "ignite  ·  b31 · 10m");
  assert.equal(more(5, 3), "+2 more");
  assert.equal(more(3, 3), "");
});

test("which Inbox messages still count, and their words", () => {
  const hosts = [
    host("b31", [sess("temper", "blocked", 60), sess("loom", "idle", 10, { limit: "" })]),
    host("gpu2", [], { reachable: false, error: "ssh: connect to host x port 22: Operation timed out" }),
  ];
  const items = [
    item("blocked", "temper"),
    item("done", "weave"),
    item("error", "", "b31"),
    item("error", "", "gpu2"),
    item("limit", "loom"),
    item("limit", "quill"),
    item("blocked", "old", "gpu2"),
  ];
  // a question (Needs You shows what waits now), a machine back up, a limit lifted: gone;
  // a machine we can't reach keeps its messages
  assert.deepEqual(
    freshMessages(items, hosts).map((it) => `${it.e.kind}:${it.e.name || it.e.host}`),
    ["done:weave", "error:gpu2", "limit:loom", "blocked:old"],
  );
  // before the fleet answers: all of them
  assert.equal(freshMessages(items, undefined).length, items.length);
  assert.equal(
    messageTitle(item("done", "weave", "b31", 3, true), "finished", NOW),
    "weave  ·  finished ×3  ·  4m  ·  new",
  );
  assert.equal(messageTitle(item("error", "", "gpu2"), "unreachable", NOW), "gpu2  ·  unreachable  ·  4m");
  assert.match(
    machineTitle(hosts[0]),
    /^(🟢|🟡) b31 {3}▲1 needs you · 2 sessions$/,
    "its circle, who needs you, the count",
  );
  assert.equal(machineTitle(host("dex42", [])), "🟢 dex42", "no sessions: just its name");
  assert.equal(machineTitle(hosts[1]), "🔴 gpu2   ssh timed out", "down: just why");
  assert.equal(
    machineTitle({ ...host("box", []), reachable: false, error: "herdr: server not running" } as Host),
    "🔴 box",
    "herdr stopped: the red circle alone",
  );
});

test("the menu's Inbox: unread only, empty after Mark All Read", () => {
  const ev = (t: number, host: string, name: string, kind: string, unread?: boolean, title = "") => ({
    t: NOW - t,
    host,
    name,
    kind,
    title,
    ...(unread === undefined ? {} : { unread }),
  });
  const hosts = [
    host("b31", [sess("temper", "blocked", 60)]),
    host("dex42", [sess("weave", "idle", 60)]),
    host("gpu2", [], { reachable: false, error: "ssh: connect to host x port 22: Operation timed out" }),
  ];
  const o = { kinds: ["blocked", "done", "error", "limit"], source: "both", hosts };
  // the user's report: answered questions and machines long back up, already read — none of it
  const read = [
    ev(3600, "dex42", "frosting-crane", "blocked", false),
    ev(3600, "dex42", "", "error", false, "timed out"),
    ev(3700, "b31", "", "error", false, "timed out"),
    ev(3800, "b31", "", "error", false, "timed out"),
    ev(3900, "b31", "weave", "done", false, "all green"),
  ];
  assert.equal(tracksRead(read), true);
  assert.deepEqual(unreadOnly(read), []);
  assert.deepEqual(inboxRows(read, o), []);
  // unread ones stay: a finished turn, a machine still down; a question waiting now is
  // the live Needs You section's, not the Inbox's; kinds not listed stay out
  const fresh = [
    ...read,
    ev(30, "dex42", "weave", "done", true, "fixed it"),
    ev(40, "dex42", "weave", "done", true, "fixed it"),
    ev(60, "b31", "temper", "blocked", true),
    ev(90, "gpu2", "", "error", true, "timed out"),
    ev(95, "b31", "temper", "new", true),
  ];
  assert.deepEqual(
    inboxRows(fresh, o).map((it) => `${it.e.kind}:${it.e.name || it.e.host}×${it.n}`),
    ["done:weave×2", "error:gpu2×1"],
  );
  // an inbox that says nothing about read (an old build): the newest still show
  const old = [ev(30, "dex42", "weave", "done"), ev(90, "gpu2", "", "error")];
  assert.equal(tracksRead(old), false);
  assert.equal(unreadOnly(old).length, 2);
  assert.equal(inboxRows(old, o).length, 2);
});

test("the menu's status lines: sessions, machines, today's usage", () => {
  assert.deepEqual(sessionsLine({ blocked: 1, working: 2, idle: 3 }), {
    title: "6 sessions",
    subtitle: "1 needs you · 2 working · 3 idle",
    state: "blocked",
  });
  assert.equal(sessionsLine({}).title, "No sessions running");
  const h = (alias: string, extra: Record<string, unknown> = {}) =>
    ({ alias, reachable: true, latency_ms: 100, sessions: [], ...extra }) as unknown as Host;
  assert.deepEqual(
    machinesLine([h("b31", { latency_ms: 90 }), h("dex42", { latency_ms: 197 }), h("local", { latency_ms: 12 })]),
    {
      title: "All 3 machines online",
      subtitle: "12–197 ms",
      tone: "good",
    },
  );
  const down = machinesLine([
    h("b31"),
    h("q7x", { reachable: false, error: "ssh: connect to host q7x port 22: Operation timed out" }),
  ]);
  assert.equal(down.title, "1 of 2 machines online");
  assert.equal(down.subtitle, "q7x offline");
  assert.equal(down.tone, "warn");
});

test("the menu's usage: each agent's week, today and a plan per account, the costliest first", () => {
  const row = (key: string, cost: number, tokens: number, estimated = true) => ({ key, cost, tokens, estimated });
  const u = agentUsage(
    [row("codex", 900, 1e9), row("claude", 1200, 4e9)],
    [row("claude", 131.06, 4e8)],
    {
      b31: {
        codex: { codex: { ts: 5, account: "c1", windows: [{ used_percent: 69, minutes: 10080 }] } },
        claude: { claude: { ts: 20, plan: "Pro", account: "p", windows: [{ used_percent: 23, minutes: 10080 }] } },
      },
      local: {
        codex: { codex: { ts: 9, account: "c1", windows: [{ used_percent: 1, minutes: 10080, resets_at: 100 }] } },
        claude: {
          claude: {
            ts: 9,
            plan: "Max 5x",
            account: "m",
            hosts: ["local", "dex42"],
            windows: [
              { used_percent: 13, minutes: 300 },
              { used_percent: 44, minutes: 10080 },
            ],
          },
        },
      },
    },
    50,
  );
  assert.deepEqual(
    u.map((x) => x.agent),
    ["claude", "codex"],
  );
  assert.deepEqual(
    u[1].quotas.map((q) => q.windows),
    [[{ minutes: 10080, pct: 1, resets_at: 100 }]],
    "one account on two machines: its newest reading",
  );
  assert.deepEqual(
    u[0].quotas.map((q) => [q.plan, q.hosts]),
    [
      ["Max 5x", ["local", "dex42"]],
      ["Pro", ["b31"]],
    ],
    "two accounts, two plans, the fullest first",
  );
  assert.equal(headlineWindow(u[0].quotas[0].windows)?.pct, 44, "the week");
  assert.equal(u[0].today?.cost, 131.06);
  assert.equal(dollars({ cost: 131.061, estimated: true }), "$131.06*");
  assert.equal(dollars({ cost: 1447.4 }), "$1447");
  assert.equal(dollars(undefined), "–");
  assert.equal(tokensText(1.2e9), "1.2B tok");
});

test("a session's name: a bare number gives way to its folder, else the agent", () => {
  const s = (name: string, cwd?: string) => ({ name, display: "", pane_id: "w1:p2", cwd });
  assert.equal(sessionName(s("frosting-crane", "/x")), "frosting-crane");
  assert.equal(sessionName(s("1", "/home/n/test_folder/")), "test_folder");
  assert.equal(sessionName(s("1"), "Claude Code"), "Claude Code");
  assert.equal(sessionName(s("", "-")), "w1:p2");
});

test("a machine's dot: green online, yellow while an agent works there, red offline", () => {
  const h = (extra: Record<string, unknown>) =>
    ({ alias: "b31", reachable: true, latency_ms: 90, sessions: [], ...extra }) as unknown as Host;
  assert.equal(stateDot(h({})), "🟢");
  assert.equal(stateDot(h({ sessions: [{ status: "working" }] })), "🟡");
  assert.equal(stateDot(h({ reachable: false, error: "ssh: connect to host b31 port 22: Operation timed out" })), "🔴");
  assert.equal(machineState(h({ sessions: [{ status: "working" }] })), "busy");
  assert.equal(machineTitle(h({ latency_ms: 900 })), "🟢 b31   900 ms — slow", "the ping only when slow");
});
