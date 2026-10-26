// The message queue (`aleph queue`): a message waits until its session is done
// with its current reply (or for the next idle agent on a machine, or for its
// time: `aleph send --after`), then goes to it. The Message Queue command, the
// new-message form and a session's "Queued Messages" list. Messages are
// handed out while the daemon (`aleph watch`) or the dashboard runs.
//
// A row: its state (queued, scheduled, sent, failed), the message's first
// line, who gets it (the session's name in the colour of its state now, with
// its agent's logo). The detail (queue-view.ts): who gets it and where it is
// in line, a track from queued through waiting to sent, the whole message.
import {
  Action,
  ActionPanel,
  Alert,
  Color,
  confirmAlert,
  environment,
  Form,
  Icon,
  Image,
  Keyboard,
  launchCommand,
  LaunchType,
  List,
  showToast,
  Toast,
  useNavigation,
} from "@raycast/api";
import { copyText, CopyAction } from "./copy";
import { useState } from "react";
import { act } from "./act";
import {
  Fleet,
  queueClear,
  queueFor,
  queueName,
  queueOnHost,
  queueRemove,
  queueSendNow,
  QueueTask,
  QUEUE_API,
  QUEUE_ARGS,
  RecentDir,
  selector,
  Session,
  shortPath,
  stateOf,
  STATUS_LABEL,
  RECENT_API,
} from "./cli";
import { chartTheme } from "./charts";
import { useAleph, useBrands, useFleet, useWatching } from "./hooks";
import { agentLogo, cardLogos, hostTag } from "./logos";
import { clock, liveOf, queueGroups, queueMarkdown, targetName, targetTooltip } from "./queue-view";
import { openDashboard, openSession } from "./terminal";
import { ACCENT, brand, DONE, DOWN, MUTED, statusColor, WARN, WORKING } from "./ui";

type Target = Pick<Session, "host" | "name" | "display" | "pane_id" | "cwd" | "title">;

const label = (s: Target) => s.display || s.name || s.pane_id;

const NOT_WATCHING = "The daemon isn't running: it waits until it is";

const HOST = "h:";
const SESS = "s:";
const ANY = "__any__";
const OTHER = "__other__";

