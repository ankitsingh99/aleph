// Opening things in a terminal tab: a session in Aleph's Focus view
// (`aleph focus host:name`), a machine's dashboard (`aleph --host H`), an
// attach in herdr (`aleph --host H resume <pane>`), new sessions, installs.
//
// One implementation, shared with the dashboard's "Open in herdr ↗":
// `aleph open-tab [--terminal T] -- <command…>` (Warp, iTerm2, Terminal,
// Ghostty, kitty, WezTerm). The launchers below are only the fallback, for a
// build without open-tab or when it fails.
import { closeMainWindow, open, showToast, Toast } from "@raycast/api";
import { execFile } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { binary, commands, Preferences, prefs, run, shQuote } from "./cli";

type Terminal = NonNullable<Preferences["terminal"]>;

/** Warp tab config the fallback writes (`aleph open-tab` writes its own, `aleph_tab`). */
const TAB = "aleph_raycast";

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

function quoted(s: string): string {
  return `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function launch(file: string, args: string[]): Promise<void> {
  return new Promise((resolve, reject) => {
    execFile(file, args, { timeout: 15_000 }, (err, _out, stderr) =>
      err ? reject(new Error(stderr.trim() || err.message)) : resolve(),
    );
  });
}

const osascript = (script: string) => launch("/usr/bin/osascript", ["-e", script]);
const shell = () => process.env.SHELL || "/bin/zsh";
/** Keep a shell after the command, so its errors stay on screen. */
const andStay = (cmd: string) => `${cmd}; exec ${shell()} -l`;

async function warp(cmd: string, title: string) {
  const dir = join(homedir(), ".warp", "tab_configs");
  mkdirSync(dir, { recursive: true });
  const toml = [
    "# managed by aleph (Raycast extension) — rewritten on every launch",
    `name = ${quoted(title)}`,
    "",
    "[[panes]]",
    'id = "main"',
    'type = "terminal"',
    `commands = [${quoted(cmd)}]`,
    "is_focused = true",
    "",
  ].join("\n");
  writeFileSync(join(dir, `${TAB}.toml`), toml);
  await open(`warp://tab_config/${TAB}`);
}

/** A new tab in iTerm2's front window (a new window when none is open). */
function iterm(cmd: string) {
  return osascript(
    [
      'tell application "iTerm"',
      "activate",
      "if (count of windows) = 0 then",
      "create window with default profile",
      "else",
      "tell current window to create tab with default profile",
      "end if",
      `tell current session of current window to write text ${quoted(cmd)}`,
      "end tell",
    ].join("\n"),
  );
}

function terminalApp(cmd: string) {
  return osascript(['tell application "Terminal"', "activate", `do script ${quoted(cmd)}`, "end tell"].join("\n"));
}

function ghostty(cmd: string) {
  return launch("/usr/bin/open", ["-na", "Ghostty", "--args", "-e", shell(), "-lc", andStay(cmd)]);
}

/** A CLI that lives inside its app bundle when it isn't on PATH. */
function appCli(name: string, bundled: string): string {
  return existsSync(bundled) ? bundled : name;
}

/** kitty needs `allow_remote_control` (and a socket) for `kitty @`. */
function kitty(cmd: string, title: string) {
  const bin = appCli("kitty", "/Applications/kitty.app/Contents/MacOS/kitty");
  return launch(bin, ["@", "launch", "--type=tab", "--tab-title", title, shell(), "-lc", andStay(cmd)]);
}

function wezterm(cmd: string) {
  const bin = appCli("wezterm", "/Applications/WezTerm.app/Contents/MacOS/wezterm");
  return launch(bin, ["cli", "spawn", "--", shell(), "-lc", andStay(cmd)]);
}

/** The extension's own launchers: only when `aleph open-tab` isn't there or failed. */
async function fallback(cmd: string, title: string, t: Terminal) {
  switch (t) {
    case "iterm":
      return iterm(cmd);
    case "ghostty":
      return ghostty(cmd);
    case "kitty":
      return kitty(cmd, title);
    case "wezterm":
      return wezterm(cmd);
    case "warp":
      return warp(cmd, title);
    default:
      // `auto`: Aleph's own choice outside a terminal is Terminal.app too
      return terminalApp(cmd);
  }
}

/** `aleph open-tab [--terminal T] -- <words…>`: the dashboard's own launcher. */
async function viaAleph(words: string[], t: Terminal) {
  const known = await commands().catch(() => new Set<string>());
  if (!known.has("open-tab")) throw new Error("this aleph has no open-tab (aleph self-update)");
  await run(["open-tab", ...(t === "auto" ? [] : ["--terminal", t]), "--", ...words], 30_000);
}

/**
 * Run `words` (a program and its arguments) in a new tab of the terminal from
 * the preferences, through `aleph open-tab`; the fallback launchers if that
 * fails. Resolves false (with a toast) when neither could open it.
 */
export async function openTab(words: string[], title = "Aleph"): Promise<boolean> {
  const t = prefs().terminal ?? "warp";
  try {
    await viaAleph(words, t);
  } catch (first) {
    try {
      await fallback(words.map(shQuote).join(" "), title, t);
    } catch (second) {
      await showToast({
        style: Toast.Style.Failure,
        title: "Couldn't open the terminal",
        message: [message(first), message(second)].filter((m, i, v) => m && v.indexOf(m) === i).join(" · "),
      });
      return false;
    }
  }
  await closeMainWindow({ clearRootSearch: true }).catch(() => undefined);
  return true;
}

/** `aleph <args…>` in a new terminal tab. */
export function openInTerminal(args: string[], title = "Aleph"): Promise<boolean> {
  return openTab([binary(), ...args], title);
}

/** A session to open: `sel` is `host:pane` (exact) or `host:name`. */
export interface Target {
  host: string;
  sel: string;
  label: string;
}

/** What opening a session does by default: Aleph's Focus view, or attaching in herdr. */
export const openWith = (): "aleph" | "herdr" => (prefs().openWith === "herdr" ? "herdr" : "aleph");

/** Aleph's Focus view on one session (`aleph focus host:name`); the machine's dashboard on builds without focus. */
export async function openInAleph(t: Target): Promise<boolean> {
  const known = await commands().catch(() => new Set<string>());
  if (!known.has("focus")) return openMachine(t.host);
  return openInTerminal(["focus", t.sel], `Aleph · ${t.label}`);
}

/** Attach in herdr: `aleph --host H resume <sel>` (the launch card, then herdr). */
export function attachInHerdr(t: Target): Promise<boolean> {
  return openInTerminal(["--host", t.host, "resume", t.sel], `herdr · ${t.label}`);
}

/** Open a session the way the `openWith` preference says. */
export function openSession(t: Target): Promise<boolean> {
  return openWith() === "herdr" ? attachInHerdr(t) : openInAleph(t);
}

/** A machine in Aleph: the dashboard with just that machine (`aleph --host H`). */
export function openMachine(host: string): Promise<boolean> {
  return openInTerminal(["--host", host], `Aleph · ${host}`);
}

/** herdr's own window on a machine: `herdr --remote H` (plain `herdr` for this computer). */
export function openHerdrOn(host: string): Promise<boolean> {
  return openTab(host === "local" ? ["herdr"] : ["herdr", "--remote", host], `herdr · ${host}`);
}

/** The Aleph dashboard (every machine). */
export function openDashboard(): Promise<boolean> {
  return openInTerminal([], "Aleph");
}
