// Dashboard pieces for the list details (Machine Stats, Machines, Usage):
// arc gauges, a per-core heat grid, trend rows with sparklines, aligned
// facts with logos, meters, a hero card, state pills, session rows and logo
// chips. SVG for Raycast's Detail markdown, built on charts.ts: drawn for the
// theme in use on a transparent background, text in the theme's text colours,
// colours that mean something (Aleph's state colours, a load ramp, a
// machine's own colour, gold for Aleph). Sized for a list's detail pane
// (PANE px wide). No Raycast imports: plain node tests it.
import { ChartTheme, clip as clipText, contrast, esc, fit, mix, svgDoc, text, textWidth } from "./charts";

/** The width of a list's detail pane, about (Raycast scales a wider image down). */
export const PANE = 400;

/** Cut to fit, from the end: here a `/` is a ratio (`22G / 24G`), not a path. */
const clip = (s: string, px: number, size: number) => clipText(s, px, size, false);

const r1 = (v: number) => Math.round(v * 10) / 10;

// ---------------------------------------------------------------- colour

/** How hot a load is (0..1), like the list's tags: green, yellow from 70 %, red from 90 %. */
export function severity(f: number | null | undefined, t: ChartTheme): string {
  const v = f ?? 0;
  return v >= 0.9 ? t.state.down : v >= 0.7 ? t.state.warn : t.state.done;
}

/** A temperature's colour (°C): yellow from 75, red from 85. */
export function tempSeverity(c: number | null | undefined, t: ChartTheme): string {
  const v = c ?? 0;
  return v >= 85 ? t.state.down : v >= 75 ? t.state.warn : t.state.done;
}

/**
 * A continuous load ramp for the core grid (with its scale shown): an idle
 * core sits just off the surface, then green, yellow, red.
 */
export function heatColor(f: number, t: ChartTheme): string {
  const v = Math.min(1, Math.max(0, f));
  const idle = mix(t.surface, t.state.done, t.dark ? 0.22 : 0.18);
  const warm = t.dark ? t.state.warn : fit("#D9A400", t, 2);
  const stops: [number, string][] = [
    [0, idle],
    [0.4, t.state.done],
    [0.75, warm],
    [1, t.state.down],
  ];
  for (let i = 1; i < stops.length; i++) {
    const [a, ca] = stops[i - 1];
    const [b, cb] = stops[i];
    if (v <= b) return mix(ca, cb, (v - a) / (b - a));
  }
  return stops[stops.length - 1][1];
}

/** Ink that reads on a fill (white or near-black, whichever contrasts more). */
function inkOn(fill: string): string {
  return contrast(fill, "#ffffff") >= contrast(fill, "#111111") ? "#ffffff" : "#111111";
}

// ---------------------------------------------------------------- small marks

/** An embedded logo (a data URI), or a monogram tile in `color` when there's none. */
export function logoMark(
  x: number,
  y: number,
  size: number,
  logo: string | undefined,
  fallback: { letter?: string; color?: string },
  t: ChartTheme,
): string {
  if (logo) {
    const h = esc(logo);
    return `<image href="${h}" xlink:href="${h}" x="${r1(x)}" y="${r1(y)}" width="${size}" height="${size}" preserveAspectRatio="xMidYMid meet"/>`;
  }
  const c = fit(fallback.color ?? t.other, t);
  const letter = (fallback.letter ?? "").slice(0, 1).toUpperCase();
  return (
    `<rect x="${r1(x)}" y="${r1(y)}" width="${size}" height="${size}" rx="${r1(size * 0.26)}" fill="${c}"/>` +
    (letter
      ? text(
          x + size / 2,
          y + size * 0.72,
          letter,
          inkOn(c),
          Math.round(size * 0.62),
          ' text-anchor="middle" font-weight="700"',
        )
      : "")
  );
}

/**
 * A session's (or machine's) state as a shape, never a font glyph: ▲ needs
 * you, a turning arc while working, ✓ done, ● idle, a clock at its limit,
 * ✕ down. Centred on (cx, cy), about 12 px.
 */
