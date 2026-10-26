// `npm test`: the Doctor, taken apart and drawn (doctor-view.ts) — each
// check in a few words, a machine's sections as one group with problems
// first, and the cards.
import assert from "node:assert/strict";
import { test } from "node:test";
import { chartTheme } from "./charts";
import {
  brief,
  checkCard,
  checkKind,
  checklist,
  checkMarkdown,
  countPills,
  countsText,
  doctorGroups,
  fixStep,
  groupFacts,
  groupMarkdown,
  groupOf,
  tildePath,
  titleOf,
  agentGroups,
  problems,
  countKinds,
  worstOf,
  CheckRow,
  DoctorGroup,
} from "./doctor-view";
import type { DoctorReport } from "./model";

const dark = chartTheme(true);
const light = chartTheme(false);
const logo = (kind: string, id: string | null | undefined) =>
  id ? `data:image/png;base64,iVBORw0KGgo=#${kind}/${id}` : undefined;
const ids: Record<string, string> = { "Claude Code": "claude", Codex: "codex", OpenCode: "opencode" };
const agentId = (n: string) => ids[n];

/** The shape `aleph doctor --json` prints (from a real fleet, names changed). */
const report: DoctorReport = {
  ok: 12,
  warn: 1,
  fail: 1,
  sections: [
    {
      section: "This Mac",
      checks: [
        { status: "ok", label: "aleph", text: "2.0.0  ·  ~/.cargo/bin/aleph", fix: [] },
        { status: "ok", label: "zsh", text: "al → aleph  ·  ~/.zshrc", fix: [] },
        {
          status: "info",
          label: "wrapper",
          text: "off — `claude` is Claude Code itself (opt in: ./install.sh --wrap-claude)",
          fix: [],
        },
        { status: "ok", label: "herdr", text: "0.9.5  ·  ~/.local/bin/herdr", fix: [] },
      ],
    },
    {
      section: "Daemon",
      checks: [
        { status: "ok", label: "service", text: "launchd dev.aleph.daemon · running (pid 24075)", fix: [] },
        {
          status: "info",
          label: "exposure",
          text: "not exposed — aleph daemon expose tailscale | cloudflare puts the API on your phone",
          fix: [],
        },
      ],
    },
    {
      section: "Host · b31",
      checks: [
        { status: "ok", label: "connects to", text: "selene@172.16.200.16", fix: [] },
        { status: "ok", label: "key login", text: "works — no password prompts", fix: [] },
      ],
    },
    {
      section: "On b31",
      checks: [
        { status: "ok", label: "herdr", text: "0.9.5  ·  /home/selene/.local/bin/herdr", fix: [] },
        {
          status: "info",
          label: "Codex",
          text: "0.163.0  ·  /home/selene/.local/bin/codex  ·  no herdr integration",
          fix: ["aleph --host b31 agents setup codex   — herdr restores its sessions with it"],
        },
        { status: "info", label: "system", text: "Linux x86_64", fix: [] },
      ],
    },
    {
      section: "Host · gpu2",
      checks: [
        { status: "ok", label: "connects to", text: "tester@gpu2.lan:2222", fix: [] },
        {
          status: "fail",
          label: "dns",
          text: "gpu2.lan doesn't resolve (failed to lookup address information: nodename nor servname provided)",
          fix: ["check HostName, your VPN, or /etc/hosts"],
        },
      ],
    },
    {
      section: "On gpu2",
      checks: [
        { status: "ok", label: "herdr", text: "0.8.0  ·  /home/tester/.local/bin/herdr", fix: [] },
        {
          status: "warn",
          label: "Claude Code",
          text: "2.1.309  ·  /home/tester/.local/bin/claude  ·  default  ·  no herdr integration",
          fix: ["aleph --host gpu2 agents setup claude   — herdr restores its sessions with it"],
        },
      ],
    },
    {
      section: "herdr · gpu2",
      checks: [{ status: "ok", label: "machine", text: "'gpu2' is saved in herdr", fix: [] }],
    },
    {
      section: "Shortcuts",
      checks: [{ status: "ok", label: "Warp", text: "Warp.app handles warp:// links", fix: [] }],
    },
  ],
};

