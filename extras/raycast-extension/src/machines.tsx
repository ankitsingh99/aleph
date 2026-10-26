// Machines (Machine Stats is part of it now): every machine in the fleet,
// live (`ls --json` — the daemon's snapshot when it runs — every 5 s, and
// `aleph stats --json` every 3 s, each the next once the last has answered),
// online ones first, the offline ones in a section of their own. A row: the
// OS logo (a red ✕ the moment a machine stops answering), who needs you (or
// how many are working, or open), CPU and GPU load coloured by how busy, and
// the link as signal bars. The detail (machine-detail.ts), all cards: the
// machine with its link's bars, latency and sparkline, its gauges (CPU,
// memory, GPU, VRAM, temperature), its sessions (or a quiet row with Quick
// Start's keycap), its GPUs, a heat grid of its cores, live trends that fill
// in while the view is open, what it is (OS, CPU, memory, uptime, herdr, its
// agents installed and running), disks, top processes and LLM servers
// (`services --json`). Down: why, and the doctor's keycap. ↵ opens it in Aleph (`aleph --host H`), ⌘↵ in
// herdr; Quick Start (⌘N) is the new-session form for that machine; the
// doctor (⌥⌘D) checks it.
import {
  Action,
  ActionPanel,
  Color,
  environment,
  Icon,
  Image,
  Keyboard,
  launchCommand,
  LaunchType,
  List,
  showToast,
  Toast,
} from "@raycast/api";
import { pausable } from "./lib/pause";
import { CopyAction } from "./lib/copy";
import { join } from "node:path";
import { useEffect, useMemo, useState } from "react";
import {
  AgentsHost,
  explainError,
  Host,
  MachineStats,
  ServicesHost,
  ServicesReport,
  SERVICES_ARGS,
  Stats,
  STATS_ARGS,
  statsApi,
  onHost,
  startHerdr,
  statusText,
  servicesApi,
} from "./lib/cli";
import { chartTheme, cardFilesIn } from "./lib/charts";
import { OpenMachineActions } from "./lib/actions";
import { DoctorDetail } from "./lib/doctor";
import { useAgentsReport, useAleph, useBrands, useFleet, useHistory, usePresenter } from "./lib/hooks";
import { agentLogo, cardLogos, gpuLogo, machineLogo, machineOsText, osLogo, rememberMachines } from "./lib/logos";
import { machineDetail } from "./lib/machine-detail";
import { fleetLink, hostLink, Tone } from "./lib/model";
import { followingTitle, navTitle, presenterItem, withBanner } from "./lib/showcase";
import {
  linkSample,
  machineBadges,
  MachineBadge,
  machineGpu,
  machinesSummary,
  sessionCounts,
} from "./lib/machines-view";
import { LinkSample, StatSample } from "./lib/samples";
import { Badge, gpuSummary, statSample, statsBadges, statsSummary } from "./lib/stats-view";
import { StartForm } from "./lib/start-form";
import { openDashboard, openInTerminal } from "./lib/terminal";
import { brand, DONE, DOWN, IDLE, loadColor, MUTED, NEEDS, WARN, WORKING } from "./lib/ui";
import { AgentsList } from "./agents";
import { ServicesList } from "./llm-servers";

/** The fleet again this long after the last answer. */
const LIVE_MS = 8_000;
/** The slower reads (agents, LLM servers, every machine's load) start this long after the list shows. */
const LATER_MS = 1_500;
/** The selected machine's stats again this long after the last answer (the daemon's samplers: no ssh per ask). */
const STATS_MS = 5_000;
/** Every machine's brief stats (the rows' load tags) this often. */
const ALL_STATS_MS = 15_000;
/** Moving between machines: the light detail until the selection rests this long. */
const SETTLE_MS = 350;

const toneColor = (t: Tone): Color.ColorLike =>
  t === "good" ? DONE : t === "warn" ? WARN : t === "bad" ? DOWN : MUTED;

const SIGNAL = [Icon.Signal0, Icon.Signal1, Icon.Signal2, Icon.Signal3, Icon.FullSignal];

