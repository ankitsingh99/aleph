// Session actions shared by Sessions, Inbox, Peek and the menu bar, and the
// Open in Aleph / Attach in herdr pair every command starts its actions with.
import {
  Action,
  ActionPanel,
  Alert,
  Color,
  confirmAlert,
  Detail,
  Form,
  Icon,
  Keyboard,
  showToast,
  Toast,
  useNavigation,
} from "@raycast/api";
import { ReactNode, useState } from "react";
import {
  approve,
  close,
  muteSession,
  NAME_RE,
  Note,
  peek,
  Peek,
  queueName,
  rename,
  selector,
  sendKeys,
  Session,
  setNote,
  shortPath,
  stateOf,
  statusText,
  peekApi,
  undoClose,
} from "./cli";
import { act } from "./act";
import { closeToast } from "./closing";
import { copyText, CopyAction } from "./copy";
import { useAleph } from "./hooks";
import { agentLogo } from "./logos";
import { QueueList } from "./queue";
import { screenText } from "./screen";
import { SendForm, SendTarget, When } from "./send-form";
import { StartForm } from "./start-form";
import {
  attachInHerdr,
  openDashboard,
  openHerdrOn,
  openInAleph,
  openInTerminal,
  openMachine,
  openWith,
  Target,
} from "./terminal";
import { ALEPH_ICON, brand, errorText, LIMIT, NEEDS, statusColor } from "./ui";

export { act };

/**
 * Open in Aleph (Focus) and Attach in herdr, the preferred one first: it's the
 * default action (↵), the other is ⌘↵. `openWith` in the preferences picks.
 */
export function OpenSessionActions({ t }: { t: Target }) {
  const aleph = <Action key="aleph" title="Open in Aleph" icon={ALEPH_ICON} onAction={() => openInAleph(t)} />;
  const herdr = <Action key="herdr" title="Attach in Herdr" icon={Icon.Terminal} onAction={() => attachInHerdr(t)} />;
  return <>{openWith() === "herdr" ? [herdr, aleph] : [aleph, herdr]}</>;
}

/** A machine: its dashboard in Aleph (`aleph --host H`) and herdr's own window on it. */
export function OpenMachineActions({ host }: { host: string }) {
  const aleph = <Action key="aleph" title="Open in Aleph" icon={ALEPH_ICON} onAction={() => openMachine(host)} />;
  const herdr = <Action key="herdr" title="Open in Herdr" icon={Icon.Terminal} onAction={() => openHerdrOn(host)} />;
  return <>{openWith() === "herdr" ? [herdr, aleph] : [aleph, herdr]}</>;
}

export type SessionRef = Pick<Session, "host" | "pane_id" | "name" | "display" | "cwd" | "title"> & {
  agent?: string;
  status?: string;
  limit?: string | null;
};

const label = (s: SessionRef) => s.display || s.name || s.pane_id;

/** What to open for a session (the pane id is exact; names can repeat). */
export const targetOf = (s: SessionRef): Target => ({ host: s.host, sel: selector(s), label: label(s) });

/** Who a message goes to. */
export const messageTarget = (s: SessionRef): SendTarget => ({
  host: s.host,
  sel: selector(s),
  name: queueName(s),
  label: label(s),
  cwd: s.cwd,
  title: s.title,
  status: s.status,
});

export async function copyOutput(s: SessionRef, text?: string) {
  try {
    const t = text ?? (await peek(selector(s)))?.text;
    if (t == null) {
      await showToast({ style: Toast.Style.Failure, title: "This aleph has no `peek` yet" });
      return;
    }
    if (!(await copyText(screenText(t)))) return;
    await showToast({ style: Toast.Style.Success, title: `Copied ${label(s)}'s output` });
  } catch (e) {
    await showToast({ style: Toast.Style.Failure, ...errorText(e) });
  }
}

/**
 * Close it after Aleph's grace (`close.grace`): the toast counts down with an
 * Undo (⌘Z) that keeps it — offered from the first moment, as the CLI (no
 * daemon) waits the grace before it answers.
 */