test("doctor: a check in a few words", () => {
  const b = brief(
    "On gpu2",
    {
      label: "Claude Code",
      text: "2.1.309  ·  /home/tester/.local/bin/claude  ·  default  ·  no herdr integration",
    },
    true,
  );
  assert.deepEqual(b, {
    title: "Claude Code",
    fact: "no herdr integration",
    version: "2.1.309",
    path: "~/.local/bin/claude",
    extra: ["default agent"],
  });
  // what follows ` — ` is why; no backticks in a row
  const w = brief("This Mac", { label: "wrapper", text: report.sections[0].checks[2].text });
  assert.equal(w.title, "Wrapper");
  assert.equal(w.fact, "off");
  assert.equal(w.why, "claude is Claude Code itself (opt in: ./install.sh --wrap-claude)");
  // a long aside in parentheses is why, not the fact
  const dns = brief("Host · gpu2", { label: "dns", text: report.sections[4].checks[1].text });
  assert.equal(dns.title, "DNS");
  assert.equal(dns.fact, "gpu2.lan doesn't resolve");
  assert.match(dns.why ?? "", /^failed to lookup/);
  // a service's state is its key fact
  assert.equal(
    brief("Daemon", { label: "service", text: report.sections[1].checks[0].text }).fact,
    "running (pid 24075)",
  );
  // a version and a path only: no fact
  assert.deepEqual(brief("This Mac", { label: "aleph", text: "2.0.0  ·  ~/.cargo/bin/aleph" }), {
    title: "aleph",
    fact: "",
    version: "2.0.0",
    path: "~/.cargo/bin/aleph",
    extra: [],
  });
  assert.equal(titleOf("herdr · gpu2", "machine"), "herdr machine");
  assert.equal(titleOf("On b31", "moshi-hook"), "moshi-hook");
  assert.equal(titleOf("Daemon", "hub.machine"), "hub.machine");
  assert.equal(titleOf("Host · b31", "key login"), "Key login");
  assert.equal(
    tildePath("/home/selene/.nvm/versions/node/v22.12.0/bin/codex"),
    "~/.nvm/versions/node/v22.12.0/bin/codex",
  );
  assert.equal(tildePath("/Users/me"), "~");
  assert.equal(tildePath("/usr/local/bin/herdr"), "/usr/local/bin/herdr");
  assert.deepEqual(groupOf("On b31"), { name: "b31", area: "tools", machine: true });
  assert.deepEqual(groupOf("Shortcuts"), { name: "This Mac", area: "shortcuts", machine: false }, "on this computer");
  assert.deepEqual(groupOf("Daemon"), { name: "This Mac", area: "daemon", machine: false });
});

test("doctor: kinds, fixes", () => {
  assert.equal(checkKind({ status: "info", fix: ["x"] }), "tip");
  assert.equal(checkKind({ status: "info", fix: [] }), "note");
  assert.equal(checkKind({ status: "fail", fix: [] }), "fail");
  const f = fixStep("aleph --host b31 agents setup codex   — herdr restores its sessions with it");
  assert.equal(f.cmd, "aleph --host b31 agents setup codex");
  assert.equal(f.why, "herdr restores its sessions with it");
  assert.deepEqual(f.run, ["--host", "b31", "agents", "setup", "codex"]);
  assert.equal(fixStep("check HostName, your VPN, or /etc/hosts").run, undefined);
});

test("doctor: a machine's sections are one group, problems first", () => {
  const g = doctorGroups(report, agentId);
  assert.deepEqual(
    g.map((x) => x.name),
    ["gpu2", "This Mac", "b31"],
    "the machine with a failure first, the rest as they came; the daemon and the shortcuts are This Mac's",
  );
  const gpu2 = g[0];
  assert.deepEqual(
    gpu2.rows.map((r) => `${r.kind} ${r.brief.title}`),
    ["fail DNS", "warn Claude Code", "ok Connects to", "ok herdr", "ok herdr machine"],
  );
  assert.equal(gpu2.rows[1].area, "agents");
  assert.equal(gpu2.rows[1].agent, "claude");
  assert.equal(gpu2.worst, "fail");
  assert.equal(groupFacts(gpu2), "tester@gpu2.lan:2222 · herdr 0.8.0");
  const b31 = g.find((x) => x.name === "b31")!;
  assert.equal(b31.rows[0].brief.title, "Codex", "a tip before the passes");
  assert.equal(groupFacts(b31), "selene@172.16.200.16 · Linux x86_64 · herdr 0.9.5");
  assert.equal(groupFacts(g[1]), "aleph 2.0.0 · herdr 0.9.5");
  assert.equal(countsText(gpu2.counts), "1 failing · 1 warning · 3 ok");
  assert.ok(
    g[1].rows.some((r) => r.brief.title.startsWith("Daemon · ")) &&
      g[1].rows.some((r) => r.brief.title.startsWith("Shortcuts · ")),
    "folded-in checks keep their part's name",
  );
  assert.equal(countsText(b31.counts), "1 tip · 3 ok");
  // unique ids for the list
  const keys = g.flatMap((x) => x.rows.map((r) => r.key));
  assert.equal(new Set(keys).size, keys.length);
});

