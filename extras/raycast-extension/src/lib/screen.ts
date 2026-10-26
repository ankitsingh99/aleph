// A session's screen, drawn: the last lines `aleph peek --ansi` read, with
// their colours, as an SVG card for a list's detail pane. As a code block it
// showed nothing: an agent's TUI sits at the bottom of a tall pane, so the
// block opened on the blank rows above it, and its rules are as wide as the
// terminal (200+ columns). Here the blank rows go, rules and boxes are
// squeezed to the card, right-aligned bits close up, long lines wrap, and
// the colours are fitted to Raycast's theme: ANSI's 16 in Aleph's own
// colours, the remote terminal's greys made relative to this theme's text.
// No Raycast imports: plain node tests it.
import { barPath, ChartTheme, contrast, esc, fit, mix, svgDoc, text, textWidth, clip } from "./charts";
import { logoMark, PANE, stateMark } from "./dash";
import { explainError } from "./model";
import { img } from "./stats-view";

// ---------------------------------------------------------------- cells

/** An SGR colour: an index into the 256-colour palette, or `#rrggbb`. */
export type Ink = number | string;

export interface Style {
  fg?: Ink;
  bg?: Ink;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
  inverse?: boolean;
  strike?: boolean;
}

/** One character on the screen and how many columns it takes there (marks join the cell before). */
export interface Cell {
  ch: string;
  w: 1 | 2;
  st: Style;
}

const WIDE: [number, number][] = [
  [0x1100, 0x115f],
  [0x2329, 0x232a],
  [0x2e80, 0x303e],
  [0x3041, 0x33ff],
  [0x3400, 0x4dbf],
  [0x4e00, 0x9fff],
  [0xa000, 0xa4cf],
  [0xac00, 0xd7a3],
  [0xf900, 0xfaff],
  [0xfe30, 0xfe4f],
  [0xff00, 0xff60],
  [0xffe0, 0xffe6],
  [0x1f300, 0x1f64f],
  [0x1f680, 0x1f6ff],
  [0x1f900, 0x1f9ff],
  [0x1fa70, 0x1faff],
  [0x20000, 0x3fffd],
];
const ZERO: [number, number][] = [
  [0x0300, 0x036f],
  [0x200b, 0x200f],
  [0x20d0, 0x20ff],
  [0xfe00, 0xfe0f],
  [0x1f3fb, 0x1f3ff],
  [0xe0100, 0xe01ef],
];
const inRanges = (cp: number, r: [number, number][]) => r.some(([a, b]) => cp >= a && cp <= b);

/** Columns a character takes in a terminal: 0 (it joins the one before), 1, or 2 (CJK, emoji). */
export function cellWidth(cp: number): 0 | 1 | 2 {
  if (inRanges(cp, ZERO)) return 0;
  return inRanges(cp, WIDE) ? 2 : 1;
}

/** One SGR sequence's parameters applied to a style (a new object: cells share theirs). */
function sgr(st: Style, params: string): Style {
  const v = params === "" ? [0] : params.split(/[;:]/).map((x) => Number.parseInt(x, 10) || 0);
  let s: Style = { ...st };
  for (let i = 0; i < v.length; i++) {
    const n = v[i];
    if (n === 0) s = {};
    else if (n === 1) s.bold = true;
    else if (n === 2) s.dim = true;
    else if (n === 3) s.italic = true;
    else if (n === 4) s.underline = true;
    else if (n === 7) s.inverse = true;
    else if (n === 9) s.strike = true;
    else if (n === 22) s.bold = s.dim = false;
    else if (n === 23) s.italic = false;
    else if (n === 24) s.underline = false;
    else if (n === 27) s.inverse = false;
    else if (n === 29) s.strike = false;
    else if (n >= 30 && n <= 37) s.fg = n - 30;
    else if (n === 39) delete s.fg;
    else if (n >= 40 && n <= 47) s.bg = n - 40;
    else if (n === 49) delete s.bg;
    else if (n >= 90 && n <= 97) s.fg = n - 90 + 8;
    else if (n >= 100 && n <= 107) s.bg = n - 100 + 8;
    else if (n === 38 || n === 48 || n === 58) {
      let ink: Ink | undefined;
      if (v[i + 1] === 5) {
        ink = (v[i + 2] ?? 0) & 255;
        i += 2;
      } else if (v[i + 1] === 2) {
        ink = rgbHex(v[i + 2] ?? 0, v[i + 3] ?? 0, v[i + 4] ?? 0);
        i += 4;
      }
      // 58: the underline's colour, not drawn
      if (ink !== undefined && n === 38) s.fg = ink;
      else if (ink !== undefined && n === 48) s.bg = ink;
    }
  }
  return s;
}

