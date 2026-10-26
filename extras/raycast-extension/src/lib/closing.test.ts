// `npm test`: a close's answer as its toast says it (closing.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import { closeToast, parseClose, secondsLeft } from "./closing";

test("the daemon's answer: closing in a few seconds, Undo keeps it", () => {
  const a = parseClose('{"closed":false,"pending":true,"deadline":1000,"deadline_ms":1000500,"grace_s":5}');
  assert.ok(a?.pending);
  assert.equal(secondsLeft(a!, 996_000), 5, "rounded up");
  assert.deepEqual(closeToast("ignite", a, 996_000), {
    title: "Closing ignite in 5s",
    message: "The agent runs until then · Undo keeps it",
    undo: true,
  });
  // past its deadline: it's being closed, too late to undo
  assert.deepEqual(closeToast("ignite", a, 1_000_600), { title: "Closing ignite…", undo: false });
  // seconds only (an answer without deadline_ms)
  assert.equal(secondsLeft({ pending: true, deadline: 1000 }, 998_000), 2);
});

test("the CLI's answers, and an aleph that prints none", () => {
  assert.equal(closeToast("weave", parseClose('{"closed":true}')).title, "Closed weave");
  assert.equal(closeToast("weave", parseClose('{"closed":false,"kept":true}')).title, "Kept weave");
  assert.equal(closeToast("weave", parseClose('{"ended":true}')).title, "weave had ended");
  assert.equal(parseClose(""), undefined);
  assert.equal(parseClose("✓ closed weave"), undefined);
  assert.equal(parseClose("[1]"), undefined);
  assert.deepEqual(closeToast("weave", undefined), { title: "Closed weave", undo: false });
});
