// `npm test`: showcase mode's real module (showcase/real.ts, what a demo
// build copies to src/lib/showcase.ts): the switch read from a state dir of
// its own, the banner card, the ✦ titles and the menu bar's choices.
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, utimesSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
  banner,
  card,
  configFile,
  following,
  followingTitle,
  menu,
  mode,
  navTitle,
  parseMode,
  parsePresenter,
  parseSync,
  parseWatermark,
  presenterItem,
  stateFile,
  tagline,
  TAGLINES,
  watermark,
  withBanner,
} from "../../showcase/real";

// a config dir of these tests' own (never the real aleph.toml): no watermark
// until a test writes one
const CONFIG = mkdtempSync(join(tmpdir(), "aleph-showcase-config-"));
process.env.XDG_CONFIG_HOME = CONFIG;
delete process.env.ALEPH_CONFIG;

/** A state dir of its own, with `showcase.json` holding `body` (none: no file). */
function stateWith(body?: string): string {
  const x = mkdtempSync(join(tmpdir(), "aleph-showcase-"));
  process.env.XDG_STATE_HOME = x;
  mkdirSync(join(x, "aleph"), { recursive: true });
  if (body !== undefined) writeFileSync(join(x, "aleph", "showcase.json"), body);
  return x;
}

// each test reads at its own clock, far apart, so the half-second look-again never hides a change
let clock = 1_000_000;
const later = () => (clock += 10_000);

test("the switch is the state dir's showcase.json", () => {
  assert.equal(stateFile({ XDG_STATE_HOME: "/x/state" }, "/home/u"), "/x/state/aleph/showcase.json");
  assert.equal(stateFile({}, "/home/u"), "/home/u/.local/state/aleph/showcase.json");
  assert.equal(stateFile({ XDG_STATE_HOME: "  " }, "/home/u"), "/home/u/.local/state/aleph/showcase.json");
});

test("on, demo and off read as masked, demo and real; anything else is real", () => {
  assert.equal(parseMode('{"on":true,"demo":false,"salt":"ab","epoch":3}'), "masked");
  assert.equal(parseMode('{"on":true,"demo":true}'), "demo");
  assert.equal(parseMode('{"on":false,"demo":true}'), "real");
  assert.equal(parseMode('{"on":"yes"}'), "real");
  assert.equal(parseMode("null"), "real");
  assert.equal(parseMode("[1,2]"), "real");
  assert.equal(parseMode("{not json"), "real");
  assert.equal(parseMode(""), "real");
  for (const [body, want] of [
    [undefined, "real"],
    ['{"on":true}', "masked"],
    ['{"on":true,"demo":true}', "demo"],
    ['{"on":false}', "real"],
    ["garbage", "real"],
  ] as const) {
    const x = stateWith(body);
    assert.equal(mode(later()), want, `${body}`);
    rmSync(x, { recursive: true, force: true });
  }
});

test("the file is read again only when its mtime moves (and looked at twice a second at most)", () => {
  const x = stateWith('{"on":true}');
  const f = join(x, "aleph", "showcase.json");
  const t = new Date("2026-10-20T10:00:00Z");
  utimesSync(f, t, t);
  assert.equal(mode(later()), "masked");
  // new contents, the same mtime: the answer kept
  writeFileSync(f, '{"on":true,"demo":true}');
  utimesSync(f, t, t);
  assert.equal(mode(later()), "masked");
  // the mtime moves: read again, but not within half a second of the last look
  const t2 = new Date("2026-10-20T10:00:05Z");
  utimesSync(f, t2, t2);
  const now = later();
  assert.equal(mode(now), "demo");
  writeFileSync(f, '{"on":false}');
  const t3 = new Date("2026-10-20T10:00:09Z");
  utimesSync(f, t3, t3);
  assert.equal(mode(now + 100), "demo", "looked at a moment ago");
  assert.equal(mode(now + 600), "real");
  // gone: real
  rmSync(f);
  assert.equal(mode(later()), "real");
  rmSync(x, { recursive: true, force: true });
});