// cards as files: Raycast reuses a card that didn't change instead of drawing it again
cardFilesIn(join(environment.supportPath, "cards"));

function Machines() {
  useBrands();
  const fleet = useFleet({ liveMs: LIVE_MS, failureTitle: "Couldn't list your machines" });
  // the list and the selected machine first; the slower reads once they're on screen
  const [later, setLater] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setLater(true), LATER_MS);
    return () => clearTimeout(t);
  }, []);
  // this computer's agents too (`local` is in the fleet)
  const agents = useAgentsReport(undefined, { execute: later }, ["--local"]);
  const services = useAleph<ServicesReport | undefined>(SERVICES_ARGS, {
    optional: "services",
    timeoutMs: 90_000,
    api: servicesApi(),
    topics: ["services"],
    execute: later,
  });
  const [showDetail, setShowDetail] = useState(true);
  const [selected, setSelected] = useState<string | null>(null);
  // showcase's presenter sync: the machine the presenter's dashboard has selected
  const presenter = usePresenter();
  const presented = presenterItem(presenter, "machines");
  const dark = environment.appearance !== "light";
  const hosts = fleet.data?.hosts ?? [];
  const online = hosts.filter((h) => hostLink(h).state !== "down");
  const offline = hosts.filter((h) => hostLink(h).state === "down");
  // the machine whose detail shows: only it is sampled live and drawn
  const focus = selected ?? online[0]?.alias ?? hosts[0]?.alias;
  // live stats for it (gauges, cores, GPUs, trends); every machine's now and then for the rows' load tags
  // (an older aleph without `stats`: the cards without them)
  // from the daemon's samplers when it runs (`stats.get`), else `aleph stats --json`
  const liveApi = useMemo(() => statsApi(focus), [focus]);
  const live = useAleph<Stats | undefined>(onHost(focus, STATS_ARGS), {
    optional: "stats",
    timeoutMs: 60_000,
    liveMs: STATS_MS,
    execute: !!focus,
    api: liveApi,
  });
  const allApi = useMemo(() => statsApi(undefined, true), []);
  const all = useAleph<Stats | undefined>(STATS_ARGS, {
    optional: "stats",
    timeoutMs: 60_000,
    pollMs: ALL_STATS_MS,
    api: allApi,
    execute: later,
  });
  // moving between machines draws the light detail; the rest once the selection rests
  const [settled, setSettled] = useState(true);
  useEffect(() => {
    setSettled(false);
    const t = setTimeout(() => setSettled(true), SETTLE_MS);
    return () => clearTimeout(t);
  }, [focus]);
  const statOf = (alias: string): MachineStats | undefined =>
    (alias === focus ? live.data?.hosts.find((x) => x.alias === alias) : undefined) ??
    all.data?.hosts.find((x) => x.alias === alias);
  // the other commands show each machine's OS logo from this
  useEffect(
    () =>
      rememberMachines(
        (all.data?.hosts ?? [])
          .filter((h) => h.reachable)
          .map((h) => ({ alias: h.alias, os: h.os, platform: h.platform })),
      ),
    [all.data],
  );
  const statSamples = useMemo(
    () =>
      live.data?.hosts
        .filter((h) => h.reachable)
        .map((h) => ({ alias: h.alias, s: statSample(h, live.data!.generated_at) })),
    [live.data],
  );
  const statHist = useHistory<StatSample>("stats", statSamples);
  const link = fleetLink(fleet.data?.hosts);
  const samples = useMemo(
    () => fleet.data?.hosts.map((h) => ({ alias: h.alias, s: linkSample(h, fleet.data!.generated_at) })),
    [fleet.data],
  );
  const hist = useHistory<LinkSample>("link", samples);
  // the one detail drawn: again only when its numbers change
  const focusHost = hosts.find((h) => h.alias === focus);
  const focusStats = focus ? statOf(focus) : undefined;
  const detail = useMemo(
    () =>
      focusHost
        ? machineDetail({
            h: focusHost,
            a: agents.data?.hosts.find((x) => x.alias === focusHost.alias && x.reachable),
            svc: services.data?.hosts.find((x) => x.alias === focusHost.alias),
            stats: focusStats,
            statHist: statHist[focusHost.alias] ?? [],
            hist: hist[focusHost.alias] ?? [],
            t: chartTheme(dark),
            logo: cardLogos(dark),
            brand,
            osText: machineOsText(focusHost.alias),
            statusText,
            light: !settled,
          })
        : "",
    [focus, settled, fleet.data, live.data, all.data, agents.data, services.data, dark],
  );
  const refresh = () => {
    fleet.revalidate();
    agents.revalidate();
    services.revalidate();
    live.revalidate();
    all.revalidate();
  };

  return (
    <List
      isShowingDetail={showDetail && hosts.length > 0}
      navigationTitle={navTitle(
        [
          "Machines",
          fleet.data ? `${link.glyph} ${link.text}` : "",
          fleet.stale ? "cached" : "",
          followingTitle(presenter),
        ]
          .filter(Boolean)
          .join(" · "),
      )}
      searchBarPlaceholder="Filter machines by name, OS, GPU, agent or LLM server"
      selectedItemId={presented}
      onSelectionChange={(id) => setSelected(id ?? null)}
      isLoading={fleet.isLoading || (live.isLoading && !live.data)}
    >
      {fleet.errorInfo && !fleet.data ? (
        <List.EmptyView icon={Icon.Warning} title={fleet.errorInfo.title} description={fleet.errorInfo.message} />
      ) : (
        <List.EmptyView
          icon={Icon.Network}
          title={fleet.isLoading ? "Looking at your machines…" : "No machines yet"}
          description="aleph setup adds them from ~/.ssh/config"
          actions={
            <ActionPanel>
              <Action
                title="Set up in Terminal"
                icon={Icon.Terminal}
                onAction={() => openInTerminal(["setup"], "Aleph · setup")}
              />
            </ActionPanel>
          }
        />
      )}
      {[
        { title: "Online", list: online },
        { title: "Offline", list: offline },
      ]
        .filter((g) => g.list.length)
        .map((g) => (
          <List.Section
            key={g.title}
            title={g.title}
            subtitle={g.title === "Online" ? machinesSummary(g.list) : String(g.list.length)}
          >
            {g.list.map((h) => (
              <MachineItem
                key={h.alias}
                h={h}
                a={agents.data?.hosts.find((x) => x.alias === h.alias && x.reachable)}
                svc={services.data?.hosts.find((x) => x.alias === h.alias)}
                m={statOf(h.alias)}
                markdown={h.alias === focus ? withBanner(detail) : ""}
                refresh={refresh}
                showDetail={showDetail}
                toggle={() => setShowDetail((x) => !x)}
                presented={h.alias === presented}
              />
            ))}
          </List.Section>
        ))}
    </List>
  );
}