const hex2 = (n: number) => Math.min(255, Math.max(0, n)).toString(16).padStart(2, "0");
const rgbHex = (r: number, g: number, b: number) => `#${hex2(r)}${hex2(g)}${hex2(b)}`;

/** Text with ANSI escapes → rows of cells. SGR is kept; cursor moves, OSC and the rest are dropped. */
export function parseScreen(s: string): Cell[][] {
  const rows: Cell[][] = [[]];
  let st: Style = {};
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    if (c === "\x1b") {
      const n = s[i + 1];
      if (n === "[") {
        let j = i + 2;
        while (j < s.length && (s.charCodeAt(j) < 0x40 || s.charCodeAt(j) > 0x7e)) j++;
        const params = s.slice(i + 2, j);
        // `CSI > 4 ; 2 m` and its kin end in m too, but aren't colours
        if (s[j] === "m" && !/^[<=>?]/.test(params)) st = sgr(st, params);
        i = j + 1;
      } else if (n === "]" || n === "P" || n === "_" || n === "^") {
        let j = i + 2;
        while (j < s.length && s[j] !== "\x07" && !(s[j] === "\x1b" && s[j + 1] === "\\")) j++;
        i = j + (s[j] === "\x07" ? 1 : 2);
      } else i += n === "(" || n === ")" || n === "#" ? 3 : 2;
      continue;
    }
    const row = rows[rows.length - 1];
    if (c === "\n") {
      rows.push([]);
      i++;
      continue;
    }
    if (c === "\t") {
      for (let k = 8 - (widthOf(row) % 8); k > 0; k--) row.push({ ch: " ", w: 1, st });
      i++;
      continue;
    }
    const cp = s.codePointAt(i) ?? 0;
    const ch = String.fromCodePoint(cp);
    i += ch.length;
    if (cp < 0x20 || (cp >= 0x7f && cp < 0xa0)) continue;
    const w = cellWidth(cp);
    if (w === 0) {
      const last = row[row.length - 1];
      if (last) row[row.length - 1] = { ...last, ch: last.ch + ch };
      continue;
    }
    row.push({ ch: cp === 0xa0 ? " " : ch, w, st });
  }
  return rows;
}

export const widthOf = (cells: Cell[]) => cells.reduce((s, c) => s + c.w, 0);
export const rowText = (cells: Cell[]) => cells.map((c) => c.ch).join("");

/** A space nothing is drawn behind. */
const blank = (c: Cell) => c.ch === " " && c.st.bg === undefined && !c.st.inverse;
/** Box drawing, blocks and ASCII rules: what a terminal-wide line is made of. */
const ruleOrBlank = (c: Cell) => c.ch === " " || /^[─-▟=_-]$/.test(c.ch);

function trimEnd(cells: Cell[]): Cell[] {
  let n = cells.length;
  while (n > 0 && blank(cells[n - 1])) n--;
  return n === cells.length ? cells : cells.slice(0, n);
}

function leading(cells: Cell[]): number {
  let n = 0;
  while (n < cells.length && blank(cells[n])) n++;
  return n;
}

/** How many cells fit in `cols` columns. */
function fitting(cells: Cell[], cols: number): number {
  let w = 0;
  for (let i = 0; i < cells.length; i++) {
    w += cells[i].w;
    if (w > cols) return i;
  }
  return cells.length;
}

/** Gaps of 4+ blanks after the indent closed to 2: what a wide terminal right-aligns comes along. */
function closeGaps(cells: Cell[]): Cell[] {
  const out: Cell[] = [];
  const lead = leading(cells);
  let gap: Cell[] = [];
  cells.forEach((c, i) => {
    if (i >= lead && blank(c)) {
      gap.push(c);
      return;
    }
    out.push(...(gap.length >= 4 ? gap.slice(0, 2) : gap), c);
    gap = [];
  });
  return out;
}

