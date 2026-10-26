// `npm test`: the session's screen as a card (screen.ts): ANSI parsing,
// fitting a wide terminal's rows to the card, the colours on each theme, and
// what the detail opens with when there's no screen to show.
import assert from "node:assert/strict";
import { test } from "node:test";
import { chartTheme, contrast } from "./charts";
import {
  cellWidth,
  fitRow,
  headCard,
  parseScreen,
  previewMarkdown,
  rowText,
  screenCard,
  screenCols,
  ScreenPaint,
  screenRows,
  screenText,
  widthOf,
} from "./screen";

const dark = chartTheme(true);
const light = chartTheme(false);
const ESC = "\x1b";

/** What `aleph peek dex42:frosting-crane --json --ansi` read: Claude Code idle at the bottom of a 40-row pane. */
const IDLE =
  "\n".repeat(36) +
  `${ESC}[0m${ESC}[38;2;136;136;136m${"─".repeat(220)}${ESC}[0m\n` +
  `❯\u00a0${ESC}[0m${ESC}[2mTry "refactor <filepath>"${ESC}[0m\n` +
  `${ESC}[0m${ESC}[38;2;136;136;136m${"─".repeat(220)}${ESC}[0m\n` +
  `  ${ESC}[0m${ESC}[38;2;175;135;255m⏵⏵ accept edits on${ESC}[0m${ESC}[38;2;153;153;153m (shift+tab to cycle)${ESC}[0m`;

const cells = (s: string) => parseScreen(s)[0];
const texts = (rows: ReturnType<typeof screenRows>) => rows.map(rowText);

test("parseScreen keeps SGR colours and drops other escapes", () => {
  const [row] = parseScreen(
    `${ESC}]0;title${ESC}\\${ESC}[?25l${ESC}[1;31mA${ESC}[0mB${ESC}[38;5;208mC${ESC}[48;2;1;2;3mD`,
  );
  assert.equal(rowText(row), "ABCD");
  assert.deepEqual(row[0].st, { bold: true, fg: 1 });
  assert.deepEqual(row[1].st, {});
  assert.equal(row[2].st.fg, 208);
  assert.deepEqual(row[3].st, { fg: 208, bg: "#010203" });
  // the modifyOtherKeys kind of `CSI … m` isn't a colour
  assert.deepEqual(cells(`${ESC}[>4;2mX`)[0].st, {});
  // 22 ends bold and dim; 39 / 49 the colours
  assert.deepEqual(cells(`${ESC}[1;2;33;44m${ESC}[22;39;49mX`)[0].st, { bold: false, dim: false });
});

test("parseScreen: wide characters take two columns, marks join the one before, NBSP is a space", () => {
  assert.equal(cellWidth("界".codePointAt(0)!), 2);
  assert.equal(cellWidth("🚀".codePointAt(0)!), 2);
  assert.equal(cellWidth("⏺".codePointAt(0)!), 1);
  assert.equal(cellWidth(0x0301), 0);
  const row = cells("é界\u00a0x");
  assert.deepEqual(
    row.map((c) => [c.ch, c.w]),
    [
      ["é", 1],
      ["界", 2],
      [" ", 1],
      ["x", 1],
    ],
  );
  assert.equal(widthOf(row), 5);
  assert.equal(widthOf(cells("\tx")), 9);
});

test("screenRows drops the blank rows an agent's TUI leaves above it", () => {
  const rows = screenRows(IDLE, 59, 14);
  assert.equal(rows.length, 4);
  assert.match(rowText(rows[1]), /^❯ Try "refactor <filepath>"$/);
  assert.equal(rowText(rows[3]), "  ⏵⏵ accept edits on (shift+tab to cycle)");
  // the 220-column rules squeezed to the card
  assert.equal(rowText(rows[0]), "─".repeat(59));
  assert.ok(rows.every((r) => widthOf(r) <= 59));
  // aleph's --ansi read comes with CRLF
  assert.deepEqual(texts(screenRows(IDLE.replace(/\n/g, "\r\n"), 59, 14)), texts(rows));
  // runs of blank rows inside close to one; only the last rows are kept
  assert.deepEqual(texts(screenRows("a\n\n\n\nb", 20, 9)), ["a", "", "b"]);
  assert.deepEqual(texts(screenRows("a\nb\nc\nd", 20, 2)), ["c", "d"]);
  // a blank first row after the cut goes too
  assert.deepEqual(texts(screenRows("a\n\n\n\nb\nc\nd", 20, 4)), ["b", "c", "d"]);
  assert.deepEqual(screenRows("\n\n   \n", 20, 9), []);
});