/** A new queued message: for one session, or the next idle agent on a machine. */
export function NewQueueForm({ refresh, initial }: { refresh?: () => void; initial?: string }) {
  const { pop } = useNavigation();
  const fleet = useFleet({ failureTitle: "Couldn't list sessions" });
  const dirs = useAleph<RecentDir[]>(["recent", "--json"], { api: RECENT_API, topics: ["history"] });
  const watching = useWatching();
  const hosts = fleet.data?.hosts ?? [];
  const sessions = hosts.flatMap((h) => h.sessions);
  // what a queue is for: a session that's busy (else any)
  const first = sessions.find((s) => stateOf(s) === "working") ?? sessions[0];
  const [target, setTarget] = useState<string | undefined>(initial);
  const [folder, setFolder] = useState(ANY);
  const [error, setError] = useState<string>();
  const [dirError, setDirError] = useState<string>();
  const current =
    target ?? (first ? `${SESS}${first.host}:${queueName(first)}` : hosts[0] && `${HOST}${hosts[0].alias}`);
  const onHost = current?.startsWith(HOST) ? current.slice(HOST.length) : undefined;
  const hostDirs = (dirs.data ?? []).filter((d) => d.host === onHost).slice(0, 12);

  async function submit(v: { target: string; text: string; folder?: string; dir?: string }) {
    const text = v.text?.trim();
    if (!text) return setError("What should it do?");
    if (v.target?.startsWith(SESS)) {
      const [host, ...rest] = v.target.slice(SESS.length).split(":");
      const name = rest.join(":");
      const ok = await act(`Queueing for ${name}…`, () => queueFor(host, name, text), refresh, {
        title: `Queued for ${name} on ${host}`,
        message: watching ? "It goes once it's done with its current reply" : NOT_WATCHING,
      });
      if (ok) pop();
    } else if (v.target?.startsWith(HOST)) {
      const host = v.target.slice(HOST.length);
      const dir = v.folder === OTHER ? v.dir?.trim() : v.folder && v.folder !== ANY ? v.folder : "";
      if (v.folder === OTHER && !dir?.startsWith("/")) return setDirError("An absolute path on the machine");
      const ok = await act(`Queueing on ${host}…`, () => queueOnHost(host, text, dir || undefined), refresh, {
        title: `Queued on ${host}`,
        message: watching ? "The next idle agent there takes it" : NOT_WATCHING,
      });
      if (ok) pop();
    }
  }

  return (
    <Form
      isLoading={fleet.isLoading && !fleet.data}
      navigationTitle="Queue Message"
      actions={
        <ActionPanel>
          <Action.SubmitForm title="Queue Message" icon={Icon.Tray} onSubmit={submit} />
        </ActionPanel>
      }
    >
      <Form.Dropdown id="target" title="For" value={current} onChange={setTarget}>
        {hosts.map((h) => (
          <Form.Dropdown.Section key={h.alias} title={h.alias}>
            {h.sessions.map((s) => (
              <Form.Dropdown.Item
                key={selector(s)}
                value={`${SESS}${s.host}:${queueName(s)}`}
                title={`${label(s)} · ${STATUS_LABEL[stateOf(s)] ?? s.status}`}
                icon={agentLogo(s.agent)}
                keywords={[s.name, s.cwd, s.agent, h.alias]}
              />
            ))}
            <Form.Dropdown.Item
              value={`${HOST}${h.alias}`}
              title={`Next idle agent on ${h.alias}`}
              icon={{ source: Icon.Shuffle, tintColor: h.color }}
            />
          </Form.Dropdown.Section>
        ))}
      </Form.Dropdown>
      <Form.TextArea
        id="text"
        title="Message"
        placeholder="run the full test suite and fix what fails"
        error={error}
        onChange={() => setError(undefined)}
        autoFocus
      />
      {onHost ? (
        <Form.Dropdown
          id="folder"
          title="Folder"
          value={folder}
          onChange={setFolder}
          info="Only an agent in this folder takes it"
        >
          <Form.Dropdown.Item value={ANY} title="Any folder" icon={Icon.HardDrive} />
          {hostDirs.length > 0 ? (
            <Form.Dropdown.Section title="Recent">
              {hostDirs.map((d) => (
                <Form.Dropdown.Item key={d.dir} value={d.dir} title={shortPath(d.dir)} icon={Icon.Folder} />
              ))}
            </Form.Dropdown.Section>
          ) : null}
          <Form.Dropdown.Item value={OTHER} title="Other…" icon={Icon.Pencil} />
        </Form.Dropdown>
      ) : null}
      {onHost && folder === OTHER ? (
        <Form.TextField
          id="dir"
          title="Path"
          placeholder="/home/me/project"
          error={dirError}
          onChange={() => setDirError(undefined)}
        />
      ) : null}
      {!watching ? <Form.Description text={`${NOT_WATCHING}.`} /> : null}
    </Form>
  );
}

/** A row's icon: the state, in its colour (a message waiting for its time: a gold clock). */
function stateIcon(t: QueueTask, now: number): Image.ImageLike {
  if (t.state === "sent") return { source: Icon.CheckCircle, tintColor: DONE };
  if (t.state !== "queued") return { source: Icon.XMarkCircle, tintColor: DOWN };
  if (t.after && t.after > now) return { source: Icon.Clock, tintColor: ACCENT };
  return { source: Icon.Tray, tintColor: WORKING };
}

