// Agents: each coding agent (Claude Code, Codex, OpenCode, …) across the
// fleet — the machines it's on and their versions, its sessions open now,
// its tokens and cost today and over 30 days, its plan limits — and what each
// machine is missing, with what installing it takes. `agents --json --local`,
// the fleet (`ls --json`, every 30 s) and `usage --by agent` (the daemon's
// `usage.query` first). The detail is SVG cards (agents-view.ts). Installing
// always asks first (an Alert with the exact command), then streams.
import {
  Action,
  ActionPanel,
  environment,
  Icon,
  Image,
  Keyboard,
  launchCommand,
  LaunchType,
  List,
  useNavigation,
} from "@raycast/api";
import { pausable } from "./lib/pause";
import { CopyAction } from "./lib/copy";
import { useCachedPromise } from "@raycast/utils";
import { useMemo, useState } from "react";
import { act } from "./lib/act";
import {
  AgentInfo,
  AgentsHost,
  agentsReport,
  explainError,
  statusText,
  UsageReport,
  usageQuery,
  UsageRow,
} from "./lib/cli";
import { chartTheme } from "./lib/charts";
import { notice } from "./lib/cards";
import {
  AgentBadge,
  agentBadges,
  agentMarkdown,
  agentsSummary,
  AgentGroup,
  groupAgents,
  groupIssues,
  missingMarkdown,
  missingSummary,
  missingTag,
  OpenNow,
  openNow,
  usageOf,
} from "./lib/agents-view";
import { DoctorDetail } from "./lib/doctor";
import { useAgentsReport, useFleet } from "./lib/hooks";
import { InstallActions, InstallView } from "./lib/install";
import { confirmInstall } from "./lib/install-flow";
import { agentLogo, cardLogos, osLogo } from "./lib/logos";
import { mode as showcaseMode, navTitle, type Mode } from "./lib/showcase";
import { img } from "./lib/stats-view";
import { StartForm } from "./lib/start-form";
import { openInTerminal, openMachine } from "./lib/terminal";
import { ALEPH_ICON, DOWN, IDLE, MUTED, NEEDS, WARN, WORKING } from "./lib/ui";

type Show = "installed" | "missing" | "all";

function Agents() {
  return <AgentsList />;
}

/** One range of usage by agent (every machine, or `host`'s). Quiet when it fails: usage is extra here. */
function useUsage(range: string, host?: string) {
  // showcase mode is part of the cache key: what was cached unmasked never shows masked
  return useCachedPromise(
    (r: string, h: string | undefined, _m: Mode) => usageQuery({ range: r, by: "agent", ...(h ? { host: h } : {}) }),
    [range, host, showcaseMode()],
    { keepPreviousData: true, onError: () => undefined },
  );
}

