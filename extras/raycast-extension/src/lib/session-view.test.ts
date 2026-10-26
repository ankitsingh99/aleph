// `npm test`: the Sessions command's words and cards (session-view.ts): a
// row's folder and state tag, a machine's section line, the facts under the
// live screen and the whole detail.
import assert from "node:assert/strict";
import { test } from "node:test";
import { chartTheme } from "./charts";
import type { Host, Note, QueueTask, Session } from "./cli";
import { folderParts, hostSummary, rowStatus, sessionFacts, sessionMarkdown, SessionView } from "./session-view";

const dark = chartTheme(true);
const NOW = 1_800_000_000;
const PNG = "data:image/png;base64,iVBORw0KGgo=";
const logo = (kind: string, id: string | null | undefined) => (id ? `${PNG}#${kind}/${id}` : undefined);
const brand = (id: string) => ({ id, name: id === "claude" ? "Claude Code" : id, color: "#d97757" });

const session = (over: Partial<Session> = {}): Session => ({
  host: "dex42",
  name: "frosting-crane",
  label: "claude_frosting-crane:test_folder",
  display: "frosting-crane",
  pane_id: "w2:p2",
  tab_id: "w2:t2",
  workspace_id: "w2",
  terminal_id: "term_1",
  agent: "claude",
  status: "idle",
  status_since: NOW - 33 * 60,
  cwd: "/home/selene/alexanders_work/test_folder",
  title: "Claude Code",
  mark: "●",
  hue: "ember",
  glyph: "●",
  color: "#f38ba8",
  focused: false,
  limit: null,
  ...over,
});

const host = (over: Partial<Host> = {}): Host => ({
  alias: "dex42",
  machine: "dex42",
  color: "#94e2d5",
  address: "selene@172.16.200.77",
  reachable: true,
  error: null,
  stale: false,
  fetched_at: NOW,
  latency_ms: 171,
  herdr_version: "0.9.5",
  sessions: [session()],
  ...over,
});

const view = (over: Partial<SessionView> = {}): SessionView => ({
  s: session(),
  host: host(),
  queued: [],
  t: dark,
  logo,
  brand,
  osText: "Ubuntu 24.04.3 LTS linux",
  ...over,
});

const texts = (v: SessionView) =>
  Object.fromEntries(sessionFacts(v).map((f) => [f.label, f.parts.map((p) => p.text).join(" ")]));

test("folderParts: the folder's name first, the path to it after", () => {
  assert.deepEqual(folderParts("/home/selene/alexanders_work/test_folder"), {
    name: "test_folder",
    parent: "~/alexanders_work",
  });
  assert.deepEqual(folderParts("/Users/me/"), { name: "~", parent: "" });
  assert.deepEqual(folderParts("/root/x"), { name: "x", parent: "~" });
  assert.deepEqual(folderParts("/srv"), { name: "srv", parent: "/" });
  assert.deepEqual(folderParts("/"), { name: "/", parent: "" });
  assert.deepEqual(folderParts("-"), { name: "", parent: "" });
  assert.deepEqual(folderParts(null), { name: "", parent: "" });
});

test("rowStatus: the state and how long, short enough beside the detail", () => {
  assert.equal(rowStatus(session(), NOW), "idle 33m");
  assert.equal(rowStatus(session({ status: "blocked", status_since: NOW - 240 }), NOW), "needs you 4m");
  assert.equal(rowStatus(session({ status_since: null }), NOW), "idle");
  // the limit's reset is in the tooltip and the detail
  assert.equal(rowStatus(session({ limit: "2:40am" }), NOW), "at limit");
});