// (navTitle, withBanner and menu read the real clock: a look at the tests'
// clock in between always makes the next one look again)
test("titles get a ✦ while it's on", () => {
  const x = stateWith('{"on":true}');
  assert.equal(navTitle("Machines"), "✦ Machines");
  writeFileSync(join(x, "aleph", "showcase.json"), '{"on":false}');
  const t = new Date(Date.now() + 60_000);
  utimesSync(join(x, "aleph", "showcase.json"), t, t);
  assert.equal(mode(later()), "real");
  assert.equal(navTitle("Machines"), "Machines");
  rmSync(x, { recursive: true, force: true });
});

test("the banner: an SVG card with the mark, the chip and the tagline of these 8 seconds", () => {
  const x = stateWith('{"on":true}');
  const now = later();
  const md = banner(now);
  assert.ok(md, "a banner while masked");
  const m = /^!\[([^\]]*)\]\(data:image\/svg\+xml;base64,([A-Za-z0-9+/=]+)\)$/.exec(md!);
  assert.ok(m, md!);
  const svg = Buffer.from(m![2], "base64").toString("utf8");
  assert.match(svg, /^<svg xmlns="http:\/\/www.w3.org\/2000\/svg"/);
  assert.match(svg, /✦ showcase/);
  assert.match(svg, /#E2B451/);
  assert.match(svg, /<path fill="#E2B451" d="M 19 7/, "the ℵ mark");
  assert.ok(m![1].includes(tagline(now)), "the alt text says the tagline");
  // the same 8 seconds: the same card; the taglines take turns
  assert.equal(banner(now + 1), md);
  assert.equal(tagline(0), TAGLINES[0]);
  assert.equal(tagline(8000), TAGLINES[1]);
  assert.equal(tagline(8000 * TAGLINES.length), TAGLINES[0]);
  // demo: its own chip; text is escaped
  assert.match(card("demo", "Don't Panic"), /✦ demo/);
  assert.match(card("demo", "Don't Panic"), /Don&#39;t Panic/);
  // off: none, and details stay as they are
  writeFileSync(join(x, "aleph", "showcase.json"), '{"on":false}');
  const t = new Date(Date.now() + 120_000);
  utimesSync(join(x, "aleph", "showcase.json"), t, t);
  assert.equal(banner(later()), null);
  rmSync(x, { recursive: true, force: true });
});

test("withBanner puts it on top of a detail, and leaves an empty one empty", () => {
  const x = stateWith('{"on":true,"demo":true}');
  const md = withBanner("## b31\n\nbody");
  assert.ok(md.startsWith("![Aleph demo: "), md.slice(0, 40));
  assert.ok(md.endsWith("\n\n## b31\n\nbody"));
  assert.equal(withBanner(""), "");
  rmSync(x, { recursive: true, force: true });
});

test("the watermark is read from aleph.toml's [showcase] table, as written", () => {
  assert.equal(configFile({ XDG_CONFIG_HOME: "/x/cfg" }, "/home/u"), "/x/cfg/aleph/aleph.toml");
  assert.equal(configFile({}, "/home/u"), "/home/u/.config/aleph/aleph.toml");
  assert.equal(configFile({ ALEPH_CONFIG: "/x/flat" }, "/home/u"), "/x/flat");
  const toml = [
    'theme = "dark"',
    "[showcase]",
    'flair = "all"',
    'watermark = "alex \\u00b7 \\"example\\" studio" # mine',
    "watermark_on = 'web,raycast'",
    "[showcase.more]",
    'watermark = "not this one"',
  ].join("\n");
  assert.deepEqual(parseWatermark(toml), { text: 'alex · "example" studio', on: "web,raycast" });
  // dotted at the top, the old key=value file, nothing at all
  assert.deepEqual(parseWatermark('showcase.watermark = "dotted"'), { text: "dotted", on: "tui,raycast,web" });
  assert.equal(parseWatermark("showcase.watermark=flat line\n").text, "flat line");
  assert.equal(parseWatermark('[[presets]]\nwatermark = "no"').text, "");
  assert.equal(parseWatermark("").text, "");
  // control characters go, and it's cut at 60
  assert.equal(parseWatermark(`showcase.watermark = "a\\tb${"x".repeat(80)}"`).text.length, 60);
});

test("the watermark rides the card and the titles while it's shown in raycast", () => {
  const x = stateWith('{"on":true}');
  const dir = join(CONFIG, "aleph");
  mkdirSync(dir, { recursive: true });
  const f = join(dir, "aleph.toml");
  writeFileSync(f, '[showcase]\nwatermark = "alex <dev>"\n');
  const t = new Date(Date.now() + 200_000);
  utimesSync(f, t, t);
  assert.equal(watermark("raycast", later()), "alex <dev>");
  assert.equal(navTitle("Usage"), "✦ Usage · alex <dev>");
  const svg = card("masked", "Make it so", 0, "alex <dev>");
  assert.match(svg, /text-anchor="end">alex &lt;dev&gt;<\/text>/);
  // a long one is cut to the room it has
  assert.match(card("demo", "x", 0, "w".repeat(80)), /w…<\/text>/);
  // not for raycast: none
  writeFileSync(f, '[showcase]\nwatermark = "alex"\nwatermark_on = "tui,web"\n');
  const t2 = new Date(Date.now() + 300_000);
  utimesSync(f, t2, t2);
  assert.equal(watermark("raycast", later()), null);
  assert.equal(watermark("web", later()), "alex");
  rmSync(f);
  assert.equal(watermark("raycast", later()), null);
  rmSync(x, { recursive: true, force: true });
});

test("the menu bar's switch names the mode and runs aleph showcase", () => {
  const x = stateWith('{"on":true,"demo":true}');
  const m = menu()!;
  assert.equal(m.title, "Showcase: demo");
  assert.deepEqual(
    m.items.map((i) => i.args.join(" ")),
    ["showcase on", "showcase on --demo", "showcase off", "showcase panic on"],
  );
  assert.deepEqual(
    m.items.map((i) => i.current),
    [false, true, false, false],
  );
  rmSync(x, { recursive: true, force: true });
});

test("presenter sync: the setting, the daemon's answer, the row to select", () => {
  assert.equal(parseSync('[showcase]\nsync = "on"\n'), true);
  assert.equal(parseSync('showcase.sync = "off"\n'), false);
  assert.equal(parseSync("showcase.sync=on\n"), true);
  assert.equal(parseSync('[other]\nsync = "on"\n'), false);
  assert.equal(parseSync(""), false);
  // what presenter.get says (names come masked, like the lists' own)
  const p = parsePresenter({
    on: true,
    view: "focus",
    host: "europa",
    pane_id: "w1:p2",
    name: "whooshing-falcon",
    gen: 7,
  });
  assert.deepEqual(p, { on: true, view: "focus", host: "europa", pane_id: "w1:p2", name: "whooshing-falcon", gen: 7 });
  assert.equal(presenterItem(p, "sessions"), "europa:w1:p2");
  assert.equal(presenterItem(p, "machines"), "europa");
  assert.equal(followingTitle(p), "▶ following the presenter");
  // a machine alone: no session to select
  const m = parsePresenter({ on: true, view: "monitor", host: "magrathea" });
  assert.equal(presenterItem(m, "sessions"), undefined);
  assert.equal(presenterItem(m, "machines"), "magrathea");
  // nobody presents
  for (const v of [{ on: false }, null, "x", { view: "fleet" }]) assert.equal(parsePresenter(v), null);
  assert.equal(presenterItem(null, "machines"), undefined);
  assert.equal(followingTitle(null), "");
  // followed only while showcase is on and showcase.sync says so
  const x = stateWith('{"on":true}');
  const dir = join(CONFIG, "aleph");
  mkdirSync(dir, { recursive: true });
  const f = join(dir, "aleph.toml");
  writeFileSync(f, '[showcase]\nsync = "on"\n');
  const t = new Date(Date.now() + 400_000);
  utimesSync(f, t, t);
  assert.equal(following(later()), true);
  writeFileSync(join(x, "aleph", "showcase.json"), '{"on":false}');
  const t2 = new Date(Date.now() + 500_000);
  utimesSync(join(x, "aleph", "showcase.json"), t2, t2);
  assert.equal(following(later()), false);
  rmSync(f);
  rmSync(x, { recursive: true, force: true });
});
