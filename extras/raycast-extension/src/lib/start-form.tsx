// The Start Session form, the Quick start (also pushed from a session's "New Session
// in This Folder", a machine, an installed agent, a preset, and the Inbox's "Start
// Again with Changes"), and starting a session again from how it was started.
// With an aleph that has `new --detach`, the session starts in the background
// with a toast offering Open in Aleph; otherwise `new` opens in the terminal.
// Agents come from `agents --json`: the ones installed on the machine first,
// then the ones that can be installed — picking one asks, installs, then starts.
// Machines show their OS logo (a red sign when one doesn't answer) and the
// form starts on Aleph's default machine; agents and presets show their logos.
import {
  Action,
  ActionPanel,
  Form,
  Icon,
  Image,
  Keyboard,
  launchCommand,
  LaunchType,
  showToast,
  Toast,
} from "@raycast/api";
import { copyText } from "./copy";
import { useCachedPromise } from "@raycast/utils";
import { useEffect, useMemo, useState } from "react";
import {
  agentIds,
  agentsReport,
  CliError,
  hasFlag,
  Host,
  NAME_RE,
  Preset,
  RecentDir,
  run,
  shortPath,
  shQuote,
  STATIC_AGENTS,
  PRESETS_API,
  RECENT_API,
} from "./cli";
import { agentTitle, defaultHost, hostTitle, startArgs, startCommand } from "./form-view";
import { useAgentsReport, useAleph, useFleet, useSettings } from "./hooks";
import { confirmInstall, installWithToast } from "./install-flow";
import { openInTerminal, openSession, openWith } from "./terminal";
import { agentLogo, machineLogo, osLogo } from "./logos";
import { ACCENT, brand, DOWN, errorText } from "./ui";

const OTHER = "__other__";
/** no --agent: aleph starts the machine's or the global default (`agent` setting) */
const AGENT_DEFAULT = "__default_agent__";
const DEFAULT = "__default__";
const NONE = "__none__";

interface Values {
  preset: string;
  host: string;
  folder: string;
  dir?: string;
  name?: string;
  prompt?: string;
  agent: string;
  worktree: boolean;
  branch?: string;
  tile: string;
}

export interface StartInitial {
  host?: string;
  dir?: string;
  agent?: string;
  prompt?: string;
  worktree?: boolean;
  /** a preset to start from (`aleph new @name`) */
  preset?: string;
}

/** How a session was started (a `history --json` entry, or what a running session tells). */
export interface Recipe {
  host: string;
  dir: string;
  agent: string;
  args?: string[];
  prompt?: string | null;
  worktree?: boolean;
}

/** `new` arguments that start it again: same machine, folder, agent, worktree, first prompt and agent args. */
export function againArgs(r: Recipe, withPrompt = true): string[] {
  const a = ["new", "--host", r.host];
  if (r.dir) a.push("--dir", r.dir);
  if (r.agent) a.push("--agent", r.agent);
  if (r.worktree) a.push("--worktree");
  if (withPrompt && r.prompt) a.push("-m", r.prompt);
  if (r.args?.length) a.push("--", ...r.args);
  return a;
}

/** The shell command that starts it again (like the dashboard's Inbox `c`). */
export function againCommand(r: Recipe): string {
  return ["aleph", ...againArgs(r)].map(shQuote).join(" ");
}

/** Start it again in the background (or the terminal, on builds without `new --detach`). */
export function startAgain(r: Recipe, detach: boolean, name?: string): Promise<boolean> {
  return startSession(againArgs(r), { host: r.host, name, where: r.dir ? shortPath(r.dir) : r.host, detach });
}

/** Can this aleph start a session in the background (`new --detach`)? */
export function useDetach() {
  return useCachedPromise(() => hasFlag("new", "--detach"), [], { keepPreviousData: true });
}

/**
 * Start `aleph new …`: in the background with a toast (Open in Aleph) when
 * `detach`, else in the terminal (the launch card attaches). Resolves true once started.
 */
