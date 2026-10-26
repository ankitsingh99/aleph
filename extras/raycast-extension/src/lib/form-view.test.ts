// `npm test`: what the forms decide on their own (form-view.ts) — the
// default machine, the `aleph new` a form makes, the words in dropdowns.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  agentTitle,
  defaultHost,
  defaultSession,
  hostTitle,
  sessionTitle,
  startArgs,
  startCommand,
  usefulTitle,
  WHEN_TITLES,
} from "./form-view";

test("Start Session: the default machine", () => {
  const hosts = [
    { alias: "gpu2", reachable: false },
    { alias: "b31", reachable: true },
    { alias: "dex42", reachable: true },
  ];
  // asked for wins, whatever it is
  assert.equal(defaultHost(hosts, { machine: "b31" }, "gpu2"), "gpu2");
  // Aleph's default machine while it answers, then the older setting, then the first that answers
  assert.equal(defaultHost(hosts, { machine: "dex42" }), "dex42");
  assert.equal(defaultHost(hosts, { machine: "gpu2", ssh_host: "dex42" }), "dex42");
  assert.equal(defaultHost(hosts, { machine: "gpu2" }), "b31");
  assert.equal(defaultHost([{ alias: "gpu2", reachable: false }], {}), "gpu2");
  assert.equal(defaultHost([], { machine: "b31" }), undefined);
});

test("Start Session: the command a form makes", () => {
  assert.deepEqual(startArgs({ host: "b31" }), ["new", "--host", "b31"]);
  assert.deepEqual(
    startArgs({
      host: "b31",
      preset: "px",
      dir: "/srv/px",
      name: "temper",
      agent: "codex",
      prompt: "  fix the tests  ",
      worktree: true,
      branch: " fix/tests ",
      tile: "3",
    }),
    [
      "new",
      "--host",
      "b31",
      "@px",
      "--dir",
      "/srv/px",
      "--name",
      "temper",
      "--agent",
      "codex",
      "--worktree=fix/tests",
      "--tile",
      "3",
      "-m",
      "fix the tests",
    ],
  );
  // one tile, a worktree without a branch, an empty prompt
  assert.deepEqual(startArgs({ host: "b31", worktree: true, tile: "1", prompt: "  " }), [
    "new",
    "--host",
    "b31",
    "--worktree",
  ]);
  assert.equal(
    startCommand({ host: "b31", dir: "/srv/my px", prompt: "it's done" }),
    `aleph new --host b31 --dir '/srv/my px' -m 'it'\\''s done'`,
  );
  assert.equal(hostTitle({ alias: "b31", reachable: true }), "b31");
  assert.equal(hostTitle({ alias: "gpu2", reachable: false }), "gpu2 — offline");
  assert.equal(agentTitle("Claude Code", "2.1.309"), "Claude Code · 2.1.309");
  assert.equal(agentTitle("Codex", null), "Codex");
});

test("Send Message: the default session, the words", () => {
  const s = (name: string, status: string, since: number, limit?: string) => ({
    host: "b31",
    name,
    display: name,
    pane_id: `w1:${name}`,
    status,
    status_since: since,
    limit: limit ?? null,
  });
  // the one that finished last, then idle, then working, then a question
  assert.equal(defaultSession([s("a", "working", 9), s("b", "done", 1), s("c", "done", 5)])?.name, "c");
  assert.equal(defaultSession([s("a", "working", 9), s("b", "idle", 1)])?.name, "b");
  assert.equal(defaultSession([s("a", "blocked", 9), s("b", "working", 1)])?.name, "b");
  // at its usage limit: last
  assert.equal(defaultSession([s("a", "idle", 9, "2:40am"), s("b", "blocked", 1)])?.name, "b");
  assert.equal(defaultSession([]), undefined);
  assert.equal(sessionTitle(s("temper", "blocked", 1)), "temper · needs you");
  assert.equal(sessionTitle(s("quill", "idle", 1, "")), "quill · at limit");
  assert.equal(usefulTitle("Claude Code", "Claude Code"), "");
  assert.equal(usefulTitle("claude-code", "Claude Code"), "");
  assert.equal(usefulTitle("selene@example: ~/work/x", "Claude Code"), "");
  assert.equal(usefulTitle("  ", "Codex"), "");
  assert.equal(usefulTitle("Fixing the flaky test", "Claude Code"), "Fixing the flaky test");
  assert.equal(WHEN_TITLES.idle, "When it's free");
});
