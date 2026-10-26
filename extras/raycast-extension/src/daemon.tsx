// Daemon: Aleph's daemon at a glance (daemon-view.ts draws it) — a status
// card (running or not in its state colour, uptime, version, the hub, the
// service's health and the one thing to do when something needs doing), the
// machines it watches with their links (its fleet snapshot, every 5 s), what
// it did today (the inbox, the queue, when usage was last read) and remote
// access as chips. From the API's `daemon.status` (live, every 5 s) with
// `aleph daemon status --json` (the service, the HTTP API's settings,
// exposure), `daemon token ls --json` (paired devices) and the settings.
// Start, stop, restart, install or uninstall it (the CLI; stopping and
// uninstalling ask first); turn the HTTP API on or off (asks first, then
// restarts it); Pair a Device: `aleph daemon pair --json` as a link, a code
// and a QR code; putting it on your tailnet runs `aleph daemon expose
// tailscale` in a terminal, where it asks before changing anything.
import {
  Action,
  ActionPanel,
  Alert,
  confirmAlert,
  Detail,
  environment,
  Icon,
  Keyboard,
  launchCommand,
  LaunchType,
  open,
  useNavigation,
} from "@raycast/api";
import { pausable } from "./lib/pause";
import { CopyAction } from "./lib/copy";
import { useCachedPromise, usePromise } from "@raycast/utils";
import { useEffect, useMemo, useState } from "react";
import { act } from "./lib/act";
import {
  daemonControl,
  daemonDetails,
  daemonLive,
  DaemonOp,
  Device,
  DEVICES_ARGS,
  INBOX_ARGS,
  InboxEvent,
  inboxApi,
  pairDevice,
  QUEUE_API,
  QUEUE_ARGS,
  QueueTask,
  setApiHttp,
} from "./lib/cli";
import { chartTheme, dataUri } from "./lib/charts";
import { daemonMarkdown, DaemonView, daemonView, nextStep, UsageToday } from "./lib/daemon-view";
import { useAleph, useFleet, useHistory, useSettings } from "./lib/hooks";
import { cardLogos, machineOsText } from "./lib/logos";
import { linkSample } from "./lib/machines-view";
import { qrSvg } from "./lib/qr";
import { LinkSample } from "./lib/samples";
import { mode as showcaseMode, navTitle, type Mode } from "./lib/showcase";
import { openInTerminal } from "./lib/terminal";

const POLL_MS = 5_000;
/** The inbox and the queue again this often; usage's last read once a minute. */
const ACTIVITY_MS = 15_000;
const USAGE_MS = 60_000;
const INBOX_N = 300;
const INBOX_CALL = inboxApi(INBOX_N);
/** Today's total from what's been read so far (the daemon reads the logs; this never does). */
const USAGE_TODAY_ARGS = ["usage", "today", "--by", "total", "--json", "--no-sync"];

/** The code as `aleph daemon pair` prints it: `ABCDE-FGHIJ`. */
const showCode = (c: string) => (c.length === 10 ? `${c.slice(0, 5)}-${c.slice(5)}` : c);

const LOCAL = /^http:\/\/(127\.|localhost|\[::1\])/;

/** Where the web console opens: the tailnet's address when there is one, else where it listens here. */
function consoleUrl(v: DaemonView): string | undefined {
  const ts = v.s.exposure?.tailscale?.find((x) => x.url)?.url;
  if (ts) return ts;
  if (!v.listening) return undefined;
  return `http://${v.listening.replace(/^0\.0\.0\.0:|^\[::\]:/, "127.0.0.1:")}`;
}