/** Wrapped at a space near the end when there is one, each continuation indented like the line. */
function wrap(cells: Cell[], cols: number): Cell[][] {
  const lead = Math.min(leading(cells), Math.floor(cols / 3));
  const indent = cells.slice(0, lead);
  const out: Cell[][] = [];
  let rest = cells;
  while (widthOf(rest) > cols) {
    const take = Math.max(lead + 1, fitting(rest, cols));
    let cut = take;
    for (let k = take; k > Math.max(lead, take * 0.66); k--) {
      if (blank(rest[k])) {
        cut = k;
        break;
      }
    }
    out.push(trimEnd(rest.slice(0, cut)));
    let next = cut;
    while (next < rest.length && blank(rest[next])) next++;
    rest = next < rest.length ? [...indent, ...rest.slice(next)] : [];
  }
  if (rest.length) out.push(rest);
  return out;
}

/**
 * One terminal row as rows `cols` wide: as it is when it fits; a rule or a
 * box's side squeezed (its last cell kept, the box's right edge); else with
 * its gaps closed, and wrapped when that's still too wide.
 */
export function fitRow(cells: Cell[], cols: number): Cell[][] {
  const row = trimEnd(cells);
  if (widthOf(row) <= cols) return [row];
  const k = fitting(row, cols - 1);
  if (row.slice(k).every(ruleOrBlank)) return [[...row.slice(0, k), row[row.length - 1]]];
  const closed = closeGaps(row);
  return widthOf(closed) <= cols ? [closed] : wrap(closed, cols);
}

/**
 * The screen as the card shows it: blank rows at the top and bottom gone,
 * runs of them inside down to one, each row fitted to `cols`, the last `max`.
 */
export function screenRows(text: string, cols: number, max: number): Cell[][] {
  const rows = parseScreen(text).map(trimEnd);
  let a = 0;
  let b = rows.length;
  while (a < b && !rows[a].length) a++;
  while (b > a && !rows[b - 1].length) b--;
  const kept: Cell[][] = [];
  for (const r of rows.slice(a, b)) if (r.length || kept[kept.length - 1].length) kept.push(r);
  const out = kept.flatMap((r) => fitRow(r, cols)).slice(-Math.max(1, max));
  while (out.length && !out[0].length) out.shift();
  return out;
}

/** The screen as plain text, without the blank rows around it (Copy Output, Peek). */
export function screenText(text: string): string {
  const rows = parseScreen(text).map((r) => rowText(trimEnd(r)));
  let a = 0;
  let b = rows.length;
  while (a < b && !rows[a]) a++;
  while (b > a && !rows[b - 1]) b--;
  return rows.slice(a, b).join("\n");
}

// ---------------------------------------------------------------- colour

/** The card's fill over the theme's surface: how much of the grid colour. */
const PANEL = 0.45;

interface Paint {
  fg: string;
  bg?: string;
  bold?: boolean;
  italic?: boolean;
  deco?: string;
}

/** xterm's 256 colours past the first 16: the 6×6×6 cube, then 24 greys. */
function xterm(n: number): string {
  if (n >= 232) {
    const g = 8 + (n - 232) * 10;
    return rgbHex(g, g, g);
  }
  const k = n - 16;
  const step = (v: number) => (v ? 55 + v * 40 : 0);
  return rgbHex(step(Math.floor(k / 36)), step(Math.floor(k / 6) % 6), step(k % 6));
}

const channels = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
/** A grey, near enough: no hue to keep. */
function neutral(hex: string): boolean {
  const c = channels(hex);
  return Math.max(...c) - Math.min(...c) <= 20;
}

/** How prominent ANSI's greys are, as text and behind it, on a dark terminal (black text is barely there). */
const FG_GREYS: Record<number, number> = { 0: 0.45, 7: 0.85, 8: 0.6, 15: 1 };
const BG_GREYS: Record<number, number> = { 0: 0, 7: 0.75, 8: 0.35, 15: 0.9 };

/**
 * How a screen's colours look on this theme. ANSI's red, green, yellow,
 * blue, magenta and cyan are Aleph's. Greys are taken as drawn for a dark
 * terminal and keep their prominence (white is the text, dim grey stays dim)
 * whichever way round this theme is. Every colour is fitted to read on the card.
 */
