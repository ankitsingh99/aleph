// `npm test`: the message queue, drawn (queue-view.ts) — sections, place in
// line, what a message waits for, the track and the detail.
import assert from "node:assert/strict";
import { test } from "node:test";
import { chartTheme } from "./charts";
import type { Fleet, QueueTask, Session } from "./cli";
import {
  liveOf,
  position,
  queueGroups,
  queueMarkdown,
  stateColor,
  steps,
  targetName,
  targetTooltip,
  track,
  waitsFor,
} from "./queue-view";

const dark = chartTheme(true);
const light = chartTheme(false);
const PNG = "data:image/png;base64,iVBORw0KGgo=";
const logo = (kind: string, id: string | null | undefined) => (id ? `${PNG}#${kind}/${id}` : undefined);
const brand = (id: string) => ({ id, name: id === "claude" ? "Claude Code" : id, color: "#d97757" });
const NOW = 1_800_000_000;

const task = (over: Partial<QueueTask> = {}): QueueTask => ({
  id: 1,
  host: "b31",
  prompt: "run the tests\nthen commit",
  dir: null,
  agent: null,
  new: false,
  to: "temper",
  added: NOW - 600,
  state: "queued",
  session: null,
  at: null,
  error: null,
  after: null,
  ...over,
});

const sess = (name: string, status: string): Session => ({
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
  status_since: NOW - 240,
  cwd: "/home/me/x",
  title: "",
  mark: "",
  hue: "",
  glyph: "",
  color: "",
  focused: false,
});

const fleet: Fleet = {
  version: 1,
  generated_at: NOW,
  hosts: [
    {
      alias: "b31",
      machine: "b31",
      color: "#89b4fa",
      address: "",
      reachable: true,
      error: null,
      stale: false,
      fetched_at: NOW,
      latency_ms: 40,
      sessions: [sess("temper", "working")],
    },
  ],
};

const svgs = (md: string) =>
  [...md.matchAll(/base64,([^)]+)\)/g)].map((m) => Buffer.from(m[1], "base64").toString("utf8"));

test("sections, place in line, what it waits for", () => {
  const all = [
    task({ id: 4 }),
    task({ id: 2 }),
    task({ id: 3, to: null, dir: "/home/me/px" }),
    task({ id: 1, state: "sent", at: NOW - 100, session: "temper" }),
    task({ id: 5, state: "failed", at: NOW - 50, error: "it's waiting on a question" }),
  ];
  const g = queueGroups(all);
  assert.deepEqual(
    g.map((x) => [x.title, x.subtitle, x.tasks.map((t) => t.id)]),
    [
      ["Queued", "3 · for 2 sessions", [2, 3, 4]],
      ["Failed", "1", [5]],
      ["Sent", "1 · the last day", [1]],
    ],
  );
  assert.deepEqual(queueGroups([]), []);
  // the same session's line, oldest first; the machine's next-idle line apart
  assert.deepEqual(position(all[0], all), { n: 2, of: 2 });
  assert.deepEqual(position(all[1], all), { n: 1, of: 2 });
  assert.deepEqual(position(all[2], all), { n: 1, of: 1 });
  assert.equal(waitsFor(task(), NOW), "its reply");
  assert.equal(waitsFor(task({ to: null, dir: "/home/me/px" }), NOW), "an idle agent in ~/px");
  assert.equal(waitsFor(task({ to: null }), NOW), "an idle agent");
  assert.match(waitsFor(task({ after: NOW + 3600 }), NOW), /\d\d:\d\d/);
  // a time that has passed: it waits for its reply like any other
  assert.equal(waitsFor(task({ after: NOW - 60 }), NOW), "its reply");
  assert.equal(targetName(task()), "temper");
  assert.equal(targetName(task({ to: null })), "next idle");
  assert.equal(targetName(task({ to: null, session: "quill", state: "sent" })), "quill");
  assert.equal(liveOf(task(), fleet)?.name, "temper");
  assert.equal(liveOf(task({ to: "gone" }), fleet), undefined);
  assert.equal(liveOf(task({ to: null }), fleet), undefined);
  assert.equal(targetTooltip(task(), liveOf(task(), fleet), NOW), "temper on b31 (working) · waits for its reply · #1");
  assert.equal(stateColor("blocked", dark), dark.state.blocked);
  assert.equal(stateColor("nope", dark), dark.other);
});

test("the track", () => {
  assert.deepEqual(
    steps(task(), NOW).map((s) => [s.label, s.sub, s.mark]),
    [
      ["Queued", "10m ago", "done"],
      ["Waits for", "its reply", "now"],
      ["Sent", "not yet", "todo"],
    ],
  );
  assert.equal(steps(task({ after: NOW + 600 }), NOW)[1].label, "Not before");
  assert.deepEqual(
    steps(task({ state: "sent", at: NOW - 120 }), NOW).map((s) => [s.label, s.sub, s.mark]),
    [
      ["Queued", "10m ago", "done"],
      ["Waited", "8m", "done"],
      ["Sent", "2m ago", "done"],
    ],
  );
  assert.equal(steps(task({ state: "failed", at: NOW - 60 }), NOW)[2].mark, "fail");
  for (const t of [dark, light]) {
    const svg = track(steps(task(), NOW), t);
    assert.match(svg, />Waits for</);
    // what's to come is dashed; a failure ends in red
    assert.match(svg, /stroke-dasharray/);
    assert.match(track(steps(task({ state: "failed", at: NOW }), NOW), t), new RegExp(t.state.down));
  }
});

test("the detail", () => {
  const all = [task({ id: 1 }), task({ id: 2 })];
  const md = queueMarkdown({
    t: all[1],
    all,
    live: liveOf(all[1], fleet),
    hostColor: "#89b4fa",
    theme: dark,
    logo,
    brand,
    now: NOW,
  });
  const [card, line] = svgs(md);
  assert.match(card, />temper</);
  assert.match(card, />2 of 2</);
  assert.match(card, />working</);
  assert.match(card, /#agent\/claude/);
  assert.match(line, />Queued</);
  // the whole message, quoted line by line
  assert.match(md, /^> run the tests\n> then commit$/m);
  const idle = queueMarkdown({ t: task({ to: null, dir: "/home/me/px" }), all, theme: light, logo, brand, now: NOW });
  assert.match(svgs(idle)[0], />Next idle agent</);
  assert.match(svgs(idle)[0], />next</);
  const failed = queueMarkdown({
    t: task({ state: "failed", at: NOW, error: "it's <waiting>" }),
    all,
    theme: dark,
    logo,
    brand,
    now: NOW,
  });
  assert.match(failed, /\*\*Couldn't send:\*\* it's \\<waiting\\>/);
  assert.match(svgs(failed)[0], />not running</);
});
