// Charts as SVG, for Raycast's Detail markdown (data URIs). No Raycast
// imports: plain node tests it. Drawn for the Raycast theme in use
// (`environment.appearance`) on a transparent background, so the view's own
// surface shows through: text in the theme's text colours, brand colours
// fitted to at least 3:1 against that surface, near-identical neighbours
// pulled apart. Thin marks, rounded data ends, a 2px gap between stacked
// segments, hairline grid, a legend for two or more series, labels only
// where they say something (the peak, each bar's value).

import { existsSync, mkdirSync, readdirSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { join } from "node:path";

/** Aleph's state colours: what a session or a machine is doing (ui.ts, ui.rs). */
export type StateName = "working" | "blocked" | "done" | "idle" | "limit" | "down" | "warn";

export interface ChartTheme {
  dark: boolean;
  /** what Raycast paints behind the image (for contrast only: never drawn) */
  surface: string;
  text: string;
  muted: string;
  grid: string;
  /** Aleph's gold, fitted to the surface */
  accent: string;
  /** "Other" and things without a colour of their own */
  other: string;
  /** working blue, needs-you orange, done green, idle teal, limit violet, down red, warning yellow */
  state: Record<StateName, string>;
}

export const GOLD = "#E2B451";

/**
 * The state colours in their dark- and light-theme shades (ui.ts's; the light
 * warning is a clearer amber here, for marks rather than text, still 3:1).
 */
const STATES: Record<StateName, [string, string]> = {
  working: ["#3B9EFF", "#0D74CE"],
  blocked: ["#F76B15", "#CC4E00"],
  done: ["#46A758", "#2A7E3B"],
  idle: ["#12A594", "#067A6F"],
  limit: ["#A875FF", "#8145B5"],
  down: ["#E5484D", "#CE2C31"],
  warn: ["#F0D84A", "#B38600"],
};

export function chartTheme(dark: boolean): ChartTheme {
  const base = dark
    ? { dark, surface: "#1e1e1e", text: "#ececec", muted: "#a0a0a0", grid: "#3a3a3a", other: "#7c7c7c" }
    : { dark, surface: "#ffffff", text: "#1d1d1d", muted: "#646464", grid: "#e2e2e2", other: "#9a9a9a" };
  const state = Object.fromEntries(Object.entries(STATES).map(([k, [d, l]]) => [k, fit(dark ? d : l, base)])) as Record<
    StateName,
    string
  >;
  return { ...base, accent: fit(GOLD, base), state };
}

const FONT = "-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Helvetica Neue', Helvetica, Arial, sans-serif";

// ---------------------------------------------------------------- colour

type RGB = [number, number, number];

function hexRgb(hex: string): RGB | undefined {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return undefined;
  const h = m[1].length === 3 ? [...m[1]].map((c) => c + c).join("") : m[1];
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255) as RGB;
}

