// Inbox: what needs you, then everything that happened — like the dashboard's
// Inbox. `aleph inbox --json` (needs you, finished, started, ended,
// unreachable, auto-approved, tasks, restarts, usage limits) merged with the
// sessions you started (`aleph history --json`, the old History command): a
// launch joins its "started" event, so one row has both what happened and how
// to start it again.
// * Now — live from the fleet: sessions waiting for you or at their limit,
//   machines that dropped off.
// * The timeline, grouped by machine (or agent, kind, day) and sorted by
//   machine (or agent, kind, time): ⌘G and ⌘⇧G, remembered. A run of
//   identical events folds into one row (×n). Each row says what's true now
//   (still waiting, back up, …).
// Actions: Open in Aleph · Attach in herdr · Reply · Approve · Queue · Start
// Again (with the recorded folder, agent and prompt) · notes. With the daemon
// (the events come from its `inbox.list`, which says what's new since the read
// mark, the daemon's or `aleph inbox`'s): Mark Read up to Here · Mark All Read · Dismiss.
// A launch context {host, name, kind?, t?} — a desktop banner's click with
// notify.desktop.click = raycast, a deeplink, the menu bar — selects that
// event's row (or that session's newest).
import {
  Action,
  ActionPanel,
  Alert,
  Color,
  confirmAlert,
  Icon,
  Keyboard,
  launchCommand,
  LaunchProps,
  LaunchType,
  List,
} from "@raycast/api";
import { pausable } from "./lib/pause";
import { CopyAction } from "./lib/copy";
import { useCachedState } from "@raycast/utils";
import { useState } from "react";
import {
  approve,
  day,
  dismissEvents,
  eventReason,
  Fleet,
  HistoryEntry,
  HISTORY_ARGS,
  Host,
  InboxEvent,
  INBOX_ARGS,
  inboxApi,
  markRead,
  Note,
  noteFor,
  NOTES_ARGS,
  selector,
  Session,
  shortPath,
  stateOf,
  statusText,
  HISTORY_API,
  NOTES_API,
} from "./lib/cli";
import {
  act,
  MessageActions,
  MuteActions,
  noteAccessory,
  NoteForm,
  OpenMachineActions,
  OpenSessionActions,
} from "./lib/actions";
import { useAleph, useBrands, useFleet, useHelpHas } from "./lib/hooks";
import { GroupBy, groupItems, GROUPS, SortBy, SORTS } from "./lib/inbox-groups";
import { agentLogo, hostTag } from "./lib/logos";
import { explainError, fold, herdrStopped, INBOX_FILTERS, keeps, landing, timeline, TimelineItem } from "./lib/model";
import { navTitle } from "./lib/showcase";
import { againArgs, againCommand, Recipe, StartForm, startAgain, useDetach } from "./lib/start-form";
import { openDashboard, openInTerminal } from "./lib/terminal";
import { ACCENT, brand, DONE, DOWN, kindLook, LIMIT, MUTED, NEEDS } from "./lib/ui";

const POLL_MS = 15_000;

interface Context {
  host?: string;
  name?: string;
  kind?: string;
  t?: number;
}

/** The session an event is about, if it's still running. */
function liveSession(f: Fleet | undefined, host: string, name: string): Session | undefined {
  if (!name) return undefined;
  return f?.hosts
    .find((h) => h.alias === host)
    ?.sessions.find((s) => s.name === name || s.display === name || s.label === name);
}

/** How a session was started: its launch, else the newest launch with that name, else what it runs now. */
function recipeFor(it: TimelineItem, launches: HistoryEntry[], live?: Session): Recipe | undefined {
  if (!it.e.name) return undefined;
  const l = it.launch ?? launches.find((x) => x.host === it.e.host && x.name === it.e.name && x.t <= it.e.t + 120);
  if (l) return l;
  if (live && live.cwd?.startsWith("/")) return { host: live.host, dir: live.cwd, agent: live.agent };
  return undefined;
}