export function stateMark(state: string, cx: number, cy: number, color: string): string {
  const x = r1(cx);
  const y = r1(cy);
  switch (state) {
    case "blocked":
      return `<path d="M${x},${r1(cy - 5.5)}L${r1(cx + 6)},${r1(cy + 4.5)}H${r1(cx - 6)}Z" fill="${color}" stroke="${color}" stroke-width="1.5" stroke-linejoin="round"/>`;
    case "working":
      return `<path d="M${r1(cx + 5)},${y}A5 5 0 1 1 ${x},${r1(cy - 5)}" fill="none" stroke="${color}" stroke-width="2.2" stroke-linecap="round"/><circle cx="${x}" cy="${y}" r="1.6" fill="${color}"/>`;
    case "done":
      return `<path d="M${r1(cx - 5)},${r1(cy + 0.5)}L${r1(cx - 1.5)},${r1(cy + 4)}L${r1(cx + 5.5)},${r1(cy - 4)}" fill="none" stroke="${color}" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>`;
    case "idle":
      return `<circle cx="${x}" cy="${y}" r="4.5" fill="${color}"/>`;
    case "limit":
      return `<circle cx="${x}" cy="${y}" r="5.5" fill="none" stroke="${color}" stroke-width="1.8"/><path d="M${x},${r1(cy - 3)}V${y}L${r1(cx + 2.5)},${r1(cy + 1.5)}" fill="none" stroke="${color}" stroke-width="1.6" stroke-linecap="round"/>`;
    case "down":
      return `<path d="M${r1(cx - 4.5)},${r1(cy - 4.5)}L${r1(cx + 4.5)},${r1(cy + 4.5)}M${r1(cx + 4.5)},${r1(cy - 4.5)}L${r1(cx - 4.5)},${r1(cy + 4.5)}" stroke="${color}" stroke-width="2.2" stroke-linecap="round"/>`;
    case "warn":
      return `<path d="M${x},${r1(cy - 5.5)}L${r1(cx + 6)},${r1(cy + 4.5)}H${r1(cx - 6)}Z" fill="none" stroke="${color}" stroke-width="1.6" stroke-linejoin="round"/><path d="M${x},${r1(cy - 2)}V${r1(cy + 1)}" stroke="${color}" stroke-width="1.6" stroke-linecap="round"/>`;
    default:
      return `<circle cx="${x}" cy="${y}" r="4.5" fill="none" stroke="${color}" stroke-width="1.6"/>`;
  }
}

/** Signal bars (0..4 lit) for a link, bottom-aligned at `y`, starting at `x`: 22 px wide. */
export function signalBars(x: number, y: number, lit: number, color: string, t: ChartTheme): string {
  return [5, 8, 11, 14]
    .map(
      (h, i) =>
        `<rect x="${r1(x + i * 6)}" y="${r1(y - h)}" width="4" height="${h}" rx="1.2" fill="${i < lit ? color : t.grid}"/>`,
    )
    .join("");
}

/** A small line over `points` (oldest first) in a box; `max` tops the scale (default: the largest point). */
export function sparkline(
  points: [number, number | null][],
  box: { x: number; y: number; w: number; h: number; from: number; to: number; max?: number; min?: number },
  color: string,
  t: ChartTheme,
  o: { area?: boolean; dot?: boolean } = {},
): string {
  const pts = points.filter((p): p is [number, number] => p[1] != null && p[0] >= box.from && p[0] <= box.to);
  if (pts.length < 2) return "";
  const lo = box.min ?? 0;
  const hi = Math.max(box.max ?? 0, ...pts.map((p) => p[1]), lo + 1e-9);
  const span = Math.max(1e-9, box.to - box.from);
  const X = (tt: number) => box.x + ((tt - box.from) / span) * box.w;
  const Y = (v: number) => box.y + box.h - ((Math.min(hi, Math.max(lo, v)) - lo) / (hi - lo)) * box.h;
  const line = pts.map((p, i) => `${i ? "L" : "M"}${r1(X(p[0]))},${r1(Y(p[1]))}`).join("");
  const parts: string[] = [];
  if (o.area !== false) {
    const base = r1(box.y + box.h);
    parts.push(
      `<path d="${line}L${r1(X(pts[pts.length - 1][0]))},${base}L${r1(X(pts[0][0]))},${base}Z" fill="${color}" fill-opacity="0.12"/>`,
    );
  }
  parts.push(
    `<path d="${line}" fill="none" stroke="${color}" stroke-width="1.8" stroke-linejoin="round" stroke-linecap="round"/>`,
  );
  if (o.dot !== false) {
    const [lx, ly] = [X(pts[pts.length - 1][0]), Y(pts[pts.length - 1][1])];
    parts.push(
      `<circle cx="${r1(lx)}" cy="${r1(ly)}" r="3" fill="${color}" stroke="${t.surface}" stroke-width="1.5"/>`,
    );
  }
  return parts.join("");
}

