// `npm test`: the Daemon command's view (daemon-view.ts) — what the daemon's
// answers mean, the last exit in words, the status card, the machines it
// watches, today's tiles, remote access and the whole markdown.
import assert from "node:assert/strict";
import { test } from "node:test";
import { chartTheme, contrast } from "./charts";
import type { DaemonStatus, Host, InboxEvent, QueueTask } from "./cli";
import {
  alertSinks,
  badgesOf,
  compact,
  daemonMarkdown,
  daemonView,
  heading,
  keyHints,
  lastExit,
  midnight,
  nextStep,
  remoteGroup,
  remoteKeys,
  stack,
  statusCard,
  statusOf,
  themed,
  todayTiles,
  uptimeText,
  watchedOf,
  watchTiles,
  WIDE,
} from "./daemon-view";

const dark = chartTheme(true);
const light = chartTheme(false);
const NOW = 1_792_737_000;
const PNG = "data:image/png;base64,iVBORw0KGgo=";
const logo = (kind: string, id: string | null | undefined) => (id ? `${PNG}#${kind}/${id}` : undefined);

/** What `aleph daemon status --json` printed on a Mac with the daemon running (trimmed). */
const details = (over: Partial<DaemonStatus> = {}): DaemonStatus => ({
  running: true,
  kind: "daemon",
  version: "2.0.0",
  pid: 24075,
  started_at: NOW - 4402,
  uptime_s: 4402,
  hub: true,
  hub_holder: 24075,
  http: null,
  hosts: ["b31", "dex42", "local"],
  socket: "/Users/me/.local/state/aleph/daemon.sock",
  binary: { version: "2.0.0", path: "/Users/me/.cargo/bin/aleph" },
  version_match: true,
  service: {
    manager: "launchd",
    name: "dev.aleph.daemon",
    file: "/Users/me/Library/LaunchAgents/dev.aleph.daemon.plist",
    installed: true,
    loaded: true,
    running: true,
    pid: 24075,
    state: "running",
    last_exit: "143",
    enabled: null,
    error: null,
    linger: null,
  },
  hub_machine: { setting: "mac", this_machine: "mac", here: true, holder: 24075 },
  logs: { path: "/Users/me/.local/state/aleph/aleph.log", bytes: 52580, service_log: null, journal: null },
  api: { http: false, listen: "127.0.0.1:7878", listen_any: false, hosts: [], answers: false, tokens: null },
  exposure: { tailscale: [], cloudflare: null },
  from: "cli",
  ...over,
});

const stopped = (over: Partial<DaemonStatus> = {}) =>
  details({
    running: false,
    kind: undefined,
    version: undefined,
    pid: undefined,
    uptime_s: undefined,
    hub: undefined,
    error: "no daemon is running",
    version_match: null,
    service: { ...details().service!, running: false, pid: null, state: "not running", last_exit: "137" },
    ...over,
  });

const host = (alias: string, over: Partial<Host> = {}): Host => ({
  alias,
  machine: alias,
  color: "#89b4fa",
  address: `me@${alias}`,
  reachable: true,
  error: null,
  stale: false,
  fetched_at: NOW,
  latency_ms: 120,
  sessions: [],
  ...over,
});

const session = (status: string) =>
  ({ host: "b31", name: "s", status, status_since: NOW - 60 }) as unknown as Host["sessions"][number];

const ev = (kind: string, t: number): InboxEvent => ({ t, host: "b31", name: "s", kind, title: "" });

const decode = (md: string) =>
  [...md.matchAll(/\(data:image\/svg\+xml;base64,([^)]+)\)/g)].map((m) => Buffer.from(m[1], "base64").toString("utf8"));

test("the last exit in words: 143 is a normal stop", () => {
  assert.deepEqual(lastExit("143"), { text: "last run was stopped (SIGTERM)", normal: true });
  assert.deepEqual(lastExit(143), { text: "last run was stopped (SIGTERM)", normal: true });
  assert.equal(lastExit("130")?.normal, true, "SIGINT");
  assert.deepEqual(lastExit("137"), { text: "last run was killed (SIGKILL)", normal: false });
  assert.equal(lastExit("139")?.text, "last run crashed (SIGSEGV)");
  assert.equal(lastExit("-9")?.text, "last run was killed (SIGKILL)");
  assert.deepEqual(lastExit("1"), { text: "last run failed (exit 1)", normal: false });
  assert.equal(lastExit("0")?.normal, true);
  assert.equal(lastExit("78: Function not implemented")?.text, "last run failed (exit 78)");
  // systemd's results
  assert.equal(lastExit("exit-code")?.normal, false);
  assert.match(lastExit("core-dump")!.text, /crashed/);
  assert.equal(lastExit("whatever")?.text, "last run: whatever");
  // never exited: nothing to say
  for (const x of ["(never exited)", "", null, undefined, "success"]) assert.equal(lastExit(x), undefined);
});