function rgbHex(c: RGB): string {
  return `#${c
    .map((v) =>
      Math.round(Math.min(1, Math.max(0, v)) * 255)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

const lin = (v: number) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
const gam = (v: number) => (v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055);

function luminance(c: RGB): number {
  const [r, g, b] = c.map(lin);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio between two colours (1 … 21). */
export function contrast(a: string, b: string): number {
  const x = hexRgb(a);
  const y = hexRgb(b);
  if (!x || !y) return 1;
  const [l1, l2] = [luminance(x), luminance(y)].sort((p, q) => q - p);
  return (l1 + 0.05) / (l2 + 0.05);
}

type Lab = [number, number, number];

function oklab(c: RGB): Lab {
  const [r, g, b] = c.map(lin);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function fromOklab([L, a, b]: Lab): RGB {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ].map(gam) as RGB;
}

/** Perceptual distance (OKLab ΔE × 100). */
export function deltaE(a: string, b: string): number {
  const x = hexRgb(a);
  const y = hexRgb(b);
  if (!x || !y) return 100;
  const [p, q] = [oklab(x), oklab(y)];
  return Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]) * 100;
}

/** `hex` with its OKLab lightness moved by `dl`, hue and chroma kept. */
function shiftL(hex: string, dl: number): string {
  const c = hexRgb(hex);
  if (!c) return hex;
  const [L, a, b] = oklab(c);
  return rgbHex(fromOklab([Math.min(1, Math.max(0, L + dl)), a, b]));
}

/** Between `a` (f = 0) and `b` (f = 1), in OKLab (no muddy midpoints). */
export function mix(a: string, b: string, f: number): string {
  const x = hexRgb(a);
  const y = hexRgb(b);
  if (!x || !y) return b;
  const [p, q] = [oklab(x), oklab(y)];
  const k = Math.min(1, Math.max(0, f));
  return rgbHex(fromOklab([0, 1, 2].map((i) => p[i] + (q[i] - p[i]) * k) as Lab));
}

/**
 * A brand colour usable as a mark on this theme's surface: at least `min`:1
 * contrast (WCAG's 3:1 for graphics), lightened on dark, darkened on light.
 * An unreadable colour string becomes the theme's `other`.
 */
export function fit(hex: string, t: Pick<ChartTheme, "dark" | "surface"> & { other?: string }, min = 3): string {
  const rgb = hexRgb(hex);
  if (!rgb) return t.other ?? "#888888";
  let c = rgbHex(rgb);
  // from the colour itself each time (small steps from a rounded colour can get stuck at black)
  for (let i = 1; i <= 50 && contrast(c, t.surface) < min; i++) c = shiftL(rgbHex(rgb), (t.dark ? 0.02 : -0.02) * i);
  return c;
}

/**
 * Colours for series shown side by side: each fitted to the surface, and one
 * too close to another (ΔE < 12, e.g. two agents in Google blue) stepped
 * lighter or darker until it reads apart. Who keeps their colour goes by
 * `keys` (the entities, alphabetically), never by rank, so a series looks
 * the same whatever its size.
 */
export function distinct(colors: string[], t: ChartTheme, keys: string[] = colors): string[] {
  const order = colors.map((_, i) => i).sort((a, b) => (keys[a] ?? "").localeCompare(keys[b] ?? "") || a - b);
  const out: string[] = new Array(colors.length);
  const placed: string[] = [];
  for (const i of order) {
    const base = fit(colors[i], t);
    let c = base;
    for (let k = 1; k <= 6 && placed.some((o) => deltaE(o, c) < 12); k++) {
      const tried = shiftL(base, 0.07 * Math.ceil(k / 2) * (k % 2 ? 1 : -1) * (t.dark ? 1 : -1));
      // stay readable on the surface
      if (contrast(tried, t.surface) >= 3) c = tried;
    }
    out[i] = c;
    placed.push(c);
  }
  return out;
}

// ---------------------------------------------------------------- scales

/** A clean axis: the top tick and the step (0, step, 2·step, … ≥ v). */
export function niceScale(v: number, ticks = 4): { max: number; step: number } {
  if (!(v > 0)) return { max: 1, step: 0.25 };
  const raw = v / ticks;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= raw) ?? 10 * mag;
  return { max: Math.ceil(v / step - 1e-9) * step, step };
}

export function esc(s: string): string {
  return s.replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] ?? c,
  );
}

/** About how wide `s` is at `size` px in the system font (for fitting labels). */
export function textWidth(s: string, size: number): number {
  let w = 0;
  for (const ch of s)
    w += /[ilIj.,:;'|!·]/.test(ch)
      ? 0.28
      : /[ftr]/.test(ch)
        ? 0.36
        : /[mwMW@%]/.test(ch)
          ? 0.82
          : /[A-Z$]/.test(ch)
            ? 0.62
            : /[0-9]/.test(ch)
              ? 0.58
              : 0.52;
  return w * size;
}

/**
 * Cut `s` to fit `px` at `size`, with an ellipsis; a path loses its start
 * (its end says more). `path`: whether it is one (default: it has a `/`).
 */
export function clip(s: string, px: number, size: number, path = s.includes("/")): string {
  if (textWidth(s, size) <= px) return s;
  let out = s;
  while (out.length > 1 && textWidth(`${out}…`, size) > px) out = path ? out.slice(1) : out.slice(0, -1);
  return path ? `…${out}` : `${out}…`;
}

export const dataUri = (svg: string) => `data:image/svg+xml;base64,${Buffer.from(svg, "utf8").toString("base64")}`;

/** Where cards go as files (`cardFilesIn`); unset: inline data URIs (tests, previews, the other commands). */
let cardDir: string | undefined;
const written = new Set<string>();

/**
 * Cards as files named by their content (`<hash>.svg`), referenced by path:
 * the detail Raycast gets is a few hundred bytes, not tens of kilobytes,
 * and a card that didn't change keeps its address, so Raycast reuses the
 * one it already drew — only what changed is drawn again. Files older than
 * an hour go when this is set.
 */
export function cardFilesIn(dir: string) {
  if (cardDir === dir) return;
  cardDir = dir;
  try {
    mkdirSync(dir, { recursive: true });
    const old = Date.now() - 3_600_000;
    for (const f of readdirSync(dir)) {
      const p = join(dir, f);
      if (statSync(p).mtimeMs < old) unlinkSync(p);
    }
  } catch {
    // a card that can't be written falls back to its data URI
  }
}

/** A card's name from its content (sha1, native and fast even for a 60 KB card). */
const hash64 = (s: string) => createHash("sha1").update(s).digest("hex").slice(0, 16);

/** A card's address in markdown: its file when cards go to files, else a data URI. */
export function cardUri(svg: string): string {
  if (!cardDir) return dataUri(svg);
  const name = `${hash64(svg)}.svg`;
  const file = join(cardDir, name);
  try {
    if (!written.has(name)) {
      if (!existsSync(file)) writeFileSync(file, svg);
      written.add(name);
    }
    return `file://${encodeURI(file)}`;
  } catch {
    return dataUri(svg);
  }
}

