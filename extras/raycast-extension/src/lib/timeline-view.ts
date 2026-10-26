// One agent conversation as a tree (the daemon's `timeline.get`, or `aleph
// timeline … --json`): its prompts, turns, tool calls and subagents, each
// with time, tokens and $, drawn as monospace lines for a Detail — the
// dashboard's tree (tui/tree_view.rs) in text. No Raycast imports: plain node
// tests it.

export interface TlSums {
  input: number;
  output: number;
  reasoning: number;
  cache: number;
  tokens: number;
  cost: number | null;
  estimated: boolean;
  requests: number;
  tools: number;
  errors: number;
}

export type TlKind = "lane" | "prompt" | "promptend" | "turn" | "group" | "tool" | "compact" | "error";

export interface TlRow {
  id: string;
  depth: number;
  kind: TlKind;
  label: string;
  start_ms: number;
  end_ms: number;
  sums: TlSums;
  foldable: boolean;
  folded: boolean;
  bad: boolean;
  lane: string;
  key: string;
  /** `13.8s · 2.0k→400 · 2 req · $0.10` (older builds: none) */
  figures?: string;
}

export interface TlTree {
  host: string;
  agent: string;
  session: string;
  start_ms: number;
  end_ms: number;
  sums: TlSums;
  lanes: { id: string; kind: string; prompts: number; sums: TlSums }[];
  rows: TlRow[];
  /** row key → its words (`--words`, `timeline.words`) */
  words?: Record<string, string>;
}

/** A conversation `timeline.recent` lists. */
export interface TlRecent {
  host: string;
  agent: string;
  session: string;
  last_ms: number;
  project?: string | null;
  project_id?: string | null;
  account?: string | null;
  account_label?: string | null;
  first?: number | null;
  prompts?: number | null;
  requests?: number | null;
  tokens?: number | null;
  cost?: number | null;
  estimated?: boolean | null;
}

export type Fold = "smart" | "open" | "folded";
export const FOLDS: Fold[] = ["smart", "open", "folded"];

/** The next way of folding (`o` in the dashboard). */
export const nextFold = (f: Fold): Fold => FOLDS[(FOLDS.indexOf(f) + 1) % FOLDS.length];

/** A row's mark, as the dashboard draws it. */
export function glyph(r: Pick<TlRow, "kind" | "folded" | "foldable">): string {
  switch (r.kind) {
    case "prompt":
      return r.folded ? "▸ " : "┌ ";
    case "promptend":
      return "└ ";
    case "lane":
      return r.folded ? "⑂▸" : "⑂ ";
    case "turn":
      return r.folded ? "◆▸" : "◆ ";
    case "group":
      return r.folded ? "▸ " : "▾ ";
    case "tool":
      return r.foldable && r.folded ? "›▸" : "› ";
    case "compact":
      return "≋ ";
    case "error":
      return "✗ ";
  }
  return "  ";
}

/** `14:02` (local). */
function clock(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

const cut = (s: string, n: number) => ([...s].length <= n ? s : `${[...s].slice(0, Math.max(0, n - 1)).join("")}…`);
const len = (s: string) => [...s].length;

/**
 * The tree as lines `width` characters wide: the time, the row (indented,
 * with its mark and its words when given), its figures right-aligned (the
 * words give way first, then the label).
 */
export function treeLines(rows: TlRow[], width = 78, words?: Record<string, string>): string[] {
  return rows.map((r) => {
    const left = `${clock(r.start_ms)} ${"  ".repeat(r.depth)}${glyph(r)}${r.label}`;
    const word = words?.[r.key];
    // with words to show, the counts give way to them first (the $ and "failed" stay)
    const figs = word
      ? fitFigures(r.figures ?? "", width - len(left) - 2 - Math.min(len(word) + 2, 26))
      : (r.figures ?? "");
    const room = width - len(left) - len(figs) - 2;
    const w = word && room >= 8 ? `  ${cut(word, room - 2)}` : "";
    const l = cut(`${left}${w}`, Math.max(12, width - len(figs) - 1));
    const pad = Math.max(1, width - len(l) - len(figs));
    return `${l}${" ".repeat(pad)}${figs}`.trimEnd();
  });
}

/** A row's figures in `room` characters: the tool and request counts go first, then the tokens, then the time. */
export function fitFigures(figs: string, room: number): string {
  let parts = figs.split(" · ").filter(Boolean);
  const order = [/ tools$/, / req$/, /→/, /^[\d.]+(ms|s|m|h)( \d+m)?$/];
  for (const re of order) {
    if (len(parts.join(" · ")) <= room) break;
    parts = parts.filter((p) => !re.test(p));
  }
  return parts.join(" · ");
}

const usd = (v: number | null | undefined) =>
  v === null || v === undefined ? "–" : v > 0 && v < 0.01 ? "<$0.01" : `$${v.toFixed(2)}`;
const k = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : String(n));

/** How long, as `3h 10m`, `4m`, `12s`. */
export function took(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ${m % 60}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

/** The detail: a heading with its totals, then the tree in a code block. */
export function treeMarkdown(
  t: TlTree,
  o: { title: string; fold: Fold; words?: boolean; width?: number; agentName?: string },
): string {
  const prompts = t.rows.filter((r) => r.kind === "prompt").length;
  const lanes = t.lanes.filter((l) => l.id).length;
  const facts = [
    o.agentName ?? t.agent,
    t.host,
    `${prompts} prompt${prompts === 1 ? "" : "s"}`,
    `${t.sums.requests} requests`,
    `${t.sums.tools} tool calls`,
    lanes ? `${lanes} subagent${lanes === 1 ? "" : "s"}` : "",
    `${k(t.sums.tokens)} tokens`,
    `${usd(t.sums.cost)}${t.sums.estimated ? " est." : ""}`,
    t.end_ms > t.start_ms ? took(t.end_ms - t.start_ms) : "",
  ].filter(Boolean);
  const head = [`## ${o.title}`, facts.join(" · ")];
  if (!t.rows.length) {
    head.push(
      "_Nothing read for it yet: its machine's logs are read about once a minute (`aleph usage sync` reads now)._",
    );
    return head.join("\n\n");
  }
  const lines = treeLines(t.rows, o.width ?? 78, o.words ? t.words : undefined);
  const errors = t.sums.errors ? ` · ${t.sums.errors} failed` : "";
  head.push(`_Folding: ${o.fold}${errors}_`);
  return `${head.join("\n\n")}\n\n\`\`\`\n${lines.join("\n")}\n\`\`\``;
}

/** The rows a fold of everything would open next (their ids). */
export const folded = (rows: TlRow[]) => rows.filter((r) => r.foldable && r.folded).map((r) => r.id);