test("uptime, midnight, compact numbers", () => {
  assert.equal(uptimeText(42), "42s");
  assert.equal(uptimeText(13 * 60 + 5), "13m");
  assert.equal(uptimeText(4402), "1h 13m");
  assert.equal(uptimeText(7200), "2h");
  assert.equal(uptimeText(3 * 86400 + 4 * 3600 + 5), "3d 4h");
  assert.equal(uptimeText(2 * 86400), "2d");
  assert.equal(uptimeText(undefined), "–");
  const m = midnight(NOW);
  assert.ok(m <= NOW && NOW - m < 86400);
  assert.equal(new Date(m * 1000).getHours(), 0);
  assert.equal(compact(950), "950");
  assert.equal(compact(12_400), "12K");
  assert.equal(compact(1_240), "1.2K");
  assert.equal(compact(50_685_272), "51M");
  assert.equal(compact(1.2e9), "1.2B");
});

test("what the answers mean", () => {
  const d = details();
  const live = { ...d, from: "api" as const };
  let v = daemonView(live, d, {});
  assert.equal(v.state, "running");
  assert.ok(v.running && v.installed);
  assert.equal(v.listening, null);
  assert.equal(v.httpOn, false);
  // the socket's answer wins: it listens
  v = daemonView({ ...live, http: "127.0.0.1:7878" }, d, {});
  assert.equal(v.listening, "127.0.0.1:7878");
  // a dashboard standing in
  assert.equal(daemonView({ ...live, kind: "dashboard" }, undefined).state, "standin");
  // not running
  v = daemonView(undefined, stopped());
  assert.equal(v.state, "stopped");
  assert.ok(!v.running && v.installed);
  // no answer yet, or no daemon in this aleph
  assert.equal(daemonView(undefined, undefined, {}, { loading: true }).state, "checking");
  assert.equal(daemonView(undefined, undefined, {}, { unsupported: true }).state, "missing");
  // api.http from the settings when the CLI hasn't said
  assert.equal(daemonView(live, undefined, { "api.http": "on" }).httpOn, true);
  assert.equal(daemonView(live, undefined, { "api.http": "off" }).httpOn, false);
});

test("the next step: only when something needs doing", () => {
  const d = details();
  assert.equal(nextStep(daemonView(d, d)), undefined, "all well: nothing to do");
  assert.equal(nextStep(daemonView(undefined, stopped()))?.op, "start");
  const bare = stopped({ service: { ...stopped().service!, installed: false } });
  assert.equal(nextStep(daemonView(undefined, bare))?.op, "install");
  const old = details({ version: "1.9.0", version_match: false });
  const step = nextStep(daemonView(old, old));
  assert.equal(step?.op, "restart");
  assert.match(step!.text, /1\.9\.0.*2\.0\.0/);
  const byHand = details({
    version: "1.9.0",
    version_match: false,
    service: { ...details().service!, installed: false },
  });
  assert.equal(nextStep(daemonView(byHand, byHand))?.op, undefined, "no service to restart it");
  const on = details({ api: { ...details().api!, http: true } });
  assert.equal(nextStep(daemonView(on, on))?.op, "restart", "api.http on, nothing listening yet");
  assert.equal(nextStep(daemonView({ ...d, kind: "dashboard" }, undefined))?.op, "install");
  const nolinger = details({ service: { ...details().service!, linger: false } });
  assert.match(nextStep(daemonView(nolinger, nolinger))!.text, /log out/);
  assert.equal(nextStep(daemonView(undefined, undefined, {}, { unsupported: true }))?.op, undefined);
});

