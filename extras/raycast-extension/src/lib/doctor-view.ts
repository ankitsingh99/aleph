// The doctor, drawn: `doctor --json` taken apart for the Doctor list and its
// detail. Each check becomes a short title, its key fact, its version and
// path (with `~`), the rest and why (as aleph's own doctor.rs `brief` does);
// a machine's sections (its ssh, what's on it, herdr's view) become one
// group, the groups with problems first and each group's problems first.
// The detail is SVG: a card for a check (its mark or logo, where it is, what
// it found, a status pill) or for a machine (a hero, pills of its counts,
// every check as an aligned checklist). No Raycast imports: plain node tests it.
import { ChartTheme, clip as clipText, fit, svgDoc, text, textWidth } from "./charts";
import { hero, logoMark, PANE, Part, stateMark } from "./dash";
import { osId } from "./logo-ids";
import { DoctorCheck, DoctorReport, runnableFix } from "./model";
import { img, LogoFn } from "./stats-view";

/** Cut to fit, from the end (a fact, a title); a path loses its start. */
const clip = (s: string, px: number, size: number) => clipText(s, px, size, /^[~/]/.test(s));
const r1 = (v: number) => Math.round(v * 10) / 10;

// ---------------------------------------------------------------- a check, in pieces

/** ✕ a problem, ! a warning, + a tip (a note with a fix), ✓ a pass, · a note. */
export type CheckKind = "fail" | "warn" | "tip" | "ok" | "note";

export function checkKind(c: Pick<DoctorCheck, "status" | "fix">): CheckKind {
  if (c.status === "fail") return "fail";
  if (c.status === "warn") return "warn";
  if (c.status === "ok") return "ok";
  return c.fix?.length ? "tip" : "note";
}

/** Problems first, then warnings, then tips; passes and notes keep their order. */
export const KIND_RANK: Record<CheckKind, number> = { fail: 0, warn: 1, tip: 2, ok: 3, note: 3 };
export const KIND_WORD: Record<CheckKind, string> = {
  fail: "failing",
  warn: "warning",
  tip: "tip",
  ok: "ok",
  note: "note",
};
export const KIND_GLYPH: Record<CheckKind, string> = { fail: "✕", warn: "!", tip: "+", ok: "✓", note: "·" };

/** A kind's colour on a card. */
export function kindColor(k: CheckKind, t: ChartTheme): string {
  return k === "fail"
    ? t.state.down
    : k === "warn"
      ? t.state.warn
      : k === "tip"
        ? t.state.working
        : k === "ok"
          ? t.state.done
          : t.other;
}

/** `/home/selene/x` or `/Users/me/x` → `~/x` (here or on the machine; display only). */
export function tildePath(p: string): string {
  const m = /^\/(?:home|Users)\/[^/]+(\/.*)?$/.exec(p);
  if (m) return `~${m[1] ?? ""}`;
  if (p === "/root" || p.startsWith("/root/")) return `~${p.slice(5)}`;
  return p;
}

/** Which part of Aleph a section is: `Host · b31` → b31's ssh, `On b31` → what's on it, `herdr · b31` → herdr's view. */
export function groupOf(section: string): { name: string; area: string; machine: boolean } {
  for (const [re, area] of [
    [/^Host · (.+)$/, "ssh"],
    [/^On (.+)$/, "tools"],
    [/^herdr · (.+)$/, "herdr"],
  ] as const) {
    const m = re.exec(section);
    if (m) return { name: m[1], area, machine: true };
  }
  // the daemon and the shortcuts live on this computer: they're part of This Mac
  if (section === "Daemon" || section === "Shortcuts")
    return { name: "This Mac", area: section.toLowerCase(), machine: false };
  return { name: section, area: "", machine: false };
}

const TOOLS = ["aleph", "herdr", "zsh", "ssh", "mosh", "moshi", "stat"];

