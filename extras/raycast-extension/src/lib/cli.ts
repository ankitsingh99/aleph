// Talking to the aleph binary (Aleph, formerly claude-herdr). Everything goes
// through execFile (no shell), stdin closed (so aleph takes its piped/JSON
// paths), a timeout on every call, and friendly errors. JSON shapes: aleph's
// --json output (ls, status, peek, stats, inbox, queue, agents, services,
// history, notes and doctor), each checked against the binary.
//
// The fleet snapshot, the inbox, the queue, usage and the daemon's status come
// from the daemon's API first (api.ts: its socket, no process to start, no
// machine to ask), and from the CLI's `--json` when no daemon answers.
import { environment, getPreferenceValues } from "@raycast/api";
import { execFile } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, statSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { act, ApiCall, apiOr, request, useRemote } from "./api";
import { asClose, CloseAnswer } from "./closing";
import { since, STATUS_LABEL } from "./fmt";
import { DoctorReport, explainError, startedLines } from "./model";

export { explainError };

export interface Preferences {
  /** `aleph` (the default), the old `claude-herdr`, or a full path */
  binaryPath?: string;
  /** where new tabs open; `auto` leaves it to Aleph's `terminal` setting */
  terminal?: "warp" | "iterm" | "terminal" | "ghostty" | "kitty" | "wezterm" | "auto";
  /** what opening a session does: Aleph's Focus view, or attach in herdr */
  openWith?: "aleph" | "herdr";
  /** a daemon on another computer: its HTTP API, and a device token for it */
  apiUrl?: string;
  apiToken?: string;
}

// ---------------------------------------------------------------- JSON contract

/** One session in `ls --json`. */
export interface Session {
  host: string;
  name: string;
  label: string;
  display: string;
  pane_id: string;
  tab_id: string;
  workspace_id: string;
  terminal_id: string;
  agent: string;
  status: "working" | "blocked" | "done" | "idle" | string;
  status_since: number | null;
  cwd: string;
  title: string;
  mark: string;
  hue: string;
  glyph: string;
  color: string;
  focused: boolean;
  /** stopped at its plan's usage limit: when it resets, as the agent said ("2:40am (Asia/Kolkata)", "in 2 hours"), "" when it didn't say */
  limit?: string | null;
}

/** One machine in `ls --json`. */
export interface Host {
  alias: string;
  machine: string;
  color: string;
  address: string;
  reachable: boolean;
  error: string | null;
  stale: boolean;
  fetched_at: number | null;
  latency_ms: number | null;
  /** herdr on that machine */
  herdr_version?: string | null;
  /** its herdr differs from (or isn't compatible with) this computer's */
  herdr_drift?: boolean;
  herdr_compatible?: boolean | null;
  herdr_restart_needed?: boolean;
  /** `Linux · Ubuntu 24.04.3 LTS` (newer aleph): what the machine is, from its herdr check */
  os?: string | null;
  sessions: Session[];
}

export interface Fleet {
  version: number;
  generated_at: number;
  hosts: Host[];
}

/** `recent --json`. */
export interface RecentDir {
  host: string;
  dir: string;
  count: number;
  last: number;
  score: number;
}

/** `preset ls --json`. */
export interface Preset {
  name: string;
  /** the first pane's fields (presets with several panes have more in `panes`) */
  host: string | null;
  dir: string | null;
  args: string | null;
  agent: string | null;
  worktree: boolean;
  prompt: string | null;
  /** newer builds: one session or several, on one machine or many */
  description?: string;
  /** tabs · grid · columns · rows */
  layout?: string;
  /** the pane opened in Focus afterwards */
  focus?: string | null;
  tags?: string[];
  /** where it's defined (aleph.toml, an included file) */
  source?: string;
  panes?: PresetPane[];
}

/** One session a preset starts. */
export interface PresetPane {
  name: string | null;
  /** an alias, `tag:gpu`, `local` or `${var}`; null: the default machine */
  host: string | null;
  agent: string | null;
  dir: string | null;
  /** null: none · `auto` · a branch */
  worktree: string | null;
  args: string[];
  prompt: string | null;
}

/** `peek <host:name> --json`: the session's fields plus its screen. */
export interface Peek {
  host: string;
  name: string;
  status: string;
  text: string;
  agent?: string;
  cwd?: string;
  title?: string;
  pane_id?: string;
  limit?: string | null;
  /** the question it's asking, when blocked */
  question?: string | null;
  /** what `approve` would send */
  approve_keys?: string[] | null;
}

export type InboxKind =
  "blocked" | "done" | "new" | "gone" | "error" | "auto" | "task" | "heal" | "limit" | "up" | "remote";

/** `inbox --json`, newest first. `name` is empty for machine events (error, heal). */
export interface InboxEvent {
  t: number;
  host: string;
  name: string;
  kind: InboxKind | string;
  title: string;
  agent?: string;
  /** a stable id (newer builds) */
  id?: string;
  /** who said so: absent (herdr's view), `agent` (its own hook, `aleph hooks`), `aleph` */
  source?: string;
  /** the agent's own words, when it said more than the title */
  body?: string;
  /** newer than the inbox's read mark (only the daemon's `inbox.list` says) */
  unread?: boolean;
}

/** `queue ls --json`. */
export interface QueueTask {
  id: number;
  host: string;
  prompt: string;
  dir: string | null;
  agent: string | null;
  new: boolean;
  /** only this session (by name) takes it, once it's done with its reply */
  to: string | null;
  added: number;
  state: "queued" | "sent" | "failed" | string;
  session: string | null;
  at: number | null;
  error: string | null;
  /** not before this unix time (`send --after 30m | 14:30`) */
  after?: number | null;
}

/** `history --json`, newest first. */
export interface HistoryEntry {
  t: number;
  host: string;
  dir: string;
  name: string;
  agent: string;
  args: string[];
  prompt: string | null;
  worktree: boolean;
}

/** One agent on one machine in `agents --json`. */
export interface AgentInfo {
  id: string;
  name: string;
  /** herdr's agent kind (null for agents herdr detects by integration only) */
  kind: string | null;
  glyph: string;
  color: string;
  command: string;
  installed: boolean;
  path: string | null;
  version: string | null;
  /** verified · plausible · unverified (installed only) */
  check: string | null;
  on_path: boolean | null;
  shadowed_by: string | null;
  start?: string;
  integration?: string | null;
  detect?: string;
  session_logs?: string | null;
  /** current · outdated · not_installed · … */
  integration_state?: string | null;
  /** its sessions come back after a herdr restart */
  persistence?: boolean;
  state?: string | null;
  resume?: string[];
  install_method?: string;
  install_command?: string | null;
  install_ready?: boolean;
  install_blocker?: string | null;
  prerequisites?: { what: string; ok: boolean; detail: string | null; fix: string | null }[];
}