test("the badges: the service, the hub, the version; a normal stop says nothing", () => {
  const d = details();
  let b = badgesOf(daemonView(d, d), logo);
  assert.deepEqual(
    b.map((x) => x.text),
    ["launchd", "hub", "up to date"],
  );
  assert.equal(b[0].note, "running · at login");
  assert.ok(b[0].logo?.endsWith("#os/apple"), "launchd wears Apple's logo");
  assert.equal(b[1].note, "runs here");
  assert.ok(!b.some((x) => /143|SIGTERM|exit/.test(`${x.text} ${x.note ?? ""}`)), "143 is a normal stop: not shown");
  // stopped and killed: said in words
  b = badgesOf(daemonView(undefined, stopped()), logo);
  assert.deepEqual(
    b.map((x) => x.text),
    ["launchd", "last run was killed (SIGKILL)"],
  );
  assert.equal(b[0].note, "loaded, not running");
  // the hub elsewhere; systemd; a different version is the next step, not a badge
  const away = details({
    hub: false,
    hub_machine: { setting: "dex42", this_machine: "mac", here: false, holder: null },
    version: "1.9.0",
    version_match: false,
    service: { ...details().service!, manager: "systemd --user", linger: false },
  });
  b = badgesOf(daemonView(away, away), logo);
  assert.equal(b.find((x) => x.text === "hub")?.note, "on dex42");
  assert.ok(b[0].logo?.endsWith("#os/linux") && b[0].text === "systemd");
  assert.ok(!b.some((x) => /1\.9\.0/.test(x.text)));
  assert.ok(b.some((x) => x.text === "linger off"));
  // colour names become the theme's
  const t = themed(
    [
      { text: "x", color: "done" },
      { text: "y", color: "accent" },
      { text: "z", color: "other" },
    ],
    dark,
  );
  assert.deepEqual(
    t.map((x) => x.color),
    [dark.state.done, dark.accent, dark.other],
  );
});

test("the status card: the state in its colour, uptime, the badges, the step", () => {
  for (const t of [dark, light]) {
    const d = details();
    const card = statusOf(daemonView(d, d), logo);
    assert.equal(card.title, "Running");
    assert.equal(card.subtitle, "Aleph 2.0.0 · pid 24075");
    assert.deepEqual(card.aside, { value: "1h 13m", label: "uptime" });
    const svg = statusCard({ ...card, badges: themed(card.badges, t) }, t);
    assert.match(svg, new RegExp(`^<svg [^>]*width="${WIDE}"`));
    assert.ok(svg.includes(">Running<") && svg.includes(">1h 13m<") && svg.includes(">ℵ<"));
    assert.ok(svg.includes(`fill="${t.state.done}" font-size="22" font-weight="700">Running`), "green title");
    assert.ok(contrast(t.state.done, t.surface) >= 3);
    assert.ok(!svg.includes("143"));
    // stopped: red, no uptime, the step with its key
    const off = statusOf(daemonView(undefined, stopped()), logo);
    const s2 = statusCard({ ...off, badges: themed(off.badges, t) }, t);
    assert.equal(off.aside, undefined);
    assert.ok(s2.includes(`fill="${t.state.down}" font-size="22" font-weight="700">Not running`));
    assert.ok(s2.includes(">↵<") && s2.includes(">start<"));
    assert.ok(s2.includes("SIGKILL"));
  }
});

test("the machines it watches: links, sessions, who needs you, offline with why", () => {
  const hosts = [
    host("b31", { latency_ms: 113, sessions: [session("blocked"), session("working")] }),
    host("gpu2", {
      reachable: false,
      latency_ms: null,
      error: "ssh: connect to host 10.0.0.9 port 22: Operation timed out",
    }),
    host("vps", { latency_ms: 950 }),
  ];
  const hist = {
    b31: [
      { t: NOW - 20, ms: 100, up: true },
      { t: NOW - 10, ms: 140, up: true },
      { t: NOW, ms: 113, up: true },
    ],
  };
  const rows = watchedOf(hosts, ["b31"], { hist, logo, osText: (a) => (a === "b31" ? "Ubuntu 22.04" : undefined) });
  assert.deepEqual(
    rows.map((r) => [r.alias, r.state, r.bars, r.link]),
    [
      ["b31", "up", 4, "113 ms"],
      ["gpu2", "down", 0, "offline"],
      ["vps", "slow", 1, "950 ms"],
    ],
  );
  assert.equal(rows[0].needs, 1);
  assert.equal(rows[0].sessions, 2);
  assert.ok(rows[0].logo?.endsWith("#os/ubuntu"));
  assert.equal(rows[0].history?.length, 3);
  assert.equal(rows[1].why, "ssh timed out");
  for (const t of [dark, light]) {
    const svg = watchTiles(rows, t);
    assert.ok(svg.includes(">b31<") && svg.includes(">113 ms<") && svg.includes("2 sessions"));
    assert.ok(svg.includes("<title>1 needs you</title>") && svg.includes(`fill="${t.state.blocked}"`), "▲ 1 by b31");
    assert.ok(svg.includes(">offline<") && svg.includes(">ssh timed out<"));
    assert.ok(svg.includes(`stroke="${t.state.down}"`), "the offline machine's tile is red");
  }
  // before the first snapshot: the daemon's names, not polled yet
  const early = watchedOf(undefined, ["b31", "dex42"]);
  assert.deepEqual(
    early.map((r) => [r.alias, r.state, r.link]),
    [
      ["b31", "unknown", "not polled yet"],
      ["dex42", "unknown", "not polled yet"],
    ],
  );
  assert.match(watchTiles([], dark), /No machines yet/);
});

