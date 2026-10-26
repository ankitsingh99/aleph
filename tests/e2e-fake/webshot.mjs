// CDP driver for webshot.sh: device emulation, a token in localStorage, one screenshot per shot.
// usage: node webshot.mjs <port> <outdir> <base> <token> name=WxH[m][l]:hash ...
import { writeFileSync } from 'node:fs';
const [port, out, base, token, ...shots] = process.argv.slice(2);
const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = list.find(t => t.type === 'page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise(r => ws.addEventListener('open', r, { once: true }));
let id = 0; const wait = new Map();
ws.addEventListener('message', ev => { const m = JSON.parse(ev.data); if (m.id && wait.has(m.id)) { wait.get(m.id)(m); wait.delete(m.id); } });
const send = (method, params = {}) => new Promise(r => { const i = ++id; wait.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const sleep = ms => new Promise(r => setTimeout(r, ms));
await send('Page.enable'); await send('Runtime.enable');
await send('Page.navigate', { url: base + '/#/pair' }); await sleep(1500);
if (token) { await send('Runtime.evaluate', { expression: `localStorage.setItem('aleph.token', ${JSON.stringify(token)})` }); }
for (const s of shots) {
  const [name, rest] = s.split('=');
  const i = rest.indexOf(':'); const size = rest.slice(0, i), hash = rest.slice(i + 1);
  const mobile = size.includes('m'), light = size.includes('l');
  const [w, h] = size.replace(/[ml]/g, '').split('x').map(Number);
  await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: light ? 'light' : 'dark' }] });
  await send('Emulation.setDeviceMetricsOverride', { width: w, height: h, deviceScaleFactor: mobile ? 2 : 1, mobile });
  await send('Emulation.setTouchEmulationEnabled', { enabled: mobile });
  await send('Page.navigate', { url: 'about:blank' }); await sleep(300);
  await send('Page.navigate', { url: base + '/' + (hash || '') });
  await sleep(3500);
  const dims = await send('Runtime.evaluate', { expression: 'JSON.stringify([innerWidth, document.documentElement.scrollWidth])', returnByValue: true });
  const r = await send('Page.captureScreenshot', { format: 'png' });
  writeFileSync(`${out}/${name}.png`, Buffer.from(r.result.data, 'base64'));
  console.log(name, dims.result.result.value);
}
ws.close();
