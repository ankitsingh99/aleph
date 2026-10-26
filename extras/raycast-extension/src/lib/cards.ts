// More dashboard pieces for the inventory details (Agents, LLM Servers,
// Machines): a checklist (✓ / ▲ / ✕ per line), item rows (a mark or logo,
// a name, what's under it, a figure on the right) and a notice card (what's
// wrong, in one or two lines). SVG for Raycast's Detail markdown, drawn like
// dash.ts: the theme's own text colours on a transparent background, colours
// that mean something. No Raycast imports: plain node tests it.
import { ChartTheme, clip as clipText, esc, svgDoc, text, textWidth } from "./charts";
import { logoMark, PANE, stateMark } from "./dash";

/** Cut to fit from the end (a model's or a path's end is cut by `clipPath`). */
const clip = (s: string, px: number, size: number) => clipText(s, px, size, false);
/** Cut to fit from the start when it's a path (`…/models/llama`), else from the end. */
const clipPath = (s: string, px: number, size: number) => clipText(s, px, size);

const r1 = (v: number) => Math.round(v * 10) / 10;

/** A card's title line: bold title, muted note after it. */
function titleLine(title: string, note: string | undefined, t: ChartTheme): string {
  return `<text x="0" y="13"><tspan fill="${t.text}" font-size="12" font-weight="600">${esc(title)}</tspan>${
    note ? `<tspan dx="6" fill="${t.muted}" font-size="12">${esc(note)}</tspan>` : ""
  }</text>`;
}

/** Words wrapped into lines no wider than `px` at `size`, at most `max` lines (the last one cut). */
export function wrap(s: string, px: number, size: number, max = 3): string[] {
  const words = s.split(/\s+/).filter(Boolean);
  const lines: string[] = [];
  let cur = "";
  for (const w of words) {
    const next = cur ? `${cur} ${w}` : w;
    if (cur && textWidth(next, size) > px) {
      lines.push(cur);
      cur = w;
    } else cur = next;
  }
  if (cur) lines.push(cur);
  if (lines.length <= max) return lines.map((l) => clip(l, px, size));
  const kept = lines.slice(0, max);
  kept[max - 1] = clip(`${kept[max - 1]} ${lines.slice(max).join(" ")}`, px, size);
  return kept;
}

// ---------------------------------------------------------------- checklist

/** ok ✓ green · warn ▲ yellow · bad ✕ red · off ○ grey (not set up, not there) · info • (a plain fact) */
export type CheckState = "ok" | "warn" | "bad" | "off" | "info";

export interface Check {
  label: string;
  state: CheckState;
  /** muted, in the value column: `installed`, `/usr/bin/curl`, `--continue` */
  value?: string;
}

export function checkColor(s: CheckState, t: ChartTheme): string {
  return s === "ok"
    ? t.state.done
    : s === "warn"
      ? t.state.warn
      : s === "bad"
        ? t.state.down
        : s === "off"
          ? t.other
          : t.muted;
}

function checkMark(s: CheckState, cx: number, cy: number, t: ChartTheme): string {
  const c = checkColor(s, t);
  switch (s) {
    case "ok":
      return stateMark("done", cx, cy, c);
    case "warn":
      return stateMark("warn", cx, cy, c);
    case "bad":
      return stateMark("down", cx, cy, c);
    case "off":
      return `<circle cx="${r1(cx)}" cy="${r1(cy)}" r="4.5" fill="none" stroke="${c}" stroke-width="1.6"/>`;
    default:
      return `<circle cx="${r1(cx)}" cy="${r1(cy)}" r="2.5" fill="${c}"/>`;
  }
}

