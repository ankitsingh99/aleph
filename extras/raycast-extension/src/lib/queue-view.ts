// The message queue, drawn: one queued (sent, failed) message's detail
// (`queue ls --json` with the fleet) as Markdown with SVG cards — who gets
// it (the session's agent logo, its state now, its machine) and where it is
// in line, a track from queued through waiting to sent — and the words a
// row needs. No Raycast imports: plain node tests it.
import type { Fleet, QueueTask, Session } from "./cli";
import { ChartTheme, clip, svgDoc, text } from "./charts";
import { hero, PANE, Part, stateMark } from "./dash";
import { duration, since, stateOf, STATUS_LABEL } from "./fmt";
import { BrandLike, img, LogoFn } from "./stats-view";

/** A session state's colour on a card (needs-you orange, working blue, …). */
export function stateColor(state: string, t: ChartTheme): string {
  const s = t.state as Record<string, string>;
  return s[state] ?? t.other;
}

const tilde = (p: string) => p.replace(/^\/(home|Users)\/[^/]+(?=\/|$)/, "~").replace(/^\/root(?=\/|$)/, "~");
const md = (s: string) => s.replace(/[<>*_`#[\]|\\]/g, (c) => `\\${c}`);

/** `14:30`, or `Mon 14:30` when it isn't today (in this computer's time zone). */
export function clock(t: number, now = Date.now() / 1000): string {
  const d = new Date(t * 1000);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return d.toDateString() === new Date(now * 1000).toDateString()
    ? time
    : `${d.toLocaleDateString([], { weekday: "short" })} ${time}`;
}

/** The running session a message is for (or went to), if it runs. */
export function liveOf(t: QueueTask, f: Fleet | undefined): Session | undefined {
  const who = t.session ?? t.to;
  if (!who) return undefined;
  return f?.hosts
    .find((h) => h.alias === t.host)
    ?.sessions.find((s) => s.name === who || s.display === who || s.label === who);
}

/** Who it goes to, short: the session's name, or `next idle` (a machine's next idle agent). */
export const targetName = (t: QueueTask) => t.session ?? t.to ?? (t.new ? "new session" : "next idle");

/** Where a queued message is in its line (the same session's, or the same machine's for the next idle agent). */
export function position(t: QueueTask, all: QueueTask[]): { n: number; of: number } {
  const line = all
    .filter((x) => x.state === "queued" && x.host === t.host && (x.to ?? "") === (t.to ?? ""))
    .sort((a, b) => a.id - b.id);
  return { n: line.findIndex((x) => x.id === t.id) + 1, of: line.length };
}

/** What it waits for, in a few words: `its reply`, `14:30`, `an idle agent in ~/x`. */
export function waitsFor(t: QueueTask, now = Date.now() / 1000): string {
  if (t.after && t.after > now) return clock(t.after, now);
  if (t.to) return "its reply";
  if (t.new) return "a new session";
  return t.dir ? `an idle agent in ${tilde(t.dir)}` : "an idle agent";
}

export interface QueueGroup {
  state: "queued" | "sent" | "failed";
  title: string;
  /** the section's subtitle: how many, and what order */
  subtitle: string;
  tasks: QueueTask[];
}

/** Queued first (oldest first: the order they go in), then sent and failed (newest first). */
export function queueGroups(tasks: QueueTask[]): QueueGroup[] {
  const when = (t: QueueTask) => t.at ?? t.added;
  const of = (s: string) => tasks.filter((t) => t.state === s);
  const queued = of("queued").sort((a, b) => a.id - b.id);
  const sent = of("sent").sort((a, b) => when(b) - when(a));
  const failed = tasks.filter((t) => t.state !== "queued" && t.state !== "sent").sort((a, b) => when(b) - when(a));
  const out: QueueGroup[] = [];
  if (queued.length) {
    const sessions = new Set(queued.map((t) => `${t.host}:${t.to ?? ""}`)).size;
    out.push({
      state: "queued",
      title: "Queued",
      subtitle: sessions > 1 ? `${queued.length} · for ${sessions} sessions` : String(queued.length),
      tasks: queued,
    });
  }
  if (failed.length) out.push({ state: "failed", title: "Failed", subtitle: String(failed.length), tasks: failed });
  if (sent.length) out.push({ state: "sent", title: "Sent", subtitle: `${sent.length} · the last day`, tasks: sent });
  return out;
}

// ---------------------------------------------------------------- the track

export interface Step {
  label: string;
  /** under the label: a time, `its reply` */
  sub: string;
  /** done · now · fail · todo */
  mark: "done" | "now" | "fail" | "todo";
}

/** The three steps a message goes through, with where it is now. */
export function steps(t: QueueTask, now = Date.now() / 1000): Step[] {
  const added = { label: "Queued", sub: `${since(t.added, now) || "0s"} ago`, mark: "done" as const };
  if (t.state === "queued") {
    const due = !!(t.after && t.after > now);
    return [
      added,
      { label: due ? "Not before" : "Waits for", sub: waitsFor(t, now), mark: "now" },
      { label: "Sent", sub: "not yet", mark: "todo" },
    ];
  }
  const at = t.at ?? t.added;
  const went = `${since(at, now) || "0s"} ago`;
  if (t.state === "sent")
    return [
      added,
      { label: "Waited", sub: duration(at - t.added), mark: "done" },
      { label: "Sent", sub: went, mark: "done" },
    ];
  return [
    added,
    { label: "Waited", sub: duration(at - t.added), mark: "done" },
    { label: "Failed", sub: went, mark: "fail" },
  ];
}

/** Steps on a line: a mark for each (✓ done, the working arc for now, ✕ failed, a ring to come), its label and time. */
export function track(list: Step[], t: ChartTheme, width = PANE): string {
  const n = list.length;
  const cell = width / n;
  const cy = 14;
  const parts: string[] = [];
  const color = (m: Step["mark"]) =>
    m === "done" ? t.state.done : m === "now" ? t.state.working : m === "fail" ? t.state.down : t.other;
  // the line between the marks: solid up to where it is, dashed after
  for (let i = 0; i < n - 1; i++) {
    const x1 = cell * i + cell / 2 + 14;
    const x2 = cell * (i + 1) + cell / 2 - 14;
    const done = list[i + 1].mark === "done" || list[i + 1].mark === "fail";
    parts.push(
      `<line x1="${x1.toFixed(1)}" x2="${x2.toFixed(1)}" y1="${cy}" y2="${cy}" stroke="${done ? color(list[i + 1].mark) : t.grid}" stroke-width="2" stroke-linecap="round"${done ? "" : ' stroke-dasharray="3 4"'}/>`,
    );
  }
  list.forEach((s, i) => {
    const cx = cell * i + cell / 2;
    const c = color(s.mark);
    parts.push(
      `<circle cx="${cx.toFixed(1)}" cy="${cy}" r="11" fill="${c}" fill-opacity="${s.mark === "todo" ? 0 : t.dark ? 0.2 : 0.14}" stroke="${s.mark === "todo" ? t.grid : "none"}" stroke-width="1.5"/>`,
    );
    parts.push(
      stateMark(
        s.mark === "done" ? "done" : s.mark === "now" ? "working" : s.mark === "fail" ? "down" : "none",
        cx,
        cy,
        s.mark === "todo" ? t.other : c,
      ),
    );
    parts.push(
      text(
        cx,
        cy + 29,
        clip(s.label, cell - 6, 12),
        s.mark === "todo" ? t.muted : t.text,
        12,
        ' text-anchor="middle" font-weight="600"',
      ),
    );
    parts.push(text(cx, cy + 45, clip(s.sub, cell - 6, 11), t.muted, 11, ' text-anchor="middle"'));
  });
  return svgDoc(width, cy + 50, parts.join(""), list.map((s) => `${s.label} ${s.sub}`).join(", "));
}

// ---------------------------------------------------------------- the detail

/** The whole detail of one message: who gets it, where it is, the words themselves. */
export function queueMarkdown(o: {
  t: QueueTask;
  all: QueueTask[];
  live?: Session;
  /** the machine's own colour */
  hostColor?: string;
  theme: ChartTheme;
  logo: LogoFn;
  brand: (id: string) => BrandLike;
  now?: number;
  width?: number;
}): string {
  const { t, live, theme } = o;
  const now = o.now ?? Date.now() / 1000;
  const w = o.width ?? PANE;
  const agent = live?.agent ?? t.agent ?? undefined;
  const b = agent ? o.brand(agent) : undefined;
  const tags: Part[] = [];
  if (live) {
    const st = stateOf(live);
    tags.push({
      text: STATUS_LABEL[st] ?? st,
      dot: stateColor(st, theme),
      note: since(live.status_since, now) || undefined,
    });
    if (b) tags.push({ text: b.name, muted: true });
  } else if (t.to || t.session) {
    tags.push({ text: "not running", muted: true });
  } else if (t.agent && b) {
    tags.push({ text: `${b.name} only`, muted: true });
  }
  const pos = t.state === "queued" ? position(t, o.all) : undefined;
  const where = [t.host, t.dir ? tilde(t.dir) : live?.cwd && live.cwd !== "-" ? tilde(live.cwd) : ""]
    .filter(Boolean)
    .join(" · ");
  const card = hero(
    {
      title: t.to || t.session ? targetName(t) : "Next idle agent",
      color: o.hostColor,
      subtitle: where,
      logo: agent ? o.logo("agent", agent) : undefined,
      letter: b?.name ?? targetName(t),
      tags,
      aside:
        pos && pos.of > 1
          ? { value: `${pos.n} of ${pos.of}`, label: "in line" }
          : t.state === "queued"
            ? { value: "next", label: "in line" }
            : { value: `#${t.id}`, label: t.state === "sent" ? "sent" : "failed" },
    },
    theme,
    w,
  );
  const out = [img(`For ${targetName(t)} on ${t.host}`, card), img("Where it is", track(steps(t, now), theme, w))];
  if (t.state !== "queued" && t.state !== "sent" && t.error) out.push(`**Couldn't send:** ${md(t.error)}`);
  out.push(
    t.prompt
      .trim()
      .split("\n")
      .map((l) => `> ${md(l)}`)
      .join("\n"),
  );
  return out.join("\n\n");
}

/** For tooltips: everything about where it goes, in one line. */
export function targetTooltip(t: QueueTask, live?: Session, now = Date.now() / 1000): string {
  const who = t.to || t.session ? `${targetName(t)} on ${t.host}` : `the next idle agent on ${t.host}`;
  const state = live ? ` (${STATUS_LABEL[stateOf(live)] ?? live.status})` : "";
  const wait = t.state === "queued" ? ` · waits for ${waitsFor(t, now)}` : "";
  return `${who}${state}${wait} · #${t.id}`;
}
