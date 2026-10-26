// `npm test`: showcase mode's safety nets as Raycast follows them (the real
// module, and the stub's inert stand-ins): the pause read from showcase.json,
// clipboard and money read from aleph.toml, the pause's card, the menu's
// way back.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
  canCopy,
  configFile,
  menu,
  moneyText,
  parsePanic,
  parseSafety,
  pauseCard,
  paused,
  pausedMarkdown,
  safety,
} from "../../showcase/real";
import * as stub from "../../showcase/stub";

/** State and config dirs of their own: `showcase.json` and `aleph.toml` holding these. */
function world(state?: string, toml?: string): string {
  const x = mkdtempSync(join(tmpdir(), "aleph-safety-"));
  process.env.XDG_STATE_HOME = join(x, "state");
  process.env.XDG_CONFIG_HOME = join(x, "config");
  mkdirSync(join(x, "state", "aleph"), { recursive: true });
  mkdirSync(join(x, "config", "aleph"), { recursive: true });
  if (state !== undefined) writeFileSync(join(x, "state", "aleph", "showcase.json"), state);
  if (toml !== undefined) writeFileSync(join(x, "config", "aleph", "aleph.toml"), toml);
  return x;
}

let clock = 5_000_000;
const later = () => (clock += 10_000);

test("the pause is showcase.json's panic, whatever the mode", () => {
  assert.equal(parsePanic('{"on":true,"panic":true}'), true);
  assert.equal(parsePanic('{"on":false,"panic":true}'), true);
  assert.equal(parsePanic('{"on":true}'), false);
  assert.equal(parsePanic('{"panic":"yes"}'), false);
  assert.equal(parsePanic("garbage"), false);
  const x = world('{"on":false,"panic":true,"epoch":4}');
  assert.equal(paused(later()), true);
  rmSync(x, { recursive: true, force: true });
  const y = world('{"on":true}');
  assert.equal(paused(later()), false);
  rmSync(y, { recursive: true, force: true });
});

test("clipboard and money come from the [showcase] table (or dotted keys), else their defaults", () => {
  assert.equal(configFile({ XDG_CONFIG_HOME: "/x/c" }, "/home/u"), "/x/c/aleph/aleph.toml");
  assert.equal(configFile({}, "/home/u"), "/home/u/.config/aleph/aleph.toml");
  assert.deepEqual(parseSafety(""), { clipboard: "masked", money: "real" });
  assert.deepEqual(
    parseSafety('hosts = ["b31"]\n\n[showcase]\nflair = "all"\nclipboard = "off"   # no copies\nmoney = \'hidden\'\n'),
    { clipboard: "off", money: "hidden" },
  );
  assert.deepEqual(parseSafety('showcase.money = "scaled"\n[ui]\nmoney = "hidden"\n'), {
    clipboard: "masked",
    money: "scaled",
  });
  // a value that isn't one of the choices: the default
  assert.deepEqual(parseSafety('[showcase]\nclipboard = "maybe"\n'), { clipboard: "masked", money: "real" });
});

test("copies and money follow the settings only while showcase is on; the pause stops copies", () => {
  const toml = '[showcase]\nclipboard = "off"\nmoney = "hidden"\n';
  const on = world('{"on":true}', toml);
  later();
  assert.deepEqual(safety(later()), { clipboard: "off", money: "hidden" });
  assert.equal(canCopy(), false);
  assert.equal(moneyText(), "$—");
  rmSync(on, { recursive: true, force: true });
  const off = world('{"on":false}', toml);
  later();
  safety(later());
  assert.equal(canCopy(), true);
  assert.equal(moneyText(), null);
  rmSync(off, { recursive: true, force: true });
  const p = world('{"on":false,"panic":true}', "");
  later();
  safety(later());
  assert.equal(canCopy(), false, "nothing copied while paused");
  rmSync(p, { recursive: true, force: true });
});

test("the pause's card: a night sky, the ℵ and be right back; the menu's way back", () => {
  const svg = pauseCard(1);
  assert.match(svg, /^<svg xmlns="http:\/\/www.w3.org\/2000\/svg"/);
  assert.match(svg, /be right back/);
  assert.match(svg, /<path fill="#E2B451" d="M 19 7/, "the ℵ mark");
  assert.ok((svg.match(/<text /g) ?? []).length > 40, "stars");
  const md = pausedMarkdown();
  assert.match(md, /^!\[Aleph: be right back\]\(data:image\/svg\+xml;base64,[A-Za-z0-9+/=]+\)$/);
  const x = world('{"on":true,"panic":true}');
  later();
  paused(later());
  const back = menu()!.items.find((i) => i.args.includes("panic"))!;
  assert.deepEqual(back.args, ["showcase", "panic", "off"]);
  assert.equal(back.current, true);
  rmSync(x, { recursive: true, force: true });
});

test("the stub's are inert", () => {
  const x = world('{"on":true,"panic":true}', '[showcase]\nclipboard = "off"\nmoney = "hidden"\n');
  assert.equal(stub.paused(), false);
  assert.equal(stub.pausedMarkdown(), "");
  assert.equal(stub.canCopy(), true);
  assert.equal(stub.moneyText(), null);
  rmSync(x, { recursive: true, force: true });
});
