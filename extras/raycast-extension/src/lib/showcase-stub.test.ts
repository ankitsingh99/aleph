// `npm test`: showcase mode's stub (showcase/stub.ts, what every build
// copies to src/lib/showcase.ts) does nothing, even with the switch on.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import * as real from "../../showcase/real";
import * as stub from "../../showcase/stub";
import { banner, menu, mode, navTitle, on, withBanner } from "../../showcase/stub";

test("the real module has everything the stub has (callers compile against either)", () => {
  for (const [k, v] of Object.entries(stub)) {
    assert.equal(typeof (real as Record<string, unknown>)[k], typeof v, k);
  }
});

test("the stub is inert, whatever showcase.json says", () => {
  const x = mkdtempSync(join(tmpdir(), "aleph-showcase-stub-"));
  const before = process.env.XDG_STATE_HOME;
  process.env.XDG_STATE_HOME = x;
  mkdirSync(join(x, "aleph"), { recursive: true });
  writeFileSync(join(x, "aleph", "showcase.json"), '{"on":true,"demo":true}');
  try {
    assert.equal(mode(), "real");
    assert.equal(on(), false);
    assert.equal(banner(), null);
    assert.equal(withBanner("## b31"), "## b31");
    assert.equal(navTitle("Machines"), "Machines");
    assert.equal(menu(), null);
  } finally {
    process.env.XDG_STATE_HOME = before;
    rmSync(x, { recursive: true, force: true });
  }
});