export interface AgentsHost {
  alias: string;
  reachable: boolean;
  error: string | null;
  /** `Linux x86_64`, `Darwin arm64` */
  os?: string | null;
  /** `Ubuntu 24.04.3 LTS`, `macOS 15.1` */
  os_name?: string | null;
  /** `2× NVIDIA GeForce RTX 4090`, `Apple GPU` */
  gpu?: string | null;
  gpus?: string[];
  home?: string | null;
  cached?: boolean;
  age_secs?: number | null;
  herdr?: { path: string | null; version: string | null } | null;
  agents: AgentInfo[];
}

export interface AgentsReport {
  hosts: AgentsHost[];
}

export interface ServiceModel {
  name: string;
  loaded: boolean;
  kind: string | null;
  params: string | null;
  quant: string | null;
  format: string | null;
  ctx: number | null;
  size: number | null;
  vram: number | null;
  mem: number | null;
  gpu_share: number | null;
}

/** One server found by `services --json`. */
export interface Service {
  id: string;
  name: string;
  /** llm-server · gateway · ui */
  kind: string;
  glyph: string;
  color: string;
  homepage: string | null;
  version: string | null;
  /** up · loading · error · down · not_listening · unknown */
  health: string;
  health_label: string;
  health_detail: string;
  url: string | null;
  openai: boolean;
  openai_base: string | null;
  auth_required: boolean;
  addr: string | null;
  bind: string | null;
  port: number | null;
  ports: number[];
  pid: number | null;
  process: string | null;
  found_by: string[];
  models: ServiceModel[];
  model_count: number;
  loaded_count: number;
  vram: number | null;
  kv_cache: number | null;
  requests_running: number | null;
  requests_waiting: number | null;
}

export interface ServicesHost {
  alias: string;
  color: string;
  reachable: boolean;
  error: string | null;
  stale?: boolean;
  partial?: boolean;
  at?: number | null;
  took_ms?: number | null;
  services: Service[];
}

export interface ServicesReport {
  generated_at: number;
  /** `services=on`: the dashboard keeps them live. Off, they're only scanned on demand (like now). */
  enabled: boolean;
  interval?: number;
  services?: string[];
  hosts: ServicesHost[];
}

export interface Gpu {
  /** `0`, `1` (a string from aleph; a number from older builds) */
  index: number | string;
  name: string;
  vendor?: string;
  util: number;
  mem_used: number;
  mem_total: number;
  temp: number | null;
  power: number | null;
  power_limit: number | null;
  /** newer builds (statscmd.rs gpu_json); null when the tool didn't say */
  mem_util?: number | null;
  temp_mem?: number | null;
  /** percent */
  fan?: number | null;
  clock_sm?: number | null;
  clock_sm_max?: number | null;
  pstate?: string | null;
  /** active clock-throttle reasons (`power cap`, `thermal`) */
  throttle?: string[] | null;
}

/** `stats --json [--host H]`. Fractions 0..1 for cpu/util, bytes for memory. */
export interface MachineStats {
  alias: string;
  color: string;
  reachable: boolean;
  error: string | null;
  hostname?: string;
  /** `Ubuntu 24.04.3 LTS`, `macOS 15.1` */
  os?: string;
  /** linux · macos */
  platform?: string;
  kernel?: string;
  cpu_model?: string;
  ncpu?: number;
  uptime_s?: number;
  load?: number[];
  cpu?: number;
  cpu_temp?: number | null;
  cores?: number[];
  mem_used?: number;
  mem_total?: number;
  swap_used?: number;
  swap_total?: number;
  net_rx?: number;
  net_tx?: number;
  disks?: { mount: string; used: number; total: number }[];
  gpus?: Gpu[];
  /** why there are no GPU stats (e.g. no nvidia-smi) */
  gpu_note?: string | null;
  gpu_procs?: { gpu: number; pid: number | string; mem: number; user: string; name: string }[];
  top?: { pid: number | string; user: string; cpu: number; rss: number; name: string }[];
  claude_procs?: number;
  claude_rss?: number;
  convos_today?: number | null;
  agent_procs?: { agent: string; count: number; rss: number }[];
  sessions_today?: { agent: string; count: number }[];
  procs_total?: number;
  procs_running?: number;
  threads_total?: number;
  users_logged?: number;
  mem_available?: number;
  mem_cached?: number;
  /** fractions of all CPU time */
  cpu_split?: { user?: number; system?: number; iowait?: number; steal?: number };
  /** every sensor read: label · °C */
  temps?: { label: string; c: number }[];
  /** per interface, bytes per second */
  nets?: { name: string; rx: number; tx: number }[];
  /** per disk, bytes per second; busy 0..1 when known */
  disk_io?: { name: string; read: number; write: number; busy: number | null }[];
}

export interface Stats {
  generated_at: number;
  hosts: MachineStats[];
}

// ---------------------------------------------------------------- running it

/** The binary's name, and its name before the rename (now a link to `aleph`). */
export const BIN = "aleph";
export const LEGACY_BIN = "claude-herdr";

/** Where installers put it: cargo, Homebrew (Apple silicon, Intel), ~/.local/bin. */
const DIRS = [
  join(homedir(), ".cargo", "bin"),
  "/opt/homebrew/bin",
  "/usr/local/bin",
  join(homedir(), ".local", "bin"),
];

export function prefs(): Preferences {
  const p = getPreferenceValues<Preferences>();
  useRemote(p.apiUrl, p.apiToken);
  return p;
}

// the daemon to ask, before anything asks it
try {
  prefs();
} catch {
  // outside Raycast (tests): the local socket
}

/**
 * The aleph binary. The preference is a name (`aleph`, the default, or the old
 * `claude-herdr`), looked up in the usual install places and then PATH, or a
 * full path, used as it is. A build from before the rename is still found.
 */
export function binary(): string {
  const p = prefs().binaryPath?.trim() || BIN;
  if (p.includes("/")) return p.startsWith("~/") ? join(homedir(), p.slice(2)) : p;
  const names = p === BIN ? [BIN, LEGACY_BIN] : p === LEGACY_BIN ? [LEGACY_BIN, BIN] : [p];
  for (const n of names) {
    for (const d of DIRS) {
      const c = join(d, n);
      if (existsSync(c)) return c;
    }
  }
  return p;
}