/** "Now": what needs you right now, live from the fleet (not from history). */
function nowItems(f: Fleet | undefined): TimelineItem[] {
  const out: TimelineItem[] = [];
  const now = Math.floor(Date.now() / 1000);
  for (const h of f?.hosts ?? []) {
    for (const s of h.sessions) {
      const kind = s.status === "blocked" ? "blocked" : s.limit != null ? "limit" : "";
      if (!kind) continue;
      const title = kind === "limit" && s.limit ? `resets ${s.limit}` : s.title;
      const e = { t: s.status_since ?? now, host: h.alias, name: s.name, kind, title, agent: s.agent };
      out.push({ key: `now|${h.alias}|${s.pane_id}|${kind}`, e, n: 1, first: e.t, logged: false });
    }
    if (!h.reachable && !herdrStopped(h.error)) {
      const e = { t: h.fetched_at ?? now, host: h.alias, name: "", kind: "error", title: h.error ?? "", agent: "" };
      out.push({ key: `now|${h.alias}|down`, e, n: 1, first: e.t, logged: false });
    }
  }
  // what blocks work first: questions, then limits, then machines
  const rank = (k: string) => (k === "blocked" ? 0 : k === "limit" ? 1 : 2);
  return out.sort((a, b) => rank(a.e.kind) - rank(b.e.kind) || a.e.t - b.e.t);
}

function Inbox(props: LaunchProps<{ launchContext?: Context }>) {
  useBrands();
  const ctx = props.launchContext;
  const events = useAleph<InboxEvent[] | undefined>(INBOX_ARGS, {
    optional: "inbox",
    pollMs: POLL_MS,
    failureTitle: "Couldn't read the inbox",
    api: inboxApi(300),
    topics: ["inbox"],
  });
  const history = useAleph<HistoryEntry[] | undefined>(HISTORY_ARGS, {
    optional: "history",
    pollMs: POLL_MS,
    api: HISTORY_API,
    topics: ["history"],
  });
  const notes = useAleph<Note[] | undefined>(NOTES_ARGS, {
    optional: "note",
    pollMs: POLL_MS,
    api: NOTES_API,
    topics: ["notes"],
  });
  // machine colours, and which sessions still run (open / reply / approve)
  const fleet = useFleet({ pollMs: POLL_MS });
  const detach = useDetach();
  const canMute = useHelpHas("notify", "mute");
  const [filter, setFilter] = useState("all");
  const [showDetail, setShowDetail] = useState(false);
  const [group, setGroup] = useCachedState<GroupBy>("inbox.group", "machine");
  const [sort, setSort] = useCachedState<SortBy>("inbox.sort", "machine");
  const view = { group, sort, setGroup, setSort };

  const unsupported = !events.isLoading && !events.errorInfo && events.data === undefined;
  // the daemon answered (only its events say whether they're new): read marks and dismissing work
  const fromDaemon = (events.data ?? []).some((e) => e.unread !== undefined);
  const unread = (events.data ?? []).filter((e) => e.unread).length;
  const launches = [...(history.data ?? [])].sort((a, b) => b.t - a.t);
  const all = fold(timeline(events.data ?? [], launches));
  const shown = all.filter((it) => keeps(filter, it));
  const nowShown = ["all", "attention", "problems"].includes(filter)
    ? nowItems(fleet.data).filter((it) => keeps(filter, it))
    : filter.startsWith("host:")
      ? nowItems(fleet.data).filter((it) => keeps(filter, it))
      : [];
  const hosts = [...new Set([...all.map((it) => it.e.host), ...(fleet.data?.hosts ?? []).map((h) => h.alias)])].sort();
  const hostOf = (h: string) => fleet.data?.hosts.find((x) => x.alias === h);
  // what "Dismiss All" takes away: every logged event the list shows (the filter's, if any)
  const allIds = shown.flatMap((it) => (it.logged ? (it.ids ?? []) : []));
  const groups = groupItems(shown, group, sort, {
    hosts: (fleet.data?.hosts ?? []).map((h) => h.alias),
    agentName: (id) => brand(id).name,
    kindWord: (k) => kindLook(k).word,
    dayOf: day,
  });
  const refresh = () => {
    // the menu bar lists what's unread: tell it at once
    launchCommand({ name: "menu-bar", type: LaunchType.Background }).catch(() => undefined);
    events.revalidate();
    history.revalidate();
    notes.revalidate();
    fleet.revalidate();
  };
  // a banner click, a deeplink or the menu bar names an event: select its row
  const wanted = landing(all, ctx);

  const row = (it: TimelineItem, live: boolean) => {
    const s = liveSession(fleet.data, it.e.host, it.e.name);
    return (
      <InboxItem
        key={it.key}
        it={it}
        now={live}
        host={hostOf(it.e.host)}
        live={s}
        fleetKnown={!!fleet.data}
        note={noteFor(notes.data, it.e.host, it.e.name)}
        recipe={recipeFor(it, launches, s)}
        detach={!!detach.data}
        canMute={canMute}
        fromDaemon={fromDaemon}
        refresh={refresh}
        showDetail={showDetail}
        toggle={() => setShowDetail((x) => !x)}
        view={view}
        allIds={allIds}
      />
    );
  };

  return (
    <List
      isLoading={events.isLoading || history.isLoading}
      isShowingDetail={showDetail && shown.length + nowShown.length > 0}
      navigationTitle={navTitle(
        ["Aleph Inbox", unread ? `${unread} new` : "", `by ${group === "day" ? "day" : group}`]
          .filter(Boolean)
          .join(" · "),
      )}
      selectedItemId={wanted?.key}
      searchBarPlaceholder="Filter by session, machine, agent, folder, prompt, note or reason"
      searchBarAccessory={
        <List.Dropdown tooltip="Show" value={filter} onChange={setFilter}>
          <List.Dropdown.Item title="Everything" value="all" icon={Icon.Tray} />
          <List.Dropdown.Section title="Kind">
            {INBOX_FILTERS.map((x) => (
              <List.Dropdown.Item
                key={x.id}
                title={x.title}
                value={x.id}
                icon={x.kinds ? kindLook(x.kinds[0]).icon : { source: Icon.Rocket, tintColor: ACCENT }}
              />
            ))}
          </List.Dropdown.Section>
          <List.Dropdown.Section title="Machine">
            {hosts.map((h) => (
              <List.Dropdown.Item
                key={h}
                title={h}
                value={`host:${h}`}
                icon={{ source: Icon.CircleFilled, tintColor: hostOf(h)?.color || Color.SecondaryText }}
              />
            ))}
          </List.Dropdown.Section>
        </List.Dropdown>
      }
    >
      {unsupported ? (
        <List.EmptyView
          icon={Icon.Info}
          title="This aleph has no `inbox` command yet"
          description="Update Aleph (aleph self-update). Aleph Sessions works meanwhile."
        />
      ) : events.errorInfo && !events.data ? (
        <List.EmptyView icon={Icon.Warning} title={events.errorInfo.title} description={events.errorInfo.message} />
      ) : (
        <List.EmptyView
          icon={Icon.Tray}
          title={events.isLoading ? "Reading the inbox…" : "Nothing here"}
          description="Messages appear while the daemon (aleph watch), the dashboard or Aleph Sessions poll your machines; the sessions you start show up too."
        />
      )}
      {nowShown.length ? (
        <List.Section title="Now" subtitle={`${nowShown.length}`}>
          {nowShown.map((it) => row(it, true))}
        </List.Section>
      ) : null}
      {groups.map((g) => (
        <List.Section key={g.key || "-"} title={g.title} subtitle={groupSubtitle(g.items)}>
          {g.items.map((it) => row(it, false))}
        </List.Section>
      ))}
    </List>
  );
}

