// A QR code as SVG (for `aleph daemon pair`'s link). Dark modules on a white
// plate of their own with the 4-module quiet zone, whatever the Raycast
// theme: scanners want dark on light. No Raycast imports (plain node tests it).
import qrcode from "qrcode-generator";

/** The modules of `text` (error correction M: the link is short, a phone screen may glare). */
export function qrModules(text: string): boolean[][] {
  const q = qrcode(0, "M");
  q.addData(text);
  q.make();
  const n = q.getModuleCount();
  return Array.from({ length: n }, (_, r) => Array.from({ length: n }, (_, c) => q.isDark(r, c)));
}

/** `text` as a QR code, `size` px square. */
export function qrSvg(text: string, size = 220): string {
  const m = qrModules(text);
  const n = m.length + 8;
  const d = m.flatMap((row, r) => row.map((dark, c) => (dark ? `M${c + 4},${r + 4}h1v1h-1z` : ""))).join("");
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${n} ${n}" shape-rendering="crispEdges">` +
    `<rect width="${n}" height="${n}" rx="2" fill="#ffffff"/><path d="${d}" fill="#000000"/></svg>`
  );
}
