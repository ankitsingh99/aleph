// Installing a missing agent on a machine (`aleph agents install <id>
// --host H --yes`). Never without an explicit confirmation: the Alert shows the
// exact command and what it needs first. The install can take minutes (npm,
// curl | bash, uv), so its output follows in a toast (and the install view).
import { confirmAlert, showToast, Toast } from "@raycast/api";
import { AgentInfo, installAgent, stripAnsi } from "./cli";
import { agentLogo } from "./logos";
import { errorText } from "./ui";

/** Why this agent can't be installed from here, or undefined when it can. */
export function installBlocker(a: AgentInfo): string | undefined {
  if (a.installed) return `${a.name} is already installed`;
  if (a.install_ready) return undefined;
  if (a.install_blocker) return a.install_blocker;
  const missing = (a.prerequisites ?? []).filter((p) => !p.ok);
  if (missing.length) return missing.map((p) => `${p.what}: ${p.fix ?? "missing"}`).join("\n");
  return a.install_command ? undefined : `${a.name} has no install command aleph can run`;
}

/** A link in the blocker ("… has no install command we can run — see https://…"). */
export function installDocs(a: AgentInfo): string | undefined {
  return /https?:\/\/\S+/.exec(a.install_blocker ?? "")?.[0]?.replace(/[).,]+$/, "");
}

/** Ask before installing: the exact command, its prerequisites, where it runs. */
export async function confirmInstall(a: AgentInfo, host: string, primary = "Install"): Promise<boolean> {
  const blocked = installBlocker(a);
  if (blocked) {
    await showToast({ style: Toast.Style.Failure, title: `Can't install ${a.name} on ${host}`, message: blocked });
    return false;
  }
  const needs = (a.prerequisites ?? []).map((p) => `${p.ok ? "✓" : "✕"} ${p.what}`).join("   ");
  return confirmAlert({
    title: `Install ${a.name} on ${host}?`,
    message: [
      a.install_command ? `$ ${a.install_command}` : "",
      needs,
      "Runs as you on that machine, never with sudo. It can take a few minutes.",
    ]
      .filter(Boolean)
      .join("\n\n"),
    icon: agentLogo(a.id),
    primaryAction: { title: primary },
  });
}

/** The last line of output worth showing in a toast. */
export function lastLine(out: string): string {
  const lines = stripAnsi(out)
    .split(/[\r\n]+/)
    .map((l) => l.trim())
    .filter(Boolean);
  return (lines[lines.length - 1] ?? "").slice(0, 120);
}

/** Install with a toast that follows the output. Resolves true when it's installed. Ask first (`confirmInstall`). */
export async function installWithToast(a: AgentInfo, host: string, onOutput?: (all: string) => void): Promise<boolean> {
  const toast = await showToast({
    style: Toast.Style.Animated,
    title: `Installing ${a.name} on ${host}…`,
    message: a.install_command ?? undefined,
  });
  let all = "";
  let last = 0;
  try {
    await installAgent(a.id, host, (chunk) => {
      all += chunk;
      onOutput?.(all);
      const now = Date.now();
      if (now - last > 300) {
        last = now;
        const l = lastLine(all);
        if (l) toast.message = l;
      }
    });
    toast.style = Toast.Style.Success;
    toast.title = `${a.name} installed on ${host}`;
    toast.message = lastLine(all) || undefined;
    return true;
  } catch (e) {
    const { title, message } = errorText(e);
    toast.style = Toast.Style.Failure;
    toast.title = `Couldn't install ${a.name} on ${host}`;
    toast.message = [title, message].filter(Boolean).join(" — ");
    const sep = all && !all.endsWith("\n") ? "\n" : "";
    onOutput?.(`${all}${sep}error: ${title}${message ? `\nhint: ${message}` : ""}\n`);
    return false;
  }
}
