// `npm test`: the pure logic in model.ts (bundled by esbuild, run by node --test).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  bySource,
  dndState,
  doctorMarkdown,
  DoctorReport,
  doctorSorted,
  doctorTally,
  EventLike,
  explainError,
  fleetLink,
  fold,
  hostLink,
  joins,
  keeps,
  landing,
  LaunchLike,
  parseKinds,
  RAYCAST_KINDS,
  runnableFix,
  startedLines,
  timeline,
} from "./model";

const ev = (t: number, name: string, kind: string, title = "Fix tests", host = "b31"): EventLike => ({
  t,
  host,
  name,
  kind,
  title,
  agent: "claude",
});

const launch = (t: number, name: string, host = "b31"): LaunchLike => ({
  t,
  host,
  name,
  dir: "/srv/px",
  agent: "claude",
  args: [],
  prompt: "profile it",
  worktree: false,
});

test("errors read like the dashboard's", () => {
  assert.equal(
    explainError("ssh: connect to host gpu2.lan port 2222: Operation timed out").head,
    "unreachable — ssh timed out",
  );
  assert.equal(
    explainError("ssh: connect to host gpu2.lan port 2222: Operation timed out").detail,
    "gpu2.lan:2222 · connect to host gpu2.lan port 2222: Operation timed out",
  );
  assert.equal(explainError("error: server_not_running").head, "herdr stopped there");
  assert.equal(explainError("").head, "unreachable");
});

test("each machine's link", () => {
  assert.equal(hostLink({ alias: "a", reachable: true, error: null, latency_ms: 40 }).state, "up");
  assert.equal(hostLink({ alias: "a", reachable: true, error: null, latency_ms: 950 }).state, "slow");
  assert.equal(hostLink({ alias: "a", reachable: false, error: "Operation timed out" }).state, "down");
  assert.equal(hostLink({ alias: "a", reachable: false, error: "no herdr server running" }).state, "stopped");
  assert.equal(hostLink({ alias: "a", reachable: true, error: null, stale: true }).state, "unknown");
});

test("the fleet's connectivity", () => {
  const up = (alias: string, ms = 40) => ({ alias, reachable: true, error: null, latency_ms: ms });
  const down = (alias: string) => ({ alias, reachable: false, error: "Operation timed out" });
  assert.equal(fleetLink(undefined).state, "unknown");
  assert.equal(fleetLink([]).state, "unknown");
  assert.deepEqual(fleetLink([up("b31", 30), up("gpu2", 50), up("dev", 40)]), {
    state: "online",
    glyph: "●",
    text: "Online · 40 ms",
    tone: "good",
  });
  assert.equal(fleetLink([up("b31"), down("gpu2")]).text, "Weak — gpu2 is offline");
  assert.equal(fleetLink([up("b31", 1200)]).text, "Weak — b31 is slow");
  assert.equal(fleetLink([down("b31"), down("gpu2")]).state, "offline");
  // herdr stopped: the machine still answers, so the link isn't down
  assert.equal(
    fleetLink([{ alias: "b31", reachable: false, error: "server_not_running", latency_ms: 30 }]).state,
    "online",
  );
});

test("a launch joins its started event; the others stand alone", () => {
  const l = launch(1000, "ignite");
  assert.ok(joins(l, ev(1003, "ignite", "new")));
  assert.ok(!joins(l, ev(1200, "ignite", "new")), "too late");
  assert.ok(!joins(l, ev(990, "ignite", "new")), "too early");
  assert.ok(!joins(l, ev(1003, "ignite", "done")), "only started events");

  const items = timeline([ev(1003, "ignite", "new"), ev(1500, "ignite", "done")], [l, launch(500, "weave")]);
  assert.deepEqual(
    items.map((i) => [i.e.t, i.e.kind, i.launch?.name ?? "", i.logged]),
    [
      [1500, "done", "", true],
      [1003, "new", "ignite", true],
      [500, "new", "weave", false],
    ],
  );
  assert.equal(items[2].e.title, "/srv/px", "a launch on its own shows its folder");
});

