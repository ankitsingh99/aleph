// `npm test`: the Local LLM Servers view (services-view.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { Service, ServiceModel, ServicesHost } from "./cli";
import { chartTheme } from "./charts";
import {
  healthTone,
  hostSummary,
  loadedText,
  loopbackOnly,
  modelRows,
  modelsLine,
  modelSub,
  reachUrl,
  serverBadges,
  serverMarkdown,
  serverTiles,
  sortedServers,
  tunnel,
} from "./services-view";

const dark = chartTheme(true);
const G = 1024 ** 3;
const PNG = "data:image/png;base64,iVBORw0KGgo=";
const logo = (kind: string, id: string | null | undefined) => (id ? `${PNG}#${kind}/${id}` : undefined);
const svgsOf = (md: string) =>
  [...md.matchAll(/base64,([^)]+)\)/g)].map((m) => Buffer.from(m[1], "base64").toString("utf8"));

const model = (name: string, over: Partial<ServiceModel> = {}): ServiceModel => ({
  name,
  loaded: false,
  kind: null,
  params: null,
  quant: null,
  format: null,
  ctx: null,
  size: null,
  vram: null,
  mem: null,
  gpu_share: null,
  ...over,
});

const server = (id: string, over: Partial<Service> = {}): Service => ({
  id,
  name: id === "ollama" ? "Ollama" : id === "vllm" ? "vLLM" : id,
  kind: "llm-server",
  glyph: "▣",
  color: "#ececec",
  homepage: null,
  version: null,
  health: "up",
  health_label: "up",
  health_detail: "",
  url: "http://127.0.0.1:11434",
  openai: true,
  openai_base: "http://127.0.0.1:11434/v1",
  auth_required: false,
  addr: "127.0.0.1",
  bind: "127.0.0.1",
  port: 11434,
  ports: [11434],
  pid: 1200,
  process: "ollama",
  found_by: ["process", "http"],
  models: [],
  model_count: 0,
  loaded_count: 0,
  vram: null,
  kv_cache: null,
  requests_running: null,
  requests_waiting: null,
  ...over,
});

const ollama = () =>
  server("ollama", {
    version: "0.5.7",
    models: [
      model("qwen2.5-coder:7b", { params: "7.6B", quant: "Q4_K_M", size: 4.4 * G }),
      model("llama3.1:8b", { loaded: true, params: "8.0B", quant: "Q4_K_M", ctx: 8192, vram: 6.2 * G }),
      model("nomic-embed-text", { kind: "embeddings", params: "137M", size: 0.26 * G }),
    ],
    model_count: 3,
    loaded_count: 1,
    vram: 6.2 * G,
  });

const vllm = () =>
  server("vllm", {
    version: "0.6.3.post1",
    bind: "0.0.0.0",
    port: 8000,
    ports: [8000, 43117],
    url: "http://127.0.0.1:8000",
    openai_base: "http://127.0.0.1:8000/v1",
    models: [model("meta-llama/Llama-3.1-8B-Instruct", { loaded: true, ctx: 32768, vram: 38 * G })],
    model_count: 1,
    loaded_count: 1,
    vram: 38 * G,
    kv_cache: 0.341,
    requests_running: 2,
    requests_waiting: 1,
  });

const host = (services: Service[], over: Partial<ServicesHost> = {}): ServicesHost => ({
  alias: "b31",
  color: "#89b4fa",
  reachable: true,
  error: null,
  services,
  ...over,
});

test("health, reach, models in words", () => {
  assert.deepEqual(["up", "loading", "not_listening", "error", "down", "unknown"].map(healthTone), [
    "good",
    "warn",
    "warn",
    "bad",
    "bad",
    "muted",
  ]);
  assert.equal(tunnel("b31", ollama()), "ssh -N -L 11434:127.0.0.1:11434 b31");
  assert.equal(tunnel("local", ollama()), undefined);
  assert.ok(loopbackOnly(ollama()) && !loopbackOnly(vllm()));
  assert.equal(modelsLine(ollama()), "llama3.1:8b +2", "the loaded one first");
  assert.equal(modelsLine(server("litellm", { kind: "gateway" })), "");
  assert.equal(modelsLine(server("x")), "no models");
  assert.equal(loadedText(ollama()), "1/3");
  // a server that doesn't say what's loaded: just how many
  assert.equal(
    loadedText(server("jan", { models: [model("a", { loaded: null as unknown as boolean })], model_count: 1 })),
    "1",
  );
  assert.equal(modelSub(ollama().models[1]), "8.0B · Q4_K_M · 8K ctx");
  assert.equal(modelSub(ollama().models[2]), "137M · embeddings");
});