/** A whole SVG document (no background: the view's own surface shows through). */
export function svgDoc(w: number, h: number, body: string, label: string): string {
  const [W, H] = [Math.ceil(w), Math.ceil(h)];
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${W}" height="${H}" ` +
    `viewBox="0 0 ${W} ${H}" font-family="${FONT}" role="img" aria-label="${esc(label)}">${shareImages(body)}</svg>`
  );
}

const IMAGE =
  /<image href="([^"]+)" xlink:href="\1" x="([-\d.]+)" y="([-\d.]+)" width="([\d.]+)" height="([\d.]+)"(?: preserveAspectRatio="[^"]*")?\/>/g;

/**
 * The same embedded image used more than once (an agent's logo on every
 * row) is kept once, in `<defs>`, and each use points at it: a card with a
 * dozen logos goes from ~70 KB to a few.
 */
export function shareImages(body: string): string {
  const count = new Map<string, number>();
  for (const m of body.matchAll(IMAGE)) count.set(m[1], (count.get(m[1]) ?? 0) + 1);
  const shared = [...count].filter(([, n]) => n > 1).map(([uri]) => uri);
  if (!shared.length) return body;
  const id = new Map(shared.map((uri, i) => [uri, `i${i}`]));
  const out = body.replace(IMAGE, (all, uri: string, x: string, y: string, w: string, h: string) => {
    const k = id.get(uri);
    return k ? `<use href="#${k}" xlink:href="#${k}" transform="translate(${x} ${y}) scale(${w} ${h})"/>` : all;
  });
  const defs = shared
    .map(
      (uri) =>
        `<image id="${id.get(uri)}" xlink:href="${uri}" width="1" height="1" preserveAspectRatio="xMidYMid meet"/>`,
    )
    .join("");
  return `<defs>${defs}</defs>${out}`;
}
const svg = svgDoc;

export const text = (x: number, y: number, s: string, fill: string, size: number, extra = "") =>
  `<text x="${x.toFixed(1)}" y="${y.toFixed(1)}" fill="${fill}" font-size="${size}"${extra}>${esc(s)}</text>`;

/** Runs of text one after the other on a line (the renderer spaces them, no guessing widths): [words, fill, size, weight?]. */
export function runs(x: number, y: number, parts: [string, string, number, number?][], gap = 6): string {
  const spans = parts
    .filter(([s]) => s)
    .map(
      ([s, fill, size, weight], i) =>
        `<tspan${i ? ` dx="${gap}"` : ""} fill="${fill}" font-size="${size}"${weight ? ` font-weight="${weight}"` : ""}>${esc(s)}</tspan>`,
    )
    .join("");
  return `<text x="${x.toFixed(1)}" y="${y.toFixed(1)}">${spans}</text>`;
}