/** Every agent across the fleet, or one machine's (`host`, from Machines). */
export function AgentsList({ host }: { host?: string }) {
  const agents = useAgentsReport(host, { failureTitle: "Couldn't look at the machines" }, host ? [] : ["--local"]);
  const fleet = useFleet({ pollMs: 30_000 });
  const today = useUsage("today", host);
  const month = useUsage("30d", host);
  const [show, setShow] = useState<Show>("installed");
  const [showDetail, setShowDetail] = useState(true);
  const dark = environment.appearance !== "light";
  const data = agents.data;
  const unsupported = !agents.isLoading && !agents.errorInfo && data === undefined;
  const lookAgain = () =>
    act(
      `Looking at ${host ?? "every machine"} again…`,
      () => agents.mutate(agentsReport(host, true, !host)),
      () => {
        fleet.revalidate();
        today.revalidate();
        month.revalidate();
      },
      { title: "Agents are up to date" },
    );

  const all = useMemo(() => groupAgents(data, true), [data]);
  const monthRows = month.data?.rows;
  const groups = useMemo(() => {
    const cost = (g: AgentGroup) => usageOf(monthRows, [g.id, g.kind])?.cost ?? 0;
    // the ones used most first (30 days changes slowly, so rows don't jump)
    return all.filter((g) => g.installs.length).sort((a, b) => cost(b) - cost(a));
  }, [all, monthRows]);
  const hosts = fleet.data?.hosts;
  const open = useMemo(
    () => new Map(groups.map((g) => [g.id, openNow(hosts, [g.id, g.kind], host)])),
    [groups, hosts, host],
  );
  const openTotal = [...open.values()].reduce((s, o) => s + o.total, 0);
  const offline = (data?.hosts ?? []).filter((h) => !h.reachable);
  const usage = { today: today.data, month: month.data };

  return (
    <List
      isLoading={agents.isLoading}
      isShowingDetail={showDetail && !!data?.hosts.length}
      navigationTitle={navTitle(
        [host ? `Agents on ${host}` : "Agents", agents.stale ? "cached" : ""].filter(Boolean).join(" · "),
      )}
      searchBarPlaceholder="Filter by agent or machine"
      searchBarAccessory={
        <List.Dropdown tooltip="Show" value={show} onChange={(v) => setShow(v as Show)}>
          <List.Dropdown.Item title="Installed" value="installed" icon={Icon.CheckCircle} />
          <List.Dropdown.Item title="Not Installed" value="missing" icon={Icon.Download} />
          <List.Dropdown.Item title="All Agents" value="all" icon={Icon.List} />
        </List.Dropdown>
      }
    >
      {unsupported ? (
        <List.EmptyView icon={Icon.Info} title="This aleph can't list agents yet" description="aleph self-update" />
      ) : agents.errorInfo && !data ? (
        <List.EmptyView icon={Icon.Warning} title={agents.errorInfo.title} description={agents.errorInfo.message} />
      ) : (
        <List.EmptyView icon={Icon.Person} title={agents.isLoading ? "Looking at your machines…" : "No agents found"} />
      )}
      {show !== "missing" ? (
        <List.Section
          title={host ? `On ${host}` : "Installed"}
          subtitle={agentsSummary(groups, openTotal, today.data?.rows)}
        >
          {groups.map((g) => (
            <AgentItem
              key={g.id}
              g={g}
              open={open.get(g.id)!}
              usage={usage}
              only={host}
              dark={dark}
              lookAgain={lookAgain}
              showDetail={showDetail}
              toggle={() => setShowDetail((x) => !x)}
            />
          ))}
        </List.Section>
      ) : null}
      {show !== "missing" && offline.length ? (
        <List.Section title="Offline" subtitle={`${offline.length}`}>
          {offline.map((h) => (
            <OfflineItem key={h.alias} h={h} dark={dark} lookAgain={lookAgain} />
          ))}
        </List.Section>
      ) : null}
      {show !== "installed"
        ? (data?.hosts ?? [])
            .filter((h) => h.reachable)
            .map((h) => (
              <List.Section key={h.alias} title={`Not on ${h.alias}`} subtitle={missingSummary(h)}>
                {sortMissing(h, all).map((a) => (
                  <MissingItem
                    key={`${h.alias}-${a.id}`}
                    h={h}
                    a={a}
                    g={all.find((g) => g.id === a.id)}
                    dark={dark}
                    lookAgain={lookAgain}
                    showDetail={showDetail}
                    toggle={() => setShowDetail((x) => !x)}
                  />
                ))}
              </List.Section>
            ))
        : null}
    </List>
  );
}

/** A machine's missing agents: the ones another machine has first, then those it can install, then the rest. */
function sortMissing(h: AgentsHost, all: AgentGroup[]): AgentInfo[] {
  const rank = (a: AgentInfo) => {
    const elsewhere = (all.find((g) => g.id === a.id)?.installs.length ?? 0) > 0;
    const tone = missingTag(a).tone;
    return (elsewhere ? 0 : 3) + (tone === "ready" ? 0 : tone === "blocked" ? 1 : 2);
  };
  return h.agents.filter((a) => !a.installed).sort((a, b) => rank(a) - rank(b));
}

/** An agent's badge as Raycast's accessory. */
function accessory(b: AgentBadge, g: AgentGroup): List.Item.Accessory {
  switch (b.kind) {
    case "warn":
      return { icon: { source: Icon.Warning, tintColor: WARN }, tooltip: b.tooltip };
    case "needs":
      return { tag: { value: `▲ ${b.text}`, color: NEEDS }, tooltip: b.tooltip };
    case "working":
      return { icon: { source: Icon.CircleProgress50, tintColor: WORKING }, text: b.text, tooltip: b.tooltip };
    case "open":
      return { icon: { source: Icon.CircleFilled, tintColor: IDLE }, text: b.text, tooltip: b.tooltip };
    case "cost":
      return { text: b.text, tooltip: b.tooltip };
    case "version":
      return { tag: { value: b.text, color: g.color }, tooltip: b.tooltip };
    case "machines":
      return { icon: Icon.Monitor, text: b.text, tooltip: b.tooltip };
  }
}