export async function startSession(
  args: string[],
  o: { host: string; name?: string; where?: string; detach: boolean },
): Promise<boolean> {
  if (o.detach) {
    const toast = await showToast({ style: Toast.Style.Animated, title: `Starting on ${o.host}…`, message: o.where });
    try {
      const out = await run([...args, "--detach", "--json"], 90_000);
      let name = o.name ?? "";
      let pane = "";
      // where it really started (a preset names its own machine)
      let host = o.host;
      try {
        const j = JSON.parse(out);
        name = j.name ?? name;
        pane = j.pane_id ?? j.pane ?? "";
        host = typeof j.host === "string" && j.host ? j.host : host;
      } catch {
        // plain text output: keep the name we asked for
      }
      const sel = pane ? `${host}:${pane}` : name ? `${host}:${name}` : "";
      toast.style = Toast.Style.Success;
      toast.title = name ? `${name} started on ${host}` : `Started on ${host}`;
      toast.primaryAction = sel
        ? {
            title: openWith() === "herdr" ? "Attach in herdr" : "Open in Aleph",
            onAction: () => openSession({ host, sel, label: name || host }),
          }
        : {
            title: "Open Sessions",
            onAction: () => launchCommand({ name: "sessions", type: LaunchType.UserInitiated }),
          };
      return true;
    } catch (e) {
      if (!(e instanceof CliError && e.unsupported)) {
        toast.style = Toast.Style.Failure;
        Object.assign(toast, errorText(e));
        return false;
      }
      await toast.hide();
    }
  }
  // starting draws aleph's launch card and attaches: that needs a terminal
  return openInTerminal(args, `Aleph · ${o.host}`);
}

/** What Start does with the form: start it (in the background when it can), start and attach in a terminal, or copy the command. */
type How = "start" | "terminal" | "copy";