/** The queue: every machine's (or one session's) queued, sent and failed messages. */
export function QueueList({ host, name }: { host?: string; name?: string }) {
  useBrands();
  const q = useAleph<QueueTask[] | undefined>(QUEUE_ARGS, {
    optional: "queue",
    pollMs: 5_000,
    failureTitle: "Couldn't read the queue",
    api: QUEUE_API,
    topics: ["queue"],
  });
  const fleet = useFleet();
  const watching = useWatching();
  const [showDetail, setShowDetail] = useState(true);
  const dark = environment.appearance !== "light";
  const unsupported = !q.isLoading && !q.errorInfo && q.data === undefined;
  const all = (q.data ?? []).filter((t) => (!host || t.host === host) && (!name || t.to === name));
  const groups = queueGroups(all);
  const waiting = all.filter((t) => t.state === "queued").length;
  const refresh = q.revalidate;
  const scope = name ? `${name} on ${host}` : host;

  return (
    <List
      isLoading={q.isLoading}
      isShowingDetail={showDetail && all.length > 0}
      navigationTitle={scope ? `Queue · ${scope}` : "Message Queue"}
      searchBarPlaceholder="Filter by message, session or machine"
    >
      {unsupported ? (
        <List.EmptyView
          icon={Icon.Info}
          title="This aleph has no `queue` command yet"
          description="Update Aleph (aleph self-update)."
        />
      ) : q.errorInfo && !q.data ? (
        <List.EmptyView icon={Icon.Warning} title={q.errorInfo.title} description={q.errorInfo.message} />
      ) : (
        <List.EmptyView
          icon={{ source: Icon.Tray, tintColor: WORKING }}
          title={q.isLoading ? "Reading the queue…" : "Nothing queued"}
          description={
            name ? `A message queued for ${name} goes once it's done with its reply.` : "Queue a message: ⌘N"
          }
          actions={
            <ActionPanel>
              <NewMessageAction host={host} name={name} refresh={refresh} />
            </ActionPanel>
          }
        />
      )}
      {waiting > 0 && !watching ? (
        <List.Section title="Not going out">
          <List.Item
            icon={{ source: Icon.Warning, tintColor: WARN }}
            title="Daemon isn't running"
            accessories={[{ tag: { value: `${waiting} waiting`, color: WARN } }]}
            detail={
              <List.Item.Detail
                markdown={`**${waiting} queued message${waiting === 1 ? "" : "s"} wait${waiting === 1 ? "s" : ""}** for Aleph's daemon (or the dashboard) to hand ${waiting === 1 ? "it" : "them"} out.\n\n\`aleph daemon install\` keeps one running.`}
              />
            }
            actions={
              <ActionPanel>
                <Action
                  title="Open Aleph Daemon"
                  icon={Icon.Gauge}
                  onAction={() => launchCommand({ name: "daemon", type: LaunchType.UserInitiated })}
                />
                <Action title="Open Dashboard" icon={Icon.AppWindowGrid3x3} onAction={openDashboard} />
                <CopyAction title="Copy Install Command" content="aleph daemon install" />
              </ActionPanel>
            }
          />
        </List.Section>
      ) : null}
      {groups.map((g) => (
        <List.Section key={g.state} title={g.title} subtitle={g.subtitle}>
          {g.tasks.map((t) => (
            <TaskItem
              key={t.id}
              t={t}
              all={all}
              fleet={fleet.data}
              refresh={refresh}
              scope={{ host, name }}
              dark={dark}
              showDetail={showDetail}
              toggle={() => setShowDetail((x) => !x)}
            />
          ))}
        </List.Section>
      ))}
    </List>
  );
}

function NewMessageAction({ host, name, refresh }: { host?: string; name?: string; refresh: () => void }) {
  return (
    <Action.Push
      title="New Message"
      icon={Icon.Plus}
      shortcut={Keyboard.Shortcut.Common.New}
      target={
        <NewQueueForm
          refresh={refresh}
          initial={host ? (name ? `${SESS}${host}:${name}` : `${HOST}${host}`) : undefined}
        />
      }
    />
  );
}