/** A bar from a baseline: square there, rounded (r) at the data end. `up`: a column. */
export function barPath(x: number, y: number, w: number, h: number, r: number, up: boolean): string {
  const rr = Math.max(0, Math.min(r, w / 2, h / 2));
  if (up) {
    // baseline at y + h, data end at the top
    return `M${x},${y + h}V${y + rr}Q${x},${y} ${x + rr},${y}H${x + w - rr}Q${x + w},${y} ${x + w},${y + rr}V${y + h}Z`;
  }
  // baseline at x, data end on the right
  return `M${x},${y}H${x + w - rr}Q${x + w},${y} ${x + w},${y + rr}V${y + h - rr}Q${x + w},${y + h} ${x + w - rr},${y + h}H${x}Z`;
}

// ---------------------------------------------------------------- the charts

export interface Tile {
  label: string;
  value: string;
  /** small, after the value: `est.` */
  suffix?: string;
  /** the figures under it, most important first (the last ones go when they don't fit) */
  sub: string[];
}

/** A row of stat tiles (label · value · what's under it), each with Aleph's gold rule. */
export function tiles(list: Tile[], t: ChartTheme, width = 680): string {
  const gap = 12;
  const h = 92;
  const w = (width - gap * (list.length - 1)) / Math.max(1, list.length);
  const body = list
    .map((tile, i) => {
      const x = i * (w + gap);
      const room = w - 36;
      // the value as big as fits (26 px in a wide tile, down to 16 in a narrow one), then cut
      let size = 26;
      while (size > 16 && textWidth(tile.value, size) > room) size -= 1;
      const value = clip(tile.value, room, size);
      // as many of the figures as fit, whole
      const fits = !!tile.suffix && textWidth(value, size) + textWidth(tile.suffix, 12) + 6 <= room;
      const suffix = fits ? (tile.suffix ?? "") : "";
      // a suffix with no room beside the value goes first under it (`est.` is never dropped)
      let sub = tile.suffix && !fits ? [tile.suffix, ...tile.sub] : [...tile.sub];
      while (sub.length > 1 && textWidth(sub.join(" · "), 12) > room) sub = sub.slice(0, -1);
      return [
        `<rect x="${x + 0.5}" y="0.5" width="${w - 1}" height="${h - 1}" rx="8" fill="none" stroke="${t.grid}"/>`,
        `<rect x="${x + 12}" y="16" width="3" height="${h - 32}" rx="1.5" fill="${t.accent}"/>`,
        text(x + 24, 28, clip(tile.label, room, 12), t.muted, 12),
        runs(x + 24, 58, [
          [value, t.text, size, 600],
          [suffix, t.muted, 12],
        ]),
        text(x + 24, 78, clip(sub.join(" · "), room, 12), t.muted, 12),
      ].join("");
    })
    .join("");
  return svg(
    width,
    h,
    body,
    list.map((x) => `${x.label}: ${x.value}${x.suffix ? ` ${x.suffix}` : ""}, ${x.sub.join(", ")}`).join("; "),
  );
}

export interface Series {
  label: string;
  color: string;
  /** one value per column */
  values: number[];
  /** the legend's figure (its total, formatted) */
  note?: string;
}

/**
 * Stacked columns over time (one column per day; series in the order given,
 * the first at the baseline), the peak's total on its cap, a legend below.
 */
