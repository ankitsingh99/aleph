// Sessions: every machine's agent sessions, one section per machine (its
// sessions by state and its link on the section line), "needs you" first,
// refreshed every 10 s. A row: the agent's logo, the session's name, its
// folder, its state as a coloured tag, its note and queued messages as icons.
// The detail opens on the session's live screen (`aleph peek --ansi` 3 s
// after the last read answered, drawn in its colours: screen.ts), the
// question while it waits for you, then a card of its folder, task, agent,
// machine, note and queued messages (session-view.ts).
// ↵ opens it in Aleph (Focus), ⌘↵ attaches in herdr (the openWith preference swaps them).
// A launch context {host} shows just that machine (Machines' Show Sessions).
import { Action, ActionPanel, Color, environment, Icon, LaunchProps, List } from "@raycast/api";
import { pausable } from "./lib/pause";
import { ReactNode, useEffect, useState } from "react";
import {
  Host,
  Note,
  noteFor,
  NOTES_ARGS,
  Peek,
  queueName,
  QueueTask,
  QUEUE_API,
  QUEUE_ARGS,
  selector,
  Session,
  shortPath,
  stateOf,
  STATUS_LABEL,
  statusText,
  NOTES_API,
  peekApi,
} from "./lib/cli";
import { noteAccessory, SessionActions } from "./lib/actions";
import { chartTheme } from "./lib/charts";
import { useAleph, useBrands, useFleet, useHelpHas, usePresenter } from "./lib/hooks";
import { agentLogo, cardLogos, hostTag, machineOsText } from "./lib/logos";
import { screenText } from "./lib/screen";
import { folderParts, hostSummary, rowStatus, sessionMarkdown } from "./lib/session-view";
import { followingTitle, navTitle, presenterItem, withBanner } from "./lib/showcase";
import { StartForm } from "./lib/start-form";
import { brand, statusColor, statusIcon } from "./lib/ui";

const POLL_MS = 10_000;
/** The screen again this long after the last read answered (never two at once on a slow link). */
const PEEK_MS = 3_000;
const PEEK_LINES = 80;
/** The most screen rows the card shows: what fits above the facts. */
const SCREEN_ROWS = 12;

const ORDER: Record<string, number> = { blocked: 0, working: 1, done: 2, limit: 3, idle: 4 };
const sorted = (v: Session[]) => [...v].sort((a, b) => (ORDER[stateOf(a)] ?? 9) - (ORDER[stateOf(b)] ?? 9));
const waiting = (h: Host) => Number(h.sessions.some((s) => s.status === "blocked"));
// machines with someone waiting come first
const hostOrder = (v: Host[]) => [...v].sort((a, b) => waiting(b) - waiting(a));

const ALL = "all";
const FILTERS: [string, string][] = [
  ["blocked", "Needs You"],
  ["working", "Working"],
  ["done", "Done"],
  ["limit", "At Limit"],
  ["idle", "Idle"],
];