test("identical runs fold into the newest", () => {
  const items = fold(
    timeline(
      [
        ev(400, "", "error", "timed out", "gpu2"),
        ev(300, "", "error", "timed out", "gpu2"),
        ev(200, "", "error", "timed out", "gpu2"),
        ev(100, "temper", "blocked"),
      ],
      [],
    ),
  );
  assert.equal(items.length, 2);
  assert.equal(items[0].n, 3);
  assert.equal(items[0].first, 200);
  assert.equal(items[1].e.kind, "blocked");
});

test("a banner click lands on its event", () => {
  const items = fold(
    timeline(
      [
        ev(500, "temper", "done"),
        ev(400, "temper", "blocked"),
        ev(300, "temper", "blocked"),
        ev(200, "weave", "blocked", "Fix tests", "gpu2"),
        ev(100, "temper", "new"),
      ],
      [],
    ),
  );
  // 300 is folded into the 400 row (×2)
  assert.equal(landing(items, { host: "b31", name: "temper", kind: "blocked", t: 300 })?.e.t, 400);
  assert.equal(landing(items, { host: "b31", name: "temper", kind: "done", t: 500 })?.e.kind, "done");
  assert.equal(
    landing(items, { host: "b31", name: "temper", kind: "blocked", t: 999 })?.e.t,
    400,
    "not in the list yet: its newest of that kind",
  );
  assert.equal(
    landing(items, { host: "b31", name: "temper", kind: "limit" })?.e.t,
    500,
    "no such kind: its newest row",
  );
  assert.equal(landing(items, { host: "gpu2", name: "weave", t: "200" })?.e.name, "weave", "t may come as text");
  assert.equal(landing(items, { host: "nope" }), undefined);
  assert.equal(landing(items, undefined), undefined);
});

test("filters", () => {
  const [launched, done] = [
    timeline([], [launch(1, "ignite")])[0],
    timeline([ev(5, "weave", "done", "x", "gpu2")], [])[0],
  ];
  assert.ok(keeps("launches", launched) && !keeps("launches", done));
  assert.ok(keeps("finished", done) && !keeps("problems", done));
  assert.ok(keeps("host:gpu2", done) && !keeps("host:b31", done));
  assert.ok(keeps("all", done));
});

test("notify.source, like the router's", () => {
  const herdr = { ...ev(100, "temper", "blocked"), source: "" };
  const agent = { ...ev(104, "temper", "blocked"), source: "agent", body: "Claude needs your permission to use Bash" };
  const down = ev(90, "", "error", "timed out", "gpu2");
  const all = [agent, herdr, down];
  assert.deepEqual(bySource(all, "herdr"), [herdr, down]);
  assert.deepEqual(bySource(all, "agents"), [agent, down]);
  assert.deepEqual(bySource(all, "both"), [agent, down], "the agent's words win when both said it");
  const later = { ...ev(400, "temper", "blocked"), source: "" };
  assert.deepEqual(bySource([later, agent], "both"), [later, agent], "15 s apart is another question");
});

test("do not disturb", () => {
  assert.deepEqual(dndState("", 1000), { on: false });
  assert.deepEqual(dndState("on", 1000), { on: true });
  assert.deepEqual(dndState("2000", 1000), { on: true, until: 2000 });
  assert.deepEqual(dndState("500", 1000), { on: false }, "a time that has passed");
});

test("events with ids and the agent's words", () => {
  const a = { ...ev(100, "temper", "blocked"), id: "abc" };
  assert.equal(timeline([a], [])[0].key, "abc");
  const twice = fold(
    timeline(
      [
        { ...ev(200, "temper", "blocked"), body: "Allow Bash?" },
        { ...ev(100, "temper", "blocked"), body: "Allow Edit?" },
      ],
      [],
    ),
  );
  assert.equal(twice.length, 2, "different words don't fold");
  // a folded run keeps every id (Dismiss hides them all); a launch on its own has none
  const run = fold(
    timeline(
      [
        { ...ev(300, "", "error", "timed out", "gpu2"), id: "e3" },
        { ...ev(200, "", "error", "timed out", "gpu2"), id: "e2" },
        { ...ev(100, "", "error", "timed out", "gpu2") },
      ],
      [launch(50, "ignite")],
    ),
  );
  assert.deepEqual(run[0].ids, ["e3", "e2"]);
  assert.equal(run[0].n, 3);
  assert.deepEqual(run[1].ids, [], "a launch isn't in the event log");
});

