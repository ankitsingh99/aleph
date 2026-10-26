// `npm test`: the dashboard pieces (dash.ts), the live history (samples.ts),
// the Machine Stats and Machines details (stats-view.ts, machines-view.ts)
// and which logo stands for what (logo-ids.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import { chartTheme, contrast } from "./charts";
import type { AgentsHost, Host, MachineStats, Session } from "./cli";
import {
  chips,
  coreGrid,
  facts,
  gauges,
  heatColor,
  hero,
  meters,
  pills,
  sessionList,
  severity,
  sparkline,
  tempSeverity,
  trends,
} from "./dash";
import { gpuId, osId, shortGpu } from "./logo-ids";
import { linkSample, machineBadges, machineMarkdown, sessionCounts } from "./machines-view";
import { bars, parseHistory, pushSample, seriesOf, spanText, StatSample } from "./samples";
import { gpuSummary, statGauges, statsBadges, statSample, statsMarkdown, statTrends } from "./stats-view";

const dark = chartTheme(true);
const light = chartTheme(false);
const G = 1024 ** 3;
const PNG = "data:image/png;base64,iVBORw0KGgo=";
const logo = (kind: string, id: string | null | undefined) => (id ? `${PNG}#${kind}/${id}` : undefined);
const brand = (id: string) => ({ id, name: id === "claude" ? "Claude Code" : id, color: "#d97757" });

/** Every `fill` a `<text>` or `<tspan>` wears. */
const textFills = (svg: string) => [...svg.matchAll(/<(?:text|tspan)\b[^>]*\bfill="([^"]+)"/g)].map((m) => m[1]);

const box = (over: Partial<MachineStats> = {}): MachineStats => ({
  alias: "gpu-box",
  color: "#a6e3a1",
  reachable: true,
  error: null,
  os: "Ubuntu 24.04.3 LTS",
  platform: "linux",
  kernel: "Linux 6.8.0",
  cpu_model: "AMD Ryzen 9 7950X",
  ncpu: 16,
  uptime_s: 90_000,
  load: [11.4, 10.8, 9.9],
  cpu: 0.56,
  cpu_temp: 78,
  cores: Array.from({ length: 16 }, (_, i) => i / 15),
  mem_used: 51 * G,
  mem_total: 64 * G,
  swap_used: 1 * G,
  swap_total: 8 * G,
  net_rx: 1e6,
  net_tx: 2e5,
  disks: [
    { mount: "/", used: 1.4e12, total: 1.9e12 },
    { mount: "/boot/efi", used: 6e6, total: 5e8 },
  ],
  disk_io: [{ name: "nvme0n1", read: 3e8, write: 4e7, busy: 0.6 }],
  gpus: [
    {
      index: "0",
      name: "NVIDIA GeForce RTX 4090",
      vendor: "nvidia",
      util: 0.97,
      mem_used: 22 * G,
      mem_total: 24 * G,
      temp: 81,
      power: 431,
      power_limit: 450,
    },
    {
      index: "1",
      name: "NVIDIA GeForce RTX 4090",
      vendor: "nvidia",
      util: 0.61,
      mem_used: 14 * G,
      mem_total: 24 * G,
      temp: 69,
      power: 302,
      power_limit: 450,
    },
  ],
  top: [{ pid: "1", user: "me", cpu: 3.1, rss: 2 * G, name: "python | train" }],
  agent_procs: [{ agent: "claude", count: 3, rss: 2 * G }],
  ...over,
});

test("load and heat colours", () => {
  assert.equal(severity(0.2, dark), dark.state.done);
  assert.equal(severity(0.7, dark), dark.state.warn);
  assert.equal(severity(0.95, dark), dark.state.down);
  assert.equal(severity(undefined, dark), dark.state.done);
  assert.equal(tempSeverity(70, light), light.state.done);
  assert.equal(tempSeverity(80, light), light.state.warn);
  assert.equal(tempSeverity(90, light), light.state.down);
  assert.equal(heatColor(1, dark), dark.state.down);
  assert.equal(heatColor(2, dark), dark.state.down, "clamped");
  // an idle core sits near the surface, a busy one stands out
  assert.ok(contrast(heatColor(0, dark), dark.surface) < contrast(heatColor(0.5, dark), dark.surface));
  for (const t of [dark, light])
    for (const c of Object.values(t.state)) assert.ok(contrast(c, t.surface) >= 3, `${c} on ${t.surface}`);
});