export class ScreenPaint {
  readonly panel: string;
  private readonly on: ChartTheme;
  private readonly hues: string[];
  private readonly memo = new WeakMap<Style, Paint>();

  constructor(readonly t: ChartTheme) {
    this.panel = mix(t.surface, t.grid, PANEL);
    this.on = { ...t, surface: this.panel };
    const s = t.state;
    const hues = [s.down, s.done, s.warn, s.working, s.limit, s.idle];
    this.hues = [...hues, ...hues.map((h) => mix(h, t.text, 0.25))];
  }

  /** A grey `l` of the way from the card's fill to the theme's text. */
  private grey(l: number): string {
    return mix(this.panel, this.t.text, l);
  }

  /** An ink's hue, or how far towards the text its grey is. */
  private read(ink: Ink, greys: Record<number, number>): { hue?: string; level: number } {
    if (typeof ink === "number" && ink < 16) {
      if (ink in greys) return { level: greys[ink] };
      return { hue: this.hues[(ink % 8) - 1 + (ink >= 8 ? 6 : 0)], level: 0 };
    }
    const hex = typeof ink === "number" ? xterm(ink) : ink;
    if (!neutral(hex)) return { hue: hex, level: 0 };
    const [r, g, b] = channels(hex);
    return { level: (0.299 * r + 0.587 * g + 0.114 * b) / 255 };
  }

  private fg(ink: Ink): string {
    const v = this.read(ink, FG_GREYS);
    return v.hue ?? this.grey(v.level);
  }

  /** Behind the text: a colour as a tint of its hue, a dark grey softer still (a band, not a block). */
  private bg(ink: Ink): string {
    const v = this.read(ink, BG_GREYS);
    if (v.hue) return mix(this.panel, fit(v.hue, this.on, 3), 0.3);
    return this.grey(v.level < 0.5 ? v.level * 0.6 : v.level);
  }

  paint(st: Style): Paint {
    const hit = this.memo.get(st);
    if (hit) return hit;
    let fg = st.fg === undefined ? this.t.text : this.fg(st.fg);
    let bg: string | undefined;
    if (st.inverse) {
      bg = fit(fg, this.on, 3);
      fg = st.bg === undefined ? this.panel : this.bg(st.bg);
    } else if (st.bg !== undefined) bg = this.bg(st.bg);
    if (st.dim) fg = mix(fg, bg ?? this.panel, 0.35);
    const under = bg ?? this.panel;
    fg = fit(fg, { ...this.on, surface: under }, st.dim ? 3 : 4.5);
    // fitting gave up (a colour on its own tint): whichever of the text and the fill reads
    if (contrast(fg, under) < 3)
      fg = contrast(this.t.text, under) >= contrast(this.panel, under) ? this.t.text : this.panel;
    const deco = [st.underline ? "underline" : "", st.strike ? "line-through" : ""].filter(Boolean).join(" ");
    const p: Paint = { fg, bg, bold: st.bold, italic: st.italic, deco: deco || undefined };
    this.memo.set(st, p);
    return p;
  }
}

// ---------------------------------------------------------------- the card

const MONO = "ui-monospace, 'SF Mono', SFMono-Regular, Menlo, Monaco, Consolas, monospace";
/** Font size, a column's width (Menlo / SF Mono advance 0.6 em), a row's height. */
const FS = 10.5;
const CW = FS * 0.602;
const LH = 14;
const PAD_X = 12;
const PAD_Y = 8;
const HEAD = 28;

/** How many columns the card's screen has at `width`. */
export const screenCols = (width = PANE) => Math.floor((width - 2 * PAD_X) / CW);

/** ASCII and box drawing keep the font's own advance; anything else is placed at its column. */
const steady = (ch: string) => /^[\x20-\x7e─-▟]$/.test(ch);

const r1 = (v: number) => Math.round(v * 10) / 10;