function Sessions(props: LaunchProps<{ launchContext?: { host?: string } }>) {
  useBrands();
  const only = props.launchContext?.host;
  const { data, isLoading, errorInfo, stale, revalidate } = useFleet({
    pollMs: POLL_MS,
    failureTitle: "Couldn't list sessions",
  });
  const queue = useAleph<QueueTask[] | undefined>(QUEUE_ARGS, {
    optional: "queue",
    pollMs: POLL_MS,
    api: QUEUE_API,
    topics: ["queue"],
  });
  const notes = useAleph<Note[] | undefined>(NOTES_ARGS, {
    optional: "note",
    pollMs: POLL_MS,
    api: NOTES_API,
    topics: ["notes"],
  });
  const canMute = useHelpHas("notify", "mute");
  // showcase's presenter sync: the session the presenter's dashboard has selected
  const presenter = usePresenter();
  const presented = presenterItem(presenter, "sessions");
  const [showDetail, setShowDetail] = useState(true);
  const [selected, setSelected] = useState<string>();
  const [peekSel, setPeekSel] = useState<string>();
  const [filter, setFilter] = useState(only ? `host:${only}` : ALL);

  // debounce selection → peek, so arrowing through the list doesn't fire a call per row
  useEffect(() => {
    const t = setTimeout(() => setPeekSel(selected), 180);
    return () => clearTimeout(t);
  }, [selected]);
  // --ansi: the colours, and a passive read (a text read past the screen scrolls an idle agent)
  const peek = useAleph<Peek | undefined>(["peek", peekSel ?? "", "--json", "--ansi"], {
    optional: "peek",
    timeoutMs: 20_000,
    liveMs: PEEK_MS,
    api: peekApi(peekSel ?? "", true),
    execute: showDetail && !!peekSel,
  });
  const dark = environment.appearance !== "light";

  const refresh = () => {
    revalidate();
    queue.revalidate();
    notes.revalidate();
  };
  const hosts = hostOrder(data?.hosts ?? []);
  const all = hosts.flatMap((h) => h.sessions);
  const count = (st: string) => all.filter((s) => stateOf(s) === st).length;
  const blocked = count("blocked");
  const limited = count("limit");
  const queuedFor = (s: Session) =>
    (queue.data ?? []).filter((t) => t.state === "queued" && t.host === s.host && t.to === queueName(s));
  const keep = (h: Host, s: Session) => filter === ALL || filter === stateOf(s) || filter === `host:${h.alias}`;
  const shown = hosts.filter((h) => !filter.startsWith("host:") || filter === `host:${h.alias}`);
  const detail = showDetail && all.length > 0;

  const toggle = (
    <Action
      title={showDetail ? "Hide Details" : "Show Details"}
      icon={Icon.Sidebar}
      shortcut={{ modifiers: ["cmd", "shift"], key: "d" }}
      onAction={() => setShowDetail((x) => !x)}
    />
  );
  const title = [
    "Sessions",
    blocked ? `▲ ${blocked} need${blocked === 1 ? "s" : ""} you` : "",
    limited ? `◷ ${limited} at limit` : "",
    stale ? "cached" : "",
    followingTitle(presenter),
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <List
      isLoading={isLoading}
      isShowingDetail={detail}
      searchBarPlaceholder="Filter sessions"
      navigationTitle={navTitle(title)}
      selectedItemId={presented}
      onSelectionChange={(id) => setSelected(id ?? undefined)}
      searchBarAccessory={
        <List.Dropdown tooltip="Show" value={filter} onChange={setFilter}>
          <List.Dropdown.Item title="All Sessions" value={ALL} icon={Icon.List} />
          <List.Dropdown.Section title="State">
            {FILTERS.map(([v, t]) => (
              <List.Dropdown.Item key={v} title={`${t} (${count(v)})`} value={v} icon={statusIcon(v)} />
            ))}
          </List.Dropdown.Section>
          <List.Dropdown.Section title="Machine">
            {hosts.map((h) => (
              <List.Dropdown.Item
                key={h.alias}
                title={h.alias}
                value={`host:${h.alias}`}
                icon={{ source: Icon.CircleFilled, tintColor: h.reachable ? h.color : Color.SecondaryText }}
              />
            ))}
          </List.Dropdown.Section>
        </List.Dropdown>
      }
    >
      {errorInfo && !data ? (
        <List.EmptyView icon={Icon.Warning} title={errorInfo.title} description={errorInfo.message} />
      ) : (
        <List.EmptyView
          icon={Icon.Terminal}
          title={isLoading ? "Looking for sessions…" : filter === ALL ? "No sessions running" : "No sessions here"}
          actions={
            <ActionPanel>
              <Action.Push title="Start Session" icon={Icon.Plus} target={<StartForm />} />
            </ActionPanel>
          }
        />
      )}
      {shown.map((h) => (
        <List.Section key={h.alias} title={h.alias} subtitle={hostSummary(h)}>
          {sorted(h.sessions.filter((s) => keep(h, s))).map((s) => {
            const id = selector(s);
            return (
              <SessionItem
                key={id}
                id={id}
                host={h}
                s={s}
                refresh={refresh}
                queued={queuedFor(s)}
                note={noteFor(notes.data, s.host, s.name)}
                canMute={canMute}
                peek={id === peekSel ? peek.data : undefined}
                // selected but not read yet (the debounce) counts as reading
                peekLoading={id !== peekSel || peek.isLoading}
                peekError={id === peekSel ? peek.errorInfo?.title : undefined}
                dark={dark}
                wide={!detail}
                toggle={toggle}
                presented={id === presented}
              />
            );
          })}
        </List.Section>
      ))}
    </List>
  );
}