test("today: events, auto-approvals, the queue, usage's last read", () => {
  const today = midnight(NOW) + 60;
  const events = [
    ev("blocked", today + 10),
    ev("blocked", today + 20),
    ev("done", today + 30),
    ev("auto", today + 40),
    ev("remote", today + 50),
    ev("error", today + 60),
    ev("blocked", today - 3600 * 5), // yesterday
  ];
  const queue: QueueTask[] = [
    { id: 1, state: "queued", at: null },
    { id: 2, state: "queued", at: null },
    { id: 3, state: "sent", at: today + 5 },
    { id: 4, state: "sent", at: today - 86400 },
    { id: 5, state: "failed", at: today },
  ].map((q) => ({
    host: "b31",
    prompt: "",
    dir: null,
    agent: null,
    new: false,
    to: null,
    added: 0,
    session: null,
    error: null,
    ...q,
  }));
  const usage = {
    range: "today",
    by: "total",
    rows: [],
    enabled: true,
    synced: { b31: NOW - 14 * 60, dex42: NOW - 30 * 60, gpu2: 0, old: NOW - 60 },
    total: { cost: 29.443204, tokens: 50_685_272 },
  };
  const tiles = todayTiles({
    events,
    cap: 300,
    queue,
    usage,
    settings: { auto_approve: "true" },
    hosts: ["b31", "dex42"],
    now: NOW,
  });
  assert.deepEqual(
    tiles.map((x) => [x.label, x.value]),
    [
      ["Events", "6"],
      ["Auto-approved", "1"],
      ["Queue", "2"],
      ["Usage read", "14m ago"],
    ],
  );
  assert.deepEqual(tiles[0].sub, ["2 needed you", "1 finished", "1 offline"]);
  assert.deepEqual(tiles[1].sub, ["1 remote action"]);
  assert.deepEqual(tiles[2].sub, ["1 sent today", "1 failed"]);
  assert.equal(tiles[2].suffix, "waiting");
  assert.deepEqual(tiles[3].sub, ["$29.44 today", "51M tokens"]);
  // all 300 from today: there are more
  const full = Array.from({ length: 3 }, (_, i) => ev("done", today + i));
  assert.equal(todayTiles({ events: full, cap: 3, now: NOW })[0].value, "3+");
  // nothing yet, things turned off
  const off = todayTiles({
    events: [],
    usage: { range: "today", by: "total", rows: [], enabled: false },
    settings: { auto_approve: "false" },
    now: NOW,
  });
  assert.deepEqual(off[0].sub, ["quiet so far"]);
  assert.equal(off[1].value, "off");
  assert.equal(off[2].value, "–", "the queue not read");
  assert.equal(off[3].value, "off");
  assert.equal(todayTiles({ now: NOW })[0].value, "–");
});

test("where alerts go", () => {
  assert.equal(
    alertSinks({ "notify.ntfy": "https://ntfy.sh/x", "notify.raycast": "true", moshi: "on", "notify.slack": "" }),
    "alerts go to ntfy · Raycast · Moshi",
  );
  assert.equal(alertSinks({ notify: "false", "notify.ntfy": "x" }), "alerts off");
  assert.equal(
    alertSinks({ "notify.webhook": "https://x", "notify.dnd": "on" }),
    "alerts go to webhook · do not disturb",
  );
  assert.equal(alertSinks({ "notify.dnd": "1h" }), "do not disturb");
  assert.equal(alertSinks(undefined), "");
});

