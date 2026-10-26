// `npm test`: Machines' detail in cards (machines-view.ts `machineCards`) and
// the empty states it draws with (cards.ts `hintRow`, `chipRows`).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { AgentInfo, AgentsHost, Host, Service, Session } from "./cli";
import { chartTheme } from "./charts";
import { chipRows, hintRow } from "./cards";
import { machineCards, machinesSummary, shortOs } from "./machines-view";

const dark = chartTheme(true);
const light = chartTheme(false);
const PNG = "data:image/png;base64,iVBORw0KGgo=";
const logo = (kind: string, id: string | null | undefined) => (id ? `${PNG}#${kind}/${id}` : undefined);
const brand = (id: string) => ({ id, name: id === "claude" ? "Claude Code" : id, color: "#d97757" });
const svgsOf = (md: string) =>
  [...md.matchAll(/base64,([^)]+)\)/g)].map((m) => Buffer.from(m[1], "base64").toString("utf8"));

const sess = (name: string, status: string, extra: Partial<Session> = {}): Session => ({
  host: "local",
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
  cwd: "/Users/me/work/px",
  title: "",
  mark: "",
  hue: "",
  glyph: "",
  color: "",
  focused: false,
  ...extra,
});

const host = (over: Partial<Host> = {}): Host => ({
  alias: "local",
  machine: "local",
  color: "#fab387",
  address: "this computer",
  reachable: true,
  error: null,
  stale: false,
  fetched_at: 1000,
  latency_ms: 19,
  herdr_version: "0.9.5",
  sessions: [],
  ...over,
});

const agent = (id: string, name: string, version: string) =>
  ({ id, name, installed: true, version, color: "#d97757" }) as AgentInfo;

/** This Mac, as `agents --json --local` sees it. */
const mac = {
  alias: "local",
  reachable: true,
  error: null,
  os: "Darwin arm64",
  os_name: "macOS 15.1",
  gpu: "Apple GPU",
  agents: [agent("claude", "Claude Code", "2.1.309"), agent("codex", "Codex", "0.163.0")],
} as AgentsHost;

test("Machines: the section's summary", () => {
  assert.equal(machinesSummary([host(), host({ alias: "b31" })]), "2 machines · no sessions");
  assert.equal(
    machinesSummary([host({ sessions: [sess("a", "blocked"), sess("b", "idle")] })]),
    "1 machine · 2 sessions · 1 needs you",
  );
  assert.equal(shortOs("Ubuntu 22.04.5 LTS"), "Ubuntu 22.04");
  assert.equal(shortOs("macOS 27.0"), "macOS 27.0");
});

test("Machines: no sessions is a quiet row, not prose; this computer's agents show", () => {
  const md = machineCards({
    h: host(),
    a: mac,
    svc: { alias: "local", color: "#fab387", reachable: true, error: null, services: [] },
    hist: [],
    t: dark,
    logo,
    brand,
  });
  assert.ok(!/No sessions here|_/.test(md.replace(/\(data:[^)]+\)/g, "")), "no italic sentence");
  const svgs = svgsOf(md);
  assert.equal(svgs.length, 3, "card, the empty row, agents and servers");
  assert.ok(!svgs.some((s) => s.includes(">0<")), "no row of zero pills");
  assert.ok(svgs[1].includes(">No sessions<") && svgs[1].includes(">⌘N<") && svgs[1].includes(">Quick Start<"));
  assert.ok(svgs[2].includes(">Claude Code<") && svgs[2].includes(">Codex<") && svgs[2].includes("#agent/claude"));
  assert.ok(!svgs[2].includes("LLM servers"), "no servers row when it has none");
  assert.ok(svgs[0].includes(">Apple GPU<") && svgs[0].includes("#os/apple") && svgs[0].includes(">19 ms<"));
});