const GUARD_MARK = "aleph-raycast: passthrough refused";

/** Set once `__complete commands` shows a build that honours the NO_PASSTHROUGH variable (it has `services`). */
let honoursNoPassthrough = false;

/**
 * As the opt-in `claude` wrapper, aleph hands what isn't its own command to
 * the agent it stands in for (`wraps`, Claude Code by default). From Raycast
 * that must never happen: ALEPH_NO_PASSTHROUGH (CLAUDE_HERDR_NO_PASSTHROUGH
 * for old builds) makes it refuse, ALEPH_WRAP=0 turns the wrapper off, and for
 * the oldest builds a fake `claude` that refuses goes first on PATH.
 */
function guardDir(): string {
  const dir = join(environment.supportPath, "guard");
  const f = join(dir, "claude");
  if (!existsSync(f)) {
    mkdirSync(dir, { recursive: true });
    writeFileSync(f, `#!/bin/sh\necho "${GUARD_MARK}" >&2\nexit 97\n`);
    chmodSync(f, 0o755);
  }
  return dir;
}

/**
 * Raycast starts commands with a bare PATH; aleph needs herdr and ssh.
 * Your own bins come first, like in a login shell: herdr's installer puts it in
 * ~/.local/bin, and an older Homebrew herdr mustn't win over it.
 */
function env(): NodeJS.ProcessEnv {
  const extra = [
    // builds that honour NO_PASSTHROUGH don't need the stub (and `agents` on
    // this computer shouldn't find it ahead of the real Claude Code)
    ...(honoursNoPassthrough ? [] : [guardDir()]),
    join(homedir(), ".local", "bin"),
    join(homedir(), ".cargo", "bin"),
    "/opt/homebrew/bin",
    "/usr/local/bin",
  ];
  const path = [...extra, ...(process.env.PATH ?? "/usr/bin:/bin:/usr/sbin:/sbin").split(":")];
  return {
    ...process.env,
    PATH: [...new Set(path)].join(":"),
    NO_COLOR: "1",
    TERM: "dumb",
    ALEPH_NO_PASSTHROUGH: "1",
    ALEPH_WRAP: "0",
    CLAUDE_HERDR_NO_PASSTHROUGH: "1",
  };
}

export class CliError extends Error {
  constructor(
    message: string,
    public hint?: string,
    public unsupported = false,
    /** the binary isn't there (or can't run) */
    public missing = false,
  ) {
    super(message);
  }
}

/** `error: …` / `hint: …` lines from aleph's stderr. */
function explain(args: string[], stderr: string, fallback: string): CliError {
  if (stderr.includes(GUARD_MARK) || stderr.includes("passthrough refused")) {
    return new CliError(
      `This aleph can't run \`${args.find((a) => !a.startsWith("-")) ?? args[0]}\` from Raycast`,
      "update Aleph (aleph self-update)",
      true,
    );
  }
  const lines = stderr.split("\n").map((l) => l.trim());
  const pick = (p: string) =>
    lines
      .find((l) => l.startsWith(p))
      ?.slice(p.length)
      .trim();
  const msg = pick("error:") ?? (lines.filter(Boolean).pop() || fallback);
  const unsupported = /isn't built yet|unknown command/.test(stderr);
  return new CliError(msg, pick("hint:"), unsupported);
}

type Done = (error: Error | null, stdout: string, stderr: string) => void;

function exec(args: string[], timeoutMs: number, done: Done) {
  const child = execFile(
    binary(),
    args,
    { env: env(), timeout: timeoutMs, maxBuffer: 32 * 1024 * 1024, encoding: "utf8" },
    done,
  );
  child.stdin?.end();
  return child;
}

function failure(args: string[], error: Error, stderr: string): CliError {
  const e = error as NodeJS.ErrnoException & { killed?: boolean };
  if (e.code === "ENOENT" || e.code === "EACCES") {
    return new CliError(
      `aleph not found: ${binary()}`,
      "set “Aleph Binary” in the extension's preferences (aleph, or a full path)",
      false,
      true,
    );
  }
  if (e.killed) {
    return new CliError(`aleph ${args.join(" ")} timed out`, "a machine may be slow or offline");
  }
  return explain(args, stderr, e.message);
}

export function run(args: string[], timeoutMs = 30_000): Promise<string> {
  return new Promise((resolve, reject) => {
    exec(args, timeoutMs, (error, stdout, stderr) => (error ? reject(failure(args, error, stderr)) : resolve(stdout)));
  });
}

/** Like `run`, with stderr too: aleph prints its `✓` lines there. */
export function runOutput(args: string[], timeoutMs = 30_000): Promise<{ stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    exec(args, timeoutMs, (error, stdout, stderr) =>
      error ? reject(failure(args, error, stderr)) : resolve({ stdout, stderr }),
    );
  });
}

/**
 * Like `run`, but a command that exits non-zero after printing its JSON still
 * counts (`doctor --json` exits 1 when a check fails, `agents --json` when no
 * machine answers): resolves with stdout when it parses.
 */
function runLenient(args: string[], timeoutMs = 30_000): Promise<string> {
  return new Promise((resolve, reject) => {
    exec(args, timeoutMs, (error, stdout, stderr) => {
      if (!error) return resolve(stdout);
      const killed = (error as { killed?: boolean }).killed;
      if (!killed && stdout.trim()) {
        try {
          JSON.parse(stdout);
          return resolve(stdout);
        } catch {
          // not JSON: report the failure below
        }
      }
      reject(failure(args, error, stderr));
    });
  });
}

/** Like `run`, but stdout and stderr also reach `onOutput` as they arrive (long installs). */
export function runStreaming(args: string[], onOutput: (chunk: string) => void, timeoutMs: number): Promise<string> {
  return new Promise((resolve, reject) => {
    let all = "";
    const child = exec(args, timeoutMs, (error, stdout, stderr) =>
      error ? reject(failure(args, error, stderr || all)) : resolve(all || stdout),
    );
    const take = (d: Buffer | string) => {
      const s = d.toString();
      all += s;
      onOutput(s);
    };
    child.stdout?.on("data", take);
    child.stderr?.on("data", take);
  });
}

/** `aleph <args…>` → parsed JSON. `lenient`: a non-zero exit with JSON on stdout is fine. */
export async function runJson<T>(args: string[], timeoutMs?: number, lenient = false): Promise<T> {
  const out = await (lenient ? runLenient : run)(args, timeoutMs);
  try {
    return JSON.parse(out) as T;
  } catch {
    throw new CliError(`aleph ${args.join(" ")} didn't return JSON`, out.trim().slice(0, 160));
  }
}