/** `12 · 2 questions`: how many rows, and how many of them asked something. */
function groupSubtitle(items: TimelineItem[]): string {
  const asks = items.filter((it) => it.e.kind === "blocked").length;
  return asks ? `${items.length} · ${asks} question${asks === 1 ? "" : "s"}` : `${items.length}`;
}

interface View {
  group: GroupBy;
  sort: SortBy;
  setGroup: (g: GroupBy) => void;
  setSort: (s: SortBy) => void;
}

/** Group by ⌘G and Sort by ⌘⇧G, the current choice ticked. */
function ViewActions({ view }: { view: View }) {
  return (
    <ActionPanel.Section title="View">
      <ActionPanel.Submenu title="Group by" icon={Icon.AppWindowList} shortcut={{ modifiers: ["cmd"], key: "g" }}>
        {GROUPS.map((g) => (
          <Action
            key={g.id}
            title={g.title}
            icon={g.id === view.group ? Icon.CheckCircle : Icon.Circle}
            onAction={() => view.setGroup(g.id)}
          />
        ))}
      </ActionPanel.Submenu>
      <ActionPanel.Submenu title="Sort by" icon={Icon.ArrowDown} shortcut={{ modifiers: ["cmd", "shift"], key: "g" }}>
        {SORTS.map((x) => (
          <Action
            key={x.id}
            title={x.title}
            icon={x.id === view.sort ? Icon.CheckCircle : Icon.Circle}
            onAction={() => view.setSort(x.id)}
          />
        ))}
      </ActionPanel.Submenu>
    </ActionPanel.Section>
  );
}