function rowSvg(cells: Cell[], top: number, paint: ScreenPaint): { back: string; front: string } {
  const back: string[] = [];
  const spans: string[] = [];
  const x = (col: number) => r1(PAD_X + col * CW);
  let run: { col: number; s: string; p: Paint; spaces: number } | undefined;
  const flush = () => {
    const s = run?.s.trimEnd();
    if (run && s) {
      const p = run.p;
      spans.push(
        `<tspan x="${x(run.col)}" fill="${p.fg}"${p.bold ? ' font-weight="700"' : ""}${
          p.italic ? ' font-style="italic"' : ""
        }${p.deco ? ` text-decoration="${p.deco}"` : ""}>${esc(s)}</tspan>`,
      );
    }
    run = undefined;
  };
  let bgRun: { col: number; w: number; fill: string } | undefined;
  const flushBg = () => {
    if (bgRun)
      back.push(
        `<rect x="${x(bgRun.col)}" y="${top}" width="${r1(bgRun.w * CW)}" height="${LH}" fill="${bgRun.fill}"/>`,
      );
    bgRun = undefined;
  };
  let col = 0;
  for (const c of cells) {
    const p = paint.paint(c.st);
    if (p.bg && bgRun?.fill === p.bg) bgRun.w += c.w;
    else {
      flushBg();
      if (p.bg) bgRun = { col, w: c.w, fill: p.bg };
    }
    if (c.ch === " ") {
      if (run) {
        run.s += " ";
        run.spaces++;
      }
    } else if (!steady(c.ch)) {
      flush();
      run = { col, s: c.ch, p, spaces: 0 };
      flush();
    } else if (run && run.p === p && run.spaces < 2) {
      run.s += c.ch;
      run.spaces = 0;
    } else {
      flush();
      run = { col, s: c.ch, p, spaces: 0 };
    }
    col += c.w;
  }
  flush();
  flushBg();
  return {
    back: back.join(""),
    front: spans.length ? `<text y="${r1(top + LH * 0.74)}">${spans.join("")}</text>` : "",
  };
}

export interface ScreenHead {
  /** the window's title: the session's name */
  title: string;
  /** after it, muted, with a dot in its colour: the machine */
  machine?: string;
  machineColor?: string;
  /** the agent's logo (a data URI), or a monogram */
  logo?: string;
  letter?: string;
  letterColor?: string;
  /** on the right, in `color`: the state's mark and the status (`idle · 33m`) */
  state?: string;
  status?: string;
  color?: string;
}

/**
 * The card's title bar: the agent, the session (and its machine); the state
 * on the right, the bar tinted with it. `alone`: the whole card (no screen under it).
 */
function head(h: ScreenHead, t: ChartTheme, width: number, alone = false): string {
  const parts: string[] = [];
  const c = h.color ?? t.other;
  const tint = `fill="${c}" fill-opacity="${t.dark ? 0.16 : 0.11}"`;
  if (alone) parts.push(`<rect x="0.5" y="0.5" width="${width - 1}" height="${HEAD}" rx="10" ${tint}/>`);
  else {
    parts.push(`<path d="${barPath(0.5, 0.5, width - 1, HEAD, 10, true)}" ${tint}/>`);
    parts.push(`<line x1="0.5" x2="${width - 0.5}" y1="${HEAD + 0.5}" y2="${HEAD + 0.5}" stroke="${t.grid}"/>`);
  }
  parts.push(logoMark(PAD_X, 7, 15, h.logo, { letter: h.letter ?? h.title, color: h.letterColor }, t));
  const status = h.status ? clip(h.status, width * 0.4, 11) : "";
  const sw = status ? textWidth(status, 11) + 22 : 0;
  let x = PAD_X + 22;
  const right = width - PAD_X - sw - 8;
  const title = clip(h.title, right - x, 12.5, false);
  parts.push(text(x, 18.5, title, t.text, 12.5, ' font-weight="700"'));
  x += textWidth(title, 12.5) * 1.06 + 10;
  if (h.machine && right - x > 30) {
    if (h.machineColor) {
      parts.push(`<circle cx="${r1(x + 3.5)}" cy="14.5" r="3.5" fill="${fit(h.machineColor, t)}"/>`);
      x += 11;
    }
    parts.push(text(x, 18.5, clip(h.machine, right - x, 11, false), t.muted, 11));
  }
  if (status) {
    parts.push(stateMark(h.state ?? "", width - PAD_X - textWidth(status, 11) - 11, 14.5, c));
    parts.push(text(width - PAD_X, 18.5, status, c, 11, ' text-anchor="end" font-weight="600"'));
  }
  return parts.join("");
}