// ---------------------------------------------------------------- gauges

export interface Gauge {
  label: string;
  /** 0..1; null/undefined: no reading */
  frac?: number | null;
  /** what the centre says (default: the percentage) */
  value?: string;
  /** under the label: `10 cores`, `12.6 / 16G` */
  sub?: string;
  /** the arc's colour (default: by load) */
  color?: string;
}

/** Arc point at `a` degrees (counter-clockwise from 3 o'clock, like maths). */
function polar(cx: number, cy: number, r: number, a: number): [number, number] {
  const rad = (a * Math.PI) / 180;
  return [cx + r * Math.cos(rad), cy - r * Math.sin(rad)];
}

/** An arc drawn clockwise from `a0` down to `a1` degrees. */
function arcPath(cx: number, cy: number, r: number, a0: number, a1: number): string {
  const [x0, y0] = polar(cx, cy, r, a0);
  const [x1, y1] = polar(cx, cy, r, a1);
  return `M${r1(x0)},${r1(y0)}A${r1(r)},${r1(r)} 0 ${a0 - a1 > 180 ? 1 : 0} 1 ${r1(x1)},${r1(y1)}`;
}

const SWEEP = 240;
const START = 210;

/** A row of arc gauges (CPU, memory, GPU, VRAM, temperature): the fill by load on a tint of its own colour. */
export function gauges(list: Gauge[], t: ChartTheme, width = PANE): string {
  const n = Math.max(1, list.length);
  const cell = width / n;
  const r = Math.max(22, Math.min(32, cell * 0.32));
  const sw = r >= 28 ? 7 : 6;
  const cy = 4 + sw / 2 + r;
  const labelY = cy + r * 0.5 + sw / 2 + 17;
  const h = labelY + (list.some((g) => g.sub) ? 20 : 6);
  const body = list
    .map((g, i) => {
      const cx = cell * i + cell / 2;
      const f = g.frac == null || !Number.isFinite(g.frac) ? undefined : Math.min(1, Math.max(0, g.frac));
      const c = g.color ?? (f === undefined ? t.other : severity(f, t));
      const parts = [
        `<path d="${arcPath(cx, cy, r, START, START - SWEEP)}" fill="none" stroke="${f === undefined ? t.grid : c}" stroke-opacity="${f === undefined ? 1 : 0.2}" stroke-width="${sw}" stroke-linecap="round"/>`,
      ];
      if (f !== undefined && f * SWEEP >= 1) {
        parts.push(
          `<path d="${arcPath(cx, cy, r, START, START - SWEEP * f)}" fill="none" stroke="${c}" stroke-width="${sw}" stroke-linecap="round"/>`,
        );
      }
      const value = g.value ?? (f === undefined ? "–" : `${Math.round(f * 100)}%`);
      const vs = r >= 28 ? 17 : 14;
      parts.push(
        text(
          cx,
          cy + vs * 0.36,
          clip(value, r * 1.7, vs),
          f === undefined ? t.muted : t.text,
          vs,
          ' text-anchor="middle" font-weight="600"',
        ),
      );
      parts.push(text(cx, labelY, clip(g.label, cell - 6, 12), t.text, 12, ' text-anchor="middle"'));
      if (g.sub) parts.push(text(cx, labelY + 15, clip(g.sub, cell - 6, 11), t.muted, 11, ' text-anchor="middle"'));
      return `<g><title>${esc(`${g.label}: ${value}${g.sub ? ` · ${g.sub}` : ""}`)}</title>${parts.join("")}</g>`;
    })
    .join("");
  return svgDoc(width, h, body, list.map((g) => `${g.label} ${g.value ?? ""}`).join(", "));
}

// ---------------------------------------------------------------- cores