test("fitRow squeezes a box to the card, keeping its right edge", () => {
  const top = fitRow(cells(`╭${"─".repeat(200)}╮`), 30);
  const mid = fitRow(cells(`│ > hello${" ".repeat(192)}│`), 30);
  assert.equal(top.length, 1);
  assert.equal(rowText(top[0]), `╭${"─".repeat(28)}╮`);
  assert.equal(rowText(mid[0]), `│ > hello${" ".repeat(20)}│`);
  assert.equal(widthOf(top[0]), widthOf(mid[0]));
});

test("fitRow closes a right-aligned status up, and wraps prose at a space with its indent", () => {
  const status = fitRow(cells(`  ? for shortcuts${" ".repeat(150)}◯ IDE disconnected`), 40);
  assert.deepEqual(status.map(rowText), ["  ? for shortcuts  ◯ IDE disconnected"]);
  const prose = fitRow(cells(`  ⎿  ${"word ".repeat(20).trim()}`), 24);
  assert.ok(prose.length > 1);
  assert.ok(prose.every((r) => widthOf(r) <= 24));
  assert.ok(prose.slice(1).every((r) => rowText(r).startsWith("  word")));
  assert.equal(
    prose
      .map((r) => rowText(r).trim())
      .join(" ")
      .replace(/^⎿\s+/, ""),
    "word ".repeat(20).trim(),
  );
  // a word longer than a row is cut where it has to be
  assert.deepEqual(fitRow(cells("x".repeat(25)), 10).map(rowText), ["x".repeat(10), "x".repeat(10), "x".repeat(5)]);
});

test("screenText is the screen without its colours and the blank rows around it", () => {
  assert.equal(screenText(IDLE).split("\n")[1], '❯ Try "refactor <filepath>"');
  assert.equal(screenText(IDLE).split("\n").length, 4);
  assert.equal(screenText("\n\n  \n"), "");
});

test("ScreenPaint: Aleph's colours for ANSI's, greys relative to the theme, everything readable", () => {
  for (const t of [dark, light]) {
    const p = new ScreenPaint(t);
    const st = (s: string) => cells(`${s}X`)[0].st;
    // red is Aleph's down red, blue its working blue (as fitted for text)
    assert.ok(contrast(p.paint(st(`${ESC}[31m`)).fg, p.panel) >= 4.5);
    // white text from a dark terminal is this theme's text, black on it still shows
    assert.equal(p.paint(st(`${ESC}[38;2;255;255;255m`)).fg, t.text);
    for (const s of [`${ESC}[30m`, `${ESC}[37m`, `${ESC}[38;5;232m`, `${ESC}[38;2;0;0;0m`, `${ESC}[33m`, `${ESC}[96m`])
      assert.ok(contrast(p.paint(st(s)).fg, p.panel) >= 4.5, `${t.dark ? "dark" : "light"} ${JSON.stringify(s)}`);
    // dim is quieter than the text, still 3:1
    const dim = p.paint(st(`${ESC}[2m`)).fg;
    assert.ok(contrast(dim, p.panel) >= 3 && contrast(dim, p.panel) < contrast(t.text, p.panel));
    // a background is drawn and the text on it reads; inverse swaps
    for (const s of [`${ESC}[42;30m`, `${ESC}[48;2;55;55;55m`, `${ESC}[7m`, `${ESC}[41;37m`]) {
      const x = p.paint(st(s));
      assert.ok(x.bg, s);
      assert.ok(contrast(x.fg, x.bg!) >= 3, `${t.dark ? "dark" : "light"} ${JSON.stringify(s)} ${x.fg} on ${x.bg}`);
    }
    assert.equal(p.paint(st(`${ESC}[7m`)).fg, p.panel);
  }
});