/** A load badge as Raycast's accessory: tags coloured by load, the GPU vendor's logo, herdr's warning. */
function statAccessory(b: Badge, gpuIcon?: Image.ImageLike): List.Item.Accessory {
  const color = b.tone === "bad" ? DOWN : b.tone === "warn" ? WARN : loadColor(b.frac);
  if (b.icon === "warn") return { icon: { source: Icon.Warning, tintColor: WARN }, tooltip: b.tooltip };
  return {
    ...(b.icon === "gpu" ? { icon: gpuIcon ?? { source: Icon.ComputerChip, tintColor: color } } : {}),
    tag: { value: b.text, color },
    tooltip: b.tooltip,
  };
}

/** A badge as Raycast's accessory. */
function accessory(b: MachineBadge, a?: AgentsHost): List.Item.Accessory {
  switch (b.kind) {
    case "needs":
      return { tag: { value: `▲ ${b.text}`, color: NEEDS }, tooltip: b.tooltip };
    case "working":
      return { icon: { source: Icon.CircleProgress50, tintColor: WORKING }, text: b.text, tooltip: b.tooltip };
    case "gpu":
      return { icon: gpuLogo(a?.gpu, ...(a?.gpus ?? []))?.icon ?? Icon.ComputerChip, tooltip: b.tooltip };
    case "link":
      return {
        icon: { source: SIGNAL[Math.min(4, Math.max(0, b.bars ?? 0))], tintColor: toneColor(b.tone ?? "muted") },
        tooltip: b.tooltip,
      };
    case "stopped":
      return { tag: { value: b.text, color: WARN }, tooltip: b.tooltip };
    case "offline":
      return { tag: { value: b.text, color: DOWN }, tooltip: b.tooltip };
  }
}