export function stackedColumns(
  o: { columns: string[]; series: Series[]; fmt: (v: number) => string; label: string },
  t: ChartTheme,
  width = 680,
): string {
  const left = 52;
  const right = 8;
  const top = 22;
  const ph = 168;
  const pw = width - left - right;
  const n = Math.max(1, o.columns.length);
  const totals = o.columns.map((_, i) => o.series.reduce((s, x) => s + (x.values[i] ?? 0), 0));
  const peak = Math.max(0, ...totals);
  const { max, step } = niceScale(peak);
  const y = (v: number) => top + ph - (v / max) * ph;
  const band = pw / n;
  const bw = Math.max(2, Math.min(24, band * 0.72));
  const parts: string[] = [];
  // the grid and its values
  for (let v = 0; v <= max + 1e-9; v += step) {
    const yy = Math.round(y(v)) + 0.5;
    parts.push(`<line x1="${left}" x2="${width - right}" y1="${yy}" y2="${yy}" stroke="${t.grid}" stroke-width="1"/>`);
    parts.push(text(left - 8, yy + 4, o.fmt(v), t.muted, 11, ' text-anchor="end"'));
  }
  const colors = distinct(
    o.series.map((s) => s.color),
    t,
    o.series.map((s) => s.label),
  );
  o.columns.forEach((_, i) => {
    const x = left + band * i + (band - bw) / 2;
    let base = top + ph;
    const segs = o.series.map((s, k) => ({ v: s.values[i] ?? 0, k })).filter((s) => s.v > 0);
    segs.forEach((s, j) => {
      const h = (s.v / max) * ph;
      const topmost = j === segs.length - 1;
      // a 2px gap above every segment that has another on top of it
      const drawn = topmost ? h : h - 2;
      if (drawn >= 0.5) {
        parts.push(
          `<path d="${barPath(x, base - drawn, bw, drawn, topmost ? 4 : 0, true)}" fill="${colors[s.k]}"><title>${esc(
            `${o.columns[i]} · ${o.series[s.k].label}: ${o.fmt(s.v)}`,
          )}</title></path>`,
        );
      }
      base -= h;
    });
  });
  // the peak's total on its cap
  if (peak > 0) {
    const i = totals.indexOf(peak);
    const cx = Math.min(width - right - 20, Math.max(left + 20, left + band * i + band / 2));
    parts.push(text(cx, y(peak) - 6, o.fmt(peak), t.text, 11, ' text-anchor="middle" font-weight="600"'));
  } else {
    parts.push(text(left + pw / 2, top + ph / 2, "Nothing in this range", t.muted, 13, ' text-anchor="middle"'));
  }
  // the days: as many as fit, always the last
  const every = Math.max(1, Math.ceil(46 / band));
  o.columns.forEach((c, i) => {
    if ((n - 1 - i) % every !== 0) return;
    const cx = left + band * i + band / 2;
    const half = textWidth(c, 11) / 2 + 2;
    // under its column, but never past the frame
    const [x, anchor] = cx + half > width ? [width, "end"] : cx - half < 0 ? [0, "start"] : [cx, "middle"];
    parts.push(text(x, top + ph + 16, c, t.muted, 11, ` text-anchor="${anchor}"`));
  });
  // the legend (two or more series), wrapping
  let lx = left;
  let ly = top + ph + 40;
  if (o.series.length > 1) {
    o.series.forEach((s, k) => {
      const words = s.note ? `${s.label}  ${s.note}` : s.label;
      const w = 16 + textWidth(words, 12) + 18;
      if (lx + w > width - right && lx > left) {
        lx = left;
        ly += 20;
      }
      parts.push(`<rect x="${lx}" y="${ly - 9}" width="10" height="10" rx="2.5" fill="${colors[k]}"/>`);
      parts.push(
        runs(lx + 16, ly, [
          [s.label, t.text, 12],
          [s.note ?? "", t.muted, 12],
        ]),
      );
      lx += w;
    });
  }
  const h = o.series.length > 1 ? ly + 10 : top + ph + 26;
  return svg(width, h, parts.join(""), o.label);
}

export interface Bar {
  label: string;
  value: number;
  color: string;
  /** after the value: `est.`, `3 sessions` */
  note?: string;
}

