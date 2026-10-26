// Sessions, drawn: one session's detail — its live screen (screen.ts) under
// a title bar with the agent's logo and its state, then a card of what reads
// better drawn than as labels: the folder by its name with the path after
// it, the agent's logo, the machine in its own colour with its OS and link,
// the note and its tags, the messages queued for it. And the list's words:
// a row's state tag and folder, a machine's section line. No Raycast
// imports: plain node tests it.
import type { Host, Note, QueueTask, Session } from "./cli";
import { ChartTheme, fit } from "./charts";
import { Fact, facts, PANE, Part } from "./dash";
import { since, stateOf, STATUS_LABEL } from "./fmt";
import { osId } from "./logo-ids";
import { sessionCounts, stateColor } from "./machines-view";
import { explainError } from "./model";
import { previewMarkdown } from "./screen";
import { BrandLike, img, LogoFn } from "./stats-view";

/** `/home/me/x` → `~/x` (cli.ts's `shortPath`, which plain node can't import). */
const homeless = (p: string) => p.replace(/^\/(home|Users)\/[^/]+(?=\/|$)/, "~").replace(/^\/root(?=\/|$)/, "~");

/** A folder by its name, then the path to it: `test_folder` · `~/alexanders_work`. */
export function folderParts(cwd: string | null | undefined): { name: string; parent: string } {
  const p = homeless(cwd ?? "").replace(/(.)\/+$/, "$1");
  if (!p || p === "-") return { name: "", parent: "" };
  const i = p.lastIndexOf("/");
  if (p === "~" || p === "/" || i < 0) return { name: p, parent: "" };
  return { name: p.slice(i + 1), parent: i === 0 ? "/" : p.slice(0, i) };
}

/** The glyphs aleph's short status uses (`aleph status --short`, the menu bar). */
export const STATE_GLYPH: Record<string, string> = { blocked: "▲", working: "✻", limit: "◷", done: "✓", idle: "●" };

/** A row's state tag, short enough for the list beside the detail: `needs you 4m`, `idle 33m`, `at limit`. */
export function rowStatus(s: Pick<Session, "status" | "status_since" | "limit">, now = Date.now() / 1000): string {
  const st = stateOf(s);
  const label = STATUS_LABEL[st] ?? st;
  const age = st === "limit" ? "" : since(s.status_since, now);
  return age ? `${label} ${age}` : label;
}

/** A machine's section line: its sessions by state (`▲1 ✻2 ●3`), its link, herdr when it needs a look; why it's down. */
export function hostSummary(h: Host, now = Date.now() / 1000): string {
  if (!h.reachable) {
    const cached = h.fetched_at ? `cached ${since(h.fetched_at, now)}` : "";
    return [`✕ ${explainError(h.error).head}`, cached].filter(Boolean).join(" · ");
  }
  const counts = sessionCounts(h)
    .filter((c) => c.n > 0)
    .map((c) => `${STATE_GLYPH[c.state] ?? "·"}${c.n}`)
    .join(" ");
  const herdr = h.herdr_restart_needed
    ? "herdr restart needed"
    : h.herdr_drift
      ? `herdr ${h.herdr_version ?? "?"} differs`
      : "";
  return [counts || "no sessions", h.latency_ms != null ? `${h.latency_ms} ms` : "", herdr].filter(Boolean).join(" · ");
}

export interface SessionView {
  s: Session;
  host: Host;
  note?: Note;
  /** messages queued for it, next first */
  queued: QueueTask[];
  t: ChartTheme;
  logo: LogoFn;
  brand: (id: string) => BrandLike;
  /** the machine's OS as some command last saw it (logos.ts `machineOsText`) */
  osText?: string;
}

/** The facts under the screen. Its state is on the screen's title bar, so it isn't here. */
export function sessionFacts(o: SessionView): Fact[] {
  const { s, host, t } = o;
  const b = o.brand(s.agent);
  const rows: Fact[] = [];
  const f = folderParts(s.cwd);
  if (f.name) rows.push({ label: "Folder", parts: [{ text: f.name, strong: true, note: f.parent || undefined }] });
  // herdr's title is the agent's own name until it says what it's doing
  const task = s.title?.trim();
  if (task && task !== b.name && task !== s.agent) rows.push({ label: "Working on", parts: [{ text: task }] });
  rows.push({
    label: "Agent",
    parts: [
      s.agent
        ? { text: b.name, logo: o.logo("agent", s.agent), letter: b.name, letterColor: b.color }
        : { text: "unknown", muted: true },
    ],
  });
  const os = osId(o.osText);
  const machine: Part[] = [{ text: host.alias, dot: fit(host.color, t), strong: true }];
  if (os) machine.push({ text: os.name, logo: o.logo("os", os.id) });
  if (host.reachable && host.latency_ms != null) machine.push({ text: `${host.latency_ms} ms`, muted: true });
  rows.push({ label: "Machine", parts: machine });
  if (o.note) {
    rows.push({
      label: "Note",
      parts: [{ text: o.note.text }, ...o.note.tags.map((tag) => ({ text: `#${tag}`, muted: true }))],
    });
  }
  o.queued.slice(0, 3).forEach((q, i) => {
    rows.push({ label: i ? "Then" : "Queued", parts: [{ text: q.prompt.split("\n")[0] }] });
  });
  if (o.queued.length > 3) rows.push({ label: "", parts: [{ text: `+${o.queued.length - 3} more`, muted: true }] });
  rows.push({ label: "Pane", parts: [{ text: s.pane_id, muted: true, note: s.label || undefined }] });
  return rows;
}

/** One session's whole detail: the question and the live screen, then its facts. */
export function sessionMarkdown(
  o: SessionView & {
    /** what `peek --ansi` read; undefined: no answer (yet) */
    screen?: string;
    question?: string | null;
    loading: boolean;
    error?: string;
    /** its status in words (cli.ts `statusText`: `needs you · 4m`, `◷ at limit · resets 2:40am`) */
    status: string;
    rows?: number;
    width?: number;
  },
): string {
  const { s, t } = o;
  const width = o.width ?? PANE;
  const state = stateOf(s);
  const b = o.brand(s.agent);
  const top = previewMarkdown({
    screen: o.screen,
    question: o.question,
    loading: o.loading,
    error: o.error,
    head: {
      title: s.display || s.name || s.pane_id,
      logo: o.logo("agent", s.agent),
      letter: b.name,
      letterColor: b.color,
      state,
      status: o.status,
      color: o.host.stale ? t.other : stateColor(state, t),
    },
    t,
    width,
    rows: o.rows,
  });
  return [top, img("About", facts(sessionFacts(o), t, width, { labelW: 80 }))].join("\n\n");
}