test("remote access: chips and the keys that change it", () => {
  const off = details();
  let v = daemonView(off, off);
  let g = remoteGroup(v, [], dark);
  assert.equal(g.note, "this computer only");
  assert.deepEqual(
    g.chips.map((c) => [c.text, c.note]),
    [
      ["HTTP API", "off"],
      ["Tailscale", "off"],
      ["Cloudflare", "off"],
      ["Devices", "none paired"],
    ],
  );
  assert.ok(g.chips.every((c) => c.dot === dark.other));
  assert.deepEqual(
    remoteKeys(v).map((k) => k.key),
    ["⇧⌘A", "⇧⌘P"],
  );
  // stopped: pairing waits for it
  assert.deepEqual(
    remoteKeys(daemonView(undefined, stopped())).map((k) => k.key),
    ["⇧⌘A"],
  );
  // listening, on the tailnet, two devices
  const on = details({
    http: "127.0.0.1:7878",
    api: { ...details().api!, http: true },
    exposure: { tailscale: [{ url: "https://mac.tail1234.ts.net" }], cloudflare: null },
  });
  v = daemonView({ ...on, from: "api" }, on);
  g = remoteGroup(
    v,
    [
      { name: "pixel", scope: "reply", created: NOW, last_used: NOW, push: 1 },
      { name: "ipad", scope: "read", created: NOW, last_used: null, push: 0 },
    ],
    dark,
  );
  assert.equal(g.note, "reachable from your tailnet");
  assert.deepEqual(
    g.chips.map((c) => [c.note, c.dot]),
    [
      ["127.0.0.1:7878", dark.state.done],
      ["mac.tail1234.ts.net", dark.state.done],
      ["off", dark.other],
      ["2 paired · 1 with push", dark.state.done],
    ],
  );
  assert.deepEqual(
    remoteKeys(v).map((k) => k.key),
    ["⇧⌘P", "⇧⌘W"],
    "already on the tailnet: no ⇧⌘T",
  );
  // devices unknown (an older aleph): no chip
  assert.equal(remoteGroup(v, undefined, dark).chips.length, 3);
  // on, waiting for a restart
  const pending = details({ api: { ...details().api!, http: true } });
  v = daemonView(pending, pending);
  assert.equal(remoteGroup(v, [], dark).chips[0].note, "restart to start");
  assert.deepEqual(
    remoteKeys(v).map((k) => k.key),
    ["↵"],
  );
});

test("pieces: stack, heading, key hints", () => {
  const a = heading("Watching", "3 machines", dark, 300);
  const b = keyHints([{ key: "⇧⌘P", label: "pair a device" }], dark, 300);
  const s = stack([a, "", b], 300, 10);
  const h = (svg: string) => Number(/^<svg [^>]*?height="(\d+)"/.exec(svg)![1]);
  assert.equal(h(s), h(a) + 10 + h(b));
  assert.ok(s.includes('<svg x="0" y="0" ') && s.includes(`<svg x="0" y="${h(a) + 10}" `));
  assert.match(s, /aria-label="Watching: 3 machines; ⇧⌘P pair a device"/);
  assert.ok(b.includes(">⇧⌘P<") && b.includes(">pair a device<"));
});

test("the whole view: cards, no prose", () => {
  const d = details();
  const v = daemonView({ ...d, from: "api" }, d, {});
  const md = daemonMarkdown({
    v,
    fleet: [host("b31"), host("dex42"), host("local")],
    events: [ev("blocked", NOW - 10)],
    queue: [],
    usage: { range: "today", by: "total", rows: [], enabled: true, synced: { b31: NOW - 600 } },
    devices: [],
    settings: { "notify.ntfy": "x" },
    t: dark,
    logo,
    now: NOW,
  });
  const svgs = decode(md);
  assert.equal(svgs.length, 4, "status, machines, today, remote access");
  // nothing but images: no paragraphs
  assert.equal(md.replace(/!\[[^\]]*\]\([^)]*\)/g, "").trim(), "");
  assert.ok(svgs[1].includes(">Watching<") && svgs[1].includes("3 machines"));
  assert.ok(svgs[2].includes(">Today<") && svgs[2].includes("alerts go to ntfy"));
  assert.ok(svgs[3].includes(">Remote access<") && svgs[3].includes(">HTTP API<"));
  assert.ok(!md.includes("143"));
  // stopped: the status and remote access only
  const off = decode(daemonMarkdown({ v: daemonView(undefined, stopped()), t: light, now: NOW }));
  assert.equal(off.length, 2);
  assert.ok(off[0].includes("Not running"));
  // checking / missing: the status card alone
  assert.equal(
    decode(daemonMarkdown({ v: daemonView(undefined, undefined, {}, { loading: true }), t: dark })).length,
    1,
  );
  const missing = decode(daemonMarkdown({ v: daemonView(undefined, undefined, {}, { unsupported: true }), t: dark }));
  assert.equal(missing.length, 1);
  assert.ok(missing[0].includes("self-update"));
});