/** One line per check: its mark, the label, the value in an aligned column. */
export function checks(
  rows: Check[],
  t: ChartTheme,
  width = PANE,
  o: { title?: string; note?: string; labelW?: number } = {},
): string {
  const rowH = 24;
  const top = o.title ? 22 : 0;
  const labelW = o.labelW ?? Math.min(170, Math.max(90, ...rows.map((r) => textWidth(r.label, 12) + 34)));
  const parts: string[] = [];
  if (o.title) parts.push(titleLine(o.title, o.note, t));
  rows.forEach((r, i) => {
    const y = top + i * rowH;
    const cy = y + rowH / 2;
    parts.push(
      `<g><title>${esc(`${r.label}${r.value ? `: ${r.value}` : ""}`)}</title>${checkMark(r.state, 7, cy, t)}${text(
        22,
        cy + 4.2,
        clip(r.label, labelW - 30, 12),
        r.state === "off" || r.state === "info" ? t.muted : t.text,
        12,
      )}${r.value ? text(labelW, cy + 4.2, clipPath(r.value, width - labelW, 12), r.state === "warn" || r.state === "bad" ? t.text : t.muted, 12) : ""}</g>`,
    );
  });
  return svgDoc(
    width,
    Math.max(1, top + rows.length * rowH),
    parts.join(""),
    rows.map((r) => `${r.label}${r.value ? ` ${r.value}` : ""}`).join(", "),
  );
}

// ---------------------------------------------------------------- item rows

export interface ItemRow {
  name: string;
  /** muted, on a second line: `8.0B · Q4_K_M · 8K ctx` */
  sub?: string;
  /** bold, on the right: `38G`, `2.1.309` */
  value?: string;
  /** the value's colour (default: the text colour) */
  valueColor?: string;
  /** muted, under the value */
  valueSub?: string;
  /** the leading mark: a filled dot (on), a ring (off) or a state mark, in `color` */
  mark?: { kind: "dot" | "ring" | "check" | "warn" | "down"; color: string };
  /** an embedded logo before the name (a monogram from `letter` when there's none) */
  logo?: string;
  letter?: string;
  letterColor?: string;
  /** a name that's there but not in use (a model not loaded, an agent missing) */
  muted?: boolean;
}

function rowMark(m: NonNullable<ItemRow["mark"]>, cx: number, cy: number): string {
  switch (m.kind) {
    case "dot":
      return `<circle cx="${r1(cx)}" cy="${r1(cy)}" r="4" fill="${m.color}"/>`;
    case "ring":
      return `<circle cx="${r1(cx)}" cy="${r1(cy)}" r="3.8" fill="none" stroke="${m.color}" stroke-width="1.5"/>`;
    case "check":
      return stateMark("done", cx, cy, m.color);
    case "warn":
      return stateMark("warn", cx, cy, m.color);
    case "down":
      return stateMark("down", cx, cy, m.color);
  }
}

/** One row per item (a model, a machine): mark, logo, name over what's under it, a figure on the right. */
export function itemRows(
  rows: ItemRow[],
  t: ChartTheme,
  width = PANE,
  o: { title?: string; note?: string; more?: string } = {},
): string {
  const top = o.title ? 22 : 0;
  const parts: string[] = [];
  if (o.title) parts.push(titleLine(o.title, o.note, t));
  let y = top;
  rows.forEach((r, i) => {
    const two = !!(r.sub || r.valueSub);
    const h = two ? 38 : 26;
    if (i) parts.push(`<line x1="0" x2="${width}" y1="${r1(y) + 0.5}" y2="${r1(y) + 0.5}" stroke="${t.grid}"/>`);
    const l1 = two ? y + 16 : y + 17;
    const l2 = y + 31;
    let x = 0;
    if (r.mark) {
      parts.push(rowMark(r.mark, 6, l1 - 4.2));
      x = 18;
    }
    if (r.logo || r.letter) {
      parts.push(logoMark(x, l1 - 12.5, 16, r.logo, { letter: r.letter, color: r.letterColor }, t));
      x += 22;
    }
    const vw = r.value ? textWidth(r.value, 12) * 1.05 + 10 : 0;
    const vsw = r.valueSub ? textWidth(r.valueSub, 11) + 10 : 0;
    parts.push(
      text(
        x,
        l1,
        clipPath(r.name, width - x - vw, 12.5),
        r.muted ? t.muted : t.text,
        12.5,
        r.muted ? "" : ' font-weight="600"',
      ),
    );
    if (r.value)
      parts.push(text(width, l1, r.value, r.valueColor ?? t.text, 12, ' text-anchor="end" font-weight="600"'));
    if (r.sub) parts.push(text(x, l2, clip(r.sub, width - x - vsw, 11), t.muted, 11));
    if (r.valueSub) parts.push(text(width, l2, r.valueSub, t.muted, 11, ' text-anchor="end"'));
    y += h;
  });
  if (o.more) {
    parts.push(text(0, y + 15, o.more, t.muted, 11));
    y += 20;
  }
  return svgDoc(
    width,
    Math.max(1, y),
    parts.join(""),
    rows.map((r) => `${r.name}${r.value ? ` ${r.value}` : ""}`).join(", "),
  );
}