test("screenCard draws the last rows under a title bar, sized to them", () => {
  const svg = screenCard(IDLE, dark, {
    head: {
      title: "frosting-crane",
      machine: "dex42",
      machineColor: "#94e2d5",
      state: "idle",
      status: "idle · 33m",
      color: dark.state.idle,
    },
  })!;
  assert.match(svg, /^<svg [^>]*width="400" height="(\d+)"/);
  const h = Number(/height="(\d+)"/.exec(svg)![1]);
  assert.ok(h < 120, `4 rows stay small (${h})`);
  assert.ok(svg.includes("frosting-crane") && svg.includes("idle · 33m") && svg.includes("dex42"));
  assert.ok(svg.includes("Try &quot;refactor &lt;filepath&gt;&quot;"));
  assert.ok(svg.includes("monospace"));
  // nothing on it: no card
  assert.equal(screenCard("\n\n\n", dark), undefined);
  // a long screen keeps its last rows only
  const long = Array.from({ length: 60 }, (_, i) => `line ${i}`).join("\n");
  const card = screenCard(long, light, { rows: 5 })!;
  assert.ok(card.includes("line 59") && card.includes("line 55") && !card.includes("line 54"));
  assert.equal(screenCols(400), 59);
});

test("previewMarkdown: the screen as an image; the title bar and one line when there's none", () => {
  const head = { title: "frosting-crane", status: "idle · 33m" };
  const md = previewMarkdown({ screen: IDLE, loading: false, head, t: dark });
  assert.match(md, /^!\[frosting-crane's screen\]\(data:image\/svg\+xml;base64,/);
  assert.equal(md.split("\n").length, 1);
  // no screen card, never an empty box: the bar, then what happened in one line
  const line = (o: Parameters<typeof previewMarkdown>[0]) => {
    const [bar, said, ...rest] = previewMarkdown(o).split("\n\n");
    assert.match(bar, /^!\[frosting-crane\]\(data:image\/svg\+xml;base64,/);
    assert.equal(rest.length, 0);
    assert.ok(!said.includes("\n"));
    return said;
  };
  assert.equal(line({ screen: "\n\n", loading: false, head, t: dark }), "_Nothing on the screen yet_");
  assert.equal(line({ loading: true, head, t: dark }), "_Reading the screen…_");
  assert.equal(
    line({ loading: false, error: "ssh: connect to host gpu2 port 22: Operation timed out\nmore", head, t: dark }),
    "_Can't read the screen: unreachable — ssh timed out_",
  );
  assert.equal(
    line({ loading: false, error: "no session named nope\nhint: aleph ls", head, t: dark }),
    "_Can't read the screen: no session named nope_",
  );
  assert.match(line({ loading: false, head, t: dark }), /newer aleph/);
  // the bar alone carries who and the state
  const bar = headCard({ ...head, state: "idle", color: dark.state.idle }, dark);
  assert.ok(bar.includes("frosting-crane") && bar.includes("idle · 33m"));
  assert.match(bar, /height="29"/);
});

test("previewMarkdown shows the question above the screen unless it's on the screen already", () => {
  const head = { title: "temper" };
  const q = " Do you want to run this command?\n   cargo bench --bench parse_layout\n\n ❯ 1. Yes\n   2. No";
  const other = previewMarkdown({
    screen: "⏺ I'll profile the engine first.",
    question: q,
    loading: false,
    head,
    t: light,
  });
  assert.equal(other.split("\n\n").length, 2);
  assert.match(other, /^!\[What it's asking\]/);
  const same = previewMarkdown({ screen: `⏺ Bash(cargo)\n\n${q}\n`, question: q, loading: false, head, t: light });
  assert.equal(same.split("\n\n").length, 1);
});