test("doctor: the cards", () => {
  const g = doctorGroups(report, agentId);
  const [dns, claude] = g[0].rows;
  for (const t of [dark, light]) {
    const card = checkCard(claude, t, logo);
    assert.match(card, /^<svg /);
    assert.ok(card.includes("Claude Code") && card.includes("no herdr integration"));
    assert.ok(card.includes("gpu2 · agents"));
    assert.ok(card.includes("#agent/claude"), "an agent's logo, its mark as a badge");
    assert.ok(card.includes(t.state.warn), "a warning in its colour");
    assert.ok(checkCard(dns, t, logo).includes(t.state.down));
    const list = checklist(g[0].rows, t);
    assert.ok(list.includes("DNS") && list.includes("0.8.0"));
    const pills = countPills(g[0].counts, t);
    assert.ok(pills.includes("failing") && pills.includes("warning") && pills.includes("tips"));
  }
  const md = checkMarkdown(claude, { t: dark, logo });
  assert.ok(md.includes("Herdr restores its sessions with it"), md);
  assert.ok(md.includes("↵ runs it in a terminal") && md.includes("```\naleph --host gpu2 agents setup claude\n```"));
  assert.ok(checkMarkdown(dns, { t: dark, logo }).includes("⌘C copies it"));
  const gm = groupMarkdown(g[0], { t: dark, logo, color: "#b4a0ff" });
  assert.equal((gm.match(/!\[/g) ?? []).length, 3, "a hero, pills and the checklist");
  assert.ok(gm.includes("**To fix**") && gm.includes("check HostName"));
  assert.ok(!groupMarkdown(g[1], { t: dark, logo }).includes("To fix"), "nothing to fix: no fixes");
});

test("the doctor by agent, and what needs attention", () => {
  const row = (group: string, label: string, kind: "fail" | "warn" | "ok", agent?: string) =>
    ({
      key: `${group}/${label}`,
      section: group,
      group,
      area: agent ? "agents" : "ssh",
      check: { label, status: kind === "fail" ? "fail" : kind === "warn" ? "warn" : "ok", text: label },
      kind,
      brief: { title: label, fact: "", extra: [] },
      ...(agent ? { agent } : {}),
      fixes: [],
    }) as unknown as CheckRow;
  const g = (name: string, rows: CheckRow[]) =>
    ({ name, machine: true, rows, counts: countKinds(rows), worst: worstOf(countKinds(rows)) }) as DoctorGroup;
  const groups = [
    g("b31", [row("b31", "Claude Code", "ok", "claude"), row("b31", "Codex", "warn", "codex")]),
    g("q7x", [row("q7x", "Claude Code", "fail", "claude"), row("q7x", "ssh", "ok")]),
  ];
  const by = agentGroups(groups, (id) => (id === "claude" ? "Claude Code" : "Codex"));
  assert.deepEqual(
    by.map((a) => [a.name, a.rows.map((r) => `${r.brief.title}:${r.kind}`)]),
    [
      ["Claude Code", ["q7x:fail", "b31:ok"]],
      ["Codex", ["b31:warn"]],
    ],
    "each agent's rows titled by machine, problems first; agents with problems first",
  );
  assert.deepEqual(
    problems(groups).map((r) => `${r.group}/${r.brief.title}`),
    ["q7x/Claude Code", "b31/Codex"],
  );
});