export async function closeSession(s: SessionRef, refresh?: () => void): Promise<boolean> {
  const ok = await confirmAlert({
    title: `Close ${label(s)} on ${s.host}?`,
    message: "The agent stops and its herdr pane closes after a few seconds — Undo keeps it until then.",
    icon: Icon.XMarkCircle,
    primaryAction: { title: "Close Session", style: Alert.ActionStyle.Destructive },
  });
  if (!ok) return false;
  const sel = selector(s);
  const toast = await showToast({ style: Toast.Style.Animated, title: `Closing ${label(s)}…` });
  let tick: ReturnType<typeof setInterval> | undefined;
  const stop = () => clearInterval(tick);
  const undo: Toast.ActionOptions = {
    title: "Undo",
    shortcut: { modifiers: ["cmd"], key: "z" },
    onAction: () => {
      stop();
      keepSession(s, toast, refresh);
    },
  };
  toast.primaryAction = undo;
  try {
    const a = await close(sel);
    const show = () => {
      const t = closeToast(label(s), a);
      toast.title = t.title;
      toast.message = t.message;
      toast.primaryAction = t.undo ? undo : undefined;
      if (!t.undo) stop();
    };
    toast.style = Toast.Style.Success;
    show();
    // the countdown, while it waits
    if (a?.pending) tick = setInterval(show, 1000);
    refresh?.();
    return true;
  } catch (e) {
    toast.style = Toast.Style.Failure;
    Object.assign(toast, errorText(e));
    toast.primaryAction = undefined;
    return false;
  }
}

/** Undo: the close waiting out its grace is called off (`session.close.undo`). */
async function keepSession(s: SessionRef, toast: Toast, refresh?: () => void) {
  toast.primaryAction = undefined;
  toast.style = Toast.Style.Animated;
  toast.title = `Keeping ${label(s)}…`;
  try {
    await undoClose(selector(s));
    toast.style = Toast.Style.Success;
    toast.title = `Kept ${label(s)}`;
    toast.message = "It won't close";
    refresh?.();
  } catch (e) {
    toast.style = Toast.Style.Failure;
    Object.assign(toast, errorText(e));
  }
}

/** Reply (now) and Queue (once it's free, after a while, at a time): the message form. */
export function MessageActions({ s, refresh }: { s: SessionRef; refresh?: () => void }) {
  const form = (when: When) => <SendForm target={messageTarget(s)} initialWhen={when} refresh={refresh} />;
  return (
    <>
      <Action.Push
        title="Reply"
        icon={Icon.Message}
        shortcut={{ modifiers: ["cmd", "shift"], key: "p" }}
        target={form("now")}
      />
      <Action.Push
        title="Queue Message"
        icon={Icon.Tray}
        shortcut={{ modifiers: ["cmd", "opt"], key: "p" }}
        target={form("idle")}
      />
    </>
  );
}