let commandList: Promise<Set<string>> | undefined;

/** Commands this aleph knows (`__complete commands`, offline and fast). */
export function commands(): Promise<Set<string>> {
  commandList ??= run(["__complete", "commands"], 10_000)
    .then((o) => {
      const set = new Set(
        o
          .split("\n")
          .map((l) => l.trim())
          .filter(Boolean),
      );
      honoursNoPassthrough = set.has("services");
      return set;
    })
    .catch((e) => {
      commandList = undefined;
      throw e;
    });
  return commandList;
}

/** Run a JSON command only if this binary has it; `undefined` otherwise. */
export async function optionalJson<T>(
  cmd: string,
  args: string[],
  timeoutMs?: number,
  lenient = false,
): Promise<T | undefined> {
  if (!(await commands()).has(cmd)) return undefined;
  try {
    return await runJson<T>(args, timeoutMs, lenient);
  } catch (e) {
    if (e instanceof CliError && e.unsupported) return undefined;
    throw e;
  }
}

const helps = new Map<string, Promise<string>>();

/** Does `aleph help <cmd>` mention `flag`? (feature detection; each help is read once per command run) */
/** The daemon's methods (`api.methods`), asked once per command run. */
let methodsOnce: Promise<Set<string>> | undefined;
function daemonMethods(): Promise<Set<string>> {
  methodsOnce ??= request<{ name: string }[]>("api.methods", {}, { timeoutMs: 2000 })
    .then((v) => new Set(v.map((m) => m.name)))
    .catch(() => new Set<string>());
  return methodsOnce;
}

export async function hasFlag(cmd: string, flag: string): Promise<boolean> {
  // a daemon of this round's build: every flag the extension asks about is there
  if ((await daemonMethods()).has("usage.now")) return true;
  let h = helps.get(cmd);
  if (!h) {
    h = run(["help", cmd], 10_000);
    helps.set(cmd, h);
    h.catch(() => helps.delete(cmd));
  }
  try {
    return (await h).includes(flag);
  } catch {
    return false;
  }
}

/** `--host H` goes before the command (aleph's global flag). */
export const onHost = (host: string | undefined, args: string[]) => (host ? ["--host", host, ...args] : args);

// ---------------------------------------------------------------- calls

/** The daemon's fleet snapshot (what `ls --json` prints), once it has polled. */
export const FLEET_API: ApiCall<Fleet> = {
  method: "state.snapshot",
  accept: (f) => Array.isArray(f?.hosts) && f.hosts.length > 0,
};
export const fleet = () => apiOr(FLEET_API, () => runJson<Fleet>(["ls", "--json"], 45_000));
export const RECENT_API: ApiCall<RecentDir[]> = { method: "recent.list", accept: Array.isArray };
export const recent = () => apiOr(RECENT_API, () => runJson<RecentDir[]>(["recent", "--json"]));
export const PRESETS_ARGS = ["preset", "ls", "--json"];
export const PRESETS_API: ApiCall<Preset[]> = { method: "preset.list", accept: Array.isArray };
export const presets = () => apiOr(PRESETS_API, () => runJson<Preset[]>(PRESETS_ARGS));

/** `doctor --json`: every check, by section (`--host H` before it: just that machine). Exits 1 when one fails. */
export const DOCTOR_ARGS = ["doctor", "--json"];
/** The doctor checks every machine over ssh: give it time. */
export const DOCTOR_TIMEOUT_MS = 150_000;
/** The daemon's last report (it runs a new one past `doctor.every`, or `refresh`; sections arrive as they finish). */
export const doctorApi = (host?: string, refresh = false): ApiCall<DoctorReport | undefined> => ({
  method: "doctor.get",
  params: { ...(host ? { host } : {}), ...(refresh ? { refresh: true } : {}) },
  accept: (r) => Array.isArray(r?.sections) && (r.sections.length > 0 || !!r.running),
});

/**
 * `preset launch <name> --detach`: every session of the preset (newer builds;
 * `new @name` starts only a preset's first one). Resolves with what started.
 */
export async function presetLaunch(name: string): Promise<{ name: string; host: string }[]> {
  type Started = { name: string; host: string }[];
  const r = await act<unknown>(
    "preset.launch",
    { name },
    async () => startedLines((await runOutput(["preset", "launch", name, "--detach"], 180_000)).stderr),
    180_000,
  );
  return Array.isArray(r) ? (r as Started) : ((r as { started?: Started }).started ?? []);
}

/** `preset launch <name> --dry-run`: what it would start, in words. */
export async function presetPlan(name: string): Promise<string> {
  const { stdout, stderr } = await runOutput(["preset", "launch", name, "--dry-run"], 30_000);
  return stripAnsi(`${stderr}${stdout}`).trim();
}

/** `notify dnd on | off | 30m | 2h`: do not disturb (problems in notify.quiet_pass still come through). */
export const setDnd = (v: string) => run(["notify", "dnd", v]);
/** `notify mute host:name 1h | off`: no notifications from one session for a while. */
export const muteSession = (host: string, name: string, v: string) =>
  act("notify.mute", { session: `${host}:${name}`, off: v === "off" }, () =>
    run(["notify", "mute", `${host}:${name}`, v]),
  );

/** A session's screen (`peek --json [--ansi]`): the daemon's `screen.read`, which never scrolls an idle agent with `ansi`. */
export const peekApi = (sel: string, ansi: boolean, lines = 40): ApiCall<Peek | undefined> => ({
  method: "screen.read",
  params: { session: sel, lines, ...(ansi ? { ansi: true } : {}) },
  timeoutMs: 15_000,
  accept: (p) => typeof p?.text === "string",
});
export const peek = (sel: string) =>
  apiOr(peekApi(sel, false), () => optionalJson<Peek>("peek", ["peek", sel, "--json"], 20_000));
export const stats = (host?: string) => optionalJson<Stats>("stats", onHost(host, ["stats", "--json"]), 60_000);

export const STATS_ARGS = ["stats", "--json"];
/**
 * `stats.get`: live stats from the daemon's samplers (kept running while
 * asked, so nothing is sampled per ask) — one machine's, or every machine's
 * `brief` (just what a row shows); never the process lists.
 */