test("what a preset launch started", () => {
  assert.deepEqual(startedLines("✓ drifting-sextant on b31\n✓ train on gpu2\nwarning: no gpu tag\n"), [
    { name: "drifting-sextant", host: "b31" },
    { name: "train", host: "gpu2" },
  ]);
  assert.deepEqual(startedLines("error: no preset 'x'"), []);
});

test("notify.raycast.kinds", () => {
  assert.deepEqual(parseKinds(""), RAYCAST_KINDS);
  assert.deepEqual(parseKinds("blocked, limit"), ["blocked", "limit"]);
  assert.ok(parseKinds("all").includes("gone"));
  assert.deepEqual(parseKinds("none"), []);
});

// the shape `aleph doctor --json` prints (from the fake world)
const report: DoctorReport = {
  ok: 3,
  warn: 1,
  fail: 1,
  sections: [
    {
      section: "This Mac",
      checks: [
        { status: "ok", label: "aleph", text: "2.0.0", fix: [] },
        { status: "info", label: "hub", text: "not running right now", fix: [] },
      ],
    },
    {
      section: "Host · gpu2",
      checks: [
        { status: "ok", label: "ssh alias", text: "defined in ~/.ssh/config", fix: [] },
        {
          status: "fail",
          label: "dns",
          text: "gpu2.lan doesn't resolve",
          fix: ["check HostName, your VPN, or /etc/hosts"],
        },
      ],
    },
    {
      section: "On gpu2",
      checks: [
        {
          status: "warn",
          label: "Claude Code",
          text: "integration outdated (v7 → v11)",
          fix: ["aleph --host gpu2 agents setup claude"],
        },
      ],
    },
  ],
};

test("doctor: failures first", () => {
  const s = doctorSorted(report);
  assert.deepEqual(
    s.map((x) => x.section),
    ["Host · gpu2", "On gpu2", "This Mac"],
  );
  assert.deepEqual(
    s[0].checks.map((c) => c.label),
    ["dns", "ssh alias"],
  );
  assert.deepEqual(
    s[2].checks.map((c) => c.label),
    ["hub", "aleph"],
    "info before ok",
  );
  assert.equal(doctorTally(report), "1 failing · 1 warning · 3 ok");
  const md = doctorMarkdown(report, "Doctor");
  assert.ok(md.indexOf("Host · gpu2") < md.indexOf("This Mac"));
  assert.ok(md.includes("fix: `aleph --host gpu2 agents setup claude`"));
});

test("doctor: which fixes can run", () => {
  assert.deepEqual(runnableFix("aleph --host gpu2 agents setup claude"), [
    "--host",
    "gpu2",
    "agents",
    "setup",
    "claude",
  ]);
  assert.deepEqual(runnableFix("aleph --host b31 agents setup codex   — herdr restores its sessions with it"), [
    "--host",
    "b31",
    "agents",
    "setup",
    "codex",
  ]);
  assert.deepEqual(runnableFix("aleph theme herdr --apply   — adds them, keeping a backup"), [
    "theme",
    "herdr",
    "--apply",
  ]);
  assert.equal(runnableFix("check HostName, your VPN, or /etc/hosts"), undefined);
  assert.equal(runnableFix("brew install herdr   (or: curl -fsSL https://herdr.dev/install.sh | sh)"), undefined);
  assert.equal(runnableFix("aleph host key b31; rm -rf /"), undefined, "never anything a shell would chain");
});
