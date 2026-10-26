// Messages to a session: now (`aleph send <sel> <text>`), or queued for later
// (`aleph send <sel> <text> --after idle | 30m | 14:30`) — once it's done with
// its current reply, after a while, or at a time. Reply and Queue in Sessions,
// Inbox and the menu bar (through the Send Message command) all use this form.
// The sessions show their agent's logo and state; on its own the form picks
// the session that finished last.
import {
  Action,
  ActionPanel,
  closeMainWindow,
  Form,
  Icon,
  popToRoot,
  showHUD,
  showToast,
  Toast,
  useNavigation,
} from "@raycast/api";
import { useState } from "react";
import {
  CliError,
  Fleet,
  hasFlag,
  queueFor,
  selector,
  sendAfter,
  sendPrompt,
  Session,
  shortPath,
  stateOf,
  statusText,
} from "./cli";
import { defaultSession, sessionTitle, usefulTitle, WHEN_TITLES } from "./form-view";
import { useFleet, useWatching } from "./hooks";
import { agentLogo } from "./logos";
import { brand, errorText } from "./ui";

/** When the message goes: now, once the session is free, after a while, or at a time. */
export type When = "now" | "idle" | "30m" | "1h" | "2h" | "at";

const WHENS: [When, Icon][] = [
  ["now", Icon.Message],
  ["idle", Icon.Tray],
  ["30m", Icon.Clock],
  ["1h", Icon.Clock],
  ["2h", Icon.Clock],
  ["at", Icon.Calendar],
];

/** Who gets the message. `sel` is `host:pane` (exact) or `host:name`. */
export interface SendTarget {
  host: string;
  sel: string;
  /** the session's name (what the queue matches it by) */
  name: string;
  label: string;
  cwd?: string;
  title?: string;
  /** herdr's status: a session waiting on a question refuses a message now */
  status?: string;
  agent?: string;
  status_since?: number | null;
  limit?: string | null;
}

export function sendTarget(s: Session): SendTarget {
  return {
    host: s.host,
    sel: selector(s),
    name: s.name || s.display || s.pane_id,
    label: s.display || s.name || s.pane_id,
    cwd: s.cwd,
    title: s.title,
    status: s.status,
    agent: s.agent,
    status_since: s.status_since,
    limit: s.limit,
  };
}

const TIME_RE = /^([01]?\d|2[0-3]):[0-5]\d$/;

/**
 * Send `text` to `t` now, or queue it (`send --after`). Builds without
 * `--after` can still queue for "once it's free" (`queue add --to`).
 * Resolves to what happened, for the toast.
 */
export async function deliver(t: SendTarget, text: string, when: When, at?: string): Promise<string> {
  if (when === "now") {
    await sendPrompt(t.sel, text);
    return `Sent to ${t.label}`;
  }
  const after = when === "at" ? (at ?? "").trim() : when;
  if (await hasFlag("send", "--after")) {
    await sendAfter(t.sel, text, after);
  } else if (when === "idle") {
    await queueFor(t.host, t.name, text);
  } else {
    throw new CliError("This aleph can only queue for when the session is free", "update Aleph (aleph self-update)");
  }
  return when === "idle"
    ? `Queued for ${t.label} — it goes once it's free`
    : when === "at"
      ? `Queued for ${t.label} — at ${after}`
      : `Queued for ${t.label} — in ${after.replace("m", " min").replace("h", " h")}`;
}

const NOT_WATCHING = "The daemon isn't running: it waits until it is (aleph daemon install keeps one running).";

/**
 * The message form. With `target` it writes to that session; without, it
 * offers every running session (`initialSel` picks one). `root`: the form is
 * a command of its own (Send Message), so it closes with a HUD when done.
 */