export const statsApi = (host?: string, brief = false): ApiCall<Stats | undefined> => ({
  method: "stats.get",
  params: { ...(host ? { host } : {}), ...(brief ? { brief: true } : { procs: false }) },
  accept: (s) => Array.isArray(s?.hosts) && s.hosts.length > 0,
});
export const INBOX_ARGS = ["inbox", "-n", "300", "--json"];
export const QUEUE_ARGS = ["queue", "ls", "--json"];
/** `inbox.list`: like `inbox -n N --json`, without dismissed events, with `unread`. */
export const inboxApi = (n = 300): ApiCall<InboxEvent[] | undefined> => ({
  method: "inbox.list",
  params: { n },
  accept: Array.isArray,
});
export const QUEUE_API: ApiCall<QueueTask[] | undefined> = { method: "queue.list", accept: Array.isArray };

/** The newest `n` inbox events: the daemon's (with `unread`), else `inbox -n N --json`. */
export const inbox = (n = 300) =>
  apiOr(inboxApi(n), () => optionalJson<InboxEvent[]>("inbox", ["inbox", "-n", String(n), "--json"]));

/**
 * Mark the inbox read up to `t` (default: now) — the dashboard's read mark,
 * so everything up to then stops counting as new. Through the daemon, else
 * `aleph inbox read`.
 */
export const markRead = (t?: number) =>
  request<{ seen: number }>("inbox.read", t ? { t: Math.floor(t) } : {}).catch(() =>
    runJson<{ seen: number }>(["inbox", "read", ...(t ? [String(Math.floor(t))] : []), "--json"]),
  );

/**
 * Start herdr's server on a machine where it stopped (the sessions Aleph
 * remembers there come back too): through the daemon, else `aleph restore`.
 */
export const startHerdr = (host: string) =>
  request<{ host: string; message: string }>("hosts.start_herdr", { host }).catch(() =>
    run(["--host", host, "restore", "--yes"], 180_000).then(() => ({ host, message: `herdr is running on ${host}` })),
  );

/** Archive events (by their ids), everywhere: through the daemon, else `aleph inbox dismiss`. */
export const dismissEvents = (ids: string[]) =>
  request<{ dismissed: number }>("inbox.dismiss", { ids }).catch(() =>
    runJson<{ dismissed: number }>(["inbox", "dismiss", ...ids, "--json"]),
  );
export const HISTORY_ARGS = ["history", "-n", "200", "--json"];
export const SERVICES_ARGS = ["services", "--json"];
export const AGENTS_ARGS = ["agents", "--json"];

export const queue = () => apiOr(QUEUE_API, () => optionalJson<QueueTask[]>("queue", QUEUE_ARGS));
/** Queue `text` for one session: it goes once that agent is done with its current reply. */
export const queueFor = (host: string, name: string, text: string) =>
  act("queue.add", { host, prompt: text, to: name }, () =>
    runJson<{ id: number; host: string }>(["queue", "add", text, "--to", `${host}:${name}`, "--json"]),
  ).then((r) => ({ host, ...(r as { id: number }) }));
/** Queue `text` for the next idle agent on `host` (optionally only in `dir`). */
export const queueOnHost = (host: string, text: string, dir?: string) =>
  act("queue.add", { host, prompt: text, ...(dir ? { dir } : {}) }, () =>
    runJson<{ id: number; host: string }>(["queue", "add", host, text, ...(dir ? ["--dir", dir] : []), "--json"]),
  ).then((r) => ({ host, ...(r as { id: number }) }));
export const queueRemove = (id: number) => act("queue.rm", { id }, () => run(["queue", "rm", String(id)]));
export const queueClear = (host?: string) =>
  act("queue.clear", host ? { host } : {}, () => run(["queue", "clear", ...(host ? [host] : [])]));

/** Send a queued message right away: take it off the queue (fails if it already went), then `send`. */
export async function queueSendNow(t: QueueTask): Promise<void> {
  if (!t.to) throw new CliError("This task isn't for one session", "it goes to the next idle agent on the machine");
  await queueRemove(t.id);
  try {
    await sendPrompt(`${t.host}:${t.to}`, t.prompt);
  } catch (e) {
    // put it back, so the message isn't lost
    await queueFor(t.host, t.to, t.prompt).catch(() => undefined);
    throw e;
  }
}

export const HISTORY_API: ApiCall<HistoryEntry[] | undefined> = {
  method: "history.list",
  params: { n: 200 },
  accept: Array.isArray,
};
export const history = () => apiOr(HISTORY_API, () => optionalJson<HistoryEntry[]>("history", HISTORY_ARGS));
/** LLM servers per machine: the daemon's watchers (kept while asked), else a scan now. */
export const servicesApi = (host?: string): ApiCall<ServicesReport | undefined> => ({
  method: "services.list",
  params: host ? { host } : {},
  timeoutMs: 20_000,
  accept: (r) => Array.isArray(r?.hosts),
});
export const services = (host?: string) =>
  apiOr(servicesApi(host), () => optionalJson<ServicesReport>("services", onHost(host, SERVICES_ARGS), 90_000));
/** Agents per machine, as the daemon keeps them (`refresh`: look again now; `local`: this computer too). */
export const agentsApi = (host?: string, refresh = false, local = false): ApiCall<AgentsReport | undefined> => ({
  method: "agents.list",
  params: {
    ...(host ? { host } : {}),
    ...(refresh ? { refresh: true } : {}),
    ...(local && !host ? { local: true } : {}),
  },
  timeoutMs: refresh ? 120_000 : 60_000,
  accept: (r) => Array.isArray(r?.hosts),
});
/** Agents per machine (the daemon keeps them; else aleph's 10-minute answers; `refresh` looks again; `local`: this computer too). */
export const agentsReport = (host?: string, refresh = false, local = false) =>
  apiOr(agentsApi(host, refresh, local), () =>
    optionalJson<AgentsReport>(
      "agents",
      onHost(host, [...AGENTS_ARGS, ...(refresh ? ["--refresh"] : []), ...(local && !host ? ["--local"] : [])]),
      120_000,
      true,
    ),
  );

/** An agent install can take a while (npm, curl | bash, uv): up to 15 minutes. */
export const INSTALL_TIMEOUT_MS = 15 * 60_000;
/** `agents install <id> --host H --yes`, its output streamed. Only ever called after the user confirmed. */
export const installAgent = (id: string, host: string, onOutput: (chunk: string) => void) =>
  runStreaming(["--host", host, "agents", "install", id, "--yes"], onOutput, INSTALL_TIMEOUT_MS);

/** Agent kinds: `agents --json` when this aleph has it, else the known list. */
export const STATIC_AGENTS = [
  "claude",
  "codex",
  "opencode",
  "gemini",
  "cursor",
  "devin",
  "copilot",
  "amp",
  "droid",
  "kimi",
  "qwen",
  "grok",
  "pi",
  "omp",
  "crush",
];