function MachineItem({
  h,
  a,
  svc,
  m,
  markdown,
  refresh,
  showDetail,
  toggle,
  presented,
}: {
  h: Host;
  a?: AgentsHost;
  svc?: ServicesHost;
  m?: MachineStats;
  /** its detail, drawn by the list (only the selected machine's) */
  markdown: string;
  refresh: () => void;
  showDetail: boolean;
  toggle: () => void;
  /** the presenter's dashboard has it selected (showcase's presenter sync) */
  presented?: boolean;
}) {
  const link = hostLink(h);
  const os = a?.os_name || a?.os || "";
  const osl = osLogo(a?.os_name, a?.os) ?? machineLogo(h.alias, h.os);
  const gpu = machineGpu(a);
  const installed = (a?.agents ?? []).filter((x) => x.installed);
  const servers = svc?.services ?? [];
  // a machine that stops answering shows it in its icon at once
  const icon: Image.ImageLike =
    link.state === "down"
      ? { source: Icon.WifiDisabled, tintColor: DOWN }
      : (osl?.icon ?? { source: Icon.CircleFilled, tintColor: h.color || Color.SecondaryText });
  const badges = machineBadges(h, a);
  const count = `${h.sessions.length} session${h.sessions.length === 1 ? "" : "s"}`;
  const waiting = sessionCounts(h).find((c) => c.state === "blocked")?.n ?? 0;

  // the detail hidden (⇧⌘D), the row is wide: its sessions, agents, servers and link in words
  // nobody needs you and nothing's working: how many are open, quietly
  const quiet =
    h.sessions.length && !badges.some((b) => b.kind === "needs" || b.kind === "working" || b.kind === "offline")
      ? [{ icon: { source: Icon.CircleFilled, tintColor: IDLE }, text: String(h.sessions.length), tooltip: count }]
      : [];
  // live load (CPU, GPU or memory) in place of the GPU's bare logo, once sampled
  const live = m?.reachable && link.state !== "down";
  const gpul = gpuLogo(gpuSummary(m ?? {}, a), a?.gpu, ...(a?.gpus ?? []), ...(m?.gpus ?? []).map((g) => g.name));
  const load = live ? statsBadges(m!, h, a).map((b) => statAccessory(b, gpul?.icon)) : [];
  const shownBadges = live ? badges.filter((b) => b.kind !== "gpu") : badges;
  const linkBadge = shownBadges.filter((b) => b.kind === "link").map((b) => accessory(b, a));
  const accessories: List.Item.Accessory[] = showDetail
    ? [...quiet, ...shownBadges.filter((b) => b.kind !== "link").map((b) => accessory(b, a)), ...load, ...linkBadge]
    : [
        ...(waiting ? [{ tag: { value: `▲ ${waiting}`, color: NEEDS }, tooltip: `${waiting} need you` }] : []),
        { icon: Icon.Terminal, text: String(h.sessions.length), tooltip: count },
        ...installed.slice(0, 3).map((x) => ({
          icon: agentLogo(x.id),
          tooltip: `${x.name}${x.version ? ` ${x.version}` : ""}`,
        })),
        ...(installed.length > 3 ? [{ text: `+${installed.length - 3}` }] : []),
        ...(servers.length
          ? [
              {
                icon: Icon.ComputerChip,
                text: String(servers.length),
                tooltip: servers.map((s) => `${s.name} (${s.health_label || s.health})`).join(", "),
              },
            ]
          : []),
        { tag: { value: `${link.glyph} ${link.text}`, color: toneColor(link.tone) } },
      ];

  const shown: List.Item.Accessory[] = presented
    ? [
        { tag: { value: "▶ presenter", color: Color.Yellow }, tooltip: "The presenter's dashboard has it selected" },
        ...accessories,
      ]
    : accessories;

  return (
    <List.Item
      id={h.alias}
      icon={{ value: icon, tooltip: link.state === "down" ? `${h.alias}: ${link.text}` : (osl?.name ?? h.alias) }}
      title={h.alias}
      subtitle={
        showDetail ? undefined : h.reachable ? [os, gpu].filter(Boolean).join(" · ") : explainError(h.error).head
      }
      keywords={[
        h.address,
        h.machine,
        os,
        gpu,
        ...installed.flatMap((x) => [x.id, x.name]),
        ...servers.map((s) => s.name),
      ].filter(Boolean)}
      accessories={shown}
      detail={<List.Item.Detail markdown={markdown} />}
      actions={
        <ActionPanel>
          <ActionPanel.Section title={h.alias}>
            {link.state === "stopped" ? (
              <Action
                title="Start Herdr"
                icon={Icon.Play}
                shortcut={{ modifiers: ["cmd", "opt"], key: "h" }}
                onAction={async () => {
                  const t = await showToast({ style: Toast.Style.Animated, title: `Starting herdr on ${h.alias}…` });
                  try {
                    const r = await startHerdr(h.alias);
                    t.style = Toast.Style.Success;
                    t.title = r.message;
                    refresh();
                  } catch (e) {
                    t.style = Toast.Style.Failure;
                    t.title = `herdr didn't start on ${h.alias}`;
                    t.message = String(e instanceof Error ? e.message : e);
                  }
                }}
              />
            ) : null}
            <OpenMachineActions host={h.alias} />
            <Action.Push
              title="Quick Start"
              icon={Icon.Plus}
              shortcut={Keyboard.Shortcut.Common.New}
              target={<StartForm initial={{ host: h.alias }} />}
            />
            <Action
              title="New Terminal"
              icon={Icon.Terminal}
              shortcut={{ modifiers: ["cmd", "opt"], key: "t" }}
              onAction={() => openInTerminal(["--host", h.alias, "shell"], `Terminal · ${h.alias}`)}
            />
            <Action.Push
              title="Doctor"
              icon={Icon.Heartbeat}
              shortcut={{ modifiers: ["cmd", "opt"], key: "d" }}
              target={<DoctorDetail host={h.alias} />}
            />
            <Action
              title="Show Sessions"
              icon={Icon.List}
              shortcut={{ modifiers: ["cmd", "opt"], key: "s" }}
              onAction={() =>
                launchCommand({ name: "sessions", type: LaunchType.UserInitiated, context: { host: h.alias } })
              }
            />
            <Action.Push
              title="Show Agents"
              icon={Icon.Person}
              shortcut={{ modifiers: ["cmd"], key: "g" }}
              target={<AgentsList host={h.alias} />}
            />
            <Action.Push
              title="Show LLM Servers"
              icon={Icon.ComputerChip}
              shortcut={{ modifiers: ["cmd"], key: "l" }}
              target={<ServicesList only={h.alias} />}
            />
            <Action
              title="Browse Folders"
              icon={Icon.Folder}
              shortcut={{ modifiers: ["cmd"], key: "b" }}
              onAction={() => openInTerminal(["--host", h.alias, "-i"], `Aleph · ${h.alias}`)}
            />
          </ActionPanel.Section>
          <ActionPanel.Section>
            <CopyAction title="Copy SSH Alias" content={h.alias} shortcut={Keyboard.Shortcut.Common.CopyName} />
            {h.address ? <CopyAction title="Copy Address" content={h.address} /> : null}
            {m?.reachable ? (
              <CopyAction
                title="Copy Summary"
                content={statsSummary(m, h, a)}
                shortcut={Keyboard.Shortcut.Common.Copy}
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
export default pausable(Machines);
