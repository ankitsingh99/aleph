// Showcase mode's Raycast side, for a demo build of Aleph (cargo --features
// showcase): scripts/showcase.sh copies this file to src/lib/showcase.ts
// when ALEPH_SHOWCASE=1 (otherwise the inert stub.ts goes there). The same
// names and types as the stub.
//
// * mode() reads Aleph's own switch, <state>/showcase.json (`aleph showcase
//   on [--demo] | off` writes it), synchronously: looked at again at most
//   twice a second, and read again only when its mtime moves. Cache keys
//   carry it, so what was cached unmasked never shows while masked.
// * banner() / withBanner(): a gold-on-midnight hero card in the cockpit's
//   tones (the ℵ mark, a ✦ chip, a tagline that changes every 8 s, gold,
//   teal and rose stars) on top of a detail.
// * navTitle(): ✦ before a view's title. menu(): the menu bar's switch.
// * watermark(): your own line (showcase.watermark, when watermark_on has
//   raycast), read from aleph.toml the same way (its [showcase] table):
//   on the card's right and after the titles, never masked.
//
// No Raycast imports: plain node tests it.
import { readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export type Mode = "real" | "masked" | "demo";

/** One choice in the menu bar's switch. */
export interface ShowcaseItem {
  title: string;
  subtitle: string;
  /** `aleph …` */
  args: string[];
  /** the HUD once it has switched */
  hud: string;
  current: boolean;
}

export interface ShowcaseMenu {
  title: string;
  items: ShowcaseItem[];
}

export const TAGLINES = [
  "the point that contains all points",
  "Don't Panic",
  "mostly harmless",
  "Make it so",
  "never tell me the odds",
  "ℵ₀ sessions and counting",
];

/** `<state>/showcase.json` (lib/api.ts stateDir: XDG_STATE_HOME, else ~/.local/state; then aleph). */
export function stateFile(env: NodeJS.ProcessEnv = process.env, home = homedir()): string {
  const x = env.XDG_STATE_HOME?.trim();
  return join(x || join(home, ".local", "state"), "aleph", "showcase.json");
}

/** `{on, demo, …}` → the mode; anything else is real. */
export function parseMode(text: string): Mode {
  try {
    const v = JSON.parse(text) as { on?: unknown; demo?: unknown } | null;
    if (!v || typeof v !== "object" || v.on !== true) return "real";
    return v.demo === true ? "demo" : "masked";
  } catch {
    return "real";
  }
}

const LOOK_MS = 500;
let seen: { file: string; looked: number; mtime: number | undefined; mode: Mode; panic: boolean } | undefined;

/** The mode now (`now`: ms, for tests). */
export function mode(now = Date.now()): Mode {
  return look(now).mode;
}

/** The state file as last read (read again past half a second, when its mtime moved). */
function look(now: number): NonNullable<typeof seen> {
  const file = stateFile();
  if (seen && seen.file === file && now - seen.looked >= 0 && now - seen.looked < LOOK_MS) return seen;
  let mtime: number | undefined;
  try {
    mtime = statSync(file).mtimeMs;
  } catch {
    mtime = undefined;
  }
  if (seen && seen.file === file && seen.mtime === mtime) {
    seen.looked = now;
    return seen;
  }
  let m: Mode = "real";
  let panic = false;
  if (mtime !== undefined) {
    try {
      const text = readFileSync(file, "utf8");
      m = parseMode(text);
      panic = parsePanic(text);
    } catch {
      m = "real";
    }
  }
  seen = { file, looked: now, mtime, mode: m, panic };
  return seen;
}

export function on(): boolean {
  return mode() !== "real";
}

/** The tagline for this 8-second window. */
export function tagline(now = Date.now()): string {
  return TAGLINES[Math.floor(now / 8000) % TAGLINES.length];
}

// ---------------------------------------------------------------- the watermark

/** Aleph's settings: ALEPH_CONFIG, else `$XDG_CONFIG_HOME/aleph/aleph.toml`, else `~/.config/aleph/aleph.toml` (config.rs). */
export function configFile(env: NodeJS.ProcessEnv = process.env, home = homedir()): string {
  const named = env.ALEPH_CONFIG?.trim();
  if (named) return named;
  const x = env.XDG_CONFIG_HOME?.trim();
  return join(x || join(home, ".config"), "aleph", "aleph.toml");
}

/** A TOML value as text: "basic" (with its escapes), 'literal', or bare (the old key=value file). */
function unquote(raw: string): string {
  const v = raw.trim();
  if (v.startsWith('"')) {
    const esc: Record<string, string> = { n: "\n", t: "\t", r: "\r", b: "\b", f: "\f", '"': '"', "\\": "\\" };
    let out = "";
    for (let i = 1; i < v.length; i++) {
      const c = v[i];
      if (c === '"') return out;
      if (c === "\\" && i + 1 < v.length) {
        const n = v[++i];
        if (n === "u" || n === "U") {
          const len = n === "u" ? 4 : 8;
          const cp = parseInt(v.slice(i + 1, i + 1 + len), 16);
          if (!Number.isNaN(cp)) {
            out += String.fromCodePoint(cp);
            i += len;
            continue;
          }
        }
        out += esc[n] ?? n;
        continue;
      }
      out += c;
    }
    return out;
  }
  if (v.startsWith("'")) {
    const end = v.indexOf("'", 1);
    return end < 0 ? v.slice(1) : v.slice(1, end);
  }
  return v.replace(/\s+#.*$/, "").trim();
}

export interface Watermark {
  /** showcase.watermark (control characters out, 60 at most) */
  text: string;
  /** showcase.watermark_on, a comma list */
  on: string;
}

/** showcase.watermark and showcase.watermark_on in aleph.toml's text: its [showcase] table, or dotted keys at the top (and the old key=value lines). */
export function parseWatermark(text: string): Watermark {
  let table = "";
  const got: Record<string, string> = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    if (line.startsWith("[[")) {
      table = "\u0000";
      continue;
    }
    const t = /^\[\s*([^\]]+?)\s*\]\s*(#.*)?$/.exec(line);
    if (t) {
      table = t[1].replace(/["'\s]/g, "");
      continue;
    }
    const eq = line.indexOf("=");
    if (eq < 0) continue;
    const name = line.slice(0, eq).replace(/["'\s]/g, "");
    const key = table ? `${table}.${name}` : name;
    if (key === "showcase.watermark" || key === "showcase.watermark_on") got[key] = unquote(line.slice(eq + 1));
  }
  return {
    // eslint-disable-next-line no-control-regex
    text: [...(got["showcase.watermark"] ?? "").replace(/[\u0000-\u001f\u007f]/g, "").trim()].slice(0, 60).join(""),
    on: got["showcase.watermark_on"] ?? "tui,raycast,web",
  };
}

let marked: { file: string; looked: number; mtime: number | undefined; mark: Watermark } | undefined;

/** Your watermark, when it's shown in `place` (raycast): aleph.toml looked at again at most twice a second, read again when its mtime moves. Yours: never masked. */
export function watermark(place = "raycast", now = Date.now()): string | null {
  const file = configFile();
  const fresh = marked && marked.file === file && now - marked.looked >= 0 && now - marked.looked < LOOK_MS;
  if (!fresh) {
    let mtime: number | undefined;
    try {
      mtime = statSync(file).mtimeMs;
    } catch {
      mtime = undefined;
    }
    if (marked && marked.file === file && marked.mtime === mtime) {
      marked.looked = now;
    } else {
      let mark: Watermark = { text: "", on: "tui,raycast,web" };
      if (mtime !== undefined) {
        try {
          mark = parseWatermark(readFileSync(file, "utf8"));
        } catch {
          // unreadable: none
        }
      }
      marked = { file, looked: now, mtime, mark };
    }
  }
  const m = marked!.mark;
  const where = m.on.split(",").map((x) => x.trim().toLowerCase());
  return m.text && (where.includes(place) || where.includes("all")) ? m.text : null;
}

// ---------------------------------------------------------------- the card

const GOLD = "#E2B451";
const INK = "#1B1405";
/** The cockpit's deep space (the dashboard's showcase palette), and its nebula. */
const MIDNIGHT = "#0D1222";
const TEAL = "#2CC3B3";
const ROSE = "#F0709D";
const TEXT = "#F2F2F2";
const MUTED = "#B4B4B4";
const FONT = "-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Helvetica Neue', Helvetica, Arial, sans-serif";
/** Aleph's mark (web/index.html's #mark), on a 100 × 100 box. */
const MARK =
  "M 19 7 C 30 4, 37 11, 41 17 L 78 67 C 82 74, 88 81, 94 84 L 94 91 L 68 91 C 64 87, 62 82, 58 77 L 14 29 C 7 22, 8 9, 19 7 Z " +
  "M 62 8 L 86 5 C 91 9, 89 17, 84 22 C 77 30, 71 40, 64 54 L 51 40 C 57 31, 63 22, 66 17 C 66 13, 62 11, 62 8 Z " +
  "M 33 38 L 47 50 C 38 62, 33 72, 33 81 C 33 86, 36 89, 41 89 L 41 94 L 8 94 L 8 89 C 15 89, 21 86, 23 81 C 25 70, 28 52, 33 38 Z";
/** A faint starfield on the right: x, y, glyph, size. */
const STARS: [number, number, string, number][] = [
  [306, 24, "✦", 11],
  [334, 46, "·", 18],
  [352, 22, "˚", 14],
  [374, 54, "⋆", 13],
  [320, 64, "·", 14],
  [388, 30, "✦", 8],
  [296, 50, "⋆", 9],
  [362, 68, "˚", 10],
];

const esc = (s: string) =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] ?? c);

/** Roughly how wide text is (px): good enough to place a chip after it. */
const width = (s: string, size: number, bold = false) => [...s].length * size * (bold ? 0.62 : 0.55);

/** The hero card itself (an SVG document); your watermark, when given, on the right of its first line. */
export function card(m: Exclude<Mode, "real">, line: string, twinkle = 0, mark: string | null = null): string {
  const W = 400;
  const H = 76;
  const chip = m === "demo" ? "✦ demo" : "✦ showcase";
  const chipX = 76 + width("Aleph", 18, true) + 10;
  const chipW = width(chip, 11, true) + 16;
  // the watermark: as much as fits between the chip and the edge
  const room = W - 16 - (chipX + chipW + 14);
  let wm = mark ? [...mark] : [];
  while (wm.length > 1 && width(wm.join(""), 12) > room) wm = [...wm.slice(0, -2), "…"];
  const wmText = wm.join("");
  const wmX0 = W - 16 - width(wmText, 12);
  // (each star keeps its own colour and twinkle when some make room for it)
  const stars = STARS.map(([x, y, g, s], i) => ({ x, y, g, s, i }))
    .filter(({ x, y }) => !wmText || y > 42 || x < wmX0 - 8)
    .map(({ x, y, g, s, i }) => {
      const o = [0.55, 0.32, 0.18][(i + twinkle) % 3];
      const fill = [GOLD, TEAL, GOLD, ROSE][i % 4];
      return `<text x="${x}" y="${y}" font-size="${s}" fill="${fill}" fill-opacity="${o}">${g}</text>`;
    })
    .join("");
  const wmSvg = wmText
    ? `<text x="${W - 16}" y="34" font-size="12" font-style="italic" font-weight="600" fill="${GOLD}" text-anchor="end">${esc(wmText)}</text>`
    : "";
  const body =
    `<defs><linearGradient id="sc-glow" x1="0" y1="0" x2="1" y2="0">` +
    `<stop offset="0" stop-color="${GOLD}" stop-opacity="0.22"/>` +
    `<stop offset="0.6" stop-color="${TEAL}" stop-opacity="0.04"/>` +
    `<stop offset="1" stop-color="${ROSE}" stop-opacity="0.14"/></linearGradient></defs>` +
    `<rect x="0.5" y="0.5" width="${W - 1}" height="${H - 1}" rx="14" fill="${MIDNIGHT}"/>` +
    `<rect x="0.5" y="0.5" width="${W - 1}" height="${H - 1}" rx="14" fill="url(#sc-glow)" stroke="${GOLD}" stroke-opacity="0.5"/>` +
    `<rect x="14" y="14" width="48" height="48" rx="12" fill="${GOLD}" fill-opacity="0.14"/>` +
    `<g transform="translate(18 18) scale(0.4)"><path fill="${GOLD}" d="${MARK}"/></g>` +
    `<text x="76" y="35" font-size="18" font-weight="700" fill="${TEXT}">Aleph</text>` +
    `<rect x="${chipX.toFixed(1)}" y="20" width="${chipW.toFixed(1)}" height="20" rx="10" fill="${GOLD}"/>` +
    `<text x="${(chipX + chipW / 2).toFixed(1)}" y="34" font-size="11" font-weight="700" fill="${INK}" text-anchor="middle">${esc(chip)}</text>` +
    `<text x="76" y="57" font-size="12.5" font-style="italic" fill="${MUTED}">${esc(line)}</text>` +
    stars +
    wmSvg;
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" ` +
    `font-family="${FONT}" role="img" aria-label="${esc(`Aleph ${chip}: ${line}`)}">${body}</svg>`
  );
}

let drawn: { key: string; md: string } | undefined;

/** The banner as markdown (an SVG data-URI image), or null when showcase is off. */
export function banner(now = Date.now()): string | null {
  const m = mode(now);
  if (m === "real") return null;
  const k = Math.floor(now / 8000);
  const wm = watermark("raycast", now);
  const key = `${m}|${k}|${wm ?? ""}`;
  if (drawn?.key === key) return drawn.md;
  const line = tagline(now);
  const svg = card(m, line, k % 3, wm);
  const md = `![Aleph ${m === "demo" ? "demo" : "showcase"}: ${line.replace(/[[\]]/g, "")}](data:image/svg+xml;base64,${Buffer.from(svg, "utf8").toString("base64")})`;
  drawn = { key, md };
  return md;
}

/** A detail's markdown with the banner on top (an empty detail stays empty). */
export function withBanner(md: string): string {
  if (!md) return md;
  const b = banner();
  return b ? `${b}\n\n${md}` : md;
}

/** A navigation title, with ✦ while showcase is on (and your watermark after it). */
export function navTitle(t: string): string {
  if (!on()) return t;
  const wm = watermark("raycast");
  return wm ? `✦ ${t} · ${wm}` : `✦ ${t}`;
}

/** The menu bar's switch: `Showcase: masked | demo | off` and the three ways. */
export function menu(): ShowcaseMenu | null {
  const m = mode();
  return {
    title: `Showcase: ${m === "real" ? "off" : m}`,
    items: [
      {
        title: "Masked",
        subtitle: "the live fleet, private names as space references",
        args: ["showcase", "on"],
        hud: "Showcase on: names, folders and addresses are masked",
        current: m === "masked",
      },
      {
        title: "Demo World",
        subtitle: "a scripted fleet, nothing real reached",
        args: ["showcase", "on", "--demo"],
        hud: "Showcase on: the demo world",
        current: m === "demo",
      },
      {
        title: "Off",
        subtitle: "everything as it is",
        args: ["showcase", "off"],
        hud: "Showcase off",
        current: m === "real",
      },
      // safety: the pause (the panic key's switch)
      paused()
        ? {
            title: "End the Pause",
            subtitle: "every window shows what it did",
            args: ["showcase", "panic", "off"],
            hud: "Back: every window shows what it did",
            current: true,
          }
        : {
            title: "Pause Everything",
            subtitle: "every window a starfield at once (the panic key in a dashboard)",
            args: ["showcase", "panic", "on"],
            hud: "Paused: every window shows the stars",
            current: false,
          },
    ],
  };
}

// ---------------------------------------------------------------- safety
//
// The safety nets, as Raycast follows them: the pause (a field of
// showcase.json, any mode), and two of Aleph's settings read from its
// config (`[showcase]` in aleph.toml): clipboard = off keeps copy actions
// away, money = hidden makes $ amounts read $—. The daemon does the rest
// (blank answers while paused, scaled amounts, redacted screens).

/** `{panic: true, …}` → paused. */
export function parsePanic(text: string): boolean {
  try {
    const v = JSON.parse(text) as { panic?: unknown } | null;
    return !!v && typeof v === "object" && v.panic === true;
  } catch {
    return false;
  }
}

/** Is every window paused (the panic key, `aleph showcase panic`)? */
export function paused(now = Date.now()): boolean {
  return look(now).panic;
}

export interface Safety {
  clipboard: "masked" | "off" | "real";
  money: "real" | "scaled" | "hidden";
}

/** The `[showcase]` table's clipboard and money in aleph.toml's text (or dotted keys at the top, the old key=value lines): what isn't there, or isn't one of the choices, is its default. */
export function parseSafety(toml: string): Safety {
  const out: Safety = { clipboard: "masked", money: "real" };
  let table = "";
  for (const raw of toml.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    if (line.startsWith("[[")) {
      table = "\u0000";
      continue;
    }
    const t = /^\[\s*([^\]]+?)\s*\]\s*(#.*)?$/.exec(line);
    if (t) {
      table = t[1].replace(/["'\s]/g, "");
      continue;
    }
    const eq = line.indexOf("=");
    if (eq < 0) continue;
    const name = line.slice(0, eq).replace(/["'\s]/g, "");
    const key = table ? `${table}.${name}` : name;
    const val = unquote(line.slice(eq + 1));
    if (key === "showcase.clipboard" && (val === "masked" || val === "off" || val === "real")) out.clipboard = val;
    if (key === "showcase.money" && (val === "real" || val === "scaled" || val === "hidden")) out.money = val;
  }
  return out;
}

let safe: { file: string; looked: number; mtime: number | undefined; s: Safety } | undefined;

/** The settings now (`now`: ms, for tests): read again past half a second, when the file's mtime moved. */
export function safety(now = Date.now()): Safety {
  const file = configFile();
  if (safe && safe.file === file && now - safe.looked >= 0 && now - safe.looked < LOOK_MS) return safe.s;
  let mtime: number | undefined;
  try {
    mtime = statSync(file).mtimeMs;
  } catch {
    mtime = undefined;
  }
  if (safe && safe.file === file && safe.mtime === mtime) {
    safe.looked = now;
    return safe.s;
  }
  let s: Safety = { clipboard: "masked", money: "real" };
  if (mtime !== undefined) {
    try {
      s = parseSafety(readFileSync(file, "utf8"));
    } catch {
      /* the defaults */
    }
  }
  safe = { file, looked: now, mtime, s };
  return s;
}

/** May a copy action copy? Not while paused, nor while showcase keeps copies off. */
export function canCopy(): boolean {
  if (paused()) return false;
  return !on() || safety().clipboard !== "off";
}

/** How a $ amount reads: "$—" while showcase hides money, else null (format it as ever). */
export function moneyText(): string | null {
  return on() && safety().money === "hidden" ? "$—" : null;
}

/** The pause's card: a night sky, the ℵ and "be right back" (an SVG document). */
export function pauseCard(twinkle = 0): string {
  const W = 400;
  const H = 240;
  let stars = "";
  for (let i = 0; i < 46; i++) {
    let z = (i + 1) * 2654435761;
    z = Math.imul(z ^ (z >>> 13), 1274126177) >>> 0;
    const x = ((z % 1000) / 1000) * W;
    const y = (((z >>> 10) % 1000) / 1000) * H;
    const g = ["·", "✦", "˚", "⋆", "∙"][i % 5];
    const o = [0.6, 0.35, 0.2][(i + twinkle) % 3];
    const fill = i % 9 === 0 ? GOLD : MUTED;
    stars += `<text x="${x.toFixed(1)}" y="${y.toFixed(1)}" font-size="${9 + (i % 4) * 2}" fill="${fill}" fill-opacity="${o}">${g}</text>`;
  }
  const body =
    `<defs><radialGradient id="sc-night" cx="0.5" cy="0.45" r="0.7">` +
    `<stop offset="0" stop-color="#1a1712"/><stop offset="0.65" stop-color="#0b0b0d"/><stop offset="1" stop-color="#070708"/>` +
    `</radialGradient></defs>` +
    `<rect x="0.5" y="0.5" width="${W - 1}" height="${H - 1}" rx="16" fill="url(#sc-night)" stroke="${GOLD}" stroke-opacity="0.35"/>` +
    stars +
    `<g transform="translate(${W / 2 - 24} ${H / 2 - 52}) scale(0.48)"><path fill="${GOLD}" d="${MARK}"/></g>` +
    `<text x="${W / 2}" y="${H / 2 + 34}" font-size="16" font-style="italic" fill="${MUTED}" text-anchor="middle">be right back</text>`;
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" ` +
    `font-family="${FONT}" role="img" aria-label="Aleph: be right back">${body}</svg>`
  );
}

/** The pause's card as markdown (every view shows only this while paused). */
export function pausedMarkdown(now = Date.now()): string {
  const svg = pauseCard(Math.floor(now / 1500) % 3);
  return `![Aleph: be right back](data:image/svg+xml;base64,${Buffer.from(svg, "utf8").toString("base64")})`;
}

// ---------------------------------------------------------------- presenter sync
//
// showcase.sync = on: Sessions and Machines follow the dashboard you present
// from. The daemon keeps what it shows (presenter.get, topic "presenter";
// lib/hooks.ts usePresenter asks); here, whether to follow (the setting,
// read from aleph.toml like the safety nets), what its answer means, and
// which row to select. Its names come masked, like the lists' own.

/** What the presenter's dashboard shows (the daemon's presenter.get). */
export interface Presenter {
  on: boolean;
  view: string;
  host?: string;
  pane_id?: string;
  name?: string;
  gen?: number;
}

/** showcase.sync in aleph.toml's text (its [showcase] table, dotted keys, the old key=value lines). */
export function parseSync(toml: string): boolean {
  let table = "";
  let on = false;
  for (const raw of toml.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    if (line.startsWith("[[")) {
      table = "\u0000";
      continue;
    }
    const t = /^\[\s*([^\]]+?)\s*\]\s*(#.*)?$/.exec(line);
    if (t) {
      table = t[1].replace(/["'\s]/g, "");
      continue;
    }
    const eq = line.indexOf("=");
    if (eq < 0) continue;
    const name = line.slice(0, eq).replace(/["'\s]/g, "");
    const key = table ? `${table}.${name}` : name;
    if (key === "showcase.sync") on = ["on", "true", "yes", "1"].includes(unquote(line.slice(eq + 1)).toLowerCase());
  }
  return on;
}

let synced: { file: string; looked: number; mtime: number | undefined; on: boolean } | undefined;

/** showcase.sync now: aleph.toml looked at again past half a second, read again when its mtime moved. */
function sync(now: number): boolean {
  const file = configFile();
  if (synced && synced.file === file && now - synced.looked >= 0 && now - synced.looked < LOOK_MS) return synced.on;
  let mtime: number | undefined;
  try {
    mtime = statSync(file).mtimeMs;
  } catch {
    mtime = undefined;
  }
  if (synced && synced.file === file && synced.mtime === mtime) {
    synced.looked = now;
    return synced.on;
  }
  let on = false;
  if (mtime !== undefined) {
    try {
      on = parseSync(readFileSync(file, "utf8"));
    } catch {
      on = false;
    }
  }
  synced = { file, looked: now, mtime, on };
  return on;
}

/** Does this extension follow the presenter's dashboard? While showcase mode is on and showcase.sync says so. */
export function following(now = Date.now()): boolean {
  return mode(now) !== "real" && sync(now);
}

/** presenter.get's answer as a presenter (null: nobody presents, or it isn't one). */
export function parsePresenter(v: unknown): Presenter | null {
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (o.on !== true) return null;
  const s = (k: string) => (typeof o[k] === "string" && o[k] ? (o[k] as string) : undefined);
  return {
    on: true,
    view: s("view") ?? "fleet",
    host: s("host"),
    pane_id: s("pane_id"),
    name: s("name"),
    gen: typeof o.gen === "number" ? o.gen : undefined,
  };
}

/** The row the presenter has selected: a session (`host:pane_id`, Sessions' ids) or a machine (its alias). */
export function presenterItem(p: Presenter | null, list: "sessions" | "machines"): string | undefined {
  if (!p) return undefined;
  if (list === "machines") return p.host;
  return p.host && p.pane_id ? `${p.host}:${p.pane_id}` : undefined;
}

/** What a title says while it follows the presenter. */
export function followingTitle(p: Presenter | null): string {
  return p ? "▶ following the presenter" : "";
}
