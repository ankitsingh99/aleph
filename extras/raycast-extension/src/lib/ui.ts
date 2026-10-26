import { Cache, Color, Icon, Image } from "@raycast/api";
import type { AgentInfo, AgentsReport } from "./cli";

// Aleph's "Graphite + gold" palette (ui.rs): the gold accent and the state
// colours — working blue, needs-you orange, done green, idle teal, at-limit
// violet, down red — each in its dark- and light-theme shade.
const shade = (dark: string, light: string): Color.Dynamic => ({ dark, light, adjustContrast: false });

export const ACCENT = shade("#E2B451", "#8F6400");
/** a session that's working */
export const WORKING = shade("#3B9EFF", "#0D74CE");
/** a session waiting for you */
export const NEEDS = shade("#F76B15", "#CC4E00");
/** a session that finished */
export const DONE = shade("#46A758", "#2A7E3B");
/** a session sitting idle (and finished turns in the inbox) */
export const IDLE = shade("#12A594", "#067A6F");
/** stopped at its plan's usage limit */
export const LIMIT = shade("#A875FF", "#8145B5");
/** a machine or session that's down */
export const DOWN = shade("#E5484D", "#CE2C31");
export const WARN = shade("#F0D84A", "#8A6A00");
export const MUTED = shade("#A0A0A0", "#646464");

/** The aleph mark (tintable), for "Open in Aleph" and the menu bar. */
export const MARK = "aleph-mark.png";
export const ALEPH_ICON: Image.ImageLike = { source: MARK, tintColor: ACCENT };

export function statusIcon(status: string): Image.ImageLike {
  switch (status) {
    case "blocked":
      return { source: Icon.ExclamationMark, tintColor: NEEDS };
    case "working":
      return { source: Icon.CircleProgress50, tintColor: WORKING };
    case "done":
      return { source: Icon.CheckCircle, tintColor: DONE };
    case "idle":
      return { source: Icon.CircleFilled, tintColor: IDLE };
    case "limit":
      return { source: Icon.Clock, tintColor: LIMIT };
    default:
      return { source: Icon.Circle, tintColor: Color.SecondaryText };
  }
}

export function statusColor(status: string): Color.ColorLike {
  switch (status) {
    case "blocked":
      return NEEDS;
    case "working":
      return WORKING;
    case "done":
      return DONE;
    case "idle":
      return IDLE;
    case "limit":
      return LIMIT;
    default:
      return Color.SecondaryText;
  }
}

/** How each inbox kind looks: icon, colour and a short word (the dashboard's inbox, events.rs). */
export function kindLook(kind: string): { icon: Image.ImageLike; color: Color.ColorLike; word: string; glyph: string } {
  const look = (source: Image.Source, color: Color.ColorLike, word: string, glyph: string) => ({
    icon: { source, tintColor: color },
    color,
    word,
    glyph,
  });
  switch (kind) {
    case "blocked":
      return look(Icon.ExclamationMark, NEEDS, "needs you", "▲");
    case "done":
      return look(Icon.CheckCircle, IDLE, "finished", "✓");
    case "new":
      return look(Icon.PlusCircle, DONE, "started", "+");
    case "gone":
      return look(Icon.MinusCircle, MUTED, "ended", "–");
    case "error":
      return look(Icon.Warning, WARN, "unreachable", "✕");
    case "auto":
      return look(Icon.Bolt, ACCENT, "auto-approved", "↯");
    case "task":
      return look(Icon.Tray, WORKING, "task sent", "≡");
    case "heal":
      return look(Icon.ArrowClockwise, DONE, "herdr restarted", "↻");
    case "limit":
      return look(Icon.Clock, LIMIT, "at its limit", "◷");
    case "up":
      return look(Icon.CircleFilled, DONE, "back up", "●");
    case "remote":
      return look(Icon.Mobile, WORKING, "remote action", "⇄");
    default:
      return look(Icon.Dot, Color.SecondaryText, kind, "·");
  }
}

/** Errors from lib/cli carry a hint; show both. */
export function errorText(e: unknown): { title: string; message?: string } {
  if (e instanceof Error) {
    const hint = (e as Error & { hint?: string }).hint;
    return { title: e.message, message: hint };
  }
  return { title: String(e) };
}

/** ▰▰▰▱▱▱▱▱▱▱ for a 0..1 fraction. */
export function bar(frac: number | undefined, width = 10): string {
  const f = Math.min(1, Math.max(0, frac ?? 0));
  const n = Math.round(f * width);
  return "▰".repeat(n) + "▱".repeat(width - n);
}

export { pct } from "./fmt";

/** Red over 90 %, yellow over 70 %, otherwise green. */
export function loadColor(frac: number | undefined): Color.ColorLike {
  const f = frac ?? 0;
  return f >= 0.9 ? DOWN : f >= 0.7 ? WARN : DONE;
}

export { bytes, rate } from "./fmt";

// ---------------------------------------------------------------- agent brands

export interface Brand {
  id: string;
  name: string;
  glyph: string;
  color: string;
}