export function StartForm({ initial }: { initial?: StartInitial }) {
  const fleet = useFleet({ failureTitle: "Couldn't list machines" });
  const dirs = useAleph<RecentDir[]>(["recent", "--json"], { api: RECENT_API, topics: ["history"] });
  const presetList = useAleph<Preset[]>(["preset", "ls", "--json"], { api: PRESETS_API, topics: ["config"] });
  const settings = useSettings();
  const report = useAgentsReport();
  const detach = useDetach();
  const setting = (k: string) => {
    const v = settings.data?.[k];
    return typeof v === "string" ? v : "";
  };

  const hosts = fleet.data?.hosts ?? [];
  const [preset, setPreset] = useState(initial?.preset ?? NONE);
  const [host, setHost] = useState<string | undefined>(initial?.host);
  const [folder, setFolder] = useState<string>(initial?.dir ? OTHER : DEFAULT);
  const [dir, setDir] = useState(initial?.dir ?? "");
  const [agent, setAgent] = useState(initial?.agent || AGENT_DEFAULT);
  const [prompt, setPrompt] = useState(initial?.prompt ?? "");
  const [worktree, setWorktree] = useState(initial?.worktree ?? false);
  const [dirError, setDirError] = useState<string>();
  const [nameError, setNameError] = useState<string>();
  // the machine asked for, else Aleph's default machine while it answers
  const current = defaultHost(hosts, { machine: setting("machine"), ssh_host: setting("ssh_host") }, host);

  const dirsOn = (h?: string) => (dirs.data ?? []).filter((d) => d.host === h).slice(0, 15);
  const hostDirs = useMemo(() => dirsOn(current), [dirs.data, current]);
  // this machine's agents, when aleph could look (`agents --json`)
  const onHost = report.data?.hosts.find((h) => h.alias === current && h.reachable);
  const installed = onHost?.agents.filter((a) => a.installed) ?? [];
  const installable = onHost?.agents.filter((a) => !a.installed && a.install_ready) ?? [];
  const known = agentIds(report.data);
  const agentNames = known.length ? known : STATIC_AGENTS;
  const listed = new Set(onHost ? [...installed, ...installable].map((a) => a.id) : agentNames);
  // what "Default" starts: the machine's agent, else the `agent` setting
  const defaultAgent = (current && setting(`host.${current}.agent`)) || setting("agent");

  // a preset fills the form; anything changed afterwards wins
  useEffect(() => {
    const p = presetList.data?.find((x) => x.name === preset);
    if (!p) return;
    if (p.host) setHost(p.host);
    if (p.dir) {
      setFolder(OTHER);
      setDir(p.dir);
    }
    if (p.agent) setAgent(p.agent);
    setWorktree(p.worktree);
    if (p.prompt) setPrompt(p.prompt);
  }, [preset, presetList.data]);

  /** Another machine: a recent folder of the last one doesn't go with it. */
  function pickHost(h: string) {
    setHost(h);
    if (folder !== DEFAULT && folder !== OTHER && !dirsOn(h).some((d) => d.dir === folder)) setFolder(DEFAULT);
  }

  async function submit(v: Values, how: How) {
    const d = v.folder === OTHER ? v.dir?.trim() : v.folder === DEFAULT ? "" : v.folder;
    if (v.folder === OTHER && !d?.startsWith("/")) {
      setDirError("An absolute path on the machine");
      return;
    }
    if (v.name && !NAME_RE.test(v.name)) {
      setNameError("Lowercase letters, digits, - and _ (a letter first)");
      return;
    }
    if (!v.host) {
      await showToast({ style: Toast.Style.Failure, title: "No machine", message: "aleph host pick sets one up" });
      return;
    }
    const values = {
      host: v.host,
      preset: v.preset && v.preset !== NONE ? v.preset : undefined,
      dir: d,
      name: v.name,
      agent: v.agent && v.agent !== AGENT_DEFAULT ? v.agent : undefined,
      prompt: v.prompt,
      worktree: v.worktree,
      branch: v.branch,
      tile: v.tile,
    };
    if (how === "copy") {
      if (!(await copyText(startCommand(values)))) return;
      await showToast({ style: Toast.Style.Success, title: "Copied the command" });
      return;
    }
    // an agent that isn't on this machine yet: ask, install, then start
    const info = report.data?.hosts
      .find((h) => h.alias === v.host && h.reachable)
      ?.agents.find((a) => a.id === v.agent);
    if (info && !info.installed) {
      if (!(await confirmInstall(info, v.host, "Install and Start"))) return;
      if (!(await installWithToast(info, v.host))) return;
      // aleph reuses its agent answers for 10 minutes: look again so the form knows
      report.mutate(agentsReport(undefined, true)).catch(() => undefined);
    }
    const args = startArgs(values);
    if (how === "terminal") {
      await openInTerminal(args, `Aleph · ${v.host}`);
      return;
    }
    await startSession(args, {
      host: v.host,
      name: v.name,
      where: d ? shortPath(d) : v.host,
      detach: !!detach.data,
    });
  }

  const agentItem = (id: string) => {
    const a = onHost?.agents.find((x) => x.id === id);
    const b = brand(id, a);
    return (
      <Form.Dropdown.Item
        key={id}
        value={id}
        title={agentTitle(b.name, a?.installed ? a.version : undefined)}
        icon={agentLogo(id)}
        keywords={[id, a?.command ?? ""]}
      />
    );
  };
  const missing = onHost?.agents.find((x) => x.id === agent && !x.installed);
  const hostIcon = (h: Host): Image.ImageLike => {
    if (!h.reachable) return { source: Icon.WifiDisabled, tintColor: DOWN };
    const a = report.data?.hosts.find((x) => x.alias === h.alias && x.reachable);
    return (
      osLogo(a?.os_name, a?.os)?.icon ??
      machineLogo(h.alias, h.os)?.icon ?? { source: Icon.CircleFilled, tintColor: h.color || undefined }
    );
  };

  return (
    <Form
      isLoading={fleet.isLoading && !fleet.data}
      navigationTitle={current ? `Start on ${current}` : "Start Session"}
      actions={
        <ActionPanel>
          <Action.SubmitForm title="Start Session" icon={Icon.Play} onSubmit={(v: Values) => submit(v, "start")} />
          <Action.SubmitForm
            title="Start in Terminal"
            icon={Icon.Terminal}
            shortcut={{ modifiers: ["cmd", "shift"], key: "return" }}
            onSubmit={(v: Values) => submit(v, "terminal")}
          />
          <Action.SubmitForm
            title="Copy Command"
            icon={Icon.CopyClipboard}
            shortcut={Keyboard.Shortcut.Common.Copy}
            onSubmit={(v: Values) => submit(v, "copy")}
          />
        </ActionPanel>
      }
    >
      {(presetList.data?.length ?? 0) > 0 && (
        <Form.Dropdown id="preset" title="Preset" value={preset} onChange={setPreset}>
          <Form.Dropdown.Item value={NONE} title="None" icon={Icon.Minus} />
          {presetList.data?.map((p) => (
            <Form.Dropdown.Item
              key={p.name}
              value={p.name}
              title={`@${p.name}`}
              icon={p.agent ? agentLogo(p.agent) : { source: Icon.Bookmark, tintColor: ACCENT }}
              keywords={[p.host ?? "", p.dir ?? "", p.agent ?? ""]}
            />
          ))}
        </Form.Dropdown>
      )}
      {/* controlled from the start: "" until the machines are known */}
      <Form.Dropdown id="host" title="Machine" value={current ?? ""} onChange={pickHost}>
        {!current && <Form.Dropdown.Item value="" title="Looking at your machines…" icon={Icon.CircleProgress} />}
        {hosts.map((h) => (
          <Form.Dropdown.Item
            key={h.alias}
            value={h.alias}
            title={hostTitle(h)}
            icon={hostIcon(h)}
            keywords={[h.address]}
          />
        ))}
        {current && !hosts.some((h) => h.alias === current) && <Form.Dropdown.Item value={current} title={current} />}
      </Form.Dropdown>
      <Form.Dropdown id="agent" title="Agent" value={agent} onChange={setAgent}>
        <Form.Dropdown.Item
          key={AGENT_DEFAULT}
          value={AGENT_DEFAULT}
          title={defaultAgent ? `${brand(defaultAgent).name} · default` : "Default agent"}
          icon={defaultAgent ? agentLogo(defaultAgent) : Icon.Dot}
        />
        {agent !== AGENT_DEFAULT && !listed.has(agent) ? agentItem(agent) : null}
        <Form.Dropdown.Item
          key="shell"
          value="shell"
          title="Terminal · no agent"
          icon={Icon.Terminal}
          keywords={["shell", "terminal", "bash", "zsh", "commands"]}
        />
        {onHost ? (
          <>
            <Form.Dropdown.Section title={`On ${current}`}>
              {installed.filter((a) => a.id !== "shell").map((a) => agentItem(a.id))}
            </Form.Dropdown.Section>
            {installable.length > 0 && (
              <Form.Dropdown.Section title="Installs first (asks)">
                {installable.map((a) => agentItem(a.id))}
              </Form.Dropdown.Section>
            )}
          </>
        ) : (
          agentNames.map((a) => agentItem(a))
        )}
      </Form.Dropdown>
      {missing ? <Form.Description title="Installs First" text={`$ ${missing.install_command ?? ""}`} /> : null}
      <Form.Dropdown id="folder" title="Folder" value={folder} onChange={setFolder}>
        <Form.Dropdown.Item value={DEFAULT} title="Default folder" icon={Icon.House} />
        {hostDirs.length > 0 && (
          <Form.Dropdown.Section title="Recent">
            {hostDirs.map((d) => (
              <Form.Dropdown.Item
                key={d.dir}
                value={d.dir}
                title={shortPath(d.dir)}
                icon={Icon.Folder}
                keywords={[d.dir]}
              />
            ))}
          </Form.Dropdown.Section>
        )}
        <Form.Dropdown.Item value={OTHER} title="Other…" icon={Icon.Pencil} />
      </Form.Dropdown>
      {folder === OTHER && (
        <Form.TextField
          id="dir"
          title="Path"
          placeholder="/home/me/project"
          value={dir}
          error={dirError}
          onChange={(x) => {
            setDir(x);
            setDirError(undefined);
          }}
        />
      )}
      <Form.TextArea
        id="prompt"
        title="First Prompt"
        placeholder="optional — what should it start on?"
        value={prompt}
        onChange={setPrompt}
      />
      <Form.Separator />
      <Form.TextField
        id="name"
        title="Name"
        placeholder="picked for you"
        error={nameError}
        onChange={() => setNameError(undefined)}
      />
      <Form.Checkbox id="worktree" label="In a new git worktree" value={worktree} onChange={setWorktree} />
      {worktree && <Form.TextField id="branch" title="Branch" placeholder="<agent>/<name>" />}
      <Form.Dropdown id="tile" title="Tile" defaultValue="1">
        {Array.from({ length: 9 }, (_, i) => String(i + 1)).map((n) => (
          <Form.Dropdown.Item
            key={n}
            value={n}
            title={n === "1" ? "Just this one" : `${n} agents, side by side`}
            icon={n === "1" ? Icon.Window : Icon.AppWindowGrid2x2}
          />
        ))}
      </Form.Dropdown>
      {!detach.data && !detach.isLoading && <Form.Description text="Opens in your terminal." />}
    </Form>
  );
}