/** Agent ids from any shape `agents --json` had: `{hosts:[{agents}]}`, `{agents}` or a list. */
export function agentIds(v: unknown): string[] {
  const obj = v as { agents?: unknown; hosts?: { agents?: unknown }[] } | undefined;
  const list: unknown[] = Array.isArray(v)
    ? v
    : Array.isArray(obj?.agents)
      ? obj.agents
      : Array.isArray(obj?.hosts)
        ? obj.hosts.flatMap((h) => (Array.isArray(h?.agents) ? h.agents : []))
        : [];
  const names = list
    .map((a) => {
      if (typeof a === "string") return a;
      const r = a as Record<string, unknown> | null;
      return r?.id ?? r?.name ?? r?.kind;
    })
    .filter((a): a is string => typeof a === "string" && a.length > 0);
  return [...new Set(names)];
}

export async function agents(): Promise<string[]> {
  try {
    const names = agentIds(
      await apiOr<AgentsReport | undefined>(agentsApi(), () =>
        optionalJson<AgentsReport>("agents", AGENTS_ARGS, 120_000),
      ),
    );
    return names.length ? names : STATIC_AGENTS;
  } catch {
    return STATIC_AGENTS;
  }
}

/** Unique selector for a session: `host:pane_id` (pane ids are per machine). */
export const selector = (s: Pick<Session, "host" | "pane_id">) => `${s.host}:${s.pane_id}`;
/** The name the queue matches a session by (`queue add --to host:<this>`). */
export const queueName = (s: Pick<Session, "name" | "display" | "pane_id">) => s.name || s.display || s.pane_id;

/** `send` takes words that start with `-` as flags: a leading space keeps such a message whole. */
const sendable = (text: string) => (text.startsWith("-") ? ` ${text}` : text);

/** `send <sel> <text>`: a prompt, without attaching. */
export const sendPrompt = (sel: string, text: string) =>
  act("session.send", { session: sel, text }, () => run(["send", sel, sendable(text)]));

/**
 * `send <sel> <text> --after <when>`: queue it for that session instead —
 * `idle` (once it's done with its current reply), a duration (`30m`, `2h`)
 * or a time (`14:30`). The daemon (`aleph watch`) or the dashboard sends it.
 */
export const sendAfter = (sel: string, text: string, after: string) =>
  run(["send", sel, sendable(text), "--after", after]);

/** `note --json`: every note, newest first (`note <sel> <text>` sets one). */
export interface Note {
  host: string;
  name: string;
  text: string;
  tags: string[];
  t: number;
}

export const NOTES_ARGS = ["note", "--json"];
export const NOTES_API: ApiCall<Note[] | undefined> = { method: "notes.list", accept: Array.isArray };

/** A session's note (notes are kept by machine and session name, so they outlive it). */
export const noteFor = (notes: Note[] | undefined, host: string, name: string | null | undefined) =>
  name ? notes?.find((n) => n.host === host && n.name === name) : undefined;

/** `note <sel> <text>` (one line; aleph trims it), or `note <sel> --clear` when the text is empty. */
export const setNote = (sel: string, text: string) => {
  const line = text.replace(/\s+/g, " ").trim();
  return act("note.set", { session: sel, text: line }, () =>
    run(line ? ["note", sel, sendable(line)] : ["note", sel, "--clear"]),
  );
};
/** `keys <sel> <key…>`: answer a question (1, 2, enter, esc, …). */
export const sendKeys = (sel: string, ...keys: string[]) =>
  act("session.keys", { session: sel, keys }, () => run(["keys", sel, ...keys]));
/** `approve <sel>`: pick the question's "yes" option, wherever it is (never blindly 1). */
export const approve = (sel: string) => act("session.approve", { session: sel }, () => run(["approve", sel]));
export const rename = (sel: string, name: string) =>
  act("session.rename", { session: sel, name }, () => run(["rename", sel, name]));
/**
 * `close <sel> --yes`: after Aleph's grace (`close.grace`). The daemon answers
 * at once (`{pending, deadline}`, it closes then); the CLI — no daemon — waits
 * the grace itself before it answers (an undo meanwhile keeps the session).
 */
export const close = (sel: string) =>
  act<CloseAnswer | undefined>("session.close", { session: sel }, async () => {
    try {
      // (it exits 1 with {"kept": true} when an undo kept it)
      return asClose(await runJson<unknown>(["close", sel, "--yes", "--json"], 3_700_000, true));
    } catch (e) {
      // an aleph from before the grace prints no JSON: it closed
      if (e instanceof CliError && e.message.endsWith("didn't return JSON")) return undefined;
      throw e;
    }
  });
/** `session.close.undo`: keep a session whose close is waiting out its grace. */
export const undoClose = (sel: string) =>
  act("session.close.undo", { session: sel }, () => run(["close", "--undo", sel]));

/** This extension's menu bar: what the daemon opens to refresh it (`notify.raycast.deeplink`). */
export const MENU_BAR_DEEPLINK = "raycast://extensions/aleph/aleph/menu-bar?launchType=background";

export const SETTINGS_ARGS = ["settings", "list", "--json"];
export const SETTINGS_API: ApiCall<Record<string, unknown>> = {
  method: "config.list",
  accept: (v) => !!v && typeof v === "object" && !Array.isArray(v),
};

let settingsOnce: Promise<Record<string, unknown>> | undefined;

/** Aleph's settings (`settings list --json`: every key, with defaults), read once per command run. */
export function settingsList(): Promise<Record<string, unknown>> {
  settingsOnce ??= apiOr(SETTINGS_API, () => runJson<Record<string, unknown>>(SETTINGS_ARGS)).catch((e) => {
    settingsOnce = undefined;
    throw e;
  });
  return settingsOnce;
}

/** A setting's value as text ("" when it isn't there). */
export function setting(s: Record<string, unknown> | undefined, key: string): string {
  const v = s?.[key];
  return typeof v === "string" ? v : v == null ? "" : String(v);
}

/** `aleph settings set <key> <value>` (keys the CLI doesn't know yet are kept too). */
export const settingSet = (key: string, value: string) => {
  settingsOnce = undefined;
  return act("config.set", { key, value }, () => run(["settings", "set", key, value]));
};

/** `watch_interval` from the settings, in seconds (20 when unknown). */
export function watchInterval(s?: Record<string, unknown>): number {
  const iv = Number(setting(s, "watch_interval"));
  return Number.isFinite(iv) && iv > 0 ? iv : 20;
}