/** A label as a row's title: `key login` → `Key login`, `dns` → `DNS`; tool names stay as written. */
export function titleOf(section: string, label: string): string {
  const { area } = groupOf(section);
  if (area === "herdr" && (label === "machine" || label === "session")) return `herdr ${label}`;
  if (label === "dns") return "DNS";
  if (label === "http api") return "HTTP API";
  if (TOOLS.some((x) => label === x || label.startsWith(`${x} `) || label.startsWith(`${x}-`)) || label.includes("."))
    return label;
  return label.charAt(0).toUpperCase() + label.slice(1);
}

export interface Brief {
  /** `Claude Code`, `Key login`, `DNS`, `herdr machine` */
  title: string;
  /** the key fact: `no herdr integration`, `works`, `gpu2.lan doesn't resolve` (may be empty) */
  fact: string;
  version?: string;
  /** with `~` for a home folder */
  path?: string;
  /** the rest of what it said: `default agent`, `1 agent session` */
  extra: string[];
  /** what came after ` — `, or a long aside in parentheses */
  why?: string;
}

const isVersion = (s: string) => /^v?\d+(\.\d+)+[\w.+-]*$/.test(s);
const isPath = (s: string) => /^(\/|~\/|~$)\S*$/.test(s);

/**
 * Take a check apart (`2.1.309 · /home/u/.local/bin/claude · default · no
 * herdr integration` → its version, `~/.local/bin/claude`, `default agent`,
 * and the integration as the key fact). `agent`: whether it's an agent's row.
 */