function AgentItem({
  g,
  open,
  usage,
  only,
  dark,
  lookAgain,
  showDetail,
  toggle,
}: {
  g: AgentGroup;
  open: OpenNow;
  usage: { today?: UsageReport; month?: UsageReport };
  only?: string;
  dark: boolean;
  lookAgain: () => void;
  showDetail: boolean;
  toggle: () => void;
}) {
  const ids = [g.id, g.kind];
  const today: UsageRow | undefined = usageOf(usage.today?.rows, ids);
  const month: UsageRow | undefined = usageOf(usage.month?.rows, ids);
  const issues = groupIssues(g);
  const toSetUp = g.installs.filter((i) => i.a.integration && i.a.integration_state !== "current");
  const withPath = g.installs.filter((i) => i.a.path);
  // start where it already runs, else on the first machine that has it
  const startOn = only ?? open.sessions[0]?.host ?? g.installs[0]?.host.alias;

  return (
    <List.Item
      id={g.id}
      icon={{ value: agentLogo(g.id), tooltip: g.name }}
      title={g.name}
      keywords={[g.id, g.kind ?? "", g.info.command, ...g.installs.map((i) => i.host.alias)]}
      accessories={agentBadges(g, { open, today, month, wide: !showDetail }).map((b) => accessory(b, g))}
      detail={
        <List.Item.Detail
          markdown={agentMarkdown({
            g,
            open,
            today,
            month,
            usage: !!(usage.today || usage.month),
            limits: usage.today?.limits,
            t: chartTheme(dark),
            logo: cardLogos(dark),
            statusText,
          })}
        />
      }
      actions={
        <ActionPanel>
          <ActionPanel.Section title={g.name}>
            <Action.Push
              title="Start Session"
              icon={agentLogo(g.id)}
              target={<StartForm initial={{ host: startOn, agent: g.id }} />}
            />
            {open.total ? (
              <Action
                title="Show Sessions"
                icon={Icon.List}
                shortcut={{ modifiers: ["cmd", "opt"], key: "s" }}
                onAction={() =>
                  launchCommand({
                    name: "sessions",
                    type: LaunchType.UserInitiated,
                    context: only ? { host: only } : {},
                  })
                }
              />
            ) : null}
            <Action
              title="Show Usage"
              icon={Icon.BarChart}
              shortcut={{ modifiers: ["cmd", "opt"], key: "u" }}
              onAction={() =>
                launchCommand({ name: "usage", type: LaunchType.UserInitiated, context: { agent: g.id } })
              }
            />
          </ActionPanel.Section>
          <ActionPanel.Section title="Machines">
            {g.missing.length ? <InstallOn g={g} lookAgain={lookAgain} /> : null}
            {toSetUp.map((i) => (
              <Action
                key={i.host.alias}
                title={`Set up Integration on ${i.host.alias}`}
                icon={{ source: Icon.Link, tintColor: issues.length ? WARN : undefined }}
                onAction={() =>
                  openInTerminal(["--host", i.host.alias, "agents", "setup", g.id], `herdr integration · ${g.name}`)
                }
              />
            ))}
            {withPath.length === 1 ? (
              <CopyAction
                title="Copy Path"
                content={withPath[0].a.path!}
                shortcut={Keyboard.Shortcut.Common.CopyPath}
              />
            ) : withPath.length > 1 ? (
              <ActionPanel.Submenu title="Copy Path" icon={Icon.Clipboard} shortcut={Keyboard.Shortcut.Common.CopyPath}>
                {withPath.map((i) => (
                  <CopyAction
                    key={i.host.alias}
                    title={`${i.host.alias}  ${i.a.path}`}
                    content={i.a.path!}
                    icon={osLogo(i.host.os_name, i.host.os)?.icon}
                  />
                ))}
              </ActionPanel.Submenu>
            ) : null}
            {g.newest ? (
              <CopyAction
                title="Copy Version"
                content={`${g.name} ${g.newest}`}
                shortcut={Keyboard.Shortcut.Common.Copy}
              />
            ) : null}
          </ActionPanel.Section>
          <ActionPanel.Section>
            <Action
              title={showDetail ? "Hide Details" : "Show Details"}
              icon={Icon.Sidebar}
              shortcut={{ modifiers: ["cmd", "shift"], key: "d" }}
              onAction={toggle}
            />
            <Action
              title="Look Again"
              icon={Icon.ArrowClockwise}
              shortcut={Keyboard.Shortcut.Common.Refresh}
              onAction={lookAgain}
            />
          </ActionPanel.Section>
        </ActionPanel>
      }
    />
  );
}