/**
 * The status file check from before the daemon had a socket (an older `aleph
 * watch`, or a dashboard of an older build): the watch rewrites
 * `~/.cache/aleph/status.json` after every poll, so it runs when that file is
 * younger than 3 × `watch_interval` + 5 s (`queue::watch_running`'s
 * fallback). Builds from before the rename wrote `~/.cache/claude-herdr`.
 */
function statusFileFresh(intervalSecs = 20): boolean {
  const base = process.env.XDG_CACHE_HOME?.trim() || join(homedir(), ".cache");
  for (const dir of [BIN, LEGACY_BIN]) {
    try {
      const age = (Date.now() - statSync(join(base, dir, "status.json")).mtimeMs) / 1000;
      return age < Math.max(5, intervalSecs) * 3 + 5;
    } catch {
      // not there: try the old folder
    }
  }
  return false;
}

// ---------------------------------------------------------------- the daemon

/**
 * `daemon.status` (the API; `running` is added here), or what `aleph daemon
 * status --json` says, which also knows the service, the HTTP API's
 * settings, exposure and logs.
 */
export interface DaemonStatus {
  running: boolean;
  /** `daemon`, or `dashboard` (a dashboard serving the socket while no daemon runs) */
  kind?: string;
  version?: string;
  pid?: number;
  started_at?: number;
  uptime_s?: number;
  /** this process runs the hub (auto-approve, the queue, notifications) */
  hub?: boolean;
  /** the pid holding the hub lock, when one does */
  hub_holder?: number | null;
  /** where the HTTP API listens; null: it's off */
  http?: string | null;
  hosts?: string[];
  socket?: string;
  error?: string;
  // `aleph daemon status --json` only
  binary?: { version: string; path: string };
  version_match?: boolean | null;
  service?: {
    manager: string;
    name: string;
    file: string;
    installed: boolean;
    loaded: boolean;
    running: boolean;
    pid: number | null;
    state: string | null;
    /** launchd's last exit code (`143`: 128 + SIGTERM, a normal stop) or systemd's result (`exit-code`) */
    last_exit: string | number | null;
    enabled: boolean | null;
    error: string | null;
    linger: boolean | null;
  };
  hub_machine?: { setting: string; this_machine: string; here: boolean; holder: number | null };
  logs?: { path: string; bytes: number | null; service_log: string | null; journal: string | null };
  api?: {
    http: boolean;
    listen: string;
    listen_any: boolean;
    hosts: string[];
    answers: boolean;
    /** device tokens (only counted while the API is exposed) */
    tokens?: number | null;
  };
  exposure?: {
    tailscale: { url?: string }[];
    cloudflare: { hostname?: string | null; url?: string | null; running?: boolean | null } | null;
  };
  /** who said: the socket, the CLI, or the old status-file check */
  from: "api" | "cli" | "legacy";
}

/** The running daemon's own answer (`daemon.status`); undefined when nothing answers. */
export async function daemonLive(): Promise<DaemonStatus | undefined> {
  try {
    const v = await request<Omit<DaemonStatus, "running" | "from">>("daemon.status");
    return { ...v, running: true, from: "api" };
  } catch {
    return undefined;
  }
}

/** `aleph daemon status --json` (it exits 1 when the daemon isn't running; the JSON still counts). */
export async function daemonDetails(): Promise<DaemonStatus | undefined> {
  // the daemon asks its service manager itself (every 10 s at most)
  const kept = await request<Omit<DaemonStatus, "from">>("daemon.facts", {}, { timeoutMs: 8000 }).catch(
    () => undefined,
  );
  if (kept) return { ...kept, from: "cli" };
  if (!(await commands()).has("daemon")) return undefined;
  const v = await runJson<Omit<DaemonStatus, "from">>(["daemon", "status", "--json"], 20_000, true);
  return { ...v, from: "cli" };
}

/**
 * Is the daemon running? Its socket answers (`daemon.status`); else the CLI's
 * `daemon status --json`; else — a build from before the daemon — the old
 * status-file check.
 */
export async function daemonStatus(intervalSecs?: number): Promise<DaemonStatus> {
  const live = await daemonLive();
  if (live) return live;
  try {
    const d = await daemonDetails();
    if (d) return d;
  } catch {
    // the CLI didn't say: the status file
  }
  return { running: statusFileFresh(intervalSecs), from: "legacy" };
}

/**
 * Are queued messages going out? Something answers on the socket and runs
 * the hub here (or another process holds it), like `queue::watch_running`,
 * which also counts a fresh status file from an older daemon or dashboard.
 */
export function queueRuns(d: DaemonStatus | undefined, intervalSecs?: number): boolean {
  if (d?.running && d.from === "api") return d.hub !== false || d.hub_holder != null;
  return !!d?.running || statusFileFresh(intervalSecs);
}

/** `aleph daemon pair --json`: a one-time code for a phone or another computer. */
export interface Pairing {
  url: string;
  code: string;
  name: string;
  /** read · reply · control */
  scope: string;
  /** unix time */
  expires: number;
}

/** `daemon pair --json` (needs the daemon with its HTTP API on). */
export const pairDevice = () => runJson<Pairing>(["daemon", "pair", "--json"], 20_000);

/** A paired device (`aleph daemon token ls --json`): never its token. */
export interface Device {
  name: string;
  /** read · reply · control */
  scope: string;
  created: number;
  last_used: number | null;
  /** its web-push subscriptions */
  push?: number;
}

export const DEVICES_ARGS = ["daemon", "token", "ls", "--json"];

/** Turn the HTTP API on or off (`api.http`); the daemon picks it up when it restarts. */
export const setApiHttp = (on: boolean) => settingSet("api.http", on ? "on" : "off");

export type DaemonOp = "install" | "uninstall" | "start" | "stop" | "restart";

/** `daemon <op>`; `stop --yes` also stops a daemon started by hand (Raycast asked first). */
export const daemonControl = (op: DaemonOp) => runOutput(["daemon", op, ...(op === "stop" ? ["--yes"] : [])], 60_000);

// ---------------------------------------------------------------- usage

/** One row of `usage --json` and `usage.query` (usage/cmd.rs `row_json`). */
export interface UsageRow {
  key: string;
  label: string;
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  reasoning: number;
  tokens: number;
  requests: number;
  sessions: number;
  active_seconds: number;
  cost: number;
  /** part of the cost comes from Aleph's price table, not the agent's own log */
  estimated: boolean;
  unpriced_requests: number;
  first: number;
  last: number;
  hosts: string[];
  agents: string[];
  models: string[];
  projects: string[];
}