test("Machines: sessions show only the states there are", () => {
  const svc: Service = {
    id: "ollama",
    name: "Ollama",
    kind: "llm-server",
    health: "up",
    loaded_count: 1,
    model_count: 2,
    color: "#ececec",
  } as Service;
  const md = machineCards({
    h: host({
      sessions: [
        sess("temper", "blocked", { title: "Run the benchmark" }),
        sess("quill", "idle", { title: "Claude Code" }),
      ],
    }),
    a: mac,
    svc: { alias: "local", color: "#fab387", reachable: true, error: null, services: [svc] },
    hist: [],
    t: light,
    logo,
    brand,
    now: 1240,
  });
  const svgs = svgsOf(md);
  assert.equal(svgs.length, 4, "card, pills, sessions, chips");
  assert.ok(svgs[1].includes(">needs you<") && svgs[1].includes(">idle<"));
  assert.ok(!svgs[1].includes(">working<") && !svgs[1].includes(">done<"), "no zero pills");
  assert.ok(svgs[2].includes("Run the benchmark"));
  assert.ok(
    svgs[2].includes(">px<") && !svgs[2].includes(">Claude Code<"),
    "a title that's just the agent's name: the folder",
  );
  assert.ok(svgs[3].includes(">Ollama<") && svgs[3].includes("1 loaded") && svgs[3].includes(light.state.done));
});

test("Machines: offline says why, and the doctor's keycap", () => {
  const md = machineCards({
    h: host({
      alias: "gpu2",
      reachable: false,
      error: "ssh: connect to host gpu2.lan port 2222: Operation timed out",
      fetched_at: 1000,
    }),
    hist: [],
    t: dark,
    logo,
    brand,
    now: 1300,
  });
  const svgs = svgsOf(md);
  assert.equal(svgs.length, 3);
  assert.ok(svgs[0].includes(">offline<"));
  assert.ok(
    svgs[1].includes("unreachable — ssh timed out") && svgs[1].includes("gpu2.lan:2222 · last answered 5m ago"),
  );
  assert.ok(!svgs[1].includes("Operation timed out"), "the head already says it");
  assert.ok(svgs[2].includes(">⌥⌘D<") && svgs[2].includes(">Doctor<"));
});

test("cards: hint row and chip rows", () => {
  const h = hintRow({ text: "No sessions", keys: "⌘N", action: "Quick Start" }, dark, 400);
  assert.match(h, /height="34"/);
  assert.ok(h.indexOf(">No sessions<") > 0 && h.includes("stroke-dasharray"));
  const c = chipRows(
    [
      { label: "Agents", chips: [{ text: "Claude Code", note: "2.1.309", logo: PNG }] },
      { label: "LLM servers", chips: [], empty: "none" },
    ],
    dark,
    400,
  );
  assert.ok(c.includes(">Claude Code<") && c.includes(">2.1.309<") && c.includes(">none<"));
  assert.match(c, /height="56"/, "two rows of 24 with a gap");
  // many chips wrap onto more rows
  const many = chipRows(
    [{ label: "Agents", chips: Array.from({ length: 8 }, (_, i) => ({ text: `Agent ${i}` })) }],
    dark,
    300,
  );
  assert.ok(Number(/height="(\d+)"/.exec(many)![1]) > 24);
});

test("Machines with live stats: link card, gauges, sessions under them, then the rest", async () => {
  const { machineDetail } = await import("./machine-detail");
  const h = {
    alias: "b31",
    reachable: true,
    color: "#89b4fa",
    latency_ms: 40,
    herdr_version: "0.9.5",
    sessions: [],
  } as unknown as Parameters<typeof machineDetail>[0]["h"];
  const stats = {
    alias: "b31",
    reachable: true,
    os: "Ubuntu 24.04",
    cpu: 0.2,
    mem_used: 4e9,
    mem_total: 16e9,
    cores: [0.1, 0.5],
    gpus: [],
  } as unknown as NonNullable<Parameters<typeof machineDetail>[0]["stats"]>;
  const base = {
    h,
    hist: [],
    statHist: [],
    t: dark,
    logo: () => undefined,
    brand: (id: string) => ({ id, name: id, color: "#888" }),
  };
  const md = machineDetail({ ...base, stats });
  const alts = [...md.matchAll(/!\[([^\]]*)\]/g)].map((m) => m[1]);
  assert.ok(alts[0].startsWith("b31:") && alts[0].includes("ms"), "the card with the link first");
  assert.equal(alts[1], "CPU, memory, GPU and temperature");
  assert.equal(alts[2], "No sessions", "sessions right under the gauges");
  assert.ok(alts.includes("Cores") && alts.includes("Live") && alts.includes("About"));
  // before the first sample: Machines' own cards
  const bare = machineDetail({ ...base });
  assert.ok(!bare.includes("CPU, memory, GPU and temperature") && bare.includes("No sessions"));
});