/** Every action on one session, with shortcuts. `output` is the peeked text, if any. */
export function SessionActions({
  s,
  refresh,
  output,
  extra,
  queued = 0,
  note,
  canMute = false,
}: {
  s: SessionRef;
  refresh: () => void;
  output?: string;
  extra?: ReactNode;
  /** messages queued for this session */
  queued?: number;
  note?: Note;
  /** this aleph can mute a session (`notify mute`) */
  canMute?: boolean;
}) {
  const sel = selector(s);
  return (
    <ActionPanel>
      <ActionPanel.Section title={label(s)}>
        <OpenSessionActions t={targetOf(s)} />
        <Action
          title="Approve"
          icon={Icon.Check}
          shortcut={{ modifiers: ["cmd"], key: "1" }}
          onAction={() => act(`Approving ${label(s)}…`, () => approve(sel), refresh)}
        />
        <MessageActions s={s} refresh={refresh} />
        <Action.Push
          title="Peek"
          icon={Icon.Eye}
          shortcut={Keyboard.Shortcut.Common.ToggleQuickLook}
          target={<PeekView s={s} refresh={refresh} note={note} />}
        />
        <Action.Push
          title={queued ? `Queued Messages (${queued})` : "Queued Messages"}
          icon={Icon.List}
          shortcut={{ modifiers: ["cmd", "opt"], key: "l" }}
          target={<QueueList host={s.host} name={queueName(s)} />}
        />
        <Action.Push
          title="Send Keys"
          icon={Icon.Keyboard}
          shortcut={{ modifiers: ["cmd", "shift"], key: "k" }}
          target={<KeysForm s={s} refresh={refresh} />}
        />
        <Action
          title="Send Esc"
          icon={Icon.XMarkCircle}
          shortcut={{ modifiers: ["cmd", "shift"], key: "e" }}
          onAction={() => act(`Sending esc to ${label(s)}…`, () => sendKeys(sel, "esc"), refresh)}
        />
      </ActionPanel.Section>
      <ActionPanel.Section>
        <Action.Push
          title={note ? "Edit Note" : "Add Note"}
          icon={Icon.Pencil}
          shortcut={{ modifiers: ["cmd", "shift"], key: "n" }}
          target={<NoteForm s={s} note={note} refresh={refresh} />}
        />
        {canMute && s.name ? <MuteActions host={s.host} name={s.name} /> : null}
        <Action
          title="Copy Output"
          icon={Icon.CopyClipboard}
          shortcut={Keyboard.Shortcut.Common.Copy}
          onAction={() => copyOutput(s, output)}
        />
        <CopyAction
          title="Copy Name"
          content={`${s.host}:${s.name || s.display}`}
          shortcut={Keyboard.Shortcut.Common.CopyName}
        />
        <CopyAction title="Copy Folder" content={s.cwd} shortcut={Keyboard.Shortcut.Common.CopyPath} />
        <Action
          title="Browse Folders on Machine"
          icon={Icon.Folder}
          shortcut={{ modifiers: ["cmd"], key: "b" }}
          onAction={() => openInTerminal(["--host", s.host, "-i"], `Aleph · ${s.host}`)}
        />
        <Action.Push
          title="New Session in This Folder"
          icon={Icon.Plus}
          shortcut={Keyboard.Shortcut.Common.New}
          target={
            <StartForm
              initial={{
                host: s.host,
                dir: s.cwd?.startsWith("/") ? s.cwd : undefined,
                agent: s.agent && s.agent !== "shell" ? s.agent : undefined,
              }}
            />
          }
        />
      </ActionPanel.Section>
      <ActionPanel.Section>
        <Action.Push
          title="Rename"
          icon={Icon.TextCursor}
          shortcut={Keyboard.Shortcut.Common.Edit}
          target={<RenameForm s={s} refresh={refresh} />}
        />
        <Action
          title="Close Session"
          icon={Icon.XMarkCircle}
          style={Action.Style.Destructive}
          shortcut={Keyboard.Shortcut.Common.Remove}
          onAction={() => closeSession(s, refresh)}
        />
      </ActionPanel.Section>
      <ActionPanel.Section>
        {extra}
        <Action
          title="Refresh"
          icon={Icon.ArrowClockwise}
          shortcut={Keyboard.Shortcut.Common.Refresh}
          onAction={refresh}
        />
        <Action
          title="Open Dashboard"
          icon={Icon.AppWindowGrid3x3}
          shortcut={{ modifiers: ["cmd"], key: "d" }}
          onAction={openDashboard}
        />
      </ActionPanel.Section>
    </ActionPanel>
  );
}