const esc = (s: string) => s.replace(/[<>*_`#[\]]/g, (c) => `\\${c}`);

/** An event as Markdown: what happened, then the agent's own words when it said more than the title. */
function eventMarkdown(e: TimelineItem["e"], who: string, head: string, detail: string): string {
  const body = e.body?.trim() ?? "";
  return [
    `### ${who} ${head}`,
    // the reason shows the agent's words; keep the title above them
    body && e.title.trim() && body !== e.title.trim() && detail === body ? `**${esc(e.title.trim())}**` : "",
    detail ? esc(detail) : "",
    body && body !== detail ? `> ${esc(body).replace(/\n/g, "\n> ")}` : "",
    e.source === "agent" ? "_said by the agent itself (its hook)_" : "",
  ]
    .filter(Boolean)
    .join("\n\n");
}

/** What's true now for an event: still waiting, answered, back up, running, … */
function nowTag(
  it: TimelineItem,
  live: Session | undefined,
  host: Host | undefined,
  fleetKnown: boolean,
): { value: string; color: Color.ColorLike } | undefined {
  if (!fleetKnown) return undefined;
  const e = it.e;
  if (!e.name) {
    if (e.kind !== "error" || !host) return undefined;
    return host.reachable ? { value: "back up", color: DONE } : { value: "still down", color: DOWN };
  }
  if (!live)
    return it.launch || e.kind === "blocked" || e.kind === "new" ? { value: "ended", color: MUTED } : undefined;
  const st = stateOf(live);
  if (e.kind === "blocked")
    return st === "blocked" ? { value: "still waiting", color: NEEDS } : { value: "answered", color: DONE };
  if (e.kind === "limit")
    return st === "limit" ? { value: "still at limit", color: LIMIT } : { value: "running again", color: DONE };
  return { value: statusText(live), color: st === "blocked" ? NEEDS : MUTED };
}

