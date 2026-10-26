// The menu bar: refreshed every 30 s, and at once whenever something opens
//   raycast://extensions/aleph/aleph/menu-bar?launchType=background
// (`open -g …`): the daemon does that the moment something happens when
// `notify.raycast` is on (`notify.raycast.deeplink` holds this link).
//
// * The title: ▲N when sessions need you, then ◐ or ○ when the link is weak
//   or down, and ⊘ during do not disturb. The aleph mark turns orange while
//   someone waits, blue while agents work, red offline.
// * Open Dashboard, then the sections:
//   1. Machines — `4 machines running` and the sessions by state, then each
//      machine: OS logo, name, a dot (🟢 online, 🟡 an agent working there,
//      🔴 offline), who needs you, its sessions and link; its submenu holds
//      its sessions — who needs you with Open · Attach · Approve · Reply ·
//      Queue · Mute, what's working and at its limit — then Open in Aleph.
//      Queued messages and a daemon that isn't running under them.
//   2. Usage — 7 days' cost, then each agent under a grey heading (its logo
//      and name): its 7 days, and a quota ring per account it's signed in to
//      (its plan, when it resets).
//   3. Inbox — the unread messages still worth a line (the kinds in
//      notify.raycast.kinds, from herdr, the agents' hooks or both), minus
//      what has since been put right; Mark Read · Dismiss · Mark All Read
//      with the daemon; Open Inbox.
//   4. Sessions (all, send a message, start one) and Notifications (do not
//      disturb, settings) as submenus.
// * A HUD when a session starts waiting or hits its limit (the last seen
//   sets live in LocalStorage, so a background refresh only announces
//   transitions).
import { Icon, Image, Keyboard, launchCommand, LaunchType, LocalStorage, MenuBarExtra, showHUD } from "@raycast/api";
import { copyText } from "./lib/copy";
import { getProgressIcon, useCachedPromise } from "@raycast/utils";
import { request } from "./lib/api";
import {
  approve,
  daemonStatus,
  dismissEvents,
  eventReason,
  Fleet,
  fleet,
  hasFlag,
  Host,
  inbox as readInbox,
  InboxEvent,
  limitReset,
  markRead,
  muteSession,
  queue,
  queueRuns,
  QueueTask,
  run,
  selector,
  Session,
  setDnd,
  setting,
  settingsList,
  shortPath,
  startHerdr,
  stateOf,
  statusText,
  usageQuery,
  watchInterval,
} from "./lib/cli";
import { usefulTitle } from "./lib/form-view";
import {
  menu as showcaseMenu,
  mode as showcaseMode,
  paused as showcasePaused,
  type Mode,
  type ShowcaseItem,
} from "./lib/showcase";
import { agentLogo, machineLogo } from "./lib/logos";
import { fmtLeft, planName, windowName } from "./lib/usage-view";
import {
  countsLine,
  agentUsage,
  dollars,
  headlineWindow,
  tokensText,
  inboxRows,
  inState,
  machineState,
  machineTitle,
  menuTitle,
  messageTitle,
  more,
  sessionLine,
  sessionTitle,
  stateCounts,
  tracksRead,
  usageFromNow,
  UsageNow,
} from "./lib/menu-view";
import { dndState, fleetLink, hostLink, parseKinds, TimelineItem } from "./lib/model";
import {
  attachInHerdr,
  openDashboard,
  openInAleph,
  openInTerminal,
  openMachine,
  openSession,
  openWith,
  Target,
} from "./lib/terminal";
import { ACCENT, ALEPH_ICON, brand, DONE, DOWN, kindLook, MARK, MUTED, NEEDS, WARN, WORKING } from "./lib/ui";

const SEEN_BLOCKED = "menubar.blocked";
const SEEN_LIMIT = "menubar.limit";
/** Inbox messages in the menu, newest first. */
const MESSAGES = 5;
/** Sessions at work, the latest first. */
const WORKING_MAX = 5;