/** Markdown for a session's screen: the question first while it waits for you, then the recent output. */
export function outputMarkdown(
  output: string | undefined,
  loading: boolean,
  error?: string,
  question?: string | null,
): string {
  const fence = (t: string) => "```\n" + t.replace(/```/g, "``​`").trimEnd() + "\n```";
  const q = question?.trim() ? `**Waiting for you**\n\n${fence(screenText(question))}\n\n` : "";
  if (output !== undefined) return q + (output ? fence(output) : "_no output yet_");
  if (error) return `_couldn't read the output: ${error}_`;
  if (loading) return "_reading output…_";
  return "_recent output needs `aleph peek` (a newer aleph)_";
}

const PEEK_LINES = 120;

/**
 * Peek: a session's recent output and the question it's asking, without
 * attaching (`aleph peek <sel> --json --ansi`: a passive read, again every 3 s while open).
 */
export function PeekView({ s, refresh, note }: { s: SessionRef; refresh?: () => void; note?: Note }) {
  const sel = selector(s);
  const p = useAleph<Peek | undefined>(["peek", sel, "--json", "--ansi"], {
    optional: "peek",
    timeoutMs: 20_000,
    pollMs: 3_000,
    api: peekApi(sel, true),
  });
  const pk = p.data;
  const now = { ...s, ...(pk ? { status: pk.status, limit: pk.limit ?? null } : {}) };
  const state = stateOf({ status: now.status ?? "", limit: now.limit });
  const output = pk ? screenText(pk.text).split("\n").slice(-PEEK_LINES).join("\n") : undefined;
  const b = brand(s.agent);
  const again = () => {
    p.revalidate();
    refresh?.();
  };
  const M = Detail.Metadata;
  return (
    <Detail
      isLoading={p.isLoading}
      navigationTitle={`Peek · ${label(s)} on ${s.host}`}
      markdown={outputMarkdown(output, p.isLoading, p.errorInfo?.title, pk?.question)}
      metadata={
        <M>
          <M.TagList title="Status">
            <M.TagList.Item
              text={statusText({ status: now.status ?? "", limit: now.limit })}
              color={statusColor(state)}
            />
          </M.TagList>
          <M.Label title="Machine" text={s.host} />
          {s.agent ? <M.Label title="Agent" text={b.name} icon={agentLogo(s.agent)} /> : null}
          {s.title ? <M.Label title="Working On" text={s.title} /> : null}
          <M.Label title="Folder" text={shortPath(s.cwd)} />
          {state === "limit" ? (
            <M.Label
              title="Usage Limit"
              text={now.limit ? `resets ${now.limit}` : "stopped — the agent didn't say when it resets"}
              icon={{ source: Icon.Clock, tintColor: LIMIT }}
            />
          ) : null}
          {pk?.approve_keys?.length ? (
            <M.Label
              title="Approve Sends"
              text={pk.approve_keys.join(" ")}
              icon={{ source: Icon.Check, tintColor: NEEDS }}
            />
          ) : null}
          {note ? <M.Label title="Note" text={note.text} icon={Icon.Pencil} /> : null}
        </M>
      }
      actions={
        <ActionPanel>
          <ActionPanel.Section title={label(s)}>
            <OpenSessionActions t={targetOf(s)} />
            <Action
              title="Approve"
              icon={Icon.Check}
              shortcut={{ modifiers: ["cmd"], key: "1" }}
              onAction={() => act(`Approving ${label(s)}…`, () => approve(sel), again)}
            />
            <MessageActions s={s} refresh={again} />
          </ActionPanel.Section>
          <ActionPanel.Section>
            <Action
              title="Copy Output"
              icon={Icon.CopyClipboard}
              shortcut={Keyboard.Shortcut.Common.Copy}
              onAction={() => copyOutput(s, output)}
            />
            <Action
              title="Refresh"
              icon={Icon.ArrowClockwise}
              shortcut={Keyboard.Shortcut.Common.Refresh}
              onAction={again}
            />
          </ActionPanel.Section>
        </ActionPanel>
      }
    />
  );
}

function SessionHeader({ s }: { s: SessionRef }) {
  return (
    <>
      <Form.Description title="Session" text={`${label(s)} · ${s.host} · ${shortPath(s.cwd)}`} />
      {s.title ? <Form.Description title="Working On" text={s.title} /> : null}
    </>
  );
}

/**
 * A note on a session (`aleph note`): kept by machine and session name, so it
 * outlives the session; `#tags` in it are searchable everywhere.
 */
export function NoteForm({ s, note, refresh }: { s: SessionRef; note?: Note; refresh?: () => void }) {
  const { pop } = useNavigation();
  // a running session by its pane (exact); one that ended by its name
  const sel = s.pane_id ? selector(s) : `${s.host}:${s.name}`;
  return (
    <Form
      navigationTitle={`Note on ${label(s)}`}
      actions={
        <ActionPanel>
          <Action.SubmitForm
            title="Save Note"
            icon={Icon.Pencil}
            onSubmit={async ({ text }: { text: string }) => {
              const clear = !text.trim();
              if (clear && !note) return pop();
              const ok = await act(
                clear ? "Clearing the note…" : "Saving the note…",
                () => setNote(sel, text),
                refresh,
                {
                  title: clear ? `Cleared the note on ${label(s)}` : `Noted on ${label(s)}`,
                },
              );
              if (ok) pop();
            }}
          />
          {note ? (
            <Action
              title="Clear Note"
              icon={Icon.Trash}
              style={Action.Style.Destructive}
              shortcut={Keyboard.Shortcut.Common.Remove}
              onAction={async () => {
                if (await act("Clearing the note…", () => setNote(sel, ""), refresh)) pop();
              }}
            />
          ) : null}
        </ActionPanel>
      }
    >
      <SessionHeader s={s} />
      <Form.TextField
        id="text"
        title="Note"
        placeholder="waiting on the data refresh #blocked"
        info="One line; #tags are searchable in Sessions and the Inbox (and in Aleph). Empty clears it."
        defaultValue={note?.text ?? ""}
        autoFocus
      />
      {note ? <Form.Description text={`Written ${new Date(note.t * 1000).toLocaleString()}`} /> : null}
    </Form>
  );
}

export function KeysForm({ s, refresh }: { s: SessionRef; refresh: () => void }) {
  const { pop } = useNavigation();
  return (
    <Form
      navigationTitle={`Keys for ${label(s)}`}
      actions={
        <ActionPanel>
          <Action.SubmitForm
            title="Send Keys"
            icon={Icon.Keyboard}
            onSubmit={async ({ keys }: { keys: string }) => {
              const v = keys.split(/\s+/).filter(Boolean);
              if (!v.length) return;
              if (await act(`Sending ${v.join(" ")}…`, () => sendKeys(selector(s), ...v), refresh)) pop();
            }}
          />
        </ActionPanel>
      }
    >
      <SessionHeader s={s} />
      <Form.TextField
        id="keys"
        title="Keys"
        placeholder="1 · 2 · enter · esc · up down · ctrl+c · shift+tab"
        info="Space-separated herdr key names"
        autoFocus
      />
    </Form>
  );
}

export function RenameForm({ s, refresh }: { s: SessionRef; refresh: () => void }) {
  const { pop } = useNavigation();
  const [error, setError] = useState<string>();
  return (
    <Form
      navigationTitle={`Rename ${label(s)}`}
      actions={
        <ActionPanel>
          <Action.SubmitForm
            title="Rename"
            icon={Icon.TextCursor}
            onSubmit={async ({ name }: { name: string }) => {
              if (!NAME_RE.test(name)) {
                setError("lowercase letters, digits, - and _ (start with a letter, ≤ 32)");
                return;
              }
              if (await act(`Renaming to ${name}…`, () => rename(selector(s), name), refresh)) pop();
            }}
          />
        </ActionPanel>
      }
    >
      <SessionHeader s={s} />
      <Form.TextField
        id="name"
        title="New Name"
        defaultValue={s.name}
        error={error}
        onChange={() => setError(undefined)}
        autoFocus
      />
    </Form>
  );
}

/** No notifications from one session for an hour, or again (`aleph notify mute host:name 1h | off`). */
export function MuteActions({ host, name }: { host: string; name: string }) {
  return (
    <>
      <Action
        title="Mute for an Hour"
        icon={Icon.BellDisabled}
        shortcut={{ modifiers: ["cmd", "opt"], key: "m" }}
        onAction={() =>
          act(`Muting ${name}…`, () => muteSession(host, name, "1h"), undefined, {
            title: `${name} is muted for an hour`,
            message: "its questions and finished turns don't notify you",
          })
        }
      />
      <Action
        title="Unmute"
        icon={Icon.Bell}
        onAction={() =>
          act(`Unmuting ${name}…`, () => muteSession(host, name, "off"), undefined, { title: `${name} is unmuted` })
        }
      />
    </>
  );
}

/** A note as a list accessory: its text on hover, `#tags` shown. */
export function noteAccessory(note: Note | undefined) {
  if (!note) return [];
  const tags = note.tags.map((t) => `#${t}`).join(" ");
  return [
    {
      icon: { source: Icon.Pencil, tintColor: Color.SecondaryText },
      ...(tags ? { text: tags } : {}),
      tooltip: note.text,
    },
  ];
}