/** Install an agent on a machine that doesn't have it: asks first (the exact command), then streams. */
function InstallOn({ g, lookAgain }: { g: AgentGroup; lookAgain: () => void }) {
  const { push } = useNavigation();
  return (
    <ActionPanel.Submenu title="Install on…" icon={Icon.Download} shortcut={{ modifiers: ["cmd", "shift"], key: "i" }}>
      {g.missing.map((i) => {
        const tag = missingTag(i.a);
        return (
          <Action
            key={i.host.alias}
            title={`${i.host.alias} · ${tag.text}`}
            icon={osLogo(i.host.os_name, i.host.os)?.icon ?? Icon.Monitor}
            onAction={async () => {
              if (await confirmInstall(i.a, i.host.alias))
                push(<InstallView a={i.a} host={i.host.alias} onDone={(ok) => ok && lookAgain()} />);
            }}
          />
        );
      })}
    </ActionPanel.Submenu>
  );
}

/** A missing agent's tag colour: blue it can install now, yellow something's missing first, grey by hand. */
const toneColor = (tone: "ready" | "blocked" | "manual") =>
  tone === "ready" ? WORKING : tone === "blocked" ? WARN : MUTED;

function MissingItem({
  h,
  a,
  g,
  dark,
  lookAgain,
  showDetail,
  toggle,
}: {
  h: AgentsHost;
  a: AgentInfo;
  g?: AgentGroup;
  dark: boolean;
  lookAgain: () => void;
  showDetail: boolean;
  toggle: () => void;
}) {
  const tag = missingTag(a);
  const elsewhere = g?.installs.map((i) => i.host.alias) ?? [];
  const accessories: List.Item.Accessory[] = [
    ...(elsewhere.length
      ? [{ icon: { source: Icon.CheckCircle, tintColor: MUTED }, tooltip: `installed on ${elsewhere.join(", ")}` }]
      : []),
    { tag: { value: tag.text, color: toneColor(tag.tone) }, tooltip: tag.tooltip || undefined },
  ];
  return (
    <List.Item
      id={`${h.alias}-${a.id}`}
      icon={{ value: agentLogo(a.id), tooltip: a.name }}
      title={a.name}
      keywords={[a.id, a.command, h.alias, a.kind ?? "", a.install_method ?? ""]}
      accessories={accessories}
      detail={<List.Item.Detail markdown={missingMarkdown({ h, a, g, t: chartTheme(dark), logo: cardLogos(dark) })} />}
      actions={
        <ActionPanel>
          <ActionPanel.Section title={`${a.name} on ${h.alias}`}>
            <InstallActions a={a} host={h.alias} onDone={(ok) => ok && lookAgain()} />
          </ActionPanel.Section>
          <ActionPanel.Section>
            <Action
              title={`Open ${h.alias} in Aleph`}
              icon={ALEPH_ICON}
              shortcut={Keyboard.Shortcut.Common.Open}
              onAction={() => openMachine(h.alias)}
            />
            <Action
              title={showDetail ? "Hide Details" : "Show Details"}
              icon={Icon.Sidebar}
              shortcut={{ modifiers: ["cmd", "shift"], key: "d" }}
              onAction={toggle}
            />
            <Action
              title="Look Again"
              icon={Icon.ArrowClockwise}
              shortcut={Keyboard.Shortcut.Common.Refresh}
              onAction={lookAgain}
            />
          </ActionPanel.Section>
        </ActionPanel>
      }
    />
  );
}

/** A machine that didn't answer: what went wrong, the doctor for it. */
function OfflineItem({ h, dark, lookAgain }: { h: AgentsHost; dark: boolean; lookAgain: () => void }) {
  const { head, detail } = explainError(h.error);
  const t = chartTheme(dark);
  const icon: Image.ImageLike = { source: Icon.WifiDisabled, tintColor: DOWN };
  return (
    <List.Item
      id={`offline-${h.alias}`}
      icon={icon}
      title={h.alias}
      accessories={[{ tag: { value: "offline", color: DOWN }, tooltip: head }]}
      detail={
        <List.Item.Detail
          markdown={img(`${h.alias}: ${head}`, notice({ state: "down", color: t.state.down, title: head, detail }, t))}
        />
      }
      actions={
        <ActionPanel>
          <Action.Push title="Doctor" icon={Icon.Heartbeat} target={<DoctorDetail host={h.alias} />} />
          <Action
            title="Look Again"
            icon={Icon.ArrowClockwise}
            shortcut={Keyboard.Shortcut.Common.Refresh}
            onAction={lookAgain}
          />
        </ActionPanel>
      }
    />
  );
}

// blank while showcase mode pauses every window
export default pausable(Agents);