function InboxItem({
  it,
  now,
  host,
  live,
  fleetKnown,
  note,
  recipe,
  detach,
  canMute,
  fromDaemon,
  refresh,
  showDetail,
  toggle,
  view,
  allIds,
}: {
  it: TimelineItem;
  /** a live item (the Now section) */
  now: boolean;
  host?: Host;
  live?: Session;
  fleetKnown: boolean;
  note?: Note;
  recipe?: Recipe;
  detach: boolean;
  canMute: boolean;
  /** the events came from the daemon: it can mark them read and dismiss them */
  fromDaemon: boolean;
  refresh: () => void;
  showDetail: boolean;
  toggle: () => void;
  view: View;
  allIds: string[];
}) {
  const e = it.e;
  const l = it.launch;
  const look = kindLook(e.kind);
  const { head, detail } =
    l && !it.logged ? { head: "was started by you", detail: shortPath(l.dir) } : eventReason(e as InboxEvent);
  const when = new Date(e.t * 1000);
  const who = e.name || e.host;
  const agentId = e.agent || l?.agent;
  const b = agentId ? brand(agentId) : undefined;
  const tag = now ? undefined : nowTag(it, live, host, fleetKnown);
  const session = {
    host: e.host,
    pane_id: live?.pane_id ?? "",
    name: e.name,
    display: live?.display ?? e.name,
    cwd: live?.cwd ?? l?.dir ?? "",
    title: live?.title ?? "",
    agent: live?.agent ?? agentId,
    status: live?.status,
    limit: live?.limit,
  };
  // open a running session by its pane (exact); fall back to its name while the fleet loads
  const canOpen = !!e.name && (!!live || !fleetKnown);
  const M = List.Item.Detail.Metadata;
  const text = [`${who} ${head}`, detail, `${e.host} · ${when.toLocaleString()}`].filter(Boolean).join("\n");
  const subtitle = l
    ? [shortPath(l.dir), l.prompt ? `“${l.prompt}”` : ""].filter(Boolean).join("  ·  ")
    : detail
      ? `${head} — ${detail}`
      : head;
  const startAgainAction = recipe ? (
    <Action
      key="again"
      title="Start Again"
      icon={Icon.Play}
      shortcut={{ modifiers: ["cmd", "shift"], key: "r" }}
      onAction={() => startAgain(recipe, detach)}
    />
  ) : null;

  return (
    <List.Item
      id={it.key}
      // who said it: the agent's logo (a machine's own events keep their mark); what happened is the first accessory
      icon={
        l && !it.logged
          ? { value: agentLogo(l.agent), tooltip: "you started it" }
          : agentId
            ? { value: agentLogo(agentId), tooltip: b?.name ?? agentId }
            : { value: look.icon, tooltip: look.word }
      }
      title={who}
      subtitle={showDetail ? undefined : subtitle}
      keywords={[
        e.host,
        e.kind,
        look.word,
        e.title,
        agentId ?? "",
        b?.name ?? "",
        head,
        l?.dir ?? "",
        l?.prompt ?? "",
        ...(l?.args ?? []),
        note?.text ?? "",
        ...(note?.tags ?? []).map((t) => `#${t}`),
      ].filter(Boolean)}
      accessories={
        showDetail
          ? [{ date: when }]
          : [
              ...(agentId && !(l && !it.logged) ? [{ icon: look.icon, tooltip: look.word }] : []),
              ...(it.n > 1
                ? [{ text: `×${it.n}`, tooltip: `${it.n} times since ${new Date(it.first * 1000).toLocaleString()}` }]
                : []),
              ...(e.unread ? [{ icon: { source: Icon.CircleFilled, tintColor: ACCENT }, tooltip: "new" }] : []),
              ...noteAccessory(note),
              ...(e.source === "agent"
                ? [{ icon: Icon.SpeechBubble, tooltip: "the agent said so itself (its hook)" }]
                : []),
              ...(l ? [{ icon: { source: Icon.Rocket, tintColor: ACCENT }, tooltip: "you started it" }] : []),
              ...(l?.worktree ? [{ icon: Icon.Tree, tooltip: "in a new git worktree" }] : []),
              ...(tag ? [{ tag }] : []),
              // the logo says which agent; the section may already say which machine
              ...(view.group !== "machine" ? [hostTag(e.host, host?.color)] : []),
              { date: when, tooltip: when.toLocaleString() },
            ]
      }
      detail={
        <List.Item.Detail
          markdown={eventMarkdown(e, who, head, detail)}
          metadata={
            <M>
              <M.TagList title="Event">
                <M.TagList.Item text={look.word} color={look.color} />
                {tag ? <M.TagList.Item text={tag.value} color={tag.color} /> : null}
              </M.TagList>
              <M.TagList title="Machine">
                <M.TagList.Item text={e.host} color={host?.color || Color.SecondaryText} />
              </M.TagList>
              {b ? <M.Label title="Agent" text={b.name} icon={agentLogo(agentId)} /> : null}
              {e.name ? <M.Label title="Session" text={e.name} /> : null}
              {live ? <M.Label title="Now" text={statusText(live)} /> : null}
              {note ? <M.Label title="Note" text={note.text} icon={Icon.Pencil} /> : null}
              {l ? (
                <>
                  <M.Separator />
                  <M.Label title="Started In" text={shortPath(l.dir) || "the default folder"} />
                  {l.prompt ? <M.Label title="First Prompt" text={l.prompt} /> : null}
                  {l.worktree ? <M.Label title="Worktree" text="a new git worktree" /> : null}
                  {l.args.length ? <M.Label title="Agent Arguments" text={l.args.join(" ")} /> : null}
                </>
              ) : null}
              <M.Separator />
              <M.Label
                title="When"
                text={
                  it.n > 1
                    ? `${it.n}× · ${new Date(it.first * 1000).toLocaleString()} – ${when.toLocaleString()}`
                    : when.toLocaleString()
                }
              />
            </M>
          }
        />
      }
      actions={
        <ActionPanel>
          <ActionPanel.Section title={who}>
            {canOpen ? (
              <OpenSessionActions
                t={{ host: e.host, sel: live ? selector(live) : `${e.host}:${e.name}`, label: live?.display || e.name }}
              />
            ) : (
              <OpenMachineActions host={e.host} />
            )}
            {live && stateOf(live) === "blocked" ? (
              <Action
                title="Approve"
                icon={Icon.Check}
                shortcut={{ modifiers: ["cmd"], key: "1" }}
                onAction={() => act(`Approving ${who}…`, () => approve(selector(live)), refresh)}
              />
            ) : null}
            {live ? <MessageActions s={session} refresh={refresh} /> : null}
            {startAgainAction}
            {recipe ? (
              <Action.Push
                title="Start Again with Changes"
                icon={Icon.Pencil}
                shortcut={Keyboard.Shortcut.Common.Edit}
                target={
                  <StartForm
                    initial={{
                      host: recipe.host,
                      dir: recipe.dir || undefined,
                      agent: recipe.agent || undefined,
                      prompt: recipe.prompt ?? undefined,
                      worktree: recipe.worktree,
                    }}
                  />
                }
              />
            ) : null}
            {e.name ? (
              <Action.Push
                title={note ? "Edit Note" : "Add Note"}
                icon={Icon.Pencil}
                shortcut={{ modifiers: ["cmd", "shift"], key: "n" }}
                target={<NoteForm s={session} note={note} refresh={refresh} />}
              />
            ) : null}
            {canMute && e.name && (live || !fleetKnown) ? <MuteActions host={e.host} name={e.name} /> : null}
          </ActionPanel.Section>
          {fromDaemon && it.logged && !now ? (
            <ActionPanel.Section title="Inbox">
              {e.unread ? (
                <Action
                  title="Mark Read up to Here"
                  icon={Icon.Eye}
                  shortcut={{ modifiers: ["cmd", "shift"], key: "u" }}
                  onAction={() => act("Marking read…", () => markRead(e.t), refresh, { title: "Marked read" })}
                />
              ) : null}
              <Action
                title="Mark All Read"
                icon={Icon.CheckList}
                shortcut={{ modifiers: ["cmd", "opt", "shift"], key: "u" }}
                onAction={() => act("Marking everything read…", () => markRead(), refresh, { title: "All read" })}
              />
              {allIds.length ? (
                <Action
                  title={`Dismiss All ${allIds.length}`}
                  icon={Icon.Trash}
                  style={Action.Style.Destructive}
                  shortcut={Keyboard.Shortcut.Common.RemoveAll}
                  onAction={async () => {
                    const ok = await confirmAlert({
                      title: `Dismiss ${allIds.length} messages?`,
                      message: "They leave the Inbox and the menu bar. Sessions that still need you stay in Now.",
                      primaryAction: { title: "Dismiss All", style: Alert.ActionStyle.Destructive },
                    });
                    if (ok)
                      await act("Dismissing all…", () => dismissEvents(allIds), refresh, { title: "Inbox cleared" });
                  }}
                />
              ) : null}
              {it.ids?.length ? (
                <Action
                  title={it.n > 1 ? `Dismiss These ${it.n}` : "Dismiss"}
                  icon={Icon.EyeDisabled}
                  style={Action.Style.Destructive}
                  shortcut={Keyboard.Shortcut.Common.Remove}
                  onAction={() =>
                    act("Dismissing…", () => dismissEvents(it.ids ?? []), refresh, { title: "Dismissed" })
                  }
                />
              ) : null}
            </ActionPanel.Section>
          ) : null}
          <ViewActions view={view} />
          <ActionPanel.Section>
            <Action
              title={showDetail ? "Hide Details" : "Show Details"}
              icon={Icon.Sidebar}
              shortcut={{ modifiers: ["cmd", "shift"], key: "d" }}
              onAction={toggle}
            />
            <CopyAction title="Copy Event" content={text} shortcut={Keyboard.Shortcut.Common.Copy} />
            {e.name ? (
              <CopyAction
                title="Copy Name"
                content={`${e.host}:${e.name}`}
                shortcut={Keyboard.Shortcut.Common.CopyName}
              />
            ) : (
              <CopyAction title="Copy Reason" content={explainError(e.title).detail || e.title} />
            )}
            {recipe ? <CopyAction title="Copy Command to Start It Again" content={againCommand(recipe)} /> : null}
            {recipe ? (
              <Action
                title="Start Again in Terminal"
                icon={Icon.Terminal}
                shortcut={{ modifiers: ["cmd", "shift"], key: "t" }}
                onAction={() => openInTerminal(againArgs(recipe), `Aleph · ${recipe.host}`)}
              />
            ) : null}
          </ActionPanel.Section>
          <ActionPanel.Section>
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
      }
    />
  );
}

// blank while showcase mode pauses every window
export default pausable(Inbox);