/** Horizontal bars, largest first as given: label · bar · value at its tip. */
export function barList(
  o: { bars: Bar[]; fmt: (v: number) => string; label: string },
  t: ChartTheme,
  width = 680,
): string {
  // about a third of the width for the labels (190 px at 680), the value after the bar's tip
  const labelW = Math.round(Math.min(190, width * 0.3));
  const valueW = Math.round(Math.min(120, width * 0.25));
  const row = 26;
  const bh = 12;
  const max = Math.max(0, ...o.bars.map((b) => b.value));
  const track = width - labelW - valueW - 8;
  const parts: string[] = [];
  if (!o.bars.length || max <= 0) {
    parts.push(text(0, 18, "Nothing in this range", t.muted, 13));
    return svg(width, 28, parts.join(""), o.label);
  }
  const colors = o.bars.map((b) => fit(b.color, t));
  o.bars.forEach((b, i) => {
    const cy = i * row + row / 2;
    const w = Math.max(b.value > 0 ? 3 : 0, (b.value / max) * track);
    parts.push(text(0, cy + 4, clip(b.label, labelW - 14, 12), t.text, 12));
    if (w > 0) {
      parts.push(
        `<path d="${barPath(labelW, cy - bh / 2, w, bh, 4, false)}" fill="${colors[i]}"><title>${esc(
          `${b.label}: ${o.fmt(b.value)}`,
        )}</title></path>`,
      );
    }
    parts.push(
      runs(labelW + w + 8, cy + 4, [
        [o.fmt(b.value), t.text, 12, 600],
        [b.note ?? "", t.muted, 12],
      ]),
    );
  });
  return svg(width, o.bars.length * row + 4, parts.join(""), o.label);
}

export interface Part {
  label: string;
  value: number;
  color: string;
  /** after the share in the legend: `$12.40` */
  note?: string;
}

/** One bar split by share (a 2px gap between parts), its legend with each part's share below. */
export function shareBar(o: { parts: Part[]; label: string }, t: ChartTheme, width = 680): string {
  const total = o.parts.reduce((s, p) => s + Math.max(0, p.value), 0);
  const parts: string[] = [];
  if (!(total > 0)) return svg(width, 28, text(0, 18, "Nothing in this range", t.muted, 13), o.label);
  const colors = distinct(
    o.parts.map((p) => p.color),
    t,
    o.parts.map((p) => p.label),
  );
  const bh = 14;
  let x = 0;
  const shown = o.parts.filter((p) => p.value > 0);
  shown.forEach((p, i) => {
    const k = o.parts.indexOf(p);
    const w = (p.value / total) * width;
    const last = i === shown.length - 1;
    const drawn = last ? w : Math.max(0.5, w - 2);
    const r = i === 0 || last ? 4 : 0;
    parts.push(
      `<rect x="${x.toFixed(1)}" y="0" width="${drawn.toFixed(1)}" height="${bh}" rx="${r}" fill="${colors[k]}"><title>${esc(
        `${p.label}: ${Math.round((p.value / total) * 100)}%${p.note ? ` · ${p.note}` : ""}`,
      )}</title></rect>`,
    );
    x += w;
  });
  let lx = 0;
  let ly = bh + 22;
  o.parts.forEach((p, k) => {
    const pct = `${Math.round((Math.max(0, p.value) / total) * 100)}%`;
    const words = `${p.label}  ${pct}  ${p.note ?? ""}`;
    const w = 16 + textWidth(words, 12) + 20;
    if (lx + w > width && lx > 0) {
      lx = 0;
      ly += 20;
    }
    parts.push(`<rect x="${lx}" y="${ly - 9}" width="10" height="10" rx="2.5" fill="${colors[k]}"/>`);
    parts.push(
      runs(lx + 16, ly, [
        [p.label, t.text, 12],
        [pct, t.text, 12, 600],
        [p.note ?? "", t.muted, 12],
      ]),
    );
    lx += w;
  });
  return svg(width, ly + 8, parts.join(""), o.label);
}

/**
 * This period against the one before, as running totals: the period before
 * dashed and muted, this one in gold, each line's end labelled.
 */
