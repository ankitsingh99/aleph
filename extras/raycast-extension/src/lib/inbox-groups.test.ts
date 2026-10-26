// `npm test`: the Inbox's groups and their order (inbox-groups.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import { groupItems, GroupOpts } from "./inbox-groups";
import type { TimelineItem } from "./model";

const item = (t: number, host: string, kind: string, agent = "", launched = false): TimelineItem => ({
  key: `${t}|${host}|${kind}`,
  e: { t, host, name: agent ? `s${t}` : "", kind, title: "", agent },
  n: 1,
  first: t,
  logged: !launched,
  launch: launched ? { host, name: `s${t}`, dir: "/x", agent, args: [], t, prompt: null, worktree: false } : undefined,
});

const o: GroupOpts = {
  hosts: ["b31", "dex42", "local"],
  agentName: (id) => ({ claude: "Claude Code", codex: "Codex" })[id] ?? id,
  kindWord: (k) => ({ blocked: "needs you", error: "unreachable", done: "finished", new: "started" })[k] ?? k,
  dayOf: (t) => (t >= 100 ? "Today" : "Yesterday"),
};

const rows = [
  item(150, "local", "done", "claude"),
  item(140, "dex42", "error"),
  item(130, "b31", "blocked", "codex"),
  item(120, "zeta", "done", "claude"),
  item(110, "b31", "new", "claude", true),
  item(50, "dex42", "blocked", "claude"),
];

const shape = (g: ReturnType<typeof groupItems>) => g.map((x) => [x.title, x.items.map((it) => it.e.t)]);

test("by machine, sorted by machine: the fleet's order, unknown machines after, newest first inside", () => {
  assert.deepEqual(shape(groupItems(rows, "machine", "machine", o)), [
    ["b31", [130, 110]],
    ["dex42", [140, 50]],
    ["local", [150]],
    ["zeta", [120]],
  ]);
});

test("by machine, newest first: the machine with the newest row first", () => {
  assert.deepEqual(
    groupItems(rows, "machine", "time", o).map((g) => g.title),
    ["local", "dex42", "b31", "zeta"],
  );
});

test("by agent: names, a machine's own events last; a launch counts for its agent", () => {
  assert.deepEqual(shape(groupItems(rows, "agent", "agent", o)), [
    ["Claude Code", [150, 120, 110, 50]],
    ["Codex", [130]],
    ["Machine events", [140]],
  ]);
});

test("by kind, sorted by kind: what blocks work first; a launch is 'started'", () => {
  assert.deepEqual(
    groupItems(rows, "kind", "kind", o).map((g) => g.title),
    ["Needs you", "Unreachable", "Finished", "Started"],
  );
});

test("by day, sorted by machine: days newest first, machines in order inside", () => {
  assert.deepEqual(shape(groupItems(rows, "day", "machine", o)), [
    ["Today", [130, 110, 140, 150, 120]],
    ["Yesterday", [50]],
  ]);
});