function SessionItem(p: {
  id: string;
  host: Host;
  s: Session;
  refresh: () => void;
  queued: QueueTask[];
  note?: Note;
  canMute: boolean;
  peek?: Peek;
  peekLoading: boolean;
  peekError?: string;
  dark: boolean;
  /** no detail beside the list: room for the task and the machine on the row */
  wide: boolean;
  toggle: ReactNode;
  /** the presenter's dashboard has it selected (showcase's presenter sync) */
  presented?: boolean;
}) {
  const { host, note } = p;
  // the peek hook keeps the last answer while the next one loads: only this pane's counts
  const pk =
    p.peek && (!p.peek.pane_id || (p.peek.pane_id === p.s.pane_id && p.peek.host === p.s.host)) ? p.peek : undefined;
  // peek reads the screen now: it may know about a limit before the fleet does
  const s: Session = pk && pk.limit !== undefined ? { ...p.s, limit: pk.limit } : p.s;
  const state = stateOf(s);
  const text = statusText(s);
  const color = host.stale ? Color.SecondaryText : statusColor(state);
  // the last answer was another pane's: this one's is on its way
  const reading = p.peekLoading || (!!p.peek && !pk);
  const output = pk ? screenText(pk.text).split("\n").slice(-PEEK_LINES).join("\n") : undefined;
  const b = brand(s.agent);
  const queued = p.queued.length;
  const folder = folderParts(s.cwd).name;
  const tags = (note?.tags ?? []).map((t) => `#${t}`).join(" ");

  return (
    <List.Item
      id={p.id}
      icon={{ value: agentLogo(s.agent), tooltip: s.agent ? b.name : "unknown agent" }}
      title={{ value: s.display || s.name || s.pane_id, tooltip: s.label }}
      subtitle={
        p.wide
          ? [shortPath(s.cwd), s.title].filter(Boolean).join("  ·  ")
          : { value: folder, tooltip: shortPath(s.cwd) }
      }
      keywords={[
        s.name,
        s.label,
        s.cwd,
        s.title,
        host.alias,
        s.agent,
        b.name,
        STATUS_LABEL[state] ?? state,
        note?.text ?? "",
        ...(note?.tags ?? []).map((t) => `#${t}`),
      ].filter(Boolean)}
      accessories={[
        ...(p.presented
          ? [
              {
                tag: { value: "▶ presenter", color: Color.Yellow },
                tooltip: "The presenter's dashboard has it selected",
              },
            ]
          : []),
        // beside the detail, the name keeps the room: the note and the queue as icons
        ...(p.wide
          ? noteAccessory(note)
          : note
            ? [{ icon: { source: Icon.Pencil, tintColor: Color.SecondaryText }, tooltip: `${note.text} ${tags}` }]
            : []),
        ...(queued
          ? [{ icon: Icon.Tray, text: String(queued), tooltip: `${queued} queued for after this reply` }]
          : []),
        { tag: { value: p.wide ? text : rowStatus(s), color }, tooltip: text },
        ...(p.wide ? [hostTag(host.alias, host.color, host.address)] : []),
      ]}
      detail={
        <List.Item.Detail
          isLoading={reading && !pk}
          markdown={withBanner(
            sessionMarkdown({
              s,
              host,
              note,
              queued: p.queued,
              t: chartTheme(p.dark),
              logo: cardLogos(p.dark),
              brand,
              osText: machineOsText(host.alias),
              screen: pk?.text,
              question: pk?.question,
              loading: reading,
              error: p.peekError,
              status: text,
              rows: SCREEN_ROWS,
            }),
          )}
        />
      }
      actions={
        <SessionActions
          s={s}
          refresh={p.refresh}
          output={output}
          extra={p.toggle}
          queued={queued}
          note={note}
          canMute={p.canMute}
        />
      }
    />
  );
}

// blank while showcase mode pauses every window
export default pausable(Sessions);