// ---------------------------------------------------------------- notice

export interface Notice {
  /** the mark: down ✕, warn ▲, blocked ▲ (filled), done ✓, … (dash.ts stateMark) */
  state: string;
  color: string;
  title: string;
  /** a line or two under it */
  detail?: string;
}

/** A tinted card that says what's wrong (or what to do): its mark, a bold line, a muted line or two. */
export function notice(n: Notice, t: ChartTheme, width = PANE): string {
  const pad = 12;
  const tx = 38;
  const lines = n.detail ? wrap(n.detail, width - tx - pad, 12, 3) : [];
  const h = 22 + 16 + lines.length * 16 + (lines.length ? 2 : -6);
  const parts = [
    `<rect x="0.5" y="0.5" width="${width - 1}" height="${h - 1}" rx="10" fill="${n.color}" fill-opacity="${t.dark ? 0.12 : 0.08}" stroke="${n.color}" stroke-opacity="0.45"/>`,
    stateMark(n.state, 19, 20, n.color),
    text(tx, 24, clip(n.title, width - tx - pad, 13), t.text, 13, ' font-weight="600"'),
    ...lines.map((l, i) => text(tx, 42 + i * 16, l, t.muted, 12)),
  ];
  return svgDoc(width, h, parts.join(""), [n.title, n.detail].filter(Boolean).join(": "));
}

// ---------------------------------------------------------------- hint row

/** A keycap (`⌘N`) with its left edge at x and its text's baseline at y: the SVG and its width. */
function keycap(x: number, y: number, keys: string, t: ChartTheme): [string, number] {
  const w = textWidth(keys, 11) + 12;
  return [
    `<rect x="${r1(x)}" y="${r1(y - 12.5)}" width="${r1(w)}" height="17" rx="4.5" fill="${t.grid}" fill-opacity="${t.dark ? 0.6 : 0.7}" stroke="${t.muted}" stroke-opacity="0.35"/>` +
      text(x + w / 2, y, keys, t.text, 11, ' text-anchor="middle" font-weight="600"'),
    w,
  ];
}

export interface Hint {
  /** what's there (or isn't): `No sessions` */
  text: string;
  /** the shortcut that does something about it, and what */
  keys?: string;
  action?: string;
  /** a state mark before the text (dash.ts stateMark; default: a ring, nothing there) */
  state?: string;
  color?: string;
}

/** An empty state as a quiet row: a dashed outline, a ring, the words, the shortcut's keycap on the right. */
export function hintRow(h: Hint, t: ChartTheme, width = PANE): string {
  const H = 34;
  const cy = H / 2;
  const parts = [
    `<rect x="0.5" y="0.5" width="${width - 1}" height="${H - 1}" rx="9" fill="none" stroke="${t.muted}" stroke-opacity="0.45" stroke-dasharray="4 3"/>`,
    h.state
      ? stateMark(h.state, 16, cy, h.color ?? t.muted)
      : `<circle cx="16" cy="${cy}" r="4.5" fill="none" stroke="${h.color ?? t.muted}" stroke-width="1.6"/>`,
  ];
  let right = width - 10;
  if (h.action) {
    parts.push(text(right, cy + 4.2, h.action, t.muted, 12, ' text-anchor="end"'));
    right -= textWidth(h.action, 12) + 8;
  }
  if (h.keys) {
    const kw = textWidth(h.keys, 11) + 12;
    parts.push(keycap(right - kw, cy + 4, h.keys, t)[0]);
    right -= kw + 8;
  }
  parts.push(text(30, cy + 4.2, clip(h.text, right - 30, 12), t.muted, 12));
  return svgDoc(width, H, parts.join(""), [h.text, h.keys, h.action].filter(Boolean).join(" "));
}

