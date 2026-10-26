// What the forms decide without asking anyone: which machine a new session
// starts on by default, the `aleph new` arguments a form's values make, which
// session a message goes to by default, and the words beside them. No
// Raycast imports: plain node tests it.
import type { Host, Session } from "./cli";
import { stateOf, STATUS_LABEL } from "./fmt";

const shq = (s: string) => (/^[A-Za-z0-9_@%+=:,./-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`);

// ---------------------------------------------------------------- Start Session

/**
 * The machine a new session starts on unless you pick one: the one asked
 * for, else Aleph's default machine (`machine`, or the older `ssh_host`)
 * while it answers, else the first that answers, else the first.
 */
export function defaultHost(
  hosts: Pick<Host, "alias" | "reachable">[],
  settings: { machine?: string; ssh_host?: string },
  asked?: string,
): string | undefined {
  if (asked) return asked;
  const up = hosts.filter((h) => h.reachable);
  for (const want of [settings.machine, settings.ssh_host]) if (want && up.some((h) => h.alias === want)) return want;
  return up[0]?.alias ?? hosts[0]?.alias;
}

/** The Start Session form's values, as `startArgs` reads them. */
export interface StartValues {
  host: string;
  /** a preset to start from (`@name`), or none */
  preset?: string;
  /** a folder; "" for the machine's default */
  dir?: string;
  name?: string;
  /** an agent id; "" for the machine's default */
  agent?: string;
  prompt?: string;
  worktree?: boolean;
  branch?: string;
  /** how many agents tiled in one tab (1: just the one) */
  tile?: string;
}

/** `aleph new …` for the form's values. */
export function startArgs(v: StartValues): string[] {
  const args = ["new", "--host", v.host];
  if (v.preset) args.push(`@${v.preset}`);
  if (v.dir) args.push("--dir", v.dir);
  if (v.name) args.push("--name", v.name);
  if (v.agent) args.push("--agent", v.agent);
  if (v.worktree) args.push(v.branch?.trim() ? `--worktree=${v.branch.trim()}` : "--worktree");
  if (v.tile && v.tile !== "1") args.push("--tile", v.tile);
  if (v.prompt?.trim()) args.push("-m", v.prompt.trim());
  return args;
}

/** The same, as a command to paste in a terminal. */
export const startCommand = (v: StartValues) => ["aleph", ...startArgs(v)].map(shq).join(" ");

/** A machine in the Machine dropdown: `b31`, `gpu2 — offline`. */
export const hostTitle = (h: Pick<Host, "alias" | "reachable">) => (h.reachable ? h.alias : `${h.alias} — offline`);

/** An agent in the Agent dropdown: `Claude Code · 2.1.309`. */
export const agentTitle = (name: string, version?: string | null) => (version ? `${name} · ${version}` : name);

// ---------------------------------------------------------------- Send Message

type SessionLike = Pick<Session, "host" | "name" | "display" | "pane_id" | "status" | "status_since"> & {
  limit?: string | null;
};

/**
 * The session a message goes to unless you pick one: the one that finished
 * last (it's waiting for its next prompt), else the one idle the shortest,
 * else one at work, else whoever's asking a question.
 */
export function defaultSession<T extends SessionLike>(sessions: T[]): T | undefined {
  const rank = (s: T) => ({ done: 0, idle: 1, working: 2, blocked: 3 })[stateOf(s)] ?? 4;
  return [...sessions].sort((a, b) => rank(a) - rank(b) || (b.status_since ?? 0) - (a.status_since ?? 0))[0];
}

/** A session in the Session dropdown: `temper · needs you`. */
export const sessionTitle = (s: SessionLike) =>
  `${s.display || s.name || s.pane_id} · ${STATUS_LABEL[stateOf(s)] ?? s.status}`;

/**
 * What a session is working on, when its title says so: not empty, not
 * just its agent's name (`Claude Code`), not a shell's `user@host: ~/dir`.
 */
export function usefulTitle(title: string | null | undefined, agentName?: string): string {
  const t = (title ?? "").trim();
  if (!t || /^\S+@\S+:/.test(t)) return "";
  const plain = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");
  if (agentName && plain(t) === plain(agentName)) return "";
  return t;
}

/** When a message goes, as the When dropdown offers it. */
export const WHEN_TITLES: Record<string, string> = {
  now: "Now",
  idle: "When it's free",
  "30m": "In 30 minutes",
  "1h": "In 1 hour",
  "2h": "In 2 hours",
  at: "At a time…",
};