/** `usage <range> --by <by> --json`, or the API's `usage.query` (no total or sync details). */
export interface UsageReport {
  /** `last 30 days`, `today` */
  range: string;
  by: string;
  rows: UsageRow[];
  enabled?: boolean;
  unpriced_models?: string[];
  /** machine → when its logs were last read */
  synced?: Record<string, number>;
  errors?: { host: string; error: string | null }[];
  /** plan windows from the logs (host → agent → id → snapshot) */
  limits?: import("./usage-view").Limits;
}

export interface UsageQuery {
  /** 24h · 3d · 7d · 30d · 6mo · all · today · this-week · this-month · 2026-09-01..2026-10-15 */
  range: string;
  /** total · day · agent · model · host · project · account */
  by: string;
  /** only these (each optional): an agent id, a machine alias, a model, a project folder (and what's under it) */
  agent?: string;
  host?: string;
  model?: string;
  project?: string;
  /** an account's id, email or nickname */
  account?: string;
}

const FILTERS = ["agent", "host", "model", "project", "account"] as const;

/**
 * Usage from what's been read so far: the daemon's `usage.query` (the daemon
 * keeps reading the logs), else `aleph usage … --json`. `sync`: the CLI may
 * first read the machines whose logs are due (one call per view does; the
 * others pass `--no-sync`).
 */
export function usageQuery(q: UsageQuery, sync = false): Promise<UsageReport> {
  const set = FILTERS.filter((k) => q[k]);
  const params = { range: q.range, by: q.by, ...Object.fromEntries(set.map((k) => [k, q[k]])) };
  const cli = async (): Promise<UsageReport> => {
    const args = ["usage", q.range, "--by", q.by, ...set.flatMap((k) => [`--${k}`, q[k] as string]), "--json"];
    const r = await runJson<Omit<UsageReport, "range"> & { range: { label: string } | string }>(
      sync ? args : [...args, "--no-sync"],
      sync ? 90_000 : 30_000,
    );
    return { ...r, range: typeof r.range === "string" ? r.range : r.range.label };
  };
  return apiOr<UsageReport>({ method: "usage.query", params, accept: (r) => Array.isArray(r?.rows) }, cli);
}

// ---------------------------------------------------------------- display helpers

export { duration, since, stateOf, STATUS_LABEL } from "./fmt";

/** `in 2 hours 13 minutes` → `in 2h 13m`. */
function shortDur(s: string): string {
  const units: Record<string, string> = {
    days: "d",
    day: "d",
    hours: "h",
    hour: "h",
    hrs: "h",
    hr: "h",
    minutes: "m",
    minute: "m",
    mins: "m",
    min: "m",
    seconds: "s",
    second: "s",
    secs: "s",
    sec: "s",
    and: "",
  };
  const out: string[] = [];
  for (const w of s.split(/\s+/).filter(Boolean)) {
    const u = units[w.replace(/,$/, "")];
    if (u === "") continue;
    if (u && out.length && /^\d+$/.test(out[out.length - 1])) out[out.length - 1] += u;
    else out.push(w);
  }
  return out.join(" ");
}

/** When the limit resets, short: `2:40am`, `in 2h 13m`, "" when unknown. */
export function limitReset(reset: string | null | undefined): string {
  return shortDur((reset ?? "").split(" (")[0].trim());
}

/** `◷ at limit · resets 2:40am`, `◷ at limit · back in 2h`, `◷ at limit` (like aleph's own label). */
export function limitLabel(reset: string | null | undefined): string {
  const r = limitReset(reset);
  if (!r) return "◷ at limit";
  return r.startsWith("in ") ? `◷ at limit · back ${r}` : `◷ at limit · resets ${r}`;
}

/** The status text for a session: `◷ at limit · resets 2:40am`, `needs you · 4m`, … */
export function statusText(s: { status: string; limit?: string | null; status_since?: number | null }): string {
  if (s.limit != null) return limitLabel(s.limit);
  const label = STATUS_LABEL[s.status] ?? s.status;
  const age = since(s.status_since);
  return age ? `${label} · ${age}` : label;
}

/** What an inbox event means, in words: a headline and the detail. */
export function eventReason(e: InboxEvent): { head: string; detail: string } {
  const t = e.title.trim();
  // the agent's own words (its hook), when it said more than the title
  const said = e.body?.trim() || t;
  switch (e.kind) {
    case "error":
      return explainError(t);
    case "blocked":
      return { head: "is waiting for your answer", detail: said };
    case "done":
      return { head: "finished its turn", detail: said };
    case "new":
      return { head: "started", detail: t };
    case "gone":
      return { head: "ended", detail: t };
    case "auto":
      return { head: "was answered by an auto-approve rule", detail: t.replace(/^rule /, "") };
    case "task":
      return { head: "was sent a queued task", detail: t };
    case "heal":
      return { head: "herdr was restarted to recover", detail: t };
    case "limit":
      return { head: "stopped at its usage limit", detail: t };
    case "up":
      return { head: "answers again", detail: t };
    case "remote":
      return { head: "was acted on remotely", detail: t };
    default:
      return { head: e.kind, detail: t };
  }
}

/** `/home/me/x` → `~/x` (remote paths; we don't know the remote user, so any home). */
export function shortPath(p: string): string {
  return p.replace(/^\/(home|Users)\/[^/]+(?=\/|$)/, "~").replace(/^\/root(?=\/|$)/, "~");
}

export function shQuote(s: string): string {
  return /^[A-Za-z0-9_@%+=:,./-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`;
}

// eslint-disable-next-line no-control-regex
const ANSI = /\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)|\x1b[()][0-9A-Za-z]|\x1b[=>78]/g;
export const stripAnsi = (s: string) => s.replace(ANSI, "");

export const NAME_RE = /^[a-z][a-z0-9_-]{0,31}$/;

/** Group items into day sections ("Today", "Yesterday", "Monday, Oct 20"), keeping their order. */
export function byDay<T>(items: T[], t: (x: T) => number): [string, T[]][] {
  const groups = new Map<string, T[]>();
  for (const x of items) {
    const d = day(t(x));
    groups.set(d, [...(groups.get(d) ?? []), x]);
  }
  return [...groups.entries()];
}

export function day(t: number): string {
  const d = new Date(t * 1000);
  const today = new Date();
  const y = new Date(today);
  y.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === y.toDateString()) return "Yesterday";
  return d.toLocaleDateString(undefined, { weekday: "long", month: "short", day: "numeric" });
}