// ---------------------------------------------------------------- chip rows

export interface RowChip {
  text: string;
  /** muted, after it: a version, `1 loaded` */
  note?: string;
  logo?: string;
  /** a monogram when there's no logo */
  letter?: string;
  color?: string;
  /** a health dot at its end */
  dot?: string;
}

export interface ChipRow {
  label: string;
  chips: RowChip[];
  /** shown as one faint dashed chip when there are none */
  empty?: string;
}

/** Labelled rows of logo chips (agents, LLM servers), the chips wrapping; an empty row is one faint chip. */
export function chipRows(rows: ChipRow[], t: ChartTheme, width = PANE, o: { labelW?: number } = {}): string {
  const labelW = o.labelW ?? Math.max(60, ...rows.map((r) => textWidth(r.label, 12) + 14));
  const h = 24;
  const gap = 6;
  const parts: string[] = [];
  let y = 0;
  for (const row of rows) {
    parts.push(text(0, y + 16, clip(row.label, labelW - 8, 12), t.muted, 12));
    let x = labelW;
    if (!row.chips.length) {
      const words = row.empty ?? "none";
      const w = textWidth(words, 12) + 20;
      parts.push(
        `<rect x="${r1(x) + 0.5}" y="${y + 0.5}" width="${r1(w)}" height="${h - 1}" rx="7" fill="none" stroke="${t.muted}" stroke-opacity="0.45" stroke-dasharray="3 3"/>`,
        text(x + 10, y + 16, words, t.muted, 12),
      );
      y += h + 8;
      continue;
    }
    for (const c of row.chips) {
      const mark = c.logo || c.letter ? 20 : 0;
      const noteW = c.note ? textWidth(c.note, 11) + 5 : 0;
      const w = Math.min(width - labelW, 8 + mark + textWidth(c.text, 12) + noteW + (c.dot ? 13 : 0) + 9);
      if (x > labelW && x + w > width) {
        x = labelW;
        y += h + gap;
      }
      parts.push(
        `<rect x="${r1(x)}" y="${y}" width="${r1(w)}" height="${h}" rx="7" fill="${t.grid}" fill-opacity="${t.dark ? 0.55 : 0.6}"/>`,
      );
      let cx = x + 8;
      if (mark) {
        parts.push(logoMark(cx, y + 5, 14, c.logo, { letter: c.letter ?? c.text, color: c.color }, t));
        cx += 20;
      }
      const room = x + w - cx - noteW - (c.dot ? 13 : 0) - 6;
      parts.push(
        `<text x="${r1(cx)}" y="${y + 16}"><tspan fill="${t.text}" font-size="12">${esc(clip(c.text, room, 12))}</tspan>${
          c.note ? `<tspan dx="5" fill="${t.muted}" font-size="11">${esc(c.note)}</tspan>` : ""
        }</text>`,
      );
      if (c.dot) parts.push(`<circle cx="${r1(x + w - 12)}" cy="${y + h / 2}" r="3.5" fill="${c.dot}"/>`);
      x += w + gap;
    }
    y += h + 8;
  }
  return svgDoc(
    width,
    Math.max(1, y - 8),
    parts.join(""),
    rows.map((r) => `${r.label}: ${r.chips.map((c) => c.text).join(", ") || r.empty || "none"}`).join("; "),
  );
}