test("gauges", () => {
  const svg = gauges(
    [
      { label: "CPU", frac: 0.56, sub: "16 cores" },
      { label: "Temp", frac: 0.75, value: "80°", color: dark.state.warn },
      { label: "GPU", frac: null },
    ],
    dark,
  );
  assert.match(svg, /^<svg [^>]*width="400"/);
  assert.ok(
    svg.includes(">56%<") && svg.includes(">80°<") && svg.includes(">–<"),
    "the value, an override, no reading",
  );
  assert.ok(!/NaN|undefined/.test(svg));
  assert.equal((svg.match(/<path d="M/g) ?? []).length, 5, "a track for each, a fill for the two with readings");
  for (const f of textFills(svg)) assert.ok([dark.text, dark.muted].includes(f), `text wears ${f}`);
});

test("the core grid", () => {
  const svg = coreGrid([0.1, 0.9, 0.5, 0.98], dark);
  assert.equal((svg.match(/<title>core /g) ?? []).length, 4);
  assert.ok(svg.includes("busiest #3 98%") && svg.includes("avg 62%"));
  assert.ok(svg.includes(">0%<") && svg.includes(">100%<"), "its scale");
  const many = coreGrid(
    Array.from({ length: 64 }, () => 0.5),
    light,
  );
  assert.match(many, /height="(\d+)"/);
  assert.ok(Number(/height="(\d+)"/.exec(many)![1]) > 60, "four rows of 16");
  assert.ok(!coreGrid([], dark).includes("<rect"), "nothing to draw");
});

test("trends and sparklines", () => {
  const pts: [number, number | null][] = [
    [100, 0.2],
    [103, null],
    [106, 0.6],
    [109, 0.4],
  ];
  const line = sparkline(pts, { x: 0, y: 0, w: 90, h: 20, from: 100, to: 109, max: 1 }, "#123456", dark);
  assert.ok(line.includes('stroke="#123456"') && line.includes("<circle"), "a line and its end dot");
  assert.equal(
    sparkline([[100, 1]], { x: 0, y: 0, w: 9, h: 9, from: 0, to: 200 }, "#123456", dark),
    "",
    "one point: nothing",
  );
  const svg = trends(
    [
      { label: "CPU", points: pts, max: 1, value: "40%", color: "#123456" },
      { label: "Net in", points: [[109, 5]], value: "5B/s", color: "#123456" },
    ],
    { from: 100, to: 109, title: "Live", note: "last 9 s" },
    dark,
  );
  assert.ok(svg.includes(">40%<") && svg.includes("filling in…"), "a row with one reading says so");
  assert.ok(svg.includes(">Live<") && svg.includes(">last 9 s<"));
});

test("facts, meters, hero, pills, sessions, chips", () => {
  const f = facts(
    [
      { label: "Agents", parts: Array.from({ length: 8 }, (_, i) => ({ text: `Agent number ${i}`, logo: PNG })) },
      {
        label: "herdr",
        parts: [
          { text: "0.9.5", warn: true },
          { text: "differs <here>", muted: true },
        ],
      },
    ],
    dark,
  );
  assert.ok(Number(/height="(\d+)"/.exec(f)![1]) > 60, "eight agents wrap onto more lines");
  assert.ok(f.includes("<image") && f.includes("differs &lt;here&gt;"));
  const m = meters([{ label: "/", frac: 0.74, value: "74% · 1.3T / 1.7T" }], light);
  assert.ok(m.includes(">74% · 1.3T / 1.7T<"), "a ratio isn't cut like a path");
  const down = hero(
    {
      title: "dex42",
      color: "#f9e2af",
      subtitle: "unreachable",
      link: { bars: 0, text: "offline", color: dark.state.down },
    },
    dark,
  );
  assert.ok(down.includes(`stroke="${dark.state.down}"`), "a red ✕ instead of bars");
  assert.ok(down.includes(">D<"), "a monogram without a logo");
  const up = hero(
    {
      title: "b31",
      logo: PNG,
      link: {
        bars: 3,
        text: "38 ms",
        color: dark.state.done,
        history: [
          [1, 30],
          [2, 40],
          [3, 38],
        ],
        note: "latency · last 2 s",
      },
    },
    dark,
  );
  assert.ok(up.includes("<image") && up.includes(">38 ms<") && up.includes("latency · last 2 s"));
  assert.equal((up.match(new RegExp(`fill="${dark.state.done}"`, "g")) ?? []).length >= 3, true, "three bars lit");
  const p = pills(
    [
      { n: 2, label: "needs you", state: "blocked", color: dark.state.blocked },
      { n: 0, label: "done", state: "done", color: dark.state.done },
    ],
    dark,
  );
  assert.ok(p.includes(">2<") && p.includes(">needs you<") && p.includes(">0<"));
  for (const c of textFills(p)) assert.ok([dark.text, dark.muted].includes(c));
  const s = sessionList(
    [
      {
        name: "temper<x>",
        title: "Run cargo",
        state: "blocked",
        color: dark.state.blocked,
        status: "needs you · 4m",
      },
    ],
    dark,
  );
  assert.ok(s.includes("temper&lt;x&gt;") && s.includes("needs you · 4m"));
  const c = chips(
    [
      { title: "Agents", chips: [], empty: "none found" },
      { title: "LLM servers", chips: [{ text: "Ollama", note: "1 loaded", dot: "#00ff00", logo: PNG }] },
    ],
    light,
  );
  assert.ok(c.includes(">none found<") && c.includes(">Ollama<") && c.includes(">1 loaded<"));
});

test("history: one sample per answer, a quarter of an hour kept", () => {
  const s = (t: number): StatSample => ({
    t,
    cpu: t / 1000,
    mem: null,
    gpu: null,
    vram: null,
    load: 1,
    rx: 0,
    tx: 0,
    rd: null,
    wr: null,
    temp: null,
  });
  let h: StatSample[] = [];
  h = pushSample(h, s(1000));
  h = pushSample(h, s(1000));
  assert.equal(h.length, 1, "the same answer twice counts once");
  h = pushSample(h, s(999));
  assert.equal(h.length, 1, "an older answer is dropped");
  h = pushSample(h, s(1003));
  h = pushSample(h, s(2000), 2000);
  assert.deepEqual(
    h.map((x) => x.t),
    [2000],
    "older than 15 min goes",
  );
  let many: StatSample[] = [];
  for (let i = 0; i < 400; i++) many = pushSample(many, s(i));
  assert.equal(many.length, 300);
  assert.deepEqual(seriesOf([s(1), { ...s(2), cpu: null }], "cpu"), [
    [1, 0.001],
    [2, null],
  ]);
  assert.deepEqual(parseHistory("nope"), []);
  assert.deepEqual(parseHistory('[{"t":1},{"x":2},null]'), [{ t: 1 }]);
  assert.equal(spanText([s(1)]), "");
  assert.equal(spanText([s(0), s(40)]), "last 40 s");
  assert.equal(spanText([s(0), s(300)]), "last 5 min");
  assert.deepEqual(
    [bars(40, true), bars(300, true), bars(700, true), bars(1200, true), bars(40, false), bars(null, true)],
    [4, 3, 2, 1, 0, 3],
  );
});

test("which logo: OS, GPU", () => {
  assert.equal(osId("Ubuntu 24.04.3 LTS")?.id, "ubuntu");
  assert.equal(osId(undefined, "Darwin arm64")?.id, "apple");
  assert.equal(osId("openSUSE Tumbleweed")?.id, "opensuse");
  assert.equal(osId("Debian GNU/Linux 12")?.id, "debian");
  assert.equal(osId(""), undefined);
  assert.equal(gpuId("NVIDIA H100 80GB HBM3")?.id, "nvidia");
  assert.equal(gpuId("H100 80GB HBM3", "nvidia")?.id, "nvidia");
  assert.equal(gpuId("AMD Instinct MI300X")?.id, "amd");
  assert.equal(gpuId("Apple GPU")?.id, "apple");
  assert.equal(gpuId("b31"), undefined);
  assert.equal(shortGpu("NVIDIA GeForce RTX 4090"), "RTX 4090");
  assert.equal(shortGpu("AMD Instinct MI300X"), "MI300X");
});

test("Machine Stats: samples, gauges, badges", () => {
  const h = box();
  const s = statSample(h, 5);
  assert.equal(s.t, 5);
  assert.ok(Math.abs((s.gpu ?? 0) - 0.79) < 1e-9, "the GPUs' average");
  assert.ok(Math.abs((s.vram ?? 0) - 36 / 48) < 1e-9);
  assert.equal(s.temp, 81, "the hottest sensor");
  assert.equal(s.rd, 3e8);
  assert.equal(gpuSummary(h), "2× RTX 4090");
  const g = statGauges(h, dark);
  assert.deepEqual(
    g.map((x) => x.label),
    ["CPU", "Memory", "GPU ×2", "VRAM", "Temp"],
  );
  assert.equal(g[4].value, "81°");
  assert.equal(g[4].color, dark.state.warn);
  const mac = statGauges(
    box({ gpus: [], cpu_temp: null, disks: [{ mount: "/System/Volumes/Data", used: 244 * G, total: 460 * G }] }),
    dark,
  );
  assert.deepEqual(
    mac.map((x) => x.label),
    ["CPU", "Memory", "Swap", "Disk Data"],
  );
  assert.deepEqual(
    statsBadges(h).map((b) => b.text),
    ["CPU 56%", "79%"],
  );
  assert.equal(statsBadges(h)[1].icon, "gpu");
  assert.deepEqual(
    statsBadges(box({ gpus: [] })).map((b) => b.text),
    ["CPU 56%", "MEM 80%"],
    "memory when there's no GPU",
  );
  const off = statsBadges(box({ reachable: false, error: "ssh: connect to host x port 22: Operation timed out" }));
  assert.deepEqual(
    off.map((b) => [b.text, b.tone]),
    [["offline", "bad"]],
  );
  const host = { herdr_version: "0.8.0", herdr_drift: true } as Host;
  assert.equal(statsBadges(h, host)[0].icon, "warn", "herdr's warning first");
  assert.ok(statsBadges(h, host).length <= 3);
  assert.deepEqual(
    statTrends(h, [], dark).map((r) => r.label),
    ["CPU", "GPU", "Memory", "Load", "Net in", "Net out", "Disk read", "Disk write"],
  );
});

test("Machine Stats: the detail", () => {
  const h = box();
  const hist = [statSample(h, 100), statSample(h, 103)];
  const md = statsMarkdown({ h, hist, t: dark, logo, brand });
  const imgs = md.match(/!\[[^\]]*\]\(data:image\/svg\+xml;base64,/g) ?? [];
  assert.equal(imgs.length, 7, "hero, gauges, GPUs, cores, live, about, disks");
  assert.ok(md.includes("| python \\| train |"), "table cells escaped");
  const svgs = [...md.matchAll(/base64,([^)]+)\)/g)].map((m) => Buffer.from(m[1], "base64").toString("utf8"));
  assert.ok(svgs.every((s) => s.startsWith("<svg") && !/NaN|undefined/.test(s)));
  assert.ok(
    svgs.some((s) => s.includes("#os/ubuntu")),
    "the OS logo embedded",
  );
  assert.ok(
    svgs.some((s) => s.includes("#gpu/nvidia")),
    "the GPU vendor's",
  );
  assert.ok(!svgs.some((s) => s.includes(">efi<")), "no boot partition among the disks");
  const off = statsMarkdown({
    h: box({ reachable: false, error: "ssh: connect to host 10.0.0.5 port 22: Operation timed out" }),
    hist: [],
    t: light,
    logo,
    brand,
  });
  assert.match(off, /\*\*unreachable — ssh timed out\*\*/);
});

const sess = (name: string, status: string, extra: Partial<Session> = {}): Session => ({
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
  status_since: 1000,
  cwd: "/home/me/x",
  title: `${name}'s task`,
  mark: "",
  hue: "",
  glyph: "",
  color: "",
  focused: false,
  ...extra,
});

const host = (over: Partial<Host> = {}): Host => ({
  alias: "b31",
  machine: "b31",
  color: "#89b4fa",
  address: "me@10.0.0.49",
  reachable: true,
  error: null,
  stale: false,
  fetched_at: 1000,
  latency_ms: 38,
  herdr_version: "0.9.5",
  sessions: [
    sess("temper", "blocked"),
    sess("ignite", "working"),
    sess("weave", "working"),
    sess("quill", "idle", { limit: "2:40am" }),
    sess("shell", "idle"),
  ],
  ...over,
});

test("Machines: badges", () => {
  const a = { alias: "b31", reachable: true, error: null, gpu: "2× NVIDIA GeForce RTX 4090", agents: [] } as AgentsHost;
  assert.deepEqual(
    sessionCounts(host()).map((c) => `${c.state} ${c.n}`),
    ["blocked 1", "working 2", "limit 1", "done 0", "idle 1"],
  );
  const b = machineBadges(host(), a);
  assert.deepEqual(
    b.map((x) => x.kind),
    ["needs", "gpu", "link"],
  );
  assert.equal(b[2].bars, 4);
  assert.deepEqual(
    machineBadges(host({ sessions: [sess("x", "working")], latency_ms: 950 })).map((x) => [x.kind, x.text || x.bars]),
    [
      ["working", "1"],
      ["link", 1],
    ],
  );
  assert.deepEqual(
    machineBadges(host({ reachable: false, error: "ssh: connect to host x port 22: Operation timed out" }), a).map(
      (x) => x.kind,
    ),
    ["offline"],
    "down says only that",
  );
  assert.equal(machineBadges(host({ reachable: false, error: "no herdr server running" })).at(-1)?.kind, "stopped");
  assert.deepEqual(linkSample(host(), 5), { t: 1000, ms: 38, up: true });
  assert.deepEqual(linkSample(host({ reachable: false, fetched_at: null }), 5), { t: 5, ms: null, up: false });
});

test("Machines: the detail", () => {
  const a = {
    alias: "b31",
    reachable: true,
    error: null,
    os: "Darwin arm64",
    os_name: "macOS 15.1",
    gpu: "Apple GPU",
    agents: [{ id: "claude", name: "Claude Code", installed: true, version: "2.1.200", color: "#d97757" }],
  } as AgentsHost;
  const md = machineMarkdown({
    h: host(),
    a,
    hist: [
      { t: 1, ms: 30, up: true },
      { t: 6, ms: 38, up: true },
    ],
    t: dark,
    logo,
    brand,
    now: 1240,
  });
  const svgs = [...md.matchAll(/base64,([^)]+)\)/g)].map((m) => Buffer.from(m[1], "base64").toString("utf8"));
  assert.equal(svgs.length, 4, "hero, pills, sessions, chips");
  assert.ok(svgs[0].includes("#os/apple") && svgs[0].includes(">38 ms<") && svgs[0].includes("latency · last 5 s"));
  assert.ok(svgs[1].includes(">needs you<") && svgs[1].includes(">at limit<"));
  // needs you first, at its limit before idle
  const order = [...svgs[2].matchAll(/font-weight="600">([a-z]+)</g)].map((m) => m[1]);
  assert.deepEqual(order, ["temper", "ignite", "weave", "quill", "shell"]);
  assert.ok(svgs[2].includes("needs you · 4m"));
  assert.ok(svgs[3].includes(">Claude Code<") && svgs[3].includes("#agent/claude") && svgs[3].includes(">Apple GPU<"));
  const off = machineMarkdown({
    h: host({ reachable: false, error: "ssh: Could not resolve hostname b31" }),
    hist: [],
    t: light,
    logo,
    brand,
    now: 1300,
  });
  assert.match(off, /unknown host/);
  assert.match(off, /Last answered 5m ago/);
  const empty = machineMarkdown({ h: host({ sessions: [] }), hist: [], t: light, logo, brand });
  assert.match(empty, /No sessions here/);
});