export function lineCompare(
  o: { columns: string[]; now: number[]; before: number[]; fmt: (v: number) => string; label: string },
  t: ChartTheme,
  width = 680,
): string {
  const left = 52;
  const right = 70;
  const top = 14;
  const ph = 150;
  const pw = width - left - right;
  const n = Math.max(2, o.columns.length);
  const peak = Math.max(0, ...o.now, ...o.before);
  const { max, step } = niceScale(peak);
  const x = (i: number) => left + (i / (n - 1)) * pw;
  const y = (v: number) => top + ph - (v / max) * ph;
  const parts: string[] = [];
  for (let v = 0; v <= max + 1e-9; v += step) {
    const yy = Math.round(y(v)) + 0.5;
    parts.push(`<line x1="${left}" x2="${left + pw}" y1="${yy}" y2="${yy}" stroke="${t.grid}" stroke-width="1"/>`);
    parts.push(text(left - 8, yy + 4, o.fmt(v), t.muted, 11, ' text-anchor="end"'));
  }
  if (!(peak > 0)) {
    parts.push(text(left + pw / 2, top + ph / 2, "Nothing in this range", t.muted, 13, ' text-anchor="middle"'));
    return svg(width, top + ph + 26, parts.join(""), o.label);
  }
  const line = (v: number[], color: string, extra: string) => {
    if (!v.length) return;
    const pts = v.map((val, i) => `${x(i).toFixed(1)},${y(val).toFixed(1)}`).join(" ");
    parts.push(
      `<polyline points="${pts}" fill="none" stroke="${color}" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"${extra}/>`,
    );
  };
  line(o.before, t.muted, ' stroke-dasharray="4 4"');
  line(o.now, t.accent, "");
  // each line's end: its value beside it (kept apart when they're close)
  const ends: [number[], string, string][] = [
    [o.now, t.text, "this"],
    [o.before, t.muted, "before"],
  ];
  const ys: number[] = [];
  for (const [v, fill] of ends) {
    if (!v.length) continue;
    let yy = y(v[v.length - 1]) + 4;
    if (ys.some((p) => Math.abs(p - yy) < 14)) yy = ys[0] + (yy >= ys[0] ? 14 : -14);
    ys.push(yy);
    parts.push(text(x(v.length - 1) + 8, yy, o.fmt(v[v.length - 1]), fill, 11, ' font-weight="600"'));
  }
  const every = Math.max(1, Math.ceil(46 / (pw / n)));
  o.columns.forEach((c, i) => {
    if ((o.columns.length - 1 - i) % every !== 0) return;
    parts.push(text(x(i), top + ph + 16, c, t.muted, 11, ' text-anchor="middle"'));
  });
  return svg(width, top + ph + 26, parts.join(""), o.label);
}

/** How many fell in each bucket ([from, to, count]), a column each, the bucket's bounds under it. */
export function histogram(
  o: { buckets: [number, number, number][]; fmt: (v: number) => string; label: string; unit?: string },
  t: ChartTheme,
  width = 680,
): string {
  const left = 36;
  const top = 18;
  const ph = 110;
  const b = o.buckets.filter((x) => x.length === 3);
  const parts: string[] = [];
  const peak = Math.max(0, ...b.map((x) => x[2]));
  if (!b.length || peak <= 0) return svg(width, 28, text(0, 18, "Nothing to count yet", t.muted, 13), o.label);
  const band = (width - left) / b.length;
  const bw = Math.max(3, Math.min(48, band * 0.8));
  b.forEach(([lo, hi, n], i) => {
    const h = (n / peak) * ph;
    const x0 = left + band * i + (band - bw) / 2;
    if (n > 0) {
      parts.push(
        `<path d="${barPath(x0, top + ph - Math.max(1, h), bw, Math.max(1, h), 3, true)}" fill="${t.accent}"><title>${esc(
          `${o.fmt(lo)}–${o.fmt(hi)}: ${n}${o.unit ? ` ${o.unit}` : ""}`,
        )}</title></path>`,
      );
      parts.push(text(x0 + bw / 2, top + ph - h - 5, String(n), t.text, 10, ' text-anchor="middle"'));
    }
  });
  // the bounds: as many as fit
  const every = Math.max(1, Math.ceil(54 / band));
  b.forEach(([lo], i) => {
    if (i % every !== 0) return;
    parts.push(text(left + band * i, top + ph + 15, o.fmt(lo), t.muted, 10, ' text-anchor="middle"'));
  });
  parts.push(
    `<line x1="${left}" x2="${width}" y1="${top + ph + 0.5}" y2="${top + ph + 0.5}" stroke="${t.grid}" stroke-width="1"/>`,
  );
  return svg(width, top + ph + 24, parts.join(""), o.label);
}

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/** The week's hours (7 rows, Monday first, × 24): each cell gold by how much, the busiest marked in its title. */
export function heatWeek(
  o: { cells: number[][]; fmt: (v: number) => string; label: string },
  t: ChartTheme,
  width = 680,
) {
  const left = 34;
  const top = 4;
  const cw = (width - left) / 24;
  const ch = Math.min(18, cw);
  const peak = Math.max(0, ...o.cells.flat());
  const parts: string[] = [];
  for (let d = 0; d < 7; d++) {
    parts.push(text(0, top + d * ch + ch * 0.7, WEEKDAYS[d], t.muted, 10));
    for (let h = 0; h < 24; h++) {
      const v = o.cells[d]?.[h] ?? 0;
      const f = peak > 0 ? v / peak : 0;
      const fill = v > 0 ? mix(t.surface, t.accent, 0.18 + 0.82 * f) : t.grid;
      parts.push(
        `<rect x="${(left + h * cw + 1).toFixed(1)}" y="${(top + d * ch + 1).toFixed(1)}" width="${(cw - 2).toFixed(1)}" height="${(ch - 2).toFixed(1)}" rx="2" fill="${fill}"${v > 0 ? "" : ' opacity="0.5"'}><title>${esc(
          `${WEEKDAYS[d]} ${String(h).padStart(2, "0")}:00: ${o.fmt(v)}`,
        )}</title></rect>`,
      );
    }
  }
  for (const h of [0, 6, 12, 18, 23]) {
    parts.push(text(left + h * cw + cw / 2, top + 7 * ch + 13, `${h}`, t.muted, 10, ' text-anchor="middle"'));
  }
  return svg(width, top + 7 * ch + 18, parts.join(""), o.label);
}

