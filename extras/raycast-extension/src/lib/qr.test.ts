// `npm test`: the pairing QR code (qr.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import { qrModules, qrSvg } from "./qr";

test("a pairing link as a QR code", () => {
  const url = "https://mbp.tail1234.ts.net/#/pair/k3j9x2m7qa";
  const m = qrModules(url);
  assert.ok(m.length >= 25 && m.every((r) => r.length === m.length), "square, version 2 or up");
  // the three finder patterns: dark corners
  assert.ok(m[0][0] && m[0][m.length - 1] && m[m.length - 1][0]);
  const svg = qrSvg(url, 200);
  assert.match(svg, /^<svg [^>]*width="200"/);
  assert.ok(svg.includes('fill="#ffffff"'), "its own white plate, whatever the theme");
  assert.equal((svg.match(/h1v1h-1z/g) ?? []).length, m.flat().filter(Boolean).length);
});
