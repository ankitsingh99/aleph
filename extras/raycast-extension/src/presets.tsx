// Presets: saved starting points (`aleph preset ls --json`) — one session or
// several (panes), on one machine or many — grouped by machine. A row: the
// agent's logo (several agents: theirs), how many sessions. The detail
// (presets-view.ts): a hero card, a picture of the tab it opens with each
// pane's agent, machine and folder (one-session presets: the facts), and the
// first prompts. ↵ launches one: `aleph preset launch NAME --detach` (every
// pane, in the background), or `aleph new @NAME` on builds without it. ⌘E
// opens the form with a one-session preset filled in; ⌘Y shows what it
// would start (`preset launch --dry-run`).
import { Action, ActionPanel, Color, Detail, environment, Icon, Keyboard, List, showToast, Toast } from "@raycast/api";
import { pausable } from "./lib/pause";
import { CopyAction } from "./lib/copy";
import { useCachedPromise, usePromise } from "@raycast/utils";
import { useState } from "react";
import { hasFlag, PRESETS_ARGS, presetLaunch, presetPlan, setting, shortPath, shQuote, PRESETS_API } from "./lib/cli";
import { chartTheme } from "./lib/charts";
import { useAleph, useBrands, useFleet, useSettings } from "./lib/hooks";
import { agentLogo, cardLogos } from "./lib/logos";
import {
  layoutWord,
  panesOf,
  Preset,
  PresetCtx,
  presetAgents,
  presetMarkdown,
  presetSection,
} from "./lib/presets-view";
import { StartForm, startSession, useDetach } from "./lib/start-form";
import { openInTerminal, openSession, openWith } from "./lib/terminal";
import { ACCENT, brand, errorText, MUTED } from "./lib/ui";

/** `aleph preset launch` (newer builds): every pane in one go. */
function usePresetLaunch() {
  return useCachedPromise(() => hasFlag("preset", "launch"), [], { keepPreviousData: true });
}

/** Launch in the background: `preset launch --detach`, else `new @name` (its first session). */
async function launch(p: Preset, o: { canLaunch: boolean; detach: boolean; fallbackHost: string }) {
  if (!o.canLaunch) {
    const host = p.host || o.fallbackHost;
    return startSession(["new", `@${p.name}`], { host, where: p.dir ? shortPath(p.dir) : host, detach: o.detach });
  }
  const n = panesOf(p).length;
  const toast = await showToast({
    style: Toast.Style.Animated,
    title: `Launching @${p.name}…`,
    message: n > 1 ? `${n} sessions` : undefined,
  });
  try {
    const started = await presetLaunch(p.name);
    const first = started.find((s) => s.name === p.focus) ?? started[0];
    toast.style = Toast.Style.Success;
    toast.title = started.length
      ? started.length === 1
        ? `${started[0].name} started on ${started[0].host}`
        : `@${p.name}: ${started.length} sessions started`
      : `@${p.name} launched`;
    toast.message = started.length > 1 ? started.map((s) => `${s.name} on ${s.host}`).join(" · ") : undefined;
    if (first) {
      toast.primaryAction = {
        title: openWith() === "herdr" ? "Attach in herdr" : "Open in Aleph",
        onAction: () => openSession({ host: first.host, sel: `${first.host}:${first.name}`, label: first.name }),
      };
    }
    return true;
  } catch (e) {
    toast.style = Toast.Style.Failure;
    Object.assign(toast, errorText(e));
    return false;
  }
}

function Presets() {
  useBrands();
  const { data, isLoading, errorInfo, revalidate } = useAleph<Preset[]>(PRESETS_ARGS, {
    failureTitle: "Couldn't read the presets",
    api: PRESETS_API,
    topics: ["config"],
  });
  const fleet = useFleet();
  const settings = useSettings();
  const detach = useDetach();
  const canLaunch = usePresetLaunch();
  const [showDetail, setShowDetail] = useState(true);
  const dark = environment.appearance !== "light";
  // a pane without a machine starts on the default one; without an agent, that machine's default
  const fallback = setting(settings.data, "machine") || setting(settings.data, "ssh_host") || "default";
  const ctx: PresetCtx = {
    fallback,
    defaultAgent: (h) => setting(settings.data, `host.${h}.agent`) || setting(settings.data, "agent") || "claude",
    color: (h) => fleet.data?.hosts.find((x) => x.alias === h)?.color || undefined,
  };
  const opts = { canLaunch: !!canLaunch.data, detach: !!detach.data, fallbackHost: fallback };
  // by machine, in the order they first appear; presets on several machines last
  const sections = new Map<string, Preset[]>();
  for (const p of data ?? []) {
    const s = presetSection(p, ctx);
    sections.set(s, [...(sections.get(s) ?? []), p]);
  }
  const order = [...sections.keys()].sort(
    (a, b) => Number(a === "Several machines") - Number(b === "Several machines"),
  );

  return (
    <List
      isLoading={isLoading}
      isShowingDetail={showDetail && !!data?.length}
      searchBarPlaceholder="Filter presets by name, machine, agent, folder or tag"
    >
      {errorInfo && !data ? (
        <List.EmptyView icon={Icon.Warning} title={errorInfo.title} description={errorInfo.message} />
      ) : (
        <List.EmptyView
          icon={{ source: Icon.Bookmark, tintColor: ACCENT }}
          title={isLoading ? "Reading the presets…" : "No presets yet"}
          description="aleph preset save NAME host:session saves a running session as one"
          actions={
            <ActionPanel>
              <Action.Push title="Start a Session" icon={Icon.Play} target={<StartForm />} />
              <CopyAction
                title="Copy Example Command"
                content="aleph preset add px --host b31 --dir /srv/px --agent claude"
              />
            </ActionPanel>
          }
        />
      )}
      {order.map((section) => {
        const list = sections.get(section) ?? [];
        const sessions = list.reduce((n, p) => n + panesOf(p).length, 0);
        return (
          <List.Section
            key={section}
            title={
              section === fallback && !fleet.data?.hosts.some((h) => h.alias === section) ? "Default machine" : section
            }
            subtitle={sessions > list.length ? `${list.length} · ${sessions} sessions` : String(list.length)}
          >
            {list.map((p) => (
              <PresetItem
                key={p.name}
                p={p}
                ctx={ctx}
                dark={dark}
                showDetail={showDetail}
                canLaunch={!!canLaunch.data}
                launch={() => launch(p, opts)}
                toggle={() => setShowDetail((x) => !x)}
                refresh={revalidate}
              />
            ))}
          </List.Section>
        );
      })}
    </List>
  );
}