function TaskItem({
  t,
  all,
  fleet,
  refresh,
  scope,
  dark,
  showDetail,
  toggle,
}: {
  t: QueueTask;
  all: QueueTask[];
  fleet?: Fleet;
  refresh: () => void;
  scope: { host?: string; name?: string };
  dark: boolean;
  showDetail: boolean;
  toggle: () => void;
}) {
  const now = Date.now() / 1000;
  const h = fleet?.hosts.find((x) => x.alias === t.host);
  const live = liveOf(t, fleet);
  const agent = live?.agent ?? t.agent ?? undefined;
  const when = t.at ?? t.added;
  const due = t.state === "queued" && t.after && t.after > now ? t.after : undefined;
  const first = t.prompt.trim().split("\n")[0];
  const forOne = !!(t.to || t.session);
  const accessories: List.Item.Accessory[] = [
    {
      icon: forOne ? (agent ? agentLogo(agent) : Icon.Terminal) : { source: Icon.Shuffle, tintColor: MUTED },
      tag: { value: targetName(t), color: live ? statusColor(stateOf(live)) : Color.SecondaryText },
      tooltip: targetTooltip(t, live, now),
    },
  ];
  if (!showDetail) accessories.push(hostTag(t.host, h?.color));
  if (due) accessories.push({ icon: Icon.Clock, text: clock(due, now), tooltip: "not before" });
  else if (!showDetail)
    accessories.push({
      date: new Date(when * 1000),
      tooltip: `${t.state} ${new Date(when * 1000).toLocaleString()} · #${t.id}`,
    });
  const open = live ? (
    <Action
      key="open"
      title={`Open ${live.display || live.name}`}
      icon={Icon.Terminal}
      shortcut={Keyboard.Shortcut.Common.Open}
      onAction={() => openSession({ host: t.host, sel: selector(live), label: live.display || live.name })}
    />
  ) : null;
  const again =
    t.state !== "queued" ? (
      <Action
        key="again"
        title="Queue Again"
        icon={Icon.Repeat}
        onAction={() =>
          act(
            "Queueing again…",
            () => (t.to ? queueFor(t.host, t.to, t.prompt) : queueOnHost(t.host, t.prompt, t.dir ?? undefined)),
            refresh,
          )
        }
      />
    ) : null;

  return (
    <List.Item
      id={String(t.id)}
      icon={stateIcon(t, now)}
      title={{ value: first, tooltip: t.prompt }}
      keywords={[t.host, t.to ?? "", t.session ?? "", t.dir ?? "", `#${t.id}`]}
      accessories={accessories}
      detail={
        <List.Item.Detail
          markdown={queueMarkdown({
            t,
            all,
            live,
            hostColor: h?.color,
            theme: chartTheme(dark),
            logo: cardLogos(dark),
            brand,
            now,
          })}
        />
      }
      actions={
        <ActionPanel>
          <ActionPanel.Section title={`#${t.id}`}>
            {t.state === "queued" && t.to ? (
              <Action
                title="Send Now"
                icon={Icon.Message}
                onAction={() =>
                  act(`Sending to ${t.to}…`, () => queueSendNow(t), refresh, { title: `Sent to ${t.to}` })
                }
              />
            ) : null}
            {/* a sent message: what it did is in its session */}
            {t.state === "sent" ? [open, again] : [again, open]}
            <Action
              title="Copy Message"
              icon={Icon.CopyClipboard}
              shortcut={Keyboard.Shortcut.Common.Copy}
              onAction={async () => {
                if (!(await copyText(t.prompt))) return;
                await showToast({ style: Toast.Style.Success, title: "Copied the message" });
              }}
            />
          </ActionPanel.Section>
          <ActionPanel.Section>
            <NewMessageAction host={scope.host} name={scope.name} refresh={refresh} />
            {t.state === "queued" ? (
              <Action
                title="Remove from Queue"
                icon={Icon.Trash}
                style={Action.Style.Destructive}
                shortcut={Keyboard.Shortcut.Common.Remove}
                onAction={async () => {
                  const ok = await confirmAlert({
                    title: `Remove message #${t.id}?`,
                    message: first,
                    icon: Icon.Trash,
                    primaryAction: { title: "Remove", style: Alert.ActionStyle.Destructive },
                  });
                  if (ok)
                    await act(`Removing #${t.id}…`, () => queueRemove(t.id), refresh, { title: `Removed #${t.id}` });
                }}
              />
            ) : null}
            {t.state === "queued" ? (
              <Action
                title={`Clear Queue on ${t.host}`}
                icon={Icon.XMarkTopRightSquare}
                style={Action.Style.Destructive}
                shortcut={Keyboard.Shortcut.Common.RemoveAll}
                onAction={async () => {
                  const ok = await confirmAlert({
                    title: `Clear every queued message on ${t.host}?`,
                    message: "Messages already sent stay in the list.",
                    icon: Icon.XMarkTopRightSquare,
                    primaryAction: { title: "Clear", style: Alert.ActionStyle.Destructive },
                  });
                  if (ok) await act(`Clearing ${t.host}'s queue…`, () => queueClear(t.host), refresh);
                }}
              />
            ) : null}
            <Action
              title={showDetail ? "Hide Details" : "Show Details"}
              icon={Icon.Sidebar}
              shortcut={{ modifiers: ["cmd", "shift"], key: "d" }}
              onAction={toggle}
            />
            <Action
              title="Refresh"
              icon={Icon.ArrowClockwise}
              shortcut={Keyboard.Shortcut.Common.Refresh}
              onAction={refresh}
            />
          </ActionPanel.Section>
        </ActionPanel>
      }
    />
  );
}
