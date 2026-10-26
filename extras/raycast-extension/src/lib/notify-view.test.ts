// `npm test`: Notification Settings without Raycast (notify-view.ts) — what
// the settings say now, what a saved form writes, where the phone's go.
import assert from "node:assert/strict";
import { test } from "node:test";
import { dndKeep, moshiKinds, notifyNow, notifyWrites, remoteSinks } from "./notify-view";

const LINK = "raycast://extensions/aleph/aleph/menu-bar?launchType=background";
const from = (s: Record<string, string>) => (k: string) => s[k] ?? "";

test("what the settings say now", () => {
  const now = notifyNow(
    from({
      "notify.source": "agents",
      "notify.raycast": "true",
      "notify.raycast.kinds": "",
      notify: "true",
      notify_done: "false",
      "notify.remote_on": "blocked,limit,done",
      moshi: "on",
      "moshi.events": "needs_you,done",
      "moshi.approvals": "false",
    }),
    LINK,
  );
  assert.equal(now.source, "agents");
  assert.equal(now.raycast, true);
  assert.equal(now.deeplink, LINK);
  assert.deepEqual(now.kinds, ["blocked", "done", "error", "limit"]);
  assert.equal(now.click, "aleph");
  assert.deepEqual(now.remote_on, ["blocked", "limit", "done"]);
  assert.equal(now.moshi, "on");
  assert.deepEqual(now.moshi_events, ["needs_you", "done"]);
  assert.equal(now.moshi_approvals, false);
  // a build without Moshi: no Moshi fields
  const old = notifyNow(from({}), LINK);
  assert.equal(old.moshi, undefined);
  assert.equal(old.source, "both");
  assert.equal(old.notify, true);
  assert.deepEqual(old.remote_on, ["blocked", "limit"]);
  assert.deepEqual(moshiKinds(""), ["needs_you", "limit", "error", "down", "up", "done"]);
  assert.deepEqual(moshiKinds("none"), []);
});

test("a saved form writes only what changed", () => {
  const now = notifyNow(from({ moshi: "auto" }), LINK);
  assert.deepEqual(notifyWrites(now, { ...now }), []);
  // the deeplink field isn't shown for this extension's own link: nothing to write
  assert.deepEqual(notifyWrites(now, { ...now, deeplink: undefined }), []);
  assert.deepEqual(
    notifyWrites(now, {
      ...now,
      source: "herdr",
      kinds: [],
      remote_on: ["limit", "blocked"],
      notify_done: true,
      moshi: "off",
      moshi_events: ["needs_you"],
      moshi_approvals: false,
    }),
    [
      ["notify.source", "herdr"],
      ["notify.raycast.kinds", "none"],
      ["notify_done", "true"],
      ["moshi", "off"],
      ["moshi.events", "needs_you"],
      ["moshi.approvals", "false"],
    ],
  );
  assert.deepEqual(notifyWrites(now, { ...now, deeplink: " raycast://x " }), [
    ["notify.raycast.deeplink", "raycast://x"],
  ]);
});

test("where the phone's go, do not disturb", () => {
  assert.deepEqual(remoteSinks(from({ "notify.ntfy": "https://ntfy.sh/x", "notify.webhook": " " })), ["ntfy"]);
  assert.deepEqual(remoteSinks(from({ "notify.slack": "https://hooks", "notify.webhook": "https://w" })), [
    "Slack",
    "webhook",
  ]);
  assert.deepEqual(remoteSinks(from({})), []);
  const fmt = () => "14:30";
  assert.equal(dndKeep({ on: false }, fmt), "Leave off");
  assert.equal(dndKeep({ on: true }, fmt), "Leave on");
  assert.equal(dndKeep({ on: true, until: 1 }, fmt), "Leave on until 14:30");
});
