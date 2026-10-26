// Notification Settings without Raycast: what Aleph's settings say now, which
// settings a saved form changes, and where phone notifications go. No
// Raycast imports: plain node tests it.
import { parseKinds } from "./model";

/** What the form shows (and saves). */
export interface NotifyValues {
  source: string;
  raycast: boolean;
  /** shown only when it isn't this extension's own (kept otherwise) */
  deeplink?: string;
  kinds: string[];
  notify: boolean;
  notify_done: boolean;
  click: string;
  remote_on: string[];
  moshi?: string;
  moshi_events?: string[];
  moshi_approvals?: boolean;
  dnd?: string;
}

const truthy = (v: string) => !["false", "0", "no", "off", ""].includes(v.trim().toLowerCase());
const list = (v: string[]) => (v.length ? v.join(",") : "none");
const same = (a: string[], b: string[]) => a.length === b.length && a.every((x) => b.includes(x));

/** Moshi's card kinds (moshi.events), with the words and the Inbox kind whose look they borrow. */
export const MOSHI_KINDS: [string, string, string][] = [
  ["needs_you", "Needs you", "blocked"],
  ["done", "Finished", "done"],
  ["limit", "At its usage limit", "limit"],
  ["error", "Problems", "error"],
  ["down", "Machine down", "error"],
  ["up", "Machine back up", "up"],
];

const MOSHI_DEFAULT = "needs_you,limit,error,down,up,done";

/** `moshi.events` → its kinds (empty: the default; `none`: none). */
export function moshiKinds(v: string): string[] {
  const t = v.trim();
  if (t === "none") return [];
  return (t || MOSHI_DEFAULT)
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);
}

/** The form's values from Aleph's settings (`settings list --json`, with its defaults). */
export function notifyNow(get: (key: string) => string, deeplinkDefault: string): NotifyValues {
  const has = (k: string) => get(k) !== "";
  return {
    source: get("notify.source") || "both",
    raycast: truthy(get("notify.raycast") || "false"),
    deeplink: get("notify.raycast.deeplink") || deeplinkDefault,
    kinds: parseKinds(get("notify.raycast.kinds")),
    notify: truthy(get("notify") || "true"),
    notify_done: truthy(get("notify_done") || "false"),
    click: get("notify.desktop.click") || "aleph",
    remote_on: parseKinds(get("notify.remote_on") || "blocked,limit"),
    // builds without Moshi have no `moshi` key
    ...(has("moshi")
      ? {
          moshi: get("moshi"),
          moshi_events: moshiKinds(get("moshi.events")),
          moshi_approvals: truthy(get("moshi.approvals") || "true"),
        }
      : {}),
  };
}

/** The settings a saved form changes (only those), as `[key, value]`. */
export function notifyWrites(now: NotifyValues, v: NotifyValues): [string, string][] {
  const w: [string, string][] = [];
  if (v.source !== now.source) w.push(["notify.source", v.source]);
  if (v.raycast !== now.raycast) w.push(["notify.raycast", String(v.raycast)]);
  const link = v.deeplink?.trim();
  if (link && link !== now.deeplink) w.push(["notify.raycast.deeplink", link]);
  if (!same(v.kinds, now.kinds)) w.push(["notify.raycast.kinds", list(v.kinds)]);
  if (v.notify !== now.notify) w.push(["notify", String(v.notify)]);
  if (v.notify_done !== now.notify_done) w.push(["notify_done", String(v.notify_done)]);
  if (v.click !== now.click) w.push(["notify.desktop.click", v.click]);
  if (!same(v.remote_on, now.remote_on)) w.push(["notify.remote_on", list(v.remote_on)]);
  if (v.moshi !== undefined && v.moshi !== now.moshi) w.push(["moshi", v.moshi]);
  if (v.moshi_events && now.moshi_events && !same(v.moshi_events, now.moshi_events))
    w.push(["moshi.events", list(v.moshi_events)]);
  if (v.moshi_approvals !== undefined && v.moshi_approvals !== now.moshi_approvals)
    w.push(["moshi.approvals", String(v.moshi_approvals)]);
  return w;
}

/** Where phone and webhook notifications go: the sinks that are set up (ntfy, Slack, a webhook). */
export function remoteSinks(get: (key: string) => string): string[] {
  return (
    [
      ["notify.ntfy", "ntfy"],
      ["notify.slack", "Slack"],
      ["notify.webhook", "webhook"],
    ] as const
  )
    .filter(([k]) => get(k).trim())
    .map(([, name]) => name);
}

/** `Leave off`, `Leave on until 14:30`: the do-not-disturb dropdown's first item. */
export function dndKeep(dnd: { on: boolean; until?: number }, fmt: (t: number) => string): string {
  if (!dnd.on) return "Leave off";
  return dnd.until ? `Leave on until ${fmt(dnd.until)}` : "Leave on";
}