export function brief(section: string, c: Pick<DoctorCheck, "label" | "text">, agent = false): Brief {
  const flat = c.text.replace(/`/g, "");
  const cut = flat.indexOf(" — ");
  const main = (cut >= 0 ? flat.slice(0, cut) : flat).trim();
  let why = cut >= 0 ? flat.slice(cut + 3).trim() : undefined;
  const b: Brief = { title: titleOf(section, c.label), fact: "", extra: [] };
  const rest: string[] = [];
  for (const part of main
    .split("·")
    .map((p) => p.trim())
    .filter(Boolean)) {
    if (!b.version && isVersion(part)) b.version = part;
    else if (!b.path && isPath(part)) b.path = tildePath(part);
    else rest.push(part);
  }
  // the key fact: an agent's integration, a service's state, else the first thing said
  let key = rest.findIndex((p) => p.includes("integration"));
  if (key < 0) key = rest.findIndex((p) => /^(running|stopped)\b/.test(p));
  if (key < 0) key = 0;
  if (key < rest.length) b.fact = rest.splice(key, 1)[0];
  // a long aside in parentheses is the why, not the fact
  const aside = / \(([^()]*)\)$/.exec(b.fact);
  if (aside && aside[0].length > 26) {
    b.fact = b.fact.slice(0, aside.index);
    why = why ? `${aside[1].trim()} — ${why}` : aside[1].trim();
  }
  b.extra = agent ? rest.map((x) => (x === "default" ? "default agent" : x)) : rest;
  if (why) b.why = why;
  return b;
}

export interface FixStep {
  /** as the doctor wrote it */
  text: string;
  /** the command (or advice), without its why */
  cmd: string;
  why?: string;
  /** an Aleph command that can run in a terminal: its words */
  run?: string[];
}

/** A fix's command and what it's for (`aleph agents setup codex   — herdr restores its sessions with it`). */
export function fixStep(text: string): FixStep {
  const i = text.indexOf(" — ");
  const cmd = (i >= 0 ? text.slice(0, i) : text).trim();
  const why = i >= 0 ? text.slice(i + 3).trim() : undefined;
  const run = runnableFix(text);
  return { text, cmd, ...(why ? { why } : {}), ...(run ? { run } : {}) };
}

// ---------------------------------------------------------------- groups

export interface CheckRow {
  /** unique in the report: the list item's id */
  key: string;
  section: string;
  /** the machine (or This Mac, Daemon, Shortcuts) */
  group: string;
  /** `ssh`, `agents`, `tools`, `herdr`, or `` */
  area: string;
  check: DoctorCheck;
  kind: CheckKind;
  brief: Brief;
  /** the agent's id, for an agent's row */
  agent?: string;
  fixes: FixStep[];
}

export interface Counts {
  fail: number;
  warn: number;
  tip: number;
  ok: number;
  note: number;
}

export interface DoctorGroup {
  name: string;
  /** one of the fleet's machines (not This Mac, the daemon, the shortcuts) */
  machine: boolean;
  rows: CheckRow[];
  counts: Counts;
  /** fail · warn · ok */
  worst: CheckKind;
  /** where ssh goes */
  address?: string;
  /** what `uname` says (`Linux x86_64`) */
  os?: string;
  /** its herdr's version (or this computer's) */
  herdr?: string;
  /** this computer's aleph */
  aleph?: string;
}

export function countKinds(rows: { kind: CheckKind }[]): Counts {
  const n: Counts = { fail: 0, warn: 0, tip: 0, ok: 0, note: 0 };
  for (const r of rows) n[r.kind] += 1;
  return n;
}

export const worstOf = (n: Counts): CheckKind => (n.fail ? "fail" : n.warn ? "warn" : "ok");

/**
 * The report as groups: a machine's sections as one, groups with problems
 * first (the rest in the order they came), each one's checks problems first.
 * `agentId`: an agent's id from its name (`Claude Code` → `claude`), when it's one.
 */
export function doctorGroups(r: DoctorReport, agentId: (name: string) => string | undefined = () => undefined) {
  const out: DoctorGroup[] = [];
  let i = 0;
  for (const s of r.sections) {
    const { name, area, machine } = groupOf(s.section);
    let g = out.find((x) => x.name === name);
    if (!g) {
      g = { name, machine, rows: [], counts: countKinds([]), worst: "ok" };
      out.push(g);
    }
    for (const c of s.checks) {
      const agent = area === "tools" ? agentId(c.label) : undefined;
      const b = brief(s.section, c, !!agent);
      // a folded-in part keeps its name before each of its checks: `Daemon · Service`
      if (area === "daemon" || area === "shortcuts") b.title = `${s.section} · ${b.title}`;
      g.rows.push({
        key: `${s.section}/${c.label}/${i++}`,
        section: s.section,
        group: name,
        area: agent || c.label === "agents" ? "agents" : area,
        check: c,
        kind: checkKind(c),
        brief: b,
        ...(agent ? { agent } : {}),
        fixes: [
          // one Aleph can carry out itself: `doctor --fix` in a terminal (it asks first; sudo asks there)
          ...(c.action && machine
            ? [
                {
                  text: c.action,
                  cmd: `aleph --host ${name} doctor --fix`,
                  why: c.action,
                  run: ["--host", name, "doctor", "--fix"],
                },
              ]
            : []),
          ...(c.fix ?? []).map(fixStep),
        ],
      });
    }
  }
  for (const g of out) {
    g.rows = g.rows
      .map((row, i) => ({ row, i }))
      .sort((a, b) => KIND_RANK[a.row.kind] - KIND_RANK[b.row.kind] || a.i - b.i)
      .map((x) => x.row);
    g.counts = countKinds(g.rows);
    g.worst = worstOf(g.counts);
    const find = (area: string, label: string) => g.rows.find((x) => x.area === area && x.check.label === label);
    g.address = find("ssh", "connects to")?.brief.fact || undefined;
    g.os = g.rows.find((x) => x.check.label === "system")?.check.text || undefined;
    g.herdr = g.rows.find((x) => x.check.label === "herdr")?.brief.version;
    g.aleph = g.rows.find((x) => x.check.label === "aleph")?.brief.version;
  }
  return out
    .map((g, i) => ({ g, i }))
    .sort((a, b) => KIND_RANK[a.g.worst] - KIND_RANK[b.g.worst] || a.i - b.i)
    .map((x) => x.g);
}

/** `1 failing · 1 warning · 2 tips · 14 ok` (only the kinds there are; `all 14 ok`). */
export function countsText(n: Counts): string {
  const parts = [
    n.fail ? `${n.fail} failing` : "",
    n.warn ? `${n.warn} warning${n.warn === 1 ? "" : "s"}` : "",
    n.tip ? `${n.tip} tip${n.tip === 1 ? "" : "s"}` : "",
  ].filter(Boolean);
  return [...parts, parts.length || n.fail + n.warn ? `${n.ok} ok` : `all ${n.ok} ok`].join(" · ");
}

/** A group in a few words: where ssh goes, its system, its herdr (`aleph 2.0.0 · herdr 0.9.5` for this computer). */
export function groupFacts(g: DoctorGroup): string {
  return [g.address, g.os, g.aleph && !g.machine ? `aleph ${g.aleph}` : "", g.herdr ? `herdr ${g.herdr}` : ""]
    .filter(Boolean)
    .join(" · ");
}

/** What a row says beside its title when the list is wide: the rest, then its path. */
export function rowSubtitle(b: Brief, max = 60): string {
  const s = [...b.extra, b.path ?? ""].filter(Boolean).join(" · ");
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

/** Cut a fact for a tag (`gpu2.lan doesn't resolve`, …). */
export const shortFact = (s: string, max = 28) => (s.length > max ? `${s.slice(0, max - 1)}…` : s);

// ---------------------------------------------------------------- marks

/** A kind as a shape (✕, a triangle with !, +, ✓, a dot), centred on (cx, cy), about 12 px. */
export function kindMark(k: CheckKind, cx: number, cy: number, color: string): string {
  switch (k) {
    case "fail":
      return stateMark("down", cx, cy, color);
    case "warn":
      return stateMark("warn", cx, cy, color);
    case "ok":
      return stateMark("done", cx, cy, color);
    case "tip":
      return `<path d="M${r1(cx)},${r1(cy - 5)}V${r1(cy + 5)}M${r1(cx - 5)},${r1(cy)}H${r1(cx + 5)}" stroke="${color}" stroke-width="2.2" stroke-linecap="round"/>`;
    default:
      return `<circle cx="${r1(cx)}" cy="${r1(cy)}" r="2.6" fill="${color}"/>`;
  }
}

/** Ink that reads on a fill. */
function inkOn(fill: string, t: ChartTheme): string {
  const hex = fill.replace("#", "");
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  const lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  return lum > 0.5 ? "#111111" : t.dark ? "#111111" : "#ffffff";
}

/** A status pill: the kind's mark and word on a tint of its colour; returns the SVG and its width. */
function statusPill(k: CheckKind, x: number, y: number, t: ChartTheme, word = KIND_WORD[k]): [string, number] {
  const c = kindColor(k, t);
  const w = 12 + 12 + 6 + textWidth(word, 12) + 12;
  const h = 24;
  return [
    `<rect x="${r1(x)}" y="${y}" width="${r1(w)}" height="${h}" rx="${h / 2}" fill="${c}" fill-opacity="${t.dark ? 0.18 : 0.13}"/>` +
      kindMark(k, x + 17, y + h / 2, c) +
      text(x + 29, y + 16.5, word, t.text, 12, ' font-weight="600"'),
    w,
  ];
}

// ---------------------------------------------------------------- the check's card

/**
 * A check's card: its mark (or its agent's / system's logo with the mark as
 * a badge) on a tint of its kind's colour, its name, where it is, what it
 * found, and a status pill.
 */
export function checkCard(row: CheckRow, t: ChartTheme, logo: LogoFn, width = PANE): string {
  const k = row.kind;
  const c = kindColor(k, t);
  const tile = 52;
  const parts: string[] = [];
  parts.push(
    `<rect x="0" y="2" width="${tile}" height="${tile}" rx="12" fill="${c}" fill-opacity="${t.dark ? 0.2 : 0.15}"/>`,
  );
  const mark =
    row.agent != null
      ? logo("agent", row.agent)
      : row.check.label === "system"
        ? logo("os", osId(row.check.text)?.id)
        : undefined;
  if (mark) {
    parts.push(logoMark(11, 11, 30, mark, {}, t));
    parts.push(`<circle cx="${tile - 6}" cy="${tile - 4}" r="9" fill="${c}"/>`);
    parts.push(kindMark(k, tile - 6, tile - 4, inkOn(c, t)));
  } else {
    // the mark itself, twice its size
    parts.push(`<g transform="translate(${tile / 2} ${tile / 2 + 2}) scale(2.1)">${kindMark(k, 0, 0, c)}</g>`);
  }
  const [pill, pw] = statusPill(k, 0, 0, t);
  parts.push(`<g transform="translate(${r1(width - pw)} 4)">${pill}</g>`);
  const x0 = tile + 14;
  const room = width - x0 - pw - 10;
  parts.push(text(x0, 22, clip(row.brief.title, room, 18), t.text, 18, ' font-weight="700"'));
  const place = [row.group, row.area].filter(Boolean).join(" · ");
  parts.push(text(x0, 40, clip(place, width - x0, 12), t.muted, 12));
  const fact = row.brief.fact || row.brief.version || row.brief.path || "";
  if (fact) {
    const strong = k === "fail" || k === "warn" || k === "tip";
    parts.push(
      text(
        x0,
        57,
        clip(fact, width - x0, 12),
        strong ? fit(c, t, 4.5) : t.text,
        12,
        strong ? ' font-weight="600"' : "",
      ),
    );
  }
  return svgDoc(width, 62, parts.join(""), `${row.brief.title}: ${KIND_WORD[k]}${fact ? `, ${fact}` : ""}`);
}

// ---------------------------------------------------------------- a machine's card

/** Every check of a group as aligned rows: mark, title, fact, version. */
export function checklist(rows: CheckRow[], t: ChartTheme, width = PANE): string {
  const rowH = 22;
  const titleW = Math.min(132, Math.max(70, ...rows.map((r) => textWidth(r.brief.title, 12) * 1.05 + 14)));
  const parts: string[] = [];
  rows.forEach((r, i) => {
    const y = i * rowH;
    const cy = y + rowH / 2;
    const c = kindColor(r.kind, t);
    const problem = r.kind === "fail" || r.kind === "warn";
    if (i)
      parts.push(`<line x1="22" x2="${width}" y1="${y + 0.5}" y2="${y + 0.5}" stroke="${t.grid}" stroke-width="1"/>`);
    parts.push(kindMark(r.kind, 8, cy, c));
    parts.push(
      text(
        22,
        cy + 4.5,
        clip(r.brief.title, titleW - 8, 12),
        r.kind === "note" ? t.muted : t.text,
        12,
        problem ? ' font-weight="600"' : "",
      ),
    );
    const ver = r.brief.version ?? "";
    const verW = ver ? textWidth(ver, 11) + 10 : 0;
    const fact = r.brief.fact || r.brief.path || "";
    if (fact) {
      parts.push(
        text(22 + titleW, cy + 4.5, clip(fact, width - 22 - titleW - verW, 12), problem ? fit(c, t, 4.5) : t.muted, 12),
      );
    }
    if (ver) parts.push(text(width, cy + 4, ver, t.muted, 11, ' text-anchor="end"'));
  });
  return svgDoc(
    width,
    Math.max(1, rows.length * rowH),
    parts.join(""),
    rows.map((r) => `${KIND_GLYPH[r.kind]} ${r.brief.title} ${r.brief.fact}`).join(", "),
  );
}

/**
 * Its counts as pills (mark · count · word): failing, warnings, tips, ok —
 * a zero faint, so the row keeps its shape.
 */
export function countPills(n: Counts, t: ChartTheme, width = PANE): string {
  const list: [CheckKind, number, string][] = [
    ["fail", n.fail, "failing"],
    ["warn", n.warn, n.warn === 1 ? "warning" : "warnings"],
    ["tip", n.tip, n.tip === 1 ? "tip" : "tips"],
    ["ok", n.ok, "ok"],
  ];
  const h = 26;
  const gap = 8;
  const parts: string[] = [];
  let x = 0;
  let y = 0;
  for (const [k, count, word] of list) {
    const w = 30 + textWidth(String(count), 13) * 1.05 + 5 + textWidth(word, 12) + 12;
    if (x > 0 && x + w > width) {
      x = 0;
      y += h + gap;
    }
    const on = count > 0;
    const c = kindColor(k, t);
    parts.push(
      `<rect x="${r1(x)}" y="${y}" width="${r1(w)}" height="${h}" rx="${h / 2}" fill="${on ? c : t.grid}" fill-opacity="${on ? (t.dark ? 0.17 : 0.12) : 0.45}"/>`,
    );
    parts.push(kindMark(k, x + 18, y + h / 2, on ? c : t.other));
    parts.push(
      `<text x="${r1(x + 30)}" y="${y + 17.5}"><tspan fill="${on ? t.text : t.muted}" font-size="13" font-weight="700">${count}</tspan><tspan dx="5" fill="${on ? t.text : t.muted}" font-size="12">${word}</tspan></text>`,
    );
    x += w + gap;
  }
  return svgDoc(width, y + h, parts.join(""), countsText(n));
}

const md = (s: string) => s.replace(/[<>*_`#[\]|\\]/g, (c) => `\\${c}`);

/** A fix as Markdown: its command as code, what it's for after it. */
function fixMarkdown(f: FixStep): string {
  return "```\n" + f.cmd + "\n```" + (f.why ? `\n${md(f.why)}` : "");
}

/** A machine's (or This Mac's, the daemon's) detail: a hero, pills, the checklist, the fixes for its problems. */
export function groupMarkdown(
  g: DoctorGroup,
  o: { t: ChartTheme; logo: LogoFn; color?: string; width?: number; fixes?: boolean },
): string {
  const { t } = o;
  const w = o.width ?? PANE;
  const os = osId(g.os) ?? (g.name === "This Mac" ? osId("macOS") : undefined);
  const tags: Part[] = [];
  if (g.os) tags.push({ text: g.os });
  if (g.aleph && !g.machine) tags.push({ text: `aleph ${g.aleph}` });
  if (g.herdr) tags.push({ text: `herdr ${g.herdr}` });
  const total = g.rows.length;
  const passing = g.counts.ok + g.counts.note + g.counts.tip;
  const card = hero(
    {
      title: g.name,
      color: o.color,
      subtitle: g.address ?? (g.machine ? "" : `${total} checks`),
      logo: o.logo("os", os?.id),
      letter: g.name,
      tags,
      aside: { value: `${passing}/${total}`, label: "checks fine" },
    },
    t,
    w,
  );
  const out = [img(`${g.name}: ${countsText(g.counts)}`, card)];
  out.push(img(countsText(g.counts), countPills(g.counts, t, w)));
  out.push(img(`${g.name}'s checks`, checklist(g.rows, t, w)));
  if (o.fixes !== false) {
    const todo = g.rows.filter((r) => (r.kind === "fail" || r.kind === "warn") && r.fixes.length);
    if (todo.length) {
      out.push(`**To fix** · ↵ runs the first`);
      for (const r of todo.slice(0, 4)) out.push(`${md(r.brief.title)}\n${fixMarkdown(r.fixes[0])}`);
    }
  }
  return out.join("\n\n");
}

/** A check's detail: its card, why it matters, its fix (↵ runs it when it's an Aleph command). */
export function checkMarkdown(row: CheckRow, o: { t: ChartTheme; logo: LogoFn; width?: number }): string {
  const out = [img(`${row.brief.title}: ${KIND_WORD[row.kind]}`, checkCard(row, o.t, o.logo, o.width ?? PANE))];
  const why = row.brief.why ?? row.fixes.find((f) => f.why)?.why;
  if (why) out.push(md(why.charAt(0).toUpperCase() + why.slice(1)));
  if (row.fixes.length) {
    const runs = row.fixes.some((f) => f.run);
    out.push(`**Fix**${runs ? " · ↵ runs it in a terminal" : " · ⌘C copies it"}`);
    for (const f of row.fixes) out.push("```\n" + f.cmd + "\n```");
  }
  return out.join("\n\n");
}

/** Every group's card, one after the other (Machines › Doctor). */
export function reportMarkdown(
  groups: DoctorGroup[],
  o: { t: ChartTheme; logo: LogoFn; color?: (name: string) => string | undefined; width?: number },
): string {
  return groups.map((g) => groupMarkdown(g, { ...o, color: o.color?.(g.name) })).join("\n\n---\n\n");
}

// ---------------------------------------------------------------- by agent

/** One agent's checks across the machines: each row titled by its machine. */
export interface AgentDoctor {
  agent: string;
  name: string;
  rows: CheckRow[];
  counts: Counts;
  worst: CheckKind;
}

/** The agents' checks gathered from every machine (each row titled by its machine), problems first. */
export function agentGroups(groups: DoctorGroup[], nameOf: (id: string) => string = (id) => id): AgentDoctor[] {
  const by = new Map<string, CheckRow[]>();
  for (const g of groups)
    for (const r of g.rows)
      if (r.agent) by.set(r.agent, [...(by.get(r.agent) ?? []), { ...r, brief: { ...r.brief, title: g.name } }]);
  return [...by.entries()]
    .map(([agent, rows]) => {
      const sorted = [...rows].sort((a, b) => KIND_RANK[a.kind] - KIND_RANK[b.kind] || a.group.localeCompare(b.group));
      const counts = countKinds(sorted);
      return { agent, name: nameOf(agent), rows: sorted, counts, worst: worstOf(counts) };
    })
    .sort((a, b) => KIND_RANK[a.worst] - KIND_RANK[b.worst] || a.name.localeCompare(b.name));
}

/** The checks that need doing something, failing first, from every group. */
export const problems = (groups: DoctorGroup[]): CheckRow[] =>
  groups
    .flatMap((g) => g.rows)
    .filter((r) => r.kind === "fail" || r.kind === "warn")
    .sort((a, b) => KIND_RANK[a.kind] - KIND_RANK[b.kind]);

/** An agent's detail: its card, pills, a row per machine, the fixes. */
export function agentDoctorMarkdown(a: AgentDoctor, o: { t: ChartTheme; logo: LogoFn; width?: number }): string {
  const { t } = o;
  const w = o.width ?? PANE;
  const total = a.rows.length;
  const fine = a.counts.ok + a.counts.note + a.counts.tip;
  const out = [
    img(
      `${a.name}: ${countsText(a.counts)}`,
      hero(
        {
          title: a.name,
          subtitle: `on ${total} machine${total === 1 ? "" : "s"}`,
          logo: o.logo("agent", a.agent),
          letter: a.name,
          aside: { value: `${fine}/${total}`, label: "machines fine" },
        },
        t,
        w,
      ),
    ),
    img(countsText(a.counts), countPills(a.counts, t, w)),
    img(`${a.name} on each machine`, checklist(a.rows, t, w)),
  ];
  const todo = a.rows.filter((r) => (r.kind === "fail" || r.kind === "warn") && r.fixes.length);
  if (todo.length) {
    out.push(`**To fix** · ↵ runs the first`);
    for (const r of todo.slice(0, 4)) out.push(`${md(r.brief.title)}\n${fixMarkdown(r.fixes[0])}`);
  }
  return out.join("\n\n");
}