test("hostSummary: sessions by state, the link, herdr when it needs a look; why a machine is down", () => {
  const busy = host({
    sessions: [
      session({ status: "idle" }),
      session({ status: "blocked" }),
      session({ status: "working" }),
      session({ status: "idle" }),
      session({ status: "idle", limit: "" }),
    ],
  });
  assert.equal(hostSummary(busy, NOW), "▲1 ✻1 ◷1 ●2 · 171 ms");
  assert.equal(hostSummary(host({ sessions: [] }), NOW), "no sessions · 171 ms");
  assert.equal(hostSummary(host({ herdr_restart_needed: true, latency_ms: null }), NOW), "●1 · herdr restart needed");
  assert.equal(hostSummary(host({ herdr_drift: true }), NOW), "●1 · 171 ms · herdr 0.9.5 differs");
  assert.equal(
    hostSummary(
      host({
        reachable: false,
        error: "ssh: connect to host 172.16.200.77 port 22: Operation timed out",
        fetched_at: NOW - 300,
      }),
      NOW,
    ),
    "✕ unreachable — ssh timed out · cached 5m",
  );
  // never the address: the section line stays short
  assert.ok(!hostSummary(busy, NOW).includes("192.168"));
});

test("sessionFacts: folder, task, agent, machine, note, queue, pane; no state (the screen's bar has it)", () => {
  const f = texts(view());
  assert.deepEqual(Object.keys(f), ["Folder", "Agent", "Machine", "Pane"]);
  assert.equal(f.Folder, "test_folder");
  assert.equal(f.Agent, "Claude Code");
  assert.equal(f.Machine, "dex42 Ubuntu 171 ms");
  const facts = sessionFacts(view());
  assert.equal(facts[0].parts[0].note, "~/alexanders_work");
  assert.equal(facts[1].parts[0].logo, `${PNG}#agent/claude`);
  assert.equal(facts[2].parts[1].logo, `${PNG}#os/ubuntu`);
  assert.ok(facts[2].parts[0].dot);
  // herdr's title says more than the agent's name: it's shown
  assert.equal(texts(view({ s: session({ title: "✳ Fix the flaky tests" }) }))["Working on"], "✳ Fix the flaky tests");
  // a note with its tags, the queue (first lines, three at most), a machine whose OS isn't known yet
  const note: Note = { host: "dex42", name: "frosting-crane", text: "check simd", tags: ["ivy"], t: 0 };
  const q = (id: number, prompt: string) => ({ id, prompt, state: "queued" }) as QueueTask;
  const more = texts(
    view({
      note,
      queued: [q(1, "run the int8 pass\nwith 500 images"), q(2, "b"), q(3, "c"), q(4, "d")],
      osText: undefined,
      host: host({ latency_ms: null }),
    }),
  );
  assert.equal(more.Note, "check simd #ivy");
  assert.equal(more.Queued, "run the int8 pass");
  assert.equal(more.Then, "c");
  assert.equal(more[""], "+1 more");
  assert.equal(more.Machine, "dex42");
  assert.equal(texts(view({ s: session({ agent: "" }) })).Agent, "unknown");
});

test("sessionMarkdown: the screen first, then the facts card", () => {
  const md = sessionMarkdown({
    ...view(),
    screen: "\n\n\n❯ Try it\n",
    loading: false,
    status: "idle · 33m",
  });
  const parts = md.split("\n\n");
  assert.equal(parts.length, 2);
  assert.match(parts[0], /^!\[frosting-crane's screen\]/);
  assert.match(parts[1], /^!\[About\]\(data:image\/svg\+xml;base64,/);
  const svg = Buffer.from(parts[1].split("base64,")[1].slice(0, -1), "base64").toString("utf8");
  assert.ok(svg.includes("test_folder") && svg.includes("Ubuntu") && svg.includes("171 ms"));
  // can't read it: the bar, one line, the facts
  const down = sessionMarkdown({ ...view(), loading: false, error: "permission denied", status: "idle · 33m" });
  assert.deepEqual(
    down.split("\n\n").map((p) => p.slice(0, 22)),
    ["![frosting-crane](data", "_Can't read the screen", "![About](data:image/sv"],
  );
});