/** Sessions in `state` that weren't last time; records the new set. `undefined` on the very first check. */
async function fresh(f: Fleet, key: string, state: string): Promise<{ h: string; s: Session }[] | undefined> {
  const now = f.hosts.flatMap((h) => h.sessions.filter((s) => stateOf(s) === state).map((s) => ({ h: h.alias, s })));
  const ids = now.map(({ s }) => selector(s));
  const stored = await LocalStorage.getItem<string>(key);
  let seen: string[] = [];
  try {
    seen = JSON.parse(stored ?? "[]");
  } catch {
    seen = [];
  }
  // machines we couldn't reach keep their old state: don't re-announce when they come back
  const unreachable = new Set(f.hosts.filter((h) => !h.reachable).map((h) => h.alias));
  const keep = seen.filter((id) => unreachable.has(id.split(":")[0]));
  await LocalStorage.setItem(key, JSON.stringify([...new Set([...ids, ...keep])]));
  // the very first check only records the state (like `aleph watch`)
  if (stored === undefined) return undefined;
  return now.filter(({ s }) => !seen.includes(selector(s)));
}

/** Announce sessions that started needing you (or hit their limit) since the last check. */
async function announce(f: Fleet) {
  const blocked = await fresh(f, SEEN_BLOCKED, "blocked");
  const limited = await fresh(f, SEEN_LIMIT, "limit");
  const more = (n: number) => (n > 1 ? ` (+${n - 1} more)` : "");
  if (blocked?.length) {
    const { h, s } = blocked[0];
    await showHUD(`▲ ${s.display || s.name} needs you on ${h}${more(blocked.length)}`);
  } else if (limited?.length) {
    const { h, s } = limited[0];
    const r = limitReset(s.limit);
    await showHUD(
      `◷ ${s.display || s.name} hit its usage limit on ${h}${r ? ` · resets ${r}` : ""}${more(limited.length)}`,
    );
  }
}

async function load(): Promise<Fleet> {
  const f = await fleet();
  try {
    await announce(f);
  } catch {
    // a failed HUD must never break the menu
  }
  return f;
}

/** What this aleph's `notify` can do (do not disturb, mutes): newer builds only. */
async function notifyFeatures(): Promise<{ dnd: boolean; mute: boolean }> {
  const [dnd, mute] = await Promise.all([hasFlag("notify", "dnd"), hasFlag("notify", "mute")]);
  return { dnd, mute };
}

const open = (name: string, context?: Record<string, unknown>) =>
  launchCommand({ name, type: LaunchType.UserInitiated, context });

/** A machine's icon: its OS logo. */
function machineIcon(h: Host): Image.ImageLike {
  // its OS logo at full size (its state is the circle that opens its title); a sign without one
  const logo = machineLogo(h.alias, h.os);
  if (logo) return logo.icon;
  return machineState(h) === "down"
    ? { source: Icon.WifiDisabled, tintColor: DOWN }
    : { source: Icon.Desktop, tintColor: MUTED };
}

/** `14:30`, or `Mon 14:30` when it isn't today. */
function clock(t: number): string {
  const d = new Date(t * 1000);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return d.toDateString() === new Date().toDateString()
    ? time
    : `${d.toLocaleDateString([], { weekday: "short" })} ${time}`;
}

/** The running session an inbox event is about, if it still runs. */
function liveSession(f: Fleet | undefined, host: string, name: string): Session | undefined {
  if (!name) return undefined;
  return f?.hosts
    .find((h) => h.alias === host)
    ?.sessions.find((s) => s.name === name || s.display === name || s.label === name);
}