/** Every core's load as a heat grid (one row up to 16 cores, then rows of 16 or 32), with its scale. */
export function coreGrid(cores: number[], t: ChartTheme, width = PANE): string {
  const n = cores.length;
  if (!n) return svgDoc(width, 1, "", "no per-core numbers");
  const cols = n <= 16 ? n : n <= 128 ? 16 : 32;
  const rows = Math.ceil(n / cols);
  const gap = 3;
  const cw = Math.min(30, (width - gap * (cols - 1)) / cols);
  const ch = n > 64 ? 12 : 18;
  const top = 26;
  const avg = cores.reduce((s, v) => s + v, 0) / n;
  const busiest = cores.reduce((b, v, i) => (v > cores[b] ? i : b), 0);
  const parts: string[] = [];
  parts.push(
    `<text x="0" y="13"><tspan fill="${t.text}" font-size="12" font-weight="600">Cores</tspan><tspan dx="6" fill="${t.muted}" font-size="12">${esc(
      `${n} · avg ${Math.round(avg * 100)}% · busiest #${busiest} ${Math.round(cores[busiest] * 100)}%`,
    )}</tspan></text>`,
  );
  // the scale: 0 % … 100 %
  const lx = width - 96;
  parts.push(text(lx - 4, 12, "0%", t.muted, 10, ' text-anchor="end"'));
  for (let i = 0; i < 8; i++)
    parts.push(`<rect x="${lx + i * 8}" y="5" width="7" height="8" rx="1.5" fill="${heatColor(i / 7, t)}"/>`);
  parts.push(text(lx + 68, 12, "100%", t.muted, 10));
  cores.forEach((v, i) => {
    const x = (i % cols) * (cw + gap);
    const y = top + Math.floor(i / cols) * (ch + gap);
    parts.push(
      `<rect x="${r1(x)}" y="${r1(y)}" width="${r1(cw)}" height="${ch}" rx="3" fill="${heatColor(v, t)}"><title>core ${i}: ${Math.round(v * 100)}%</title></rect>`,
    );
  });
  return svgDoc(width, top + rows * (ch + gap), parts.join(""), `${n} cores, average ${Math.round(avg * 100)}%`);
}

// ---------------------------------------------------------------- trends

export interface Trend {
  label: string;
  /** [unix seconds, value] oldest first; null: no reading then */
  points: [number, number | null][];
  /** the top of its scale (1 for a fraction); default: the largest point */
  max?: number;
  /** now, as text */
  value: string;
  color: string;
}

/**
 * One row per measure: its label, a sparkline over the same time span for
 * every row (so they line up), and its value now. One scale per row, never
 * two in one plot. A row with fewer than two readings says so.
 */
export function trends(
  rows: Trend[],
  o: { from: number; to: number; title?: string; note?: string },
  t: ChartTheme,
  width = PANE,
): string {
  const labelW = 70;
  const valueW = 96;
  const rowH = 30;
  const top = o.title ? 22 : 0;
  const sx = labelW;
  const sw = width - labelW - valueW - 8;
  const parts: string[] = [];
  if (o.title) {
    parts.push(
      `<text x="0" y="13"><tspan fill="${t.text}" font-size="12" font-weight="600">${esc(o.title)}</tspan>${
        o.note ? `<tspan dx="6" fill="${t.muted}" font-size="12">${esc(o.note)}</tspan>` : ""
      }</text>`,
    );
  }
  rows.forEach((row, i) => {
    const y0 = top + i * rowH;
    const base = y0 + rowH - 6;
    parts.push(text(0, y0 + 19, clip(row.label, labelW - 8, 12), t.muted, 12));
    parts.push(
      `<line x1="${sx}" x2="${sx + sw}" y1="${r1(base) + 0.5}" y2="${r1(base) + 0.5}" stroke="${t.grid}" stroke-width="1"/>`,
    );
    const line = sparkline(
      row.points,
      { x: sx, y: y0 + 4, w: sw, h: rowH - 11, from: o.from, to: o.to, max: row.max },
      row.color,
      t,
    );
    parts.push(line || text(sx + sw / 2, y0 + 19, "filling in…", t.muted, 11, ' text-anchor="middle"'));
    parts.push(text(width, y0 + 19, clip(row.value, valueW, 12), t.text, 12, ' text-anchor="end" font-weight="600"'));
  });
  return svgDoc(width, top + rows.length * rowH, parts.join(""), rows.map((r) => `${r.label} ${r.value}`).join(", "));
}

// ---------------------------------------------------------------- facts

export interface Part {
  text: string;
  /** an embedded logo before the text */
  logo?: string;
  /** a monogram when there's no logo: its letter and colour */
  letter?: string;
  letterColor?: string;
  /** a dot before the text (a machine's colour, a server's health) */
  dot?: string;
  muted?: boolean;
  strong?: boolean;
  /** a warning sign before it */
  warn?: boolean;
  /** muted words right after the text, kept with it on one line (`in`, a version, `1.9G`) */
  note?: string;
  /** drawn as a chip tinted with this colour, its dot in front (a model, a project, a machine) */
  chip?: string;
}

/** A chip's padding around its words: dot and gaps in front, a little after (with slack for estimated widths). */
const CHIP_PAD = 25;
const chipWords = (s: string, size: number) => textWidth(s, size) * 1.08;

function partWidth(p: Part, size: number): number {
  if (p.chip) return CHIP_PAD + chipWords(p.text, size);
  return (
    (p.dot ? 12 : 0) +
    (p.logo || p.letter ? size + 5 : 0) +
    (p.warn ? 16 : 0) +
    textWidth(p.text, size) * (p.strong ? 1.05 : 1) +
    (p.note ? 4 + textWidth(p.note, size) : 0)
  );
}

/** Parts on one line from (x, y), cut at `right`; returns the SVG and where it ended. */
function line(
  parts: Part[],
  x: number,
  y: number,
  right: number,
  t: ChartTheme,
  size = 12,
  gap = 10,
): [string, number] {
  const out: string[] = [];
  let cx = x;
  // plain parts in a row share one <text>, spaced by the renderer (estimated widths drift over a long row)
  let run: string[] = [];
  let runX = x;
  const flush = () => {
    if (run.length) out.push(`<text x="${r1(runX)}" y="${r1(y)}">${run.join("")}</text>`);
    run = [];
  };
  for (const p of parts) {
    if (cx >= right - 8) break;
    if (cx > x) cx += gap;
    if (p.dot || p.warn || p.logo || p.letter || p.chip) flush();
    if (p.chip) {
      // a pill in a tint of its colour, the colour's dot in front, the words in the text's ink
      const s = clip(p.text, right - cx - CHIP_PAD, size);
      const w = CHIP_PAD + chipWords(s, size);
      const h = size + 8;
      out.push(
        `<rect x="${r1(cx)}" y="${r1(y - size - 1)}" width="${r1(w)}" height="${h}" rx="${h / 2}" fill="${p.chip}" fill-opacity="${t.dark ? 0.2 : 0.14}"/>`,
        `<circle cx="${r1(cx + 10)}" cy="${r1(y - size * 0.34)}" r="3.5" fill="${p.chip}"/>`,
        text(cx + 18, y, s, p.muted ? t.muted : t.text, size),
      );
      cx += w;
      continue;
    }
    if (p.dot) {
      out.push(`<circle cx="${r1(cx + 4)}" cy="${r1(y - size * 0.34)}" r="4" fill="${p.dot}"/>`);
      cx += 12;
    }
    if (p.warn) {
      out.push(stateMark("warn", cx + 6, y - size * 0.36, t.state.warn));
      cx += 16;
    }
    if (p.logo || p.letter) {
      out.push(logoMark(cx, y - size * 0.95, size + 1, p.logo, { letter: p.letter, color: p.letterColor }, t));
      cx += size + 5;
    }
    const room = right - cx;
    const s = clip(p.text, room, size);
    const w = textWidth(s, size) * (p.strong ? 1.05 : 1);
    const note = p.note && w + 4 + textWidth(p.note, size) <= room ? p.note : "";
    if (!run.length) runX = cx;
    run.push(
      `<tspan${run.length ? ` dx="${gap}"` : ""} fill="${p.muted ? t.muted : t.text}" font-size="${size}"${
        p.strong ? ' font-weight="600"' : ""
      }>${esc(s)}</tspan>${note ? `<tspan dx="4" fill="${t.muted}" font-size="${size}">${esc(note)}</tspan>` : ""}`,
    );
    cx += w + (note ? 4 + textWidth(note, size) : 0);
  }
  flush();
  return [out.join(""), cx];
}

export interface Fact {
  label: string;
  parts: Part[];
}

/** Label · value rows, the values aligned; a row's parts wrap onto more lines when they don't fit. */
export function facts(rows: Fact[], t: ChartTheme, width = PANE, o: { title?: string; labelW?: number } = {}): string {
  const labelW = o.labelW ?? 70;
  const parts: string[] = [];
  let y = 0;
  if (o.title) {
    parts.push(text(0, 13, o.title, t.text, 12, ' font-weight="600"'));
    y = 22;
  }
  for (const row of rows) {
    // chips need a little more room between lines
    const chipped = row.parts.some((p) => p.chip);
    const lh = chipped ? 25 : 19;
    const gap = chipped ? 6 : 10;
    const baseline = y + (chipped ? 16 : 14);
    parts.push(text(0, baseline, clip(row.label, labelW - 8, 12), t.muted, 12));
    // pack the parts into lines
    const lines: Part[][] = [[]];
    let used = 0;
    for (const p of row.parts) {
      const w = partWidth(p, 12) + (used ? gap : 0);
      if (used && labelW + used + w > width) {
        lines.push([]);
        used = 0;
      }
      lines[lines.length - 1].push(p);
      used += w;
    }
    lines.forEach((ps, k) => parts.push(line(ps, labelW, baseline + k * lh, width, t, 12, gap)[0]));
    y += lines.length * lh + (chipped ? 5 : 3);
  }
  return svgDoc(
    width,
    y + 2,
    parts.join(""),
    rows.map((r) => `${r.label}: ${r.parts.map((p) => p.text).join(" ")}`).join("; "),
  );
}

// ---------------------------------------------------------------- meters

export interface Meter {
  label: string;
  /** 0..1 */
  frac?: number | null;
  /** at the right: `3% · 14 / 494G` */
  value: string;
  /** the fill (default: by load) */
  color?: string;
}

/** Horizontal meters: label, the fill on a tint of its own colour, the figures. */
export function meters(
  rows: Meter[],
  t: ChartTheme,
  width = PANE,
  o: { title?: string; labelW?: number } = {},
): string {
  const labelW = o.labelW ?? 70;
  const valueW = 128;
  const rowH = 22;
  const top = o.title ? 22 : 0;
  const bx = labelW;
  const bw = width - labelW - valueW - 8;
  const parts: string[] = [];
  if (o.title) parts.push(text(0, 13, o.title, t.text, 12, ' font-weight="600"'));
  rows.forEach((m, i) => {
    const y = top + i * rowH;
    const f = m.frac == null || !Number.isFinite(m.frac) ? undefined : Math.min(1, Math.max(0, m.frac));
    const c = m.color ?? (f === undefined ? t.other : severity(f, t));
    parts.push(text(0, y + 15, clip(m.label, labelW - 8, 12), t.muted, 12));
    parts.push(
      `<rect x="${bx}" y="${y + 7}" width="${r1(bw)}" height="8" rx="4" fill="${f === undefined ? t.grid : c}" fill-opacity="${f === undefined ? 1 : 0.2}"/>`,
    );
    if (f !== undefined && f > 0) {
      parts.push(`<rect x="${bx}" y="${y + 7}" width="${r1(Math.max(4, bw * f))}" height="8" rx="4" fill="${c}"/>`);
    }
    parts.push(text(width, y + 15, clip(m.value, valueW, 12), t.text, 12, ' text-anchor="end"'));
  });
  return svgDoc(width, top + rows.length * rowH, parts.join(""), rows.map((m) => `${m.label} ${m.value}`).join(", "));
}

// ---------------------------------------------------------------- the hero card

export interface HeroLink {
  /** 0..4 bars lit */
  bars: number;
  /** `405 ms`, `offline` */
  text: string;
  color: string;
  /** latency readings for the sparkline, [unix s, ms] oldest first */
  history?: [number, number | null][];
  /** under the sparkline: `latency · 4 min` */
  note?: string;
}

export interface Hero {
  title: string;
  /** the machine's own colour: the logo tile is tinted with it */
  color?: string;
  subtitle?: string;
  logo?: string;
  /** a monogram when there's no logo */
  letter?: string;
  /** the third line: OS, herdr, GPU, … */
  tags?: Part[];
  link?: HeroLink;
  /** instead of a link: a figure on the right (`8d` · `uptime`) */
  aside?: { value: string; label: string };
}

/** The card a detail opens with: the logo, the name and what it is, its link (or a figure) on the right. */
export function hero(h: Hero, t: ChartTheme, width = PANE): string {
  const parts: string[] = [];
  const tile = 52;
  // the tile in a tint of the machine's own colour (its identity, as on the dashboard), a rule of it underneath
  const own = h.color ? fit(h.color, t) : undefined;
  parts.push(
    `<rect x="0" y="2" width="${tile}" height="${tile}" rx="12" fill="${own ?? t.grid}" fill-opacity="${own ? (t.dark ? 0.2 : 0.16) : t.dark ? 0.55 : 0.6}"/>`,
  );
  if (own) parts.push(`<rect x="14" y="${tile - 2}" width="${tile - 28}" height="3" rx="1.5" fill="${own}"/>`);
  parts.push(logoMark(10, 10, 32, h.logo, { letter: h.letter ?? h.title, color: h.color }, t));
  const x0 = tile + 14;
  const right = h.link || h.aside ? width - 118 : width;
  const title = clip(h.title, right - x0 - 16, 18);
  parts.push(text(x0, 22, title, t.text, 18, ' font-weight="700"'));
  if (h.subtitle) parts.push(text(x0, 40, clip(h.subtitle, right - x0, 12), t.muted, 12));
  if (h.tags?.length) parts.push(line(h.tags, x0, 58, right, t, 11, 9)[0]);
  if (h.link) {
    const l = h.link;
    const tw = textWidth(l.text, 13) * 1.05;
    // down: a red ✕ where the bars would be
    parts.push(
      l.bars > 0
        ? signalBars(width - tw - 30, 21, l.bars, l.color, t)
        : stateMark("down", width - tw - 16, 15, l.color),
    );
    parts.push(text(width, 21, l.text, t.text, 13, ' text-anchor="end" font-weight="600"'));
    const now = l.history?.length ? l.history[l.history.length - 1][0] : 0;
    const from = l.history?.length ? l.history[0][0] : 0;
    const spark =
      l.history && now > from
        ? sparkline(l.history, { x: width - 104, y: 28, w: 104, h: 18, from, to: now }, l.color, t, { area: true })
        : "";
    if (spark) parts.push(spark);
    if (l.note) parts.push(text(width, 58, l.note, t.muted, 11, ' text-anchor="end"'));
  } else if (h.aside) {
    parts.push(text(width, 24, h.aside.value, t.text, 18, ' text-anchor="end" font-weight="600"'));
    parts.push(text(width, 41, h.aside.label, t.muted, 11, ' text-anchor="end"'));
  }
  return svgDoc(width, 62, parts.join(""), [h.title, h.subtitle, h.link?.text].filter(Boolean).join(", "));
}

// ---------------------------------------------------------------- pills

export interface Pill {
  n: number;
  label: string;
  /** blocked · working · done · idle · limit · … (the mark) */
  state: string;
  color: string;
}

/** Counts per state as pills (mark · count · label); a zero is shown faint so the row keeps its shape. */
export function pills(list: Pill[], t: ChartTheme, width = PANE): string {
  const h = 26;
  const gap = 8;
  const parts: string[] = [];
  let x = 0;
  let y = 0;
  for (const p of list) {
    const count = String(p.n);
    const w = 12 + 12 + 6 + textWidth(count, 13) * 1.05 + 5 + textWidth(p.label, 12) + 12;
    if (x > 0 && x + w > width) {
      x = 0;
      y += h + gap;
    }
    const on = p.n > 0;
    parts.push(
      `<rect x="${r1(x)}" y="${y}" width="${r1(w)}" height="${h}" rx="${h / 2}" fill="${on ? p.color : t.grid}" fill-opacity="${on ? (t.dark ? 0.17 : 0.12) : 0.45}"/>`,
    );
    parts.push(stateMark(p.state, x + 18, y + h / 2, on ? p.color : t.other));
    parts.push(
      `<text x="${r1(x + 30)}" y="${y + 17.5}"><tspan fill="${on ? t.text : t.muted}" font-size="13" font-weight="700">${count}</tspan><tspan dx="5" fill="${on ? t.text : t.muted}" font-size="12">${esc(p.label)}</tspan></text>`,
    );
    x += w + gap;
  }
  return svgDoc(width, y + h, parts.join(""), list.map((p) => `${p.n} ${p.label}`).join(", "));
}

// ---------------------------------------------------------------- sessions

export interface SessionRow {
  name: string;
  title?: string;
  /** blocked · working · done · idle · limit */
  state: string;
  color: string;
  /** `needs you · 4m` */
  status: string;
  logo?: string;
  /** a monogram when there's no logo */
  agentLetter?: string;
  agentColor?: string;
}

/** One row per session: its state mark, the agent's logo, the name, what it's doing, how long. */
export function sessionList(rows: SessionRow[], t: ChartTheme, width = PANE): string {
  const rowH = 30;
  const parts: string[] = [];
  rows.forEach((r, i) => {
    const y = i * rowH;
    const cy = y + rowH / 2;
    if (i)
      parts.push(`<line x1="0" x2="${width}" y1="${y + 0.5}" y2="${y + 0.5}" stroke="${t.grid}" stroke-width="1"/>`);
    parts.push(`<circle cx="11" cy="${cy}" r="10" fill="${r.color}" fill-opacity="${t.dark ? 0.18 : 0.13}"/>`);
    parts.push(stateMark(r.state, 11, cy, r.color));
    parts.push(logoMark(28, cy - 8, 16, r.logo, { letter: r.agentLetter, color: r.agentColor }, t));
    const status = clip(r.status, 110, 11);
    const right = width - textWidth(status, 11) - 12;
    const name = clip(r.name, Math.min(150, right - 52), 13);
    parts.push(text(52, cy + 4.5, name, t.text, 13, ' font-weight="600"'));
    const tx = 52 + textWidth(name, 13) * 1.06 + 8;
    if (r.title && right - tx > 24) parts.push(text(tx, cy + 4.5, clip(r.title, right - tx, 12), t.muted, 12));
    parts.push(text(width, cy + 4, status, t.muted, 11, ' text-anchor="end"'));
  });
  return svgDoc(
    width,
    Math.max(1, rows.length * rowH),
    parts.join(""),
    rows.map((r) => `${r.name} ${r.status}`).join(", "),
  );
}

// ---------------------------------------------------------------- chips

export interface Chip {
  text: string;
  /** after it, muted: a version, `3 loaded` */
  note?: string;
  logo?: string;
  /** a monogram when there's no logo */
  letter?: string;
  color?: string;
  /** a health dot at its end */
  dot?: string;
}

export interface ChipGroup {
  title: string;
  /** after the title, muted: `2 installed` */
  note?: string;
  chips: Chip[];
  /** shown when there are no chips */
  empty?: string;
}

/** Groups of logo chips (the agents installed, the LLM servers, the GPU), each wrapping onto more rows. */
export function chips(groups: ChipGroup[], t: ChartTheme, width = PANE): string {
  const h = 26;
  const gap = 6;
  const parts: string[] = [];
  let y = 0;
  for (const g of groups) {
    parts.push(
      `<text x="0" y="${y + 13}"><tspan fill="${t.text}" font-size="12" font-weight="600">${esc(g.title)}</tspan>${
        g.note ? `<tspan dx="6" fill="${t.muted}" font-size="12">${esc(g.note)}</tspan>` : ""
      }</text>`,
    );
    y += 20;
    if (!g.chips.length) {
      parts.push(text(0, y + 14, g.empty ?? "none", t.muted, 12));
      y += 28;
      continue;
    }
    let x = 0;
    for (const c of g.chips) {
      const mark = c.logo || c.letter || c.color ? 22 : 0;
      const noteW = c.note ? textWidth(c.note, 11) + 6 : 0;
      const w = Math.min(width, 10 + mark + textWidth(c.text, 12) + noteW + (c.dot ? 14 : 0) + 10);
      if (x > 0 && x + w > width) {
        x = 0;
        y += h + gap;
      }
      parts.push(
        `<rect x="${r1(x)}" y="${y}" width="${r1(w)}" height="${h}" rx="8" fill="${t.grid}" fill-opacity="${t.dark ? 0.55 : 0.6}"/>`,
      );
      let cx = x + 10;
      if (mark) {
        parts.push(logoMark(cx, y + 5, 16, c.logo, { letter: c.letter ?? c.text, color: c.color }, t));
        cx += 22;
      }
      const room = x + w - cx - noteW - (c.dot ? 14 : 0) - 8;
      const label = clip(c.text, room, 12);
      parts.push(
        `<text x="${r1(cx)}" y="${y + 17.5}"><tspan fill="${t.text}" font-size="12">${esc(label)}</tspan>${
          c.note ? `<tspan dx="6" fill="${t.muted}" font-size="11">${esc(c.note)}</tspan>` : ""
        }</text>`,
      );
      if (c.dot) parts.push(`<circle cx="${r1(x + w - 13)}" cy="${y + h / 2}" r="4" fill="${c.dot}"/>`);
      x += w + gap;
    }
    y += h + 14;
  }
  return svgDoc(
    width,
    Math.max(1, y - 8),
    parts.join(""),
    groups.map((g) => `${g.title}: ${g.chips.map((c) => c.text).join(", ")}`).join("; "),
  );
}