function PresetItem({
  p,
  ctx,
  dark,
  showDetail,
  canLaunch,
  launch,
  toggle,
  refresh,
}: {
  p: Preset;
  ctx: PresetCtx;
  dark: boolean;
  showDetail: boolean;
  canLaunch: boolean;
  launch: () => void;
  toggle: () => void;
  refresh: () => void;
}) {
  const panes = panesOf(p);
  const single = panes.length === 1;
  const agents = presetAgents(p, ctx);
  const command = (canLaunch ? ["aleph", "preset", "launch", p.name] : ["aleph", "new", `@${p.name}`])
    .map(shQuote)
    .join(" ");
  const accessories: List.Item.Accessory[] = [];
  if (!single) {
    if (agents.length > 1)
      accessories.push(...agents.slice(0, 3).map((a) => ({ icon: agentLogo(a), tooltip: brand(a).name })));
    accessories.push({
      icon: { source: Icon.AppWindowGrid2x2, tintColor: MUTED },
      text: String(panes.length),
      tooltip: `${panes.length} sessions · ${layoutWord(p)}`,
    });
  } else if (panes[0].worktree) {
    accessories.push({ icon: { source: Icon.Tree, tintColor: MUTED }, tooltip: "in a new git worktree" });
  }
  if (!showDetail)
    for (const tag of p.tags ?? []) accessories.push({ tag: { value: `#${tag}`, color: Color.SecondaryText } });
  return (
    <List.Item
      id={p.name}
      icon={agents.length === 1 ? agentLogo(agents[0]) : { source: Icon.AppWindowGrid2x2, tintColor: ACCENT }}
      title={p.name}
      subtitle={showDetail || !single ? undefined : p.dir ? shortPath(p.dir) : undefined}
      keywords={[
        `@${p.name}`,
        p.description ?? "",
        ...(p.tags ?? []),
        ...panes.flatMap((x) => [x.name ?? "", x.host ?? "", x.dir ?? "", x.agent ?? "", brand(x.agent).name]),
      ].filter(Boolean)}
      accessories={accessories}
      detail={
        <List.Item.Detail markdown={presetMarkdown({ p, ctx, t: chartTheme(dark), logo: cardLogos(dark), brand })} />
      }
      actions={
        <ActionPanel>
          <ActionPanel.Section title={`@${p.name}`}>
            <Action title="Launch" icon={Icon.Play} onAction={launch} />
            {single ? (
              <Action.Push
                title="Launch with Changes"
                icon={Icon.Pencil}
                shortcut={Keyboard.Shortcut.Common.Edit}
                target={<StartForm initial={{ preset: p.name }} />}
              />
            ) : null}
            {canLaunch ? (
              <Action.Push
                title="What It Would Start"
                icon={Icon.Eye}
                shortcut={Keyboard.Shortcut.Common.ToggleQuickLook}
                target={<PlanView p={p} />}
              />
            ) : null}
            <Action
              title="Launch in Terminal"
              icon={Icon.Terminal}
              shortcut={{ modifiers: ["cmd", "shift"], key: "t" }}
              onAction={() =>
                openInTerminal(canLaunch ? ["preset", "launch", p.name] : ["new", `@${p.name}`], `Aleph · @${p.name}`)
              }
            />
          </ActionPanel.Section>
          <ActionPanel.Section>
            <CopyAction title="Copy Command" content={command} shortcut={Keyboard.Shortcut.Common.Copy} />
            {single && p.dir ? (
              <CopyAction title="Copy Folder" content={p.dir} shortcut={Keyboard.Shortcut.Common.CopyPath} />
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

/** `preset launch NAME --dry-run`: what it would start, where. */
function PlanView({ p }: { p: Preset }) {
  const { data, isLoading, error } = usePromise(presetPlan, [p.name]);
  const md = data
    ? "```\n" + data + "\n```"
    : error
      ? `**${errorText(error).title}**\n\n${errorText(error).message ?? ""}`
      : "_working out what it would start…_";
  return (
    <Detail
      isLoading={isLoading}
      navigationTitle={`@${p.name} would start`}
      markdown={md}
      actions={
        <ActionPanel>
          <CopyAction title="Copy Plan" content={data ?? ""} />
        </ActionPanel>
      }
    />
  );
}

// blank while showcase mode pauses every window
export default pausable(Presets);