export interface ScreenOpts {
  width?: number;
  /** the most rows shown: the last ones */
  rows?: number;
  /** a title bar */
  head?: ScreenHead;
  /** a rule down the left side in this colour (the question it's asking, in needs-you orange) */
  accent?: string;
}

/** The screen as a card: its last rows in the theme's colours, under a title bar. Undefined when it's blank. */
export function screenCard(screen: string, t: ChartTheme, o: ScreenOpts = {}): string | undefined {
  const width = o.width ?? PANE;
  const rows = screenRows(screen, screenCols(width), o.rows ?? 14);
  if (!rows.length) return undefined;
  const paint = new ScreenPaint(t);
  const top = (o.head ? HEAD : 0) + PAD_Y;
  const height = top + rows.length * LH + PAD_Y;
  const drawn = rows.map((r, i) => rowSvg(r, top + i * LH, paint));
  const body = [
    `<rect x="0.5" y="0.5" width="${width - 1}" height="${height - 1}" rx="10" fill="${t.grid}" fill-opacity="${PANEL}" stroke="${t.grid}"/>`,
    o.head ? head(o.head, t, width) : "",
    o.accent ? `<rect x="4" y="${top}" width="3" height="${rows.length * LH}" rx="1.5" fill="${o.accent}"/>` : "",
    drawn.map((d) => d.back).join(""),
    `<g font-family="${MONO}" font-size="${FS}">${drawn.map((d) => d.front).join("")}</g>`,
  ].join("");
  const said = rows
    .map((r) => rowText(r).trim())
    .filter(Boolean)
    .join(" / ");
  return svgDoc(width, height, body, (o.head ? `${o.head.title}: ${said}` : said).slice(0, 400));
}

/** The title bar on its own: who and what state, when there's no screen to show under it. */
export function headCard(h: ScreenHead, t: ChartTheme, width = PANE): string {
  return svgDoc(width, HEAD + 1, head(h, t, width, true), [h.title, h.status].filter(Boolean).join(": "));
}

// ---------------------------------------------------------------- the detail's top

/** Whether every line of `q` is somewhere in the rows (wrapping and spacing aside). */
function onScreen(q: string, rows: Cell[][]): boolean {
  const squash = (s: string) => s.replace(/\s+/g, "");
  const shown = squash(rows.map(rowText).join(""));
  const lines = screenText(q).split("\n").map(squash).filter(Boolean);
  return lines.length > 0 && lines.every((l) => shown.includes(l));
}

const md = (s: string) => s.replace(/[<>*_`#[\]|\\]/g, (c) => `\\${c}`);

/**
 * What a session's detail opens with: the question it's asking (when it
 * isn't on the screen already), then the screen. When the screen can't be
 * read, isn't read yet, or is blank: the title bar and one line saying so.
 */
export function previewMarkdown(o: {
  /** what `peek --ansi` read; undefined: no answer (yet) */
  screen?: string;
  question?: string | null;
  loading: boolean;
  error?: string;
  head: ScreenHead;
  t: ChartTheme;
  width?: number;
  rows?: number;
}): string {
  const width = o.width ?? PANE;
  const bar = (line: string) => `${img(o.head.title, headCard(o.head, o.t, width))}\n\n${line}`;
  if (o.screen === undefined) {
    if (o.error) {
      // ssh's words as the dashboard says them ("unreachable — ssh timed out"), else its first line
      const e = explainError(o.error);
      return bar(`_Can't read the screen: ${md(clip(e.head === "error" ? e.detail : e.head, 340, 13, false))}_`);
    }
    return bar(o.loading ? "_Reading the screen…_" : "_The live screen needs a newer aleph (`aleph peek`)_");
  }
  const rows = o.rows ?? 14;
  const out: string[] = [];
  const asked = o.question?.trim() ? o.question : undefined;
  const ask =
    asked && !onScreen(asked, screenRows(o.screen, screenCols(width), rows))
      ? screenCard(asked, o.t, { width, rows: 10, accent: o.t.state.blocked })
      : undefined;
  if (ask) out.push(img("What it's asking", ask));
  const card = screenCard(o.screen, o.t, { width, rows: ask ? Math.min(rows, 8) : rows, head: o.head });
  out.push(card ? img(`${o.head.title}'s screen`, card) : bar("_Nothing on the screen yet_"));
  return out.join("\n\n");
}