/**
 * Every agent in Aleph's registry (agents.rs, icons.rs), as `agents --json`
 * names them — only until an answer from the CLI has been read (`useBrands`).
 */
const FALLBACK: Brand[] = [
  { id: "claude", name: "Claude Code", glyph: "✻", color: "#d97757" },
  { id: "codex", name: "Codex", glyph: "◎", color: "#10a37f" },
  { id: "opencode", name: "OpenCode", glyph: "⌬", color: "#fab283" },
  { id: "omp", name: "OMP (oh-my-pi)", glyph: "⊕", color: "#c878c8" },
  { id: "pi", name: "Pi", glyph: "π", color: "#78c878" },
  { id: "cursor", name: "Cursor CLI", glyph: "▲", color: "#dcdcdc" },
  { id: "devin", name: "Devin CLI", glyph: "◆", color: "#50c8a0" },
  { id: "gemini", name: "Gemini CLI", glyph: "✦", color: "#4285f4" },
  { id: "agy", name: "Antigravity CLI", glyph: "◒", color: "#4285f4" },
  { id: "cline", name: "Cline CLI", glyph: "◭", color: "#64b4ff" },
  { id: "mastracode", name: "Mastra Code", glyph: "◩", color: "#ff7850" },
  { id: "copilot", name: "GitHub Copilot CLI", glyph: "◈", color: "#8957e5" },
  { id: "kimi", name: "Kimi Code", glyph: "◐", color: "#5a8cff" },
  { id: "kiro", name: "Kiro CLI", glyph: "◬", color: "#8c64ff" },
  { id: "droid", name: "Factory Droid", glyph: "⬢", color: "#ff9632" },
  { id: "amp", name: "Amp", glyph: "↯", color: "#ff5c5c" },
  { id: "grok", name: "Grok CLI", glyph: "✕", color: "#e6e6e6" },
  { id: "hermes", name: "Hermes Agent", glyph: "☿", color: "#b4b4ff" },
  { id: "kilo", name: "Kilo Code CLI", glyph: "◇", color: "#f0c83c" },
  { id: "qodercli", name: "Qoder CLI", glyph: "◫", color: "#00c8b4" },
  { id: "qwen", name: "Qwen Code", glyph: "◉", color: "#6e50f0" },
  { id: "letta", name: "Letta Code", glyph: "◑", color: "#ffb400" },
  { id: "maki", name: "Maki", glyph: "◍", color: "#ff6496" },
  { id: "muse", name: "Muse Code", glyph: "♪", color: "#0082fa" },
  { id: "crush", name: "Crush", glyph: "❖", color: "#ff50b4" },
  { id: "commandcode", name: "Command Code", glyph: "⌘", color: "#ffd23c" },
  { id: "prime", name: "Prime Agent", glyph: "ρ", color: "#ffffff" },
];

const cache = new Cache({ namespace: "brands" });
const KEY = "agents";
const AT = "agents.at";
/** How long an answer from the CLI is good for (the registry only changes with a new aleph). */
const FRESH_MS = 24 * 3600_000;

/** Remember every agent's brand from an `agents --json` answer (shared by all commands). True when stored. */
export function rememberBrands(r: AgentsReport | undefined): boolean {
  if (!r?.hosts?.length) return false;
  const seen = new Map<string, Brand>();
  for (const h of r.hosts)
    for (const a of h.agents ?? []) {
      if (!a.id || !a.name) continue;
      if (!seen.has(a.id)) seen.set(a.id, { id: a.id, name: a.name, glyph: a.glyph, color: a.color });
      if (a.kind && a.kind !== a.id && !seen.has(a.kind))
        seen.set(a.kind, { id: a.id, name: a.name, glyph: a.glyph, color: a.color });
    }
  if (!seen.size) return false;
  cache.set(KEY, JSON.stringify([...seen.values()]));
  cache.set(AT, String(Date.now()));
  return true;
}

/** Was the brand list read from the CLI in the last day? */
export function brandsFresh(): boolean {
  const at = Number(cache.get(AT) ?? 0);
  return Date.now() - at < FRESH_MS && !!cache.get(KEY);
}

/** Every agent Aleph knows: from the CLI when it has answered, else the built-in list. */
export function brands(): Brand[] {
  try {
    const v = JSON.parse(cache.get(KEY) ?? "[]") as Brand[];
    return Array.isArray(v) && v.length ? v : FALLBACK;
  } catch {
    return FALLBACK;
  }
}

/** An agent's glyph, name and brand colour (`claude` → ✻ Claude Code, orange). */
export function brand(agent: string | null | undefined, info?: Pick<AgentInfo, "name" | "glyph" | "color">): Brand {
  const id = agent ?? "";
  if (info) return { id, name: info.name, glyph: info.glyph, color: info.color };
  const b = brands().find((x) => x.id === id) ?? FALLBACK.find((x) => x.id === id);
  if (b) return b;
  if (id === "shell") return { id, name: "shell", glyph: "$", color: "#a0a0a0" };
  return { id, name: id || "unknown", glyph: "●", color: "#a0a0a0" };
}