/** Days as a calendar (a column a week, Monday on top), each cell gold by how much. `days`: oldest first, `YYYY-MM-DD`. */
export function calendarHeat(
  o: { days: { day: string; value: number }[]; fmt: (v: number) => string; label: string },
  t: ChartTheme,
  width = 680,
) {
  const parts: string[] = [];
  if (!o.days.length) return svg(width, 28, text(0, 18, "Nothing in this range", t.muted, 13), o.label);
  const left = 34;
  const weekday = (s: string) => (new Date(`${s}T12:00:00Z`).getUTCDay() + 6) % 7;
  const lead = weekday(o.days[0].day);
  const weeks = Math.ceil((lead + o.days.length) / 7);
  const cell = Math.min(18, (width - left) / Math.max(1, weeks));
  const peak = Math.max(0, ...o.days.map((d) => d.value));
  for (const d of [0, 2, 4]) parts.push(text(0, d * cell + cell * 0.7, WEEKDAYS[d], t.muted, 10));
  o.days.forEach((d, i) => {
    const k = lead + i;
    const [col, row] = [Math.floor(k / 7), k % 7];
    const f = peak > 0 ? d.value / peak : 0;
    const fill = d.value > 0 ? mix(t.surface, t.accent, 0.18 + 0.82 * f) : t.grid;
    parts.push(
      `<rect x="${(left + col * cell + 1).toFixed(1)}" y="${(row * cell + 1).toFixed(1)}" width="${(cell - 2).toFixed(1)}" height="${(cell - 2).toFixed(1)}" rx="2" fill="${fill}"${d.value > 0 ? "" : ' opacity="0.5"'}><title>${esc(
        `${d.day}: ${o.fmt(d.value)}`,
      )}</title></rect>`,
    );
    // the month's first day names it under the grid (the first column too, when its month has room)
    const roomy = i === 0 && !o.days.slice(1, 21).some((x) => x.day.endsWith("-01"));
    if (d.day.endsWith("-01") || roomy) {
      const m = new Date(`${d.day}T12:00:00Z`).toLocaleString("en", { month: "short", timeZone: "UTC" });
      parts.push(text(left + col * cell, 7 * cell + 13, m, t.muted, 10));
    }
  });
  return svg(width, 7 * cell + 18, parts.join(""), o.label);
}