export function SendForm({
  target,
  initialSel,
  initialWhen = "now",
  initialText = "",
  refresh,
  root = false,
}: {
  target?: SendTarget;
  initialSel?: string;
  initialWhen?: When;
  initialText?: string;
  refresh?: () => void;
  root?: boolean;
}) {
  const { pop } = useNavigation();
  const fleet = useFleet({ execute: !target, failureTitle: target ? undefined : "Couldn't list sessions" });
  const watching = useWatching();
  const [when, setWhen] = useState<When>(initialWhen);
  const [sel, setSel] = useState<string | undefined>(initialSel);
  const [textError, setTextError] = useState<string>();
  const [atError, setAtError] = useState<string>();

  const sessions = (fleet.data?.hosts ?? []).flatMap((h) => h.sessions);
  const first = pick(fleet.data, initialSel) ?? defaultSession(sessions);
  const current = target ? undefined : (sel ?? (first ? selector(first) : undefined));
  const chosen = sessions.find((s) => selector(s) === current);
  const t = target ?? (chosen ? sendTarget(chosen) : undefined);

  async function submit(v: { text: string; at?: string }, how: When) {
    const text = v.text?.trim() ?? "";
    if (!text) return setTextError("What should it do?");
    if (how === "at" && !TIME_RE.test(v.at?.trim() ?? "")) return setAtError("A time like 14:30");
    if (!t) {
      await showToast({ style: Toast.Style.Failure, title: "Pick a session first" });
      return;
    }
    const toast = await showToast({
      style: Toast.Style.Animated,
      title: how === "now" ? `Sending to ${t.label}…` : `Queueing for ${t.label}…`,
    });
    try {
      const done = await deliver(t, text, how, v.at);
      refresh?.();
      if (root) {
        await toast.hide();
        await showHUD(`${done}${how !== "now" && !watching ? " (the daemon isn't running)" : ""}`);
        await popToRoot({ clearSearchBar: true });
        await closeMainWindow();
        return;
      }
      toast.style = Toast.Style.Success;
      toast.title = done;
      toast.message = how !== "now" && !watching ? NOT_WATCHING : undefined;
      pop();
    } catch (e) {
      toast.style = Toast.Style.Failure;
      Object.assign(toast, errorText(e));
    }
  }

  const label = t?.label ?? "";
  const doing = target ? usefulTitle(target.title, target.agent ? brand(target.agent).name : undefined) : "";
  return (
    <Form
      isLoading={!target && fleet.isLoading && !fleet.data}
      navigationTitle={label ? `Message ${label}` : "Send Message"}
      actions={
        <ActionPanel>
          <Action.SubmitForm
            title={when === "now" ? "Send" : "Queue Message"}
            icon={when === "now" ? Icon.Message : Icon.Tray}
            onSubmit={(v: { text: string; at?: string }) => submit(v, when)}
          />
          {when === "now" ? (
            <Action.SubmitForm
              title="Queue Until It's Free"
              icon={Icon.Tray}
              shortcut={{ modifiers: ["cmd", "shift"], key: "return" }}
              onSubmit={(v: { text: string; at?: string }) => submit(v, "idle")}
            />
          ) : (
            <Action.SubmitForm
              title="Send Now Instead"
              icon={Icon.Message}
              shortcut={{ modifiers: ["cmd", "shift"], key: "return" }}
              onSubmit={(v: { text: string; at?: string }) => submit(v, "now")}
            />
          )}
        </ActionPanel>
      }
    >
      {target ? (
        <>
          <Form.Description
            title="To"
            text={[
              `${target.label} on ${target.host}`,
              target.status
                ? statusText({ status: target.status, limit: target.limit, status_since: target.status_since })
                : "",
              target.cwd && target.cwd !== "-" ? shortPath(target.cwd) : "",
            ]
              .filter(Boolean)
              .join(" · ")}
          />
          {doing ? <Form.Description title="Doing" text={doing} /> : null}
        </>
      ) : current ? (
        <Form.Dropdown id="session" title="To" value={current} onChange={setSel}>
          {(fleet.data?.hosts ?? []).map((h) =>
            h.sessions.length ? (
              <Form.Dropdown.Section key={h.alias} title={h.alias}>
                {h.sessions.map((s) => (
                  <Form.Dropdown.Item
                    key={selector(s)}
                    value={selector(s)}
                    title={sessionTitle(s)}
                    icon={agentLogo(s.agent)}
                    keywords={[s.name, s.cwd, s.agent, s.title, h.alias]}
                  />
                ))}
              </Form.Dropdown.Section>
            ) : null,
          )}
        </Form.Dropdown>
      ) : (
        <Form.Description
          title="To"
          text={fleet.isLoading ? "Looking for sessions…" : "No sessions running — Aleph Start Session starts one"}
        />
      )}
      <Form.TextArea
        id="text"
        title="Message"
        placeholder="then commit it · run the full test suite · …"
        defaultValue={initialText}
        error={textError}
        onChange={() => setTextError(undefined)}
        autoFocus
      />
      <Form.Dropdown id="when" title="Send" value={when} onChange={(v) => setWhen(v as When)}>
        {WHENS.map(([v, icon]) => (
          <Form.Dropdown.Item key={v} value={v} title={WHEN_TITLES[v]} icon={icon} />
        ))}
      </Form.Dropdown>
      {when === "at" ? (
        <Form.TextField
          id="at"
          title="Time"
          placeholder="14:30"
          info="Today, or tomorrow once it's past; it still waits for the session to be free"
          error={atError}
          onChange={() => setAtError(undefined)}
        />
      ) : null}
      {when === "now" && t?.status && stateOf({ status: t.status, limit: t.limit }) === "blocked" ? (
        <Form.Description text="It's waiting on a question: answer that first, or queue this (⇧⌘↵)." />
      ) : null}
      {when !== "now" && !watching ? <Form.Description text={NOT_WATCHING} /> : null}
    </Form>
  );
}

/** The session `sel` names in the fleet (`host:pane` or `host:name`). */
function pick(f: Fleet | undefined, sel: string | undefined): Session | undefined {
  if (!f || !sel) return undefined;
  const all = f.hosts.flatMap((h) => h.sessions);
  return (
    all.find((s) => selector(s) === sel) ??
    all.find((s) => `${s.host}:${s.name}` === sel || `${s.host}:${s.display}` === sel)
  );
}