export default function MenuBar() {
  // showcase mode is part of each cache key: what was cached unmasked never shows masked
  const sm = showcaseMode();
  const { data, isLoading, error, revalidate } = useCachedPromise((_m: Mode) => load(), [sm], {
    keepPreviousData: true,
  });
  const inbox = useCachedPromise((_m: Mode) => readInbox(80).catch(() => undefined), [sm], {
    keepPreviousData: true,
  });
  const q = useCachedPromise((_m: Mode) => queue().catch(() => undefined), [sm], { keepPreviousData: true });
  const settings = useCachedPromise((_m: Mode) => settingsList().catch(() => undefined), [sm], {
    keepPreviousData: true,
  });
  const features = useCachedPromise(notifyFeatures, [], { keepPreviousData: true });
  const status = useCachedPromise((_m: Mode) => daemonStatus(), [sm], { keepPreviousData: true });
  // today's spend and each agent's plan window: the daemon's minute-by-minute
  // summary (one call), else two queries of the analytics store
  const usage = useCachedPromise(
    async (_m: Mode) => {
      const now = await request<UsageNow>("usage.now", {}, { timeoutMs: 4000 }).catch(() => undefined);
      if (now?.total) return usageFromNow(now);
      const [week, today] = await Promise.all([
        usageQuery({ range: "7d", by: "agent" }),
        usageQuery({ range: "today", by: "agent" }),
      ]);
      const sum = (rows: { cost: number; tokens: number; estimated?: boolean }[] = []) => ({
        key: "",
        cost: rows.reduce((n, r) => n + r.cost, 0),
        tokens: rows.reduce((n, r) => n + r.tokens, 0),
        estimated: rows.some((r) => r.estimated && r.cost > 0),
      });
      return {
        perAgent: agentUsage(week.rows, today.rows, today.limits ?? week.limits),
        week: sum(week.rows),
        today: sum(today.rows),
        late: "",
      };
    },
    [sm],
    { keepPreviousData: true, onError: () => undefined },
  );
  // a demo build's switch (Showcase: masked / demo / off); none in other builds
  const showcase = showcaseMenu();
  const everything = () => {
    revalidate();
    inbox.revalidate();
    q.revalidate();
    settings.revalidate();
    status.revalidate();
    usage.revalidate();
  };

  const all = (data?.hosts ?? []).flatMap((h) => h.sessions);
  const counts = stateCounts(all);
  const blocked = counts.blocked ?? 0;
  const busy = inState(all, "working", WORKING_MAX);
  const queued = (q.data ?? []).filter((t: QueueTask) => t.state === "queued").length;
  const link = fleetLink(data?.hosts);
  const kinds = parseKinds(setting(settings.data, "notify.raycast.kinds"));
  const source = setting(settings.data, "notify.source") || "both";
  // unread only (none after Mark All Read), and only what still counts
  const rows = inboxRows(inbox.data ?? [], { kinds, source, hosts: data?.hosts });
  const messages = rows.slice(0, MESSAGES);
  // events that say what's read can be marked read and dismissed
  const fromDaemon = tracksRead(inbox.data ?? []);
  const unread = rows.length;
  const daemon = status.data ? queueRuns(status.data, watchInterval(settings.data)) : true;
  const dnd = dndState(setting(settings.data, "notify.dnd"));
  const canDnd = !!features.data?.dnd;
  const canMute = !!features.data?.mute;
  const summary = countsLine(counts);
  const perAgent = usage.data?.perAgent ?? [];
  const weekTotal = usage.data?.week ?? { key: "", cost: 0, tokens: 0 };
  const todayTotal = usage.data?.today ?? { key: "", cost: 0, tokens: 0 };
  const now = Date.now() / 1000;

  const icon: Image.ImageLike = {
    source: MARK,
    tintColor: link.state === "offline" ? DOWN : blocked ? NEEDS : busy.length ? WORKING : ACCENT,
  };

  const setDndTo = async (v: string, said: string) => {
    try {
      await setDnd(v);
      await showHUD(said);
      settings.revalidate();
    } catch (e) {
      await showHUD(`Couldn't change do not disturb: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  // showcase: while every window is paused, only the mark and "be right
  // back" (what's cached stays out of sight), and the way back
  if (showcasePaused()) {
    return (
      <MenuBarExtra icon={{ source: MARK, tintColor: ACCENT }} tooltip="Aleph — be right back">
        <MenuBarExtra.Item title="Be right back" icon={{ source: Icon.Stars, tintColor: ACCENT }} />
        {(showcase?.items ?? [])
          .filter((it) => it.args.includes("panic"))
          .map((it) => (
            <MenuBarExtra.Item
              key={it.title}
              title={it.title}
              icon={Icon.Play}
              onAction={() => switchShowcase(it, everything)}
            />
          ))}
      </MenuBarExtra>
    );
  }

  return (
    <MenuBarExtra
      icon={icon}
      title={menuTitle(blocked, link, dnd.on)}
      tooltip={data ? `Aleph — ${summary} · ${link.text}${dnd.on ? " · do not disturb" : ""}` : "Aleph"}
      isLoading={isLoading || inbox.isLoading}
    >
      {error && !data ? <MenuBarExtra.Item title={`Couldn't check: ${error.message}`} icon={Icon.Warning} /> : null}
      <MenuBarExtra.Section>
        <MenuBarExtra.Item
          title="Open Dashboard"
          icon={ALEPH_ICON}
          shortcut={{ modifiers: ["cmd"], key: "d" }}
          onAction={openDashboard}
        />
      </MenuBarExtra.Section>
      {/* Machines: each one (its dot, its OS logo, its session count), its sessions in its submenu */}
      <MenuBarExtra.Section title="Machines">
        {(data?.hosts ?? []).map((h) => {
          const order = (s: Session) => ["blocked", "limit", "working", "done", "idle"].indexOf(stateOf(s)) + 1 || 9;
          const sessions = [...h.sessions].sort((a, b) => order(a) - order(b));
          return (
            <MenuBarExtra.Submenu key={h.alias} icon={machineIcon(h)} title={machineTitle(h)}>
              {sessions.map((s) =>
                // who needs you: its own submenu with Approve, Reply, Queue, Mute; the rest open on click
                stateOf(s) === "blocked" ? (
                  <MenuBarExtra.Submenu key={selector(s)} icon={agentLogo(s.agent)} title={sessionTitle(s)}>
                    <SessionMenu s={s} canMute={canMute} />
                  </MenuBarExtra.Submenu>
                ) : stateOf(s) === "limit" ? (
                  <LiveItem
                    key={selector(s)}
                    s={s}
                    subtitle={[`at its limit`, limitReset(s.limit) ? `resets ${limitReset(s.limit)}` : ""]
                      .filter(Boolean)
                      .join(" · ")}
                  />
                ) : (
                  <LiveItem key={selector(s)} s={s} subtitle={statusText(s)} />
                ),
              )}
              {sessions.length ? <MenuBarExtra.Separator /> : null}
              {hostLink(h).state === "stopped" ? (
                <MenuBarExtra.Item
                  title="Start herdr"
                  subtitle="   its sessions come back too"
                  icon={{ source: Icon.Play, tintColor: WARN }}
                  onAction={async () => {
                    await showHUD(`Starting herdr on ${h.alias}…`);
                    try {
                      await showHUD((await startHerdr(h.alias)).message);
                    } catch (e) {
                      await showHUD(`herdr didn't start on ${h.alias}: ${e instanceof Error ? e.message : e}`);
                    }
                    revalidate();
                  }}
                />
              ) : null}
              {h.reachable ? (
                <MenuBarExtra.Item
                  title="New Terminal"
                  subtitle="   no agent, for commands"
                  icon={Icon.Terminal}
                  onAction={() => openInTerminal(["--host", h.alias, "shell"], `Terminal · ${h.alias}`)}
                />
              ) : null}
              <MenuBarExtra.Item
                title={`Open ${h.alias} in Aleph`}
                icon={ALEPH_ICON}
                onAction={() => openMachine(h.alias)}
              />
              <MenuBarExtra.Item title="Show in Machines" icon={Icon.Network} onAction={() => open("machines")} />
            </MenuBarExtra.Submenu>
          );
        })}
        {queued > 0 ? (
          <MenuBarExtra.Item
            title={`${queued} queued message${queued === 1 ? "" : "s"}`}
            subtitle={daemon ? undefined : "   waiting for the daemon"}
            icon={{ source: Icon.Tray, tintColor: daemon ? WORKING : WARN }}
            onAction={() => open("queue")}
          />
        ) : null}
        {status.data && !status.data.running && status.data.from !== "legacy" ? (
          <MenuBarExtra.Item
            title="Daemon isn't running"
            tooltip="Queued messages wait and nothing notifies you until it runs"
            icon={{ source: Icon.Warning, tintColor: WARN }}
            onAction={() => open("daemon")}
          />
        ) : null}
      </MenuBarExtra.Section>
      {/* 1. Now: the sessions — who needs you, what's working, what's at its limit — and what waits */}
      {perAgent.length ? (
        <MenuBarExtra.Section title="Weekly Usage">
          <MenuBarExtra.Item
            icon={{ source: Icon.BarChart, tintColor: ACCENT }}
            title={dollars(weekTotal)}
            subtitle={`   today ${dollars(todayTotal)} · ${tokensText(weekTotal.tokens)}${
              usage.data?.late ? ` · ${usage.data.late}` : ""
            }`}
            tooltip="Open Usage · * part of the cost from Aleph's price table"
            onAction={() => open("usage")}
          />
          {perAgent.flatMap((u) => {
            const b = brand(u.agent);
            // the ring's colour by how full: green, orange from 70 %, red from 90 % (Aleph's state colours)
            const tone = (pct: number) => (pct >= 90 ? "#E5484D" : pct >= 70 ? "#F76B15" : "#46A758");
            const items = [
              // the agent's grey heading (no action): its logo, name and week (the section says it's the week)
              <MenuBarExtra.Item
                key={`${u.agent}-head`}
                icon={agentLogo(u.agent)}
                title={`${b.name}   ${dollars(u.week)}`}
                subtitle={`   ${tokensText(u.week?.tokens ?? 0)}${u.today?.cost ? ` · today ${dollars(u.today)}` : ""}`}
                tooltip={`${b.name} this week`}
              />,
            ];
            // one line per account it's signed in to: its plan, how full its week is
            for (const q of u.quotas) {
              const w = headlineWindow(q.windows);
              if (!w) continue;
              const others = q.windows
                .filter((x) => x !== w)
                .map((x) => `${windowName(x.minutes)} ${Math.round(x.pct)}%`);
              const plan = q.plan ? planName(q.plan, u.agent) : "";
              items.push(
                <MenuBarExtra.Item
                  key={`${u.agent}-${q.hosts.join(",")}`}
                  icon={getProgressIcon(Math.min(1, w.pct / 100), tone(w.pct))}
                  title={`${Math.round(w.pct)}% used${plan ? ` · ${plan}` : ""}${
                    w.minutes === 10080 ? "" : ` (${windowName(w.minutes)})`
                  }`}
                  subtitle={[
                    w.resets_at && w.resets_at > now ? `   resets in ${fmtLeft(w.resets_at - now)}` : "",
                    ...others,
                    u.quotas.length > 1 ? q.hosts.join(", ") : "",
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                  tooltip={q.windows
                    .map(
                      (x) =>
                        `${windowName(x.minutes)} ${Math.round(x.pct)}%${
                          x.resets_at && x.resets_at > now ? ` · resets in ${fmtLeft(x.resets_at - now)}` : ""
                        }`,
                    )
                    .concat(`on ${q.hosts.join(", ")}`)
                    .join("\n")}
                  onAction={() => open("usage", { agent: u.agent })}
                />,
              );
            }
            return items;
          })}
        </MenuBarExtra.Section>
      ) : null}
      <MenuBarExtra.Section title={unread ? `Inbox · ${unread} new` : "Inbox"}>
        {messages.map((it) => (
          <MessageMenu
            key={it.key}
            it={it}
            live={liveSession(data, it.e.host, it.e.name)}
            known={!!data}
            canMute={canMute}
            fromDaemon={fromDaemon}
            changed={inbox.revalidate}
          />
        ))}
        {unread > messages.length ? (
          <MenuBarExtra.Item title={more(unread, messages.length)} onAction={() => open("inbox")} />
        ) : null}
        {fromDaemon && unread ? (
          <MenuBarExtra.Item
            title="Mark All Read"
            icon={Icon.CheckList}
            onAction={() => readFromMenu(undefined, "Inbox marked read", inbox.revalidate)}
          />
        ) : null}
        <MenuBarExtra.Item
          title="Open Inbox…"
          icon={Icon.Tray}
          shortcut={{ modifiers: ["cmd"], key: "i" }}
          onAction={() => open("inbox")}
        />
      </MenuBarExtra.Section>
      <MenuBarExtra.Section>
        <MenuBarExtra.Submenu icon={Icon.List} title="Sessions">
          <MenuBarExtra.Item
            title="All Sessions…"
            icon={Icon.List}
            shortcut={{ modifiers: ["cmd"], key: "l" }}
            onAction={() => open("sessions")}
          />
          <MenuBarExtra.Item
            title="Send Message…"
            icon={Icon.Message}
            shortcut={{ modifiers: ["cmd"], key: "m" }}
            onAction={() => open("send-message")}
          />
          <MenuBarExtra.Item
            title="Start Session…"
            icon={Icon.Plus}
            shortcut={Keyboard.Shortcut.Common.New}
            onAction={() => open("start-session")}
          />
          <MenuBarExtra.Submenu icon={Icon.Terminal} title="New Terminal">
            {(data?.hosts ?? [])
              .filter((h) => h.reachable)
              .map((h) => (
                <MenuBarExtra.Item
                  key={h.alias}
                  title={h.alias}
                  icon={machineIcon(h)}
                  onAction={() => openInTerminal(["--host", h.alias, "shell"], `Terminal · ${h.alias}`)}
                />
              ))}
          </MenuBarExtra.Submenu>
        </MenuBarExtra.Submenu>
        <MenuBarExtra.Submenu
          icon={dnd.on ? { source: Icon.BellDisabled, tintColor: WARN } : Icon.Bell}
          title={dnd.on ? `Notifications  ·  quiet${dnd.until ? ` until ${clock(dnd.until)}` : ""}` : "Notifications"}
        >
          {canDnd ? (
            <MenuBarExtra.Section title={`Do Not Disturb  ·  ${dnd.on ? "on" : "off"}`}>
              {dnd.on ? (
                <MenuBarExtra.Item
                  title="Turn Off"
                  icon={Icon.Bell}
                  onAction={() => setDndTo("off", "Notifications are back on")}
                />
              ) : null}
              <MenuBarExtra.Item
                title="For 30 Minutes"
                onAction={() => setDndTo("30m", "Do not disturb for 30 minutes")}
              />
              <MenuBarExtra.Item title="For 1 Hour" onAction={() => setDndTo("1h", "Do not disturb for an hour")} />
              <MenuBarExtra.Item title="For 2 Hours" onAction={() => setDndTo("2h", "Do not disturb for 2 hours")} />
              <MenuBarExtra.Item
                title="Until I Turn It Off"
                onAction={() => setDndTo("on", "Do not disturb until you turn it off")}
              />
              <MenuBarExtra.Item
                title={`${setting(settings.data, "notify.quiet_pass") || "error"} still comes through`}
                icon={{ source: Icon.Info, tintColor: MUTED }}
              />
            </MenuBarExtra.Section>
          ) : null}
          <MenuBarExtra.Section>
            <MenuBarExtra.Item
              title="Notification Settings…"
              icon={Icon.Gear}
              onAction={() => open("notification-settings")}
            />
          </MenuBarExtra.Section>
        </MenuBarExtra.Submenu>
        {showcase ? (
          <MenuBarExtra.Submenu icon={{ source: Icon.Stars, tintColor: ACCENT }} title={showcase.title}>
            {showcase.items.map((it) => (
              <MenuBarExtra.Item
                key={it.title}
                title={it.title}
                subtitle={`   ${it.subtitle}`}
                icon={it.current ? { source: Icon.Checkmark, tintColor: ACCENT } : Icon.Circle}
                onAction={() => switchShowcase(it, everything)}
              />
            ))}
          </MenuBarExtra.Submenu>
        ) : null}
      </MenuBarExtra.Section>
    </MenuBarExtra>
  );
}

const targetOf = (s: Session): Target => ({ host: s.host, sel: selector(s), label: s.display || s.name });

async function approveFromMenu(s: Session) {
  const name = s.display || s.name;
  try {
    await approve(selector(s));
    await showHUD(`Approved ${name} on ${s.host}`);
  } catch (e) {
    await showHUD(`Couldn't approve ${name}: ${e instanceof Error ? e.message : String(e)}`);
  }
}

/** `aleph showcase on [--demo] | off`, then everything asked again (under its new cache keys). */
async function switchShowcase(it: ShowcaseItem, changed: () => void) {
  try {
    await run(it.args, 15_000);
    await showHUD(it.hud);
    changed();
  } catch (e) {
    await showHUD(`Couldn't switch: ${e instanceof Error ? e.message : String(e)}`);
  }
}

/** Mark the inbox read up to `t` (everything: now), through the daemon. */
async function readFromMenu(t: number | undefined, said: string, changed: () => void) {
  try {
    await markRead(t);
    await showHUD(said);
    changed();
  } catch (e) {
    await showHUD(`Couldn't mark it read: ${e instanceof Error ? e.message : String(e)}`);
  }
}

async function dismissFromMenu(ids: string[], changed: () => void) {
  try {
    await dismissEvents(ids);
    await showHUD("Dismissed");
    changed();
  } catch (e) {
    await showHUD(`Couldn't dismiss it: ${e instanceof Error ? e.message : String(e)}`);
  }
}

async function muteFromMenu(host: string, name: string) {
  try {
    await muteSession(host, name, "1h");
    await showHUD(`${name} on ${host} is muted for an hour`);
  } catch (e) {
    await showHUD(`Couldn't mute ${name}: ${e instanceof Error ? e.message : String(e)}`);
  }
}

/** Open in Aleph and Attach in herdr, the preferred one first. */
function OpenItems({ t }: { t: Target }) {
  const aleph = (
    <MenuBarExtra.Item key="aleph" title="Open in Aleph" icon={ALEPH_ICON} onAction={() => openInAleph(t)} />
  );
  const herdr = (
    <MenuBarExtra.Item key="herdr" title="Attach in herdr" icon={Icon.Terminal} onAction={() => attachInHerdr(t)} />
  );
  return <>{openWith() === "herdr" ? [herdr, aleph] : [aleph, herdr]}</>;
}

/** Approve (while it asks) / Reply / Queue on a running session (and Mute, on builds that have it). */
function MessageItems({ s, canMute }: { s: Session; canMute: boolean }) {
  const sel = selector(s);
  return (
    <>
      {s.status === "blocked" ? (
        <MenuBarExtra.Item
          title="Approve"
          icon={{ source: Icon.Check, tintColor: DONE }}
          onAction={() => approveFromMenu(s)}
        />
      ) : null}
      <MenuBarExtra.Item
        title="Reply…"
        icon={Icon.Message}
        onAction={() => open("send-message", { sel, when: "now" })}
      />
      <MenuBarExtra.Item
        title="Queue for When It's Free…"
        icon={Icon.Tray}
        onAction={() => open("send-message", { sel, when: "idle" })}
      />
      {canMute && s.name ? (
        <MenuBarExtra.Item
          title="Mute for an Hour"
          icon={Icon.BellDisabled}
          onAction={() => muteFromMenu(s.host, s.name)}
        />
      ) : null}
    </>
  );
}

/** A session that needs you: what it's on (when its title says), and every way to answer. */
function SessionMenu({ s, canMute }: { s: Session; canMute: boolean }) {
  const doing = usefulTitle(s.title, brand(s.agent).name);
  return (
    <>
      {doing ? <MenuBarExtra.Item title={clip(doing, 70)} tooltip={doing} /> : null}
      <OpenItems t={targetOf(s)} />
      <MessageItems s={s} canMute={canMute} />
    </>
  );
}

/** A working (or limited) session: click opens it, ⌥-click queues a message for when it's free. */
function LiveItem({ s, subtitle }: { s: Session; subtitle?: string }) {
  const agentName = brand(s.agent).name;
  const l = sessionLine(s, undefined, agentName);
  const task = usefulTitle(s.title, agentName);
  const about = [task, s.cwd && s.cwd !== "-" ? shortPath(s.cwd) : ""].filter(Boolean).join(" · ");
  // where and for how long, then what it's doing when herdr's title says
  const said = [subtitle ?? l.subtitle, task ? clip(task, 40) : ""].filter(Boolean).join(" · ");
  return (
    <MenuBarExtra.Item
      icon={agentLogo(s.agent)}
      title={l.title}
      subtitle={`   ${said}`}
      tooltip={`${about ? `${about} — ` : ""}click to open, ⌥-click to queue a message`}
      onAction={() => openSession(targetOf(s))}
      alternate={
        <MenuBarExtra.Item
          icon={Icon.Tray}
          title={`Queue for ${l.title}…`}
          subtitle={`   ${s.host}`}
          onAction={() => open("send-message", { sel: selector(s), when: "idle" })}
        />
      }
    />
  );
}

const clip = (s: string, n = 90) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

/** One Inbox message: what happened (in the agent's words when it said more), and what you can do about it. */
function MessageMenu({
  it,
  live,
  known,
  canMute,
  fromDaemon,
  changed,
}: {
  it: TimelineItem;
  live?: Session;
  known: boolean;
  canMute: boolean;
  /** the daemon answered: Mark Read and Dismiss work */
  fromDaemon: boolean;
  changed: () => void;
}) {
  const e = it.e;
  const look = kindLook(e.kind);
  const { head, detail } = eventReason(e as InboxEvent);
  const who = e.name || e.host;
  const sel = live ? selector(live) : `${e.host}:${e.name}`;
  const text = [`${who} ${head}`, detail, `${e.host} · ${new Date(e.t * 1000).toLocaleString()}`]
    .filter(Boolean)
    .join("\n");
  const said = usefulTitle(detail, e.agent ? brand(e.agent).name : undefined);
  return (
    <MenuBarExtra.Submenu icon={look.icon} title={messageTitle(it, look.word)}>
      {said ? <MenuBarExtra.Item title={clip(said)} tooltip={detail} /> : null}
      {e.source === "agent" ? (
        <MenuBarExtra.Item title="Said by the agent" icon={{ source: Icon.SpeechBubble, tintColor: MUTED }} />
      ) : null}
      {e.name && (live || !known) ? (
        <OpenItems t={{ host: e.host, sel, label: e.name }} />
      ) : (
        <MenuBarExtra.Item
          title={e.name ? `Open ${e.host} in Aleph (it ended)` : `Open ${e.host} in Aleph`}
          icon={ALEPH_ICON}
          onAction={() => openMachine(e.host)}
        />
      )}
      {live ? <MessageItems s={live} canMute={canMute} /> : null}
      <MenuBarExtra.Separator />
      {fromDaemon && e.unread ? (
        <MenuBarExtra.Item
          title="Mark Read"
          tooltip="This message and the older ones"
          icon={Icon.Eye}
          onAction={() => readFromMenu(e.t, "Marked read", changed)}
        />
      ) : null}
      {fromDaemon && it.ids?.length ? (
        <MenuBarExtra.Item
          title={it.n > 1 ? `Dismiss All ${it.n}` : "Dismiss"}
          icon={Icon.EyeDisabled}
          onAction={() => dismissFromMenu(it.ids ?? [], changed)}
        />
      ) : null}
      <MenuBarExtra.Item
        title="Show in Inbox"
        icon={Icon.Tray}
        onAction={() => open("inbox", { host: e.host, name: e.name, kind: e.kind, t: e.t })}
      />
      <MenuBarExtra.Item
        title="Copy"
        icon={Icon.CopyClipboard}
        onAction={async () => {
          if (!(await copyText(text, true))) return;
          await showHUD("Copied");
        }}
      />
    </MenuBarExtra.Submenu>
  );
}
