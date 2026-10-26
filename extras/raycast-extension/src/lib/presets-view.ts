// Presets, drawn: one preset's detail (`preset ls --json`) as Markdown with
// SVG cards — a hero card, a picture of the tab it opens (each pane where
// herdr puts it: grid, columns, rows, an exact layout, or one tab per
// session) with its agent's logo, its machine and folder, and the facts of
// a one-session preset — and what a list row needs (its machines, agents,
// section). No Raycast imports: plain node tests it.
import type { Preset as PresetJson, PresetPane } from "./cli";
import { ChartTheme, clip, esc, fit, mix, svgDoc, text, textWidth } from "./charts";
import { Fact, facts, hero, logoMark, PANE, Part } from "./dash";
import { BrandLike, img, LogoFn } from "./stats-view";

/** `preset ls --json`, with the exact layout's shape when it has one (`(train | (review / eval))`). */
export type Preset = PresetJson & { shape?: string | null; autostart?: boolean };

/** `/home/me/x` → `~/x` (any home: we don't know the remote user). */
export const tilde = (p: string) => p.replace(/^\/(home|Users)\/[^/]+(?=\/|$)/, "~").replace(/^\/root(?=\/|$)/, "~");

const md = (s: string) => s.replace(/[<>*_`#[\]|\\]/g, (c) => `\\${c}`);

/** The sessions a preset starts (old one-session presets have no `panes`). */
export function panesOf(p: Preset): PresetPane[] {
  if (p.panes?.length) return p.panes;
  return [
    {
      name: null,
      host: p.host,
      agent: p.agent,
      dir: p.dir,
      worktree: p.worktree ? "auto" : null,
      args: p.args ? p.args.split(/\s+/).filter(Boolean) : [],
      prompt: p.prompt,
    },
  ];
}

/** What a list row and the detail need to know about the fleet and Aleph's settings. */
export interface PresetCtx {
  /** where a pane without a machine starts (the `machine` setting) */
  fallback: string;
  /** the agent a pane without one starts on that machine (`host.<a>.agent`, else `agent`) */
  defaultAgent: (host: string) => string;
  /** a machine's own colour (hex), when the fleet knows it */
  color: (alias: string) => string | undefined;
}

/** The machines a preset starts on, in pane order (a pane without one: the default machine). */
export function presetHosts(p: Preset, ctx: Pick<PresetCtx, "fallback">): string[] {
  return [...new Set(panesOf(p).map((x) => x.host || ctx.fallback))];
}

/** The agents it starts, in pane order (a pane without one: that machine's default). */
export function presetAgents(p: Preset, ctx: Pick<PresetCtx, "fallback" | "defaultAgent">): string[] {
  return [
    ...new Set(
      panesOf(p)
        .map((x) => x.agent || ctx.defaultAgent(x.host || ctx.fallback))
        .filter(Boolean),
    ),
  ];
}

/** The list section a preset goes in: its one machine, else "Several machines". */
export function presetSection(p: Preset, ctx: Pick<PresetCtx, "fallback">): string {
  const hosts = presetHosts(p, ctx);
  return hosts.length === 1 ? hosts[0] : "Several machines";
}

/** The layout in words: `grid`, `3 columns`, `exact layout`, `3 tabs`. */
export function layoutWord(p: Preset): string {
  const n = panesOf(p).length;
  if (n === 1) return "one session";
  if (p.shape) return "exact layout";
  switch (p.layout) {
    case "grid":
      return "grid";
    case "columns":
      return `${n} columns`;
    case "rows":
      return `${n} rows`;
    default:
      return `${n} tabs`;
  }
}

// ---------------------------------------------------------------- the layout

/** Where herdr puts each pane: a split (side by side `right`, stacked `down`) or a pane. */
export type Split = { pane: number } | { dir: "right" | "down"; parts: { node: Split; w: number }[] };

/** Equal parts, side by side or stacked. */
const chain = (nodes: Split[], dir: "right" | "down"): Split =>
  nodes.length === 1 ? nodes[0] : { dir, parts: nodes.map((node) => ({ node, w: 1 })) };

/** herdr's even grid (layouts.rs `grid`): ⌈√n⌉ columns, the first ones a row taller, numbered down each column. */
export function gridSplit(n: number): Split {
  const cols = Math.ceil(Math.sqrt(n));
  const base = Math.floor(n / cols);
  const extra = n % cols;
  let next = 0;
  const columns: Split[] = [];
  for (let c = 0; c < cols; c++) {
    const rows = base + (c < extra ? 1 : 0);
    columns.push(
      chain(
        Array.from({ length: rows }, (_, k) => ({ pane: next + k })),
        "down",
      ),
    );
    next += rows;
  }
  return chain(columns, "right");
}

/**
 * An exact layout as `preset ls` describes it (layouts.rs `describe`):
 * `(train | (review / eval 70%))` — `|` side by side, `/` stacked, a
 * percentage for the first part when it isn't half. Leaves are pane names.
 */
export function parseShape(shape: string, names: (string | null)[]): Split | undefined {
  let i = 0;
  const used = new Set<number>();
  const ws = () => {
    while (shape[i] === " ") i++;
  };
  const node = (): Split | undefined => {
    ws();
    if (shape[i] === "(") {
      i++;
      const a = node();
      ws();
      let ratio = 0.5;
      const pct = /^(\d+(?:\.\d+)?)%/.exec(shape.slice(i));
      if (pct) {
        ratio = Number(pct[1]) / 100;
        i += pct[0].length;
        ws();
      }
      const sep = shape[i];
      if (sep !== "|" && sep !== "/") return undefined;
      i++;
      const b = node();
      ws();
      if (shape[i] !== ")" || !a || !b) return undefined;
      i++;
      return {
        dir: sep === "|" ? "right" : "down",
        parts: [
          { node: a, w: ratio },
          { node: b, w: 1 - ratio },
        ],
      };
    }
    const m = /^[^()|/\s]+/.exec(shape.slice(i));
    if (!m) return undefined;
    i += m[0].length;
    let k = names.findIndex((n, j) => n === m[0] && !used.has(j));
    if (k < 0) k = names.findIndex((_, j) => !used.has(j));
    if (k < 0) return undefined;
    used.add(k);
    return { pane: k };
  };
  const out = node();
  ws();
  return out && i === shape.length ? out : undefined;
}

/** The split a preset's tab is cut into; undefined: one tab per session (`tabs`). */
export function presetSplit(p: Preset): Split | undefined {
  const panes = panesOf(p);
  const n = panes.length;
  if (n < 2) return undefined;
  if (p.shape) {
    const s = parseShape(
      p.shape,
      panes.map((x) => x.name),
    );
    if (s) return s;
  }
  if (p.layout === "grid") return gridSplit(n);
  if (p.layout === "columns")
    return chain(
      panes.map((_, i) => ({ pane: i })),
      "right",
    );
  if (p.layout === "rows")
    return chain(
      panes.map((_, i) => ({ pane: i })),
      "down",
    );
  return undefined;
}

export interface Cell {
  pane: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Each pane's rectangle in a box, `gap` px between neighbours. */
export function cells(s: Split, x: number, y: number, w: number, h: number, gap = 6): Cell[] {
  if ("pane" in s) return [{ pane: s.pane, x, y, w, h }];
  const total = s.parts.reduce((a, p) => a + p.w, 0) || 1;
  const room = (s.dir === "right" ? w : h) - gap * (s.parts.length - 1);
  const out: Cell[] = [];
  let at = s.dir === "right" ? x : y;
  for (const p of s.parts) {
    const size = (room * p.w) / total;
    out.push(...(s.dir === "right" ? cells(p.node, at, y, size, h, gap) : cells(p.node, x, at, w, size, gap)));
    at += size + gap;
  }
  return out;
}

/** How many panes deep the stack is at its tallest (rows of text the picture needs). */
function depth(s: Split): number {
  if ("pane" in s) return 1;
  const d = s.parts.map((p) => depth(p.node));
  return s.dir === "down" ? d.reduce((a, b) => a + b, 0) : Math.max(...d);
}

/** One pane as it appears in the picture. */
export interface PaneLook {
  /** the pane's name, else its agent's */
  name: string;
  agent: string;
  agentName: string;
  agentColor: string;
  logo?: string;
  host: string;
  hostColor?: string;
  /** `~/x`, or "" for the machine's default folder */
  dir: string;
  /** `new worktree`, a branch, or "" */
  worktree: string;
  /** the default machine stood in for a missing one */
  defaultHost: boolean;
}

export function paneLooks(p: Preset, ctx: PresetCtx, logo: LogoFn, brand: (id: string) => BrandLike): PaneLook[] {
  return panesOf(p).map((x) => {
    const host = x.host || ctx.fallback;
    const agent = x.agent || ctx.defaultAgent(host);
    const b = brand(agent);
    return {
      name: x.name || b.name,
      agent,
      agentName: b.name,
      agentColor: b.color,
      logo: logo("agent", agent),
      host,
      hostColor: ctx.color(host),
      dir: x.dir ? tilde(x.dir) : "",
      worktree: x.worktree ? (x.worktree === "auto" ? "new worktree" : x.worktree) : "",
      defaultHost: !x.host,
    };
  });
}

/** A pane drawn in its rectangle: a tint of its agent's colour, the logo, the name, then the machine and folder as fit. */
function paneCell(c: Cell, look: PaneLook, t: ChartTheme, focus: boolean): string {
  const tint = fit(look.agentColor, t);
  const parts: string[] = [
    `<rect x="${c.x + 0.5}" y="${c.y + 0.5}" width="${Math.max(0, c.w - 1)}" height="${Math.max(0, c.h - 1)}" rx="8" fill="${tint}" fill-opacity="${t.dark ? 0.13 : 0.09}" stroke="${focus ? t.accent : mix(t.grid, tint, 0.35)}" stroke-width="${focus ? 1.5 : 1}"/>`,
  ];
  const pad = 9;
  const right = c.x + c.w - pad;
  const logo = c.w >= 70;
  const nx = c.x + pad + (logo ? 22 : 0);
  const dot = look.hostColor ? fit(look.hostColor, t) : t.other;
  const where = (x: number, y: number, s: string) => {
    parts.push(`<circle cx="${(x + 4).toFixed(1)}" cy="${(y - 4).toFixed(1)}" r="3.5" fill="${dot}"/>`);
    parts.push(text(x + 12, y, clip(s, right - x - 12, 11), t.muted, 11));
  };
  const folder = [look.dir || "default folder", look.worktree].filter(Boolean).join(" · ");
  const mark = (y: number) =>
    logo
      ? parts.push(logoMark(c.x + pad, y - 13, 16, look.logo, { letter: look.agentName, color: look.agentColor }, t))
      : 0;
  if (c.w >= 240 && c.h < 62) {
    // wide and short (rows): all of it on one line
    const y = c.y + c.h / 2 + 4.5;
    mark(y);
    const name = clip(look.name, 110, 12, false);
    parts.push(text(nx, y, name, t.text, 12, ' font-weight="600"'));
    where(nx + textWidth(name, 12) * 1.06 + 12, y, `${look.host} · ${folder}`);
  } else {
    // one line: the name; two: + the machine and folder; three: the folder on its own line
    const lines = c.h >= 62 ? 3 : c.h >= 42 ? 2 : 1;
    const y1 = lines === 1 ? c.y + c.h / 2 + 4.5 : c.y + 21;
    mark(y1);
    parts.push(text(nx, y1, clip(look.name, right - nx, 12, false), t.text, 12, ' font-weight="600"'));
    if (lines >= 2) where(c.x + pad, y1 + 17, lines === 2 ? `${look.host} · ${folder}` : look.host);
    if (lines === 3) parts.push(text(c.x + pad, y1 + 33, clip(folder, right - c.x - pad, 11), t.muted, 11));
  }
  const tip = [look.name, look.agentName, look.host, look.dir, look.worktree].filter(Boolean).join(" · ");
  return `<g><title>${esc(tip)}</title>${parts.join("")}</g>`;
}

/**
 * The tab a preset opens, as a picture: its panes where herdr puts them, or
 * — one tab per session — a card per tab. The pane opened in Focus
 * afterwards is ringed in gold.
 */
export function layoutCard(p: Preset, looks: PaneLook[], t: ChartTheme, width = PANE): string {
  const split = presetSplit(p);
  const focus = looks.findIndex((l, i) => (panesOf(p)[i].name ?? l.name) === p.focus);
  const parts: string[] = [];
  const top = 22;
  const n = looks.length;
  const note = focus >= 0 ? `opens ${looks[focus].name} in Focus` : "";
  parts.push(
    `<text x="0" y="13"><tspan fill="${t.text}" font-size="12" font-weight="600">${esc(
      split ? `One tab · ${layoutWord(p)}` : `${n} tabs`,
    )}</tspan>${note ? `<tspan dx="6" fill="${t.muted}" font-size="12">${esc(note)}</tspan>` : ""}</text>`,
  );
  let h: number;
  if (split) {
    const d = depth(split);
    // three lines a pane when there's room, fewer when stacked deep
    h = Math.min(280, Math.max(84, d * (d > 3 ? 50 : 68)));
    for (const c of cells(split, 0, top, width, h)) parts.push(paneCell(c, looks[c.pane], t, c.pane === focus));
  } else {
    // one card per tab, a tab's ear on top
    const per = n <= 3 ? n : n === 4 ? 2 : 3;
    const gap = 8;
    const cw = (width - gap * (per - 1)) / per;
    const ch = 72;
    const rows = Math.ceil(n / per);
    looks.forEach((l, i) => {
      const x = (i % per) * (cw + gap);
      const y = top + 7 + Math.floor(i / per) * (ch + 7 + gap);
      const ear = Math.min(46, cw * 0.4);
      parts.push(
        `<path d="M${(x + 6).toFixed(1)},${y + 1}V${y - 3}Q${(x + 6).toFixed(1)},${y - 7} ${(x + 10).toFixed(1)},${y - 7}H${(x + 6 + ear - 4).toFixed(1)}Q${(x + 6 + ear).toFixed(1)},${y - 7} ${(x + 6 + ear).toFixed(1)},${y - 3}V${y + 1}Z" fill="${fit(l.agentColor, t)}" fill-opacity="${t.dark ? 0.35 : 0.3}"/>`,
      );
      parts.push(paneCell({ pane: i, x, y, w: cw, h: ch }, l, t, i === focus));
    });
    h = 7 + rows * (ch + 7 + gap) - gap - 7;
  }
  return svgDoc(width, top + h + 1, parts.join(""), `${p.name}: ${layoutWord(p)}`);
}

// ---------------------------------------------------------------- the detail

/** A transparent pixel: the hero's own logo slot, when the logos are drawn over it. */
const BLANK =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

/** Two or three agents' logos in the hero's tile (two side by side, a third under them). */
function logoCluster(looks: PaneLook[], t: ChartTheme): string {
  const seen = new Map<string, PaneLook>();
  for (const l of looks) if (!seen.has(l.agent)) seen.set(l.agent, l);
  const list = [...seen.values()].slice(0, 3);
  const at: [number, number, number][] =
    list.length === 2
      ? [
          [5, 18, 20],
          [27, 18, 20],
        ]
      : [
          [7, 9, 17],
          [28, 9, 17],
          [17.5, 29, 17],
        ];
  return list
    .map((l, i) => logoMark(at[i][0], at[i][1], at[i][2], l.logo, { letter: l.agentName, color: l.agentColor }, t))
    .join("");
}

/**
 * The hero card: the preset's name, its description (else its folder, or
 * its layout), its agent's logo (several agents: theirs together), its
 * machines and tags; how many sessions on the right.
 */
export function presetHero(p: Preset, looks: PaneLook[], t: ChartTheme, width = PANE): string {
  const n = looks.length;
  const agents = new Set(looks.map((l) => l.agent));
  const hosts = [...new Set(looks.map((l) => l.host))];
  const first = looks[0];
  const tags: Part[] = [];
  if (n > 1)
    for (const h of hosts.slice(0, 3)) {
      const l = looks.find((x) => x.host === h);
      tags.push({ text: h, dot: l?.hostColor ? fit(l.hostColor, t) : t.other });
    }
  for (const tag of p.tags ?? []) tags.push({ text: `#${tag}`, muted: true });
  // what the picture under it doesn't say: the folder of a one-session preset, the agents of the others
  const subtitle =
    p.description?.trim() ||
    (n === 1 ? first?.dir || "the machine's default folder" : [...new Set(looks.map((l) => l.agentName))].join(", "));
  const many = agents.size > 1;
  const svg = hero(
    {
      title: p.name,
      color: hosts.length === 1 ? first?.hostColor : undefined,
      subtitle,
      logo: many ? BLANK : first?.logo,
      letter: first?.agentName,
      tags,
      aside: n > 1 ? { value: String(n), label: "sessions" } : undefined,
    },
    t,
    width,
  );
  return many ? svg.replace(/<\/svg>$/, `${logoCluster(looks, t)}</svg>`) : svg;
}

/** Where presets are saved unless an included file says otherwise (not worth a line). */
const MAIN_CONFIG = /\/aleph\/aleph\.toml$/;

/** A one-session preset's facts: agent, machine, folder (when the description took its place), worktree, arguments. */
export function presetFacts(p: Preset, look: PaneLook, pane: PresetPane, t: ChartTheme): Fact[] {
  const rows: Fact[] = [
    {
      label: "Agent",
      parts: [
        {
          text: look.agentName,
          logo: look.logo,
          letter: look.agentName,
          letterColor: look.agentColor,
          note: pane.agent ? undefined : "default",
        },
      ],
    },
    {
      label: "Machine",
      parts: [
        {
          text: look.host,
          dot: look.hostColor ? fit(look.hostColor, t) : t.other,
          note: look.defaultHost ? "default" : undefined,
        },
      ],
    },
  ];
  if (p.description?.trim())
    rows.push({
      label: "Folder",
      parts: [look.dir ? { text: look.dir } : { text: "the machine's default", muted: true }],
    });
  if (look.worktree) rows.push({ label: "Worktree", parts: [{ text: look.worktree }] });
  if (pane.args.length) rows.push({ label: "Args", parts: [{ text: pane.args.join(" ") }] });
  if (p.source && !MAIN_CONFIG.test(p.source))
    rows.push({ label: "Saved in", parts: [{ text: tilde(p.source), muted: true }] });
  return rows;
}

/** The whole detail of one preset. */
export function presetMarkdown(o: {
  p: Preset;
  ctx: PresetCtx;
  t: ChartTheme;
  logo: LogoFn;
  brand: (id: string) => BrandLike;
  width?: number;
}): string {
  const { p, t } = o;
  const w = o.width ?? PANE;
  const panes = panesOf(p);
  const looks = paneLooks(p, o.ctx, o.logo, o.brand);
  const out = [img(`${p.name}: ${layoutWord(p)}`, presetHero(p, looks, t, w))];
  if (panes.length === 1) {
    out.push(img("What it starts", facts(presetFacts(p, looks[0], panes[0], t), t, w)));
  } else {
    out.push(img(`The tab it opens: ${layoutWord(p)}`, layoutCard(p, looks, t, w)));
    const extra: Fact[] = [];
    panes.forEach((x, i) => {
      const bits: Part[] = [];
      if (x.args.length) bits.push({ text: x.args.join(" ") });
      if (x.worktree && x.worktree !== "auto") bits.push({ text: x.worktree, note: "branch" });
      if (bits.length) extra.push({ label: looks[i].name, parts: bits });
    });
    if (p.source && !MAIN_CONFIG.test(p.source))
      extra.push({ label: "Saved in", parts: [{ text: tilde(p.source), muted: true }] });
    if (extra.length) out.push(img("Arguments and branches", facts(extra, t, w, { labelW: 84 })));
  }
  // the first prompts, as they'll be sent
  const quote = (s: string) => md(s).replace(/\n/g, "\n> ");
  panes.forEach((x, i) => {
    const prompt = x.prompt?.trim();
    if (prompt) out.push(panes.length === 1 ? `> ${quote(prompt)}` : `> **${md(looks[i].name)}** · ${quote(prompt)}`);
  });
  return out.join("\n\n");
}