test("the URL from here", () => {
  const ow = server("openwebui", { kind: "ui", bind: "0.0.0.0", port: 3000, url: "http://127.0.0.1:3000" });
  assert.equal(reachUrl({ alias: "gpu2" }, ow, "tester@gpu2.lan:2222"), "http://gpu2.lan:3000", "not the ssh port");
  assert.equal(reachUrl({ alias: "b31" }, ow, "selene@172.16.200.16"), "http://172.16.200.16:3000");
  assert.equal(reachUrl({ alias: "b31" }, ow, "fe80::1"), "http://[fe80::1]:3000");
  assert.equal(reachUrl({ alias: "b31" }, ow, "me@[fe80::1]:22"), "http://[fe80::1]:3000");
  assert.equal(reachUrl({ alias: "b31" }, ollama(), "selene@172.16.200.16"), undefined, "loopback only");
  assert.equal(reachUrl({ alias: "b31" }, ow, undefined), undefined);
  assert.equal(reachUrl({ alias: "local" }, ollama(), "this computer"), "http://127.0.0.1:11434");
});

test("rows and sections", () => {
  const loading = server("llamacpp", { health: "loading", health_label: "loading" });
  const sorted = sortedServers([ollama(), loading, vllm()]);
  assert.deepEqual(
    sorted.map((s) => s.id),
    ["ollama", "vllm", "llamacpp"],
    "up first, then by models loaded and name",
  );
  assert.deepEqual(
    serverBadges(ollama()).map((b) => [b.kind, b.text]),
    [
      ["models", "1/3"],
      ["health", "up"],
    ],
  );
  assert.deepEqual(
    serverBadges(server("litellm", { auth_required: true }), true).map((b) => b.kind),
    ["auth", "port", "health"],
  );
  assert.equal(serverBadges(loading).at(-1)?.tone, "warn");
  assert.equal(hostSummary(host([ollama(), vllm()])), "2 up · 2 loaded · 44G VRAM");
  assert.equal(hostSummary(host([ollama(), loading])), "1 of 2 up · 1 loaded · 6.2G VRAM");
  assert.equal(hostSummary(host([])), "none found");
  assert.equal(
    hostSummary(host([], { reachable: false, error: "ssh: connect to host x port 22: Connection refused" })).length > 0,
    true,
  );
});

test("the server's detail", () => {
  const rows = modelRows(ollama(), dark);
  assert.deepEqual(
    rows.map((r) => [r.name, r.value, r.valueSub]),
    [
      ["llama3.1:8b", "6.2G", "VRAM"],
      ["nomic-embed-text", "266M", "on disk"],
      ["qwen2.5-coder:7b", "4.4G", "on disk"],
    ],
  );
  assert.equal(rows[0].mark?.color, dark.state.done);
  assert.ok(rows[1].muted && rows[1].mark?.kind === "ring");
  assert.equal(serverTiles(ollama()).length, 0, "its models' rows say it all");
  assert.deepEqual(
    serverTiles(vllm()).map((x) => [x.label, x.value]),
    [
      ["VRAM", "38G"],
      ["KV cache", "34%"],
      ["Requests", "2"],
    ],
  );
  const md = serverMarkdown({ h: { alias: "b31" }, s: vllm(), t: dark, logo, address: "selene@10.0.0.49" });
  const svgs = svgsOf(md);
  assert.equal(svgs.length, 4, "card, figures, models, endpoint");
  assert.ok(
    svgs[0].includes("#service/vllm") && svgs[0].includes(">0.6.3.post1<") && svgs[0].includes("b31 · server · :8000"),
  );
  assert.ok(svgs[1].includes(">34%<") && svgs[1].includes("1 waiting"));
  assert.ok(svgs[2].includes("Llama-3.1-8B-Instruct") && svgs[2].includes("1 of 1 loaded"));
  assert.ok(svgs[3].includes(">http://10.0.0.49:8000<") && svgs[3].includes("every interface"));
  assert.ok(!md.includes("|"), "no Markdown tables");
  // not up: a notice says why; loopback on another machine: the tunnel
  const down = serverMarkdown({
    h: { alias: "gpu2" },
    s: server("llamacpp", { health: "error", health_label: "error", health_detail: "HTTP 500 from /health" }),
    t: dark,
    logo,
  });
  const d = svgsOf(down);
  assert.ok(d[1].includes("HTTP 500 from /health") && d[1].includes(dark.state.down));
  assert.ok(d.at(-1)!.includes("gpu2 only · ⌘T tunnels"));
  const many = serverMarkdown({
    h: { alias: "b31" },
    s: server("x", { models: Array.from({ length: 12 }, (_, i) => model(`m${i}`)), model_count: 12 }),
    t: dark,
    logo,
  });
  assert.ok(svgsOf(many)[1].includes("+4 more"));
});