/** Until the daemon's API listens again after a restart (a few seconds at most). */
async function apiListening(tries = 12): Promise<boolean> {
  for (let i = 0; i < tries; i++) {
    if ((await daemonLive())?.http) return true;
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}

function Daemon() {
  // showcase mode is part of each cache key: what was cached unmasked never shows masked
  const sm = showcaseMode();
  const live = useCachedPromise((_m: Mode) => daemonLive(), [sm], { keepPreviousData: true });
  const details = useCachedPromise((_m: Mode) => daemonDetails().catch(() => undefined), [sm], {
    keepPreviousData: true,
  });
  const settings = useSettings();
  const { revalidate: again } = live;
  useEffect(() => {
    const t = setInterval(again, POLL_MS);
    return () => clearInterval(t);
  }, [again]);
  const loading = live.isLoading || details.isLoading;
  const unsupported = !loading && !live.data && !details.data;
  const v = daemonView(live.data, details.data, settings.data, { loading, unsupported });
  const s = v.s;

  // what it's doing: only asked while it runs
  const fleet = useFleet({ liveMs: POLL_MS, execute: v.running });
  const events = useAleph<InboxEvent[] | undefined>(INBOX_ARGS, {
    optional: "inbox",
    pollMs: ACTIVITY_MS,
    api: INBOX_CALL,
    topics: ["inbox"],
    execute: v.running,
  });
  const queue = useAleph<QueueTask[] | undefined>(QUEUE_ARGS, {
    optional: "queue",
    pollMs: ACTIVITY_MS,
    api: QUEUE_API,
    topics: ["queue"],
    execute: v.running,
  });
  const usage = useAleph<UsageToday | undefined>(USAGE_TODAY_ARGS, {
    optional: "usage",
    pollMs: USAGE_MS,
    api: { method: "usage.query", params: { range: "today", by: "total" }, accept: (u) => Array.isArray(u?.rows) },
    topics: ["usage"],
    execute: v.running,
  });
  const devices = useAleph<Device[] | undefined>(DEVICES_ARGS, {
    optional: "daemon",
    api: { method: "tokens.list", accept: Array.isArray },
  });
  const samples = useMemo(
    () => fleet.data?.hosts.map((h) => ({ alias: h.alias, s: linkSample(h, fleet.data!.generated_at) })),
    [fleet.data],
  );
  const hist = useHistory<LinkSample>("link", samples);
  const dark = environment.appearance !== "light";
  const { push } = useNavigation();

  const refresh = () => {
    live.revalidate();
    details.revalidate();
    settings.revalidate();
    devices.revalidate();
    if (v.running) {
      fleet.revalidate();
      events.revalidate();
      queue.revalidate();
      usage.revalidate();
    }
  };

  const control = async (op: DaemonOp) => {
    const words: Record<DaemonOp, [string, string]> = {
      install: ["Installing the daemon…", "Installed: it runs now and at every login"],
      uninstall: ["Uninstalling the daemon…", "Uninstalled"],
      start: ["Starting the daemon…", "Started"],
      stop: ["Stopping the daemon…", "Stopped"],
      restart: ["Restarting the daemon…", "Restarted"],
    };
    if (op === "stop" || op === "uninstall") {
      const ok = await confirmAlert({
        title: op === "stop" ? "Stop the daemon?" : "Uninstall the daemon?",
        message:
          op === "stop"
            ? `Until it runs again, queued messages wait, nothing notifies you and auto-approve is off.${v.installed ? " It starts again at login." : ""}`
            : "It stops, and no longer starts at login. Your settings, inbox and usage stay.",
        icon: Icon.Stop,
        primaryAction: { title: op === "stop" ? "Stop" : "Uninstall", style: Alert.ActionStyle.Destructive },
      });
      if (!ok) return;
    }
    const [doing, done] = words[op];
    await act(doing, () => daemonControl(op), refresh, { title: done });
  };

  /** `api.http` on or off (asked first), and the daemon restarted to pick it up. */
  const setApi = async (on: boolean): Promise<boolean> => {
    const listen = s.api?.listen ?? "127.0.0.1:7878";
    const restarts = v.running && v.installed;
    const ok = await confirmAlert({
      title: on ? "Turn on the HTTP API?" : "Turn off the HTTP API?",
      message: on
        ? `It listens on ${listen}${s.api?.listen_any ? "" : " (this computer only)"}.${restarts ? " The daemon restarts." : ""}`
        : `Paired devices and the web console can't reach it any more.${restarts ? " The daemon restarts." : ""}`,
      icon: Icon.Globe,
      primaryAction: { title: on ? "Turn On" : "Turn Off", style: on ? undefined : Alert.ActionStyle.Destructive },
    });
    if (!ok) return false;
    return act(
      on ? "Turning on the HTTP API…" : "Turning off the HTTP API…",
      async () => {
        await setApiHttp(on);
        if (restarts) await daemonControl("restart");
        if (restarts && on) await apiListening();
      },
      refresh,
      {
        title: on ? "The HTTP API is on" : "The HTTP API is off",
        message: v.running && !v.installed ? "Restart the daemon to apply it" : undefined,
      },
    );
  };

  const pair = async () => {
    if (v.listening) return push(<PairView />);
    if (await setApi(true)) push(<PairView />);
  };

  const step = nextStep(v);
  const web = consoleUrl(v);
  const exposed = !!(s.exposure?.tailscale?.some((x) => x.url) || s.exposure?.cloudflare?.hostname);
  const canStop = v.running && s.kind !== "dashboard";
  const stepTitle: Record<string, string> = { start: "Start", install: "Install", restart: "Restart" };
  const first = step?.op ? (
    <Action
      title={step.op === "install" && v.running ? "Install as a Service" : stepTitle[step.op]}
      icon={step.op === "restart" ? Icon.ArrowClockwise : step.op === "install" ? Icon.Download : Icon.Play}
      onAction={() => control(step.op as DaemonOp)}
    />
  ) : null;
  const logs = (
    <Action
      title="Show Logs"
      icon={Icon.Terminal}
      shortcut={{ modifiers: ["cmd", "shift"], key: "l" }}
      onAction={() => openInTerminal(["daemon", "logs", "-f"], "Aleph · daemon logs")}
    />
  );
  // ↵: what needs doing; else Pair a Device while the API listens; else the logs
  const logsFirst = !first && !(v.running && v.listening);
  const go = (name: string) => () => launchCommand({ name, type: LaunchType.UserInitiated });

  return (
    <Detail
      isLoading={loading}
      navigationTitle={navTitle("Aleph Daemon")}
      markdown={daemonMarkdown({
        v,
        fleet: fleet.data?.hosts,
        hist,
        events: events.data,
        cap: INBOX_N,
        queue: queue.data,
        usage: usage.data,
        devices: devices.data,
        settings: settings.data,
        t: chartTheme(dark),
        logo: cardLogos(dark),
        osText: machineOsText,
      })}
      actions={
        <ActionPanel>
          <ActionPanel.Section>
            {first}
            {logsFirst ? logs : null}
            {v.running ? (
              <Action
                title="Pair a Device"
                icon={Icon.Mobile}
                shortcut={{ modifiers: ["cmd", "shift"], key: "p" }}
                onAction={pair}
              />
            ) : null}
            {v.state !== "missing" ? (
              <Action
                title={v.httpOn ? "Turn off the HTTP API" : "Turn on the HTTP API"}
                icon={v.httpOn ? Icon.XMarkCircle : Icon.Globe}
                style={v.httpOn ? Action.Style.Destructive : undefined}
                shortcut={{ modifiers: ["cmd", "shift"], key: "a" }}
                onAction={() => setApi(!v.httpOn)}
              />
            ) : null}
            {web && v.running ? (
              <Action
                title="Open Web Console"
                icon={Icon.Window}
                shortcut={{ modifiers: ["cmd", "shift"], key: "w" }}
                onAction={() => open(web)}
              />
            ) : null}
            {v.listening && !exposed ? (
              <Action
                title="Put It on Your Tailnet"
                icon={Icon.Network}
                shortcut={{ modifiers: ["cmd", "shift"], key: "t" }}
                onAction={() => openInTerminal(["daemon", "expose", "tailscale"], "Aleph · expose on Tailscale")}
              />
            ) : null}
            {exposed ? (
              <Action
                title="Stop Exposing It"
                icon={Icon.EyeDisabled}
                onAction={() => openInTerminal(["daemon", "expose", "off"], "Aleph · expose off")}
              />
            ) : null}
            {web ? (
              <CopyAction title="Copy API Address" content={web} shortcut={Keyboard.Shortcut.Common.Copy} />
            ) : null}
          </ActionPanel.Section>
          <ActionPanel.Section title="What It's Doing">
            {logsFirst ? null : logs}
            <Action
              title="Daemon Doctor"
              icon={Icon.Heartbeat}
              shortcut={{ modifiers: ["cmd", "shift"], key: "d" }}
              onAction={() => openInTerminal(["daemon", "doctor"], "Aleph · daemon doctor")}
            />
            <Action
              title="Open Inbox"
              icon={Icon.Tray}
              shortcut={{ modifiers: ["cmd", "shift"], key: "i" }}
              onAction={go("inbox")}
            />
            <Action
              title="Open Machines"
              icon={Icon.Network}
              shortcut={{ modifiers: ["cmd", "shift"], key: "m" }}
              onAction={go("machines")}
            />
            <Action
              title="Open Message Queue"
              icon={Icon.List}
              shortcut={{ modifiers: ["cmd", "shift"], key: "q" }}
              onAction={go("queue")}
            />
            <Action
              title="Open Usage"
              icon={Icon.BarChart}
              shortcut={{ modifiers: ["cmd", "shift"], key: "u" }}
              onAction={go("usage")}
            />
          </ActionPanel.Section>
          <ActionPanel.Section title="Service">
            {!v.running && v.installed && step?.op !== "start" ? (
              <Action title="Start" icon={Icon.Play} onAction={() => control("start")} />
            ) : null}
            {!v.installed && v.state !== "missing" && step?.op !== "install" ? (
              <Action
                title={v.running ? "Install as a Service" : "Install"}
                icon={Icon.Download}
                onAction={() => control("install")}
              />
            ) : null}
            {v.running && v.installed && step?.op !== "restart" ? (
              <Action title="Restart" icon={Icon.ArrowClockwise} onAction={() => control("restart")} />
            ) : null}
            {canStop ? (
              <Action
                title="Stop"
                icon={Icon.Stop}
                style={Action.Style.Destructive}
                shortcut={Keyboard.Shortcut.Common.Remove}
                onAction={() => control("stop")}
              />
            ) : null}
            {v.installed ? (
              <Action
                title="Uninstall"
                icon={Icon.Trash}
                style={Action.Style.Destructive}
                onAction={() => control("uninstall")}
              />
            ) : null}
            {s.socket ? <CopyAction title="Copy Socket Path" content={s.socket} /> : null}
            {s.logs?.path ? <CopyAction title="Copy Log Path" content={s.logs.path} /> : null}
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

/** A one-time code (5 minutes) for a phone or another computer: the link, the code, a QR code. */
function PairView() {
  const { data, isLoading, error, revalidate } = usePromise(pairDevice, [], {
    failureToastOptions: { title: "Couldn't make a pairing code" },
  });
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now() / 1000), 5_000);
    return () => clearInterval(t);
  }, []);
  const expired = !!data && now >= data.expires;
  const until = data
    ? new Date(data.expires * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : "";
  const lines = !data
    ? [
        "## Pair a device",
        error
          ? `Couldn't make a code: ${error.message}${(error as { hint?: string }).hint ? `\n\n${(error as { hint?: string }).hint}` : ""}`
          : "Making a one-time code…",
      ]
    : [
        `## Pair a device${expired ? " — this code has expired" : ""}`,
        expired ? "Make a new one (⌘N)." : `![QR code for the link](${dataUri(qrSvg(data.url))})`,
        `**Link** \`${data.url}\``,
        `**Code** \`${showCode(data.code)}\` · works once, until ${until}`,
        `Scope: **${data.scope}**${data.scope === "reply" ? " (it can read, answer, send and queue; `aleph daemon pair --scope control` in a terminal gives more)" : ""}.`,
        "On the phone: scan it or open the link. On an iPhone, Share → Add to Home Screen, open Aleph from the Home Screen and enter the code there (that's where notifications work).",
        LOCAL.test(data.url)
          ? "_That address only works on this computer. For a phone: put it on your tailnet (`aleph daemon expose tailscale`), then pair again._"
          : "",
      ];
  return (
    <Detail
      isLoading={isLoading}
      navigationTitle="Pair a Device"
      markdown={lines.filter(Boolean).join("\n\n")}
      actions={
        <ActionPanel>
          {data && !expired ? (
            <>
              <CopyAction title="Copy Link" content={data.url} />
              <CopyAction title="Copy Code" content={data.code} shortcut={Keyboard.Shortcut.Common.Copy} />
            </>
          ) : null}
          <Action
            title="New Code"
            icon={Icon.ArrowClockwise}
            shortcut={Keyboard.Shortcut.Common.New}
            onAction={revalidate}
          />
        </ActionPanel>
      }
    />
  );
}

// blank while showcase mode pauses every window
export default pausable(Daemon);
