// Local LLM Servers, drawn: each server (Ollama, vLLM, LM Studio, …) on each
// machine (`services --json`) as a list row's badges and Markdown with SVG
// cards — a card with its logo, health and version, its figures (models
// loaded, VRAM, KV cache, requests), its models (loaded first) and where it
// listens. No Raycast imports: plain node tests it.
import type { Service, ServiceModel, ServicesHost } from "./cli";
import { ChartTheme, Tile, tiles } from "./charts";
import { itemRows, ItemRow, notice } from "./cards";
import { Fact, facts, hero, PANE, Part } from "./dash";
import { bytes, pct } from "./fmt";
import { explainError } from "./model";
import { img, LogoFn } from "./stats-view";

/** server · gateway · web UI */
export const KIND_WORD: Record<string, string> = { "llm-server": "server", gateway: "gateway", ui: "web UI" };
export const kindWord = (k: string) => KIND_WORD[k] ?? k;

export type Health = "good" | "warn" | "bad" | "muted";

/** up green · loading or not listening yellow · error or down red · unknown grey */
export function healthTone(h: string): Health {
  return h === "up"
    ? "good"
    : h === "loading" || h === "not_listening"
      ? "warn"
      : h === "error" || h === "down"
        ? "bad"
        : "muted";
}

export function toneColor(tone: Health, t: ChartTheme): string {
  return tone === "good" ? t.state.done : tone === "warn" ? t.state.warn : tone === "bad" ? t.state.down : t.other;
}

/** `ssh -N -L 11434:127.0.0.1:11434 b31`: reach a server that only listens on its machine. */
export function tunnel(host: string, s: Pick<Service, "port" | "addr">): string | undefined {
  if (!s.port || host === "local") return undefined;
  return `ssh -N -L ${s.port}:${s.addr || "127.0.0.1"}:${s.port} ${host}`;
}

/** Only this machine can reach it (it listens on loopback). */
export const loopbackOnly = (s: Pick<Service, "bind">) => !!s.bind && /^(127\.|::1$|localhost$)/.test(s.bind);

/** Its models, loaded first (then by name). */
export function sortedModels(s: Pick<Service, "models">): ServiceModel[] {
  const rank = (m: ServiceModel) => (m.loaded ? 0 : m.loaded == null ? 1 : 2);
  return [...s.models].sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
}

/** `llama3.1:8b +2`: the model that's loaded (else the first), and how many more. */
export function modelsLine(s: Pick<Service, "models" | "kind">): string {
  if (!s.models.length) return s.kind === "llm-server" ? "no models" : "";
  const first = sortedModels(s)[0].name;
  const rest = s.models.length - 1;
  return rest > 0 ? `${first} +${rest}` : first;
}

/** `1/3` loaded, or `3` when the server doesn't say what's loaded. */
export function loadedText(s: Pick<Service, "models" | "loaded_count" | "model_count">): string {
  const known = s.models.some((m) => m.loaded != null);
  const total = s.model_count || s.models.length;
  return known ? `${s.loaded_count}/${total}` : String(total);
}

/** The servers in the order they matter: up first, then the most models loaded, then by name. */
export function sortedServers(list: Service[]): Service[] {
  const rank = (s: Service) => ["good", "warn", "bad", "muted"].indexOf(healthTone(s.health));
  return [...list].sort((a, b) => rank(a) - rank(b) || b.loaded_count - a.loaded_count || a.name.localeCompare(b.name));
}

/** A machine's section line: `6 up · 9 loaded · 68G VRAM`, `2 of 3 up`, or why it couldn't be scanned. */
export function hostSummary(h: ServicesHost): string {
  if (!h.reachable) return explainError(h.error).head;
  const n = h.services.length;
  if (!n) return "none found";
  const up = h.services.filter((s) => s.health === "up").length;
  const loaded = h.services.reduce((x, s) => x + s.loaded_count, 0);
  const vram = h.services.reduce((x, s) => x + (s.vram ?? 0), 0);
  return [
    up === n ? `${n} up` : `${up} of ${n} up`,
    loaded ? `${loaded} loaded` : "",
    vram ? `${bytes(vram)} VRAM` : "",
    h.partial ? "partial" : "",
  ]
    .filter(Boolean)
    .join(" · ");
}

/** A server row's accessory, before it becomes Raycast's. */
export interface ServerBadge {
  kind: "auth" | "models" | "vram" | "port" | "health";
  text: string;
  tooltip: string;
  tone?: Health;
}

/** A server's row: needs a key, its models loaded, its health; with the detail hidden also its VRAM and port. */
export function serverBadges(s: Service, wide = false): ServerBadge[] {
  const out: ServerBadge[] = [];
  if (s.auth_required) out.push({ kind: "auth", text: "", tooltip: "needs an API key" });
  if (s.models.length || s.model_count)
    out.push({
      kind: "models",
      text: loadedText(s),
      tooltip: s.models.some((m) => m.loaded != null)
        ? `${s.loaded_count} of ${s.model_count || s.models.length} models loaded`
        : `${s.model_count || s.models.length} models`,
    });
  if (wide && s.vram) out.push({ kind: "vram", text: bytes(s.vram), tooltip: "VRAM in use" });
  if (wide && s.port) out.push({ kind: "port", text: `:${s.port}`, tooltip: s.url ?? "" });
  const label = s.health_label || s.health;
  out.push({
    kind: "health",
    text: label,
    tooltip: [label, s.health_detail].filter(Boolean).join(" — "),
    tone: healthTone(s.health),
  });
  return out;
}

// ---------------------------------------------------------------- the detail

/** `8.0B · Q4_K_M · 8K ctx · gguf · embeddings` */
export function modelSub(m: ServiceModel): string {
  const ctx = m.ctx ? `${m.ctx >= 1024 ? `${Math.round(m.ctx / 1024)}K` : m.ctx} ctx` : "";
  return [m.params, m.quant, ctx, m.kind && m.kind !== "llm" ? m.kind : ""].filter(Boolean).join(" · ");
}

/** One row per model: loaded (a green dot) or not (a ring), what it is, its VRAM (or size on disk, muted). */
export function modelRows(s: Pick<Service, "models">, t: ChartTheme): ItemRow[] {
  return sortedModels(s).map((m) => {
    const vram = m.vram ?? (m.loaded ? m.mem : null);
    return {
      name: m.name,
      mark:
        m.loaded === true
          ? { kind: "dot", color: t.state.done }
          : { kind: "ring", color: m.loaded == null ? t.muted : t.other },
      muted: m.loaded === false,
      sub: modelSub(m) || undefined,
      value: vram ? bytes(vram) : m.size ? bytes(m.size) : undefined,
      valueColor: vram ? undefined : t.muted,
      valueSub: vram
        ? m.gpu_share != null && m.gpu_share < 0.99
          ? `${pct(m.gpu_share)} on GPU`
          : "VRAM"
        : m.size
          ? "on disk"
          : undefined,
    };
  });
}

/**
 * The live figures, as tiles, for a server that reports them (vLLM, SGLang,
 * Aphrodite …): KV cache and requests, with its VRAM. None for the others —
 * their models' rows already say what's loaded and how much VRAM it takes.
 */
export function serverTiles(s: Service): Tile[] {
  const out: Tile[] = [];
  if (s.kv_cache == null && s.requests_running == null && s.requests_waiting == null) return out;
  if (s.vram) out.push({ label: "VRAM", value: bytes(s.vram), sub: ["in use"] });
  if (s.kv_cache != null) out.push({ label: "KV cache", value: pct(s.kv_cache), sub: ["used"] });
  if (s.requests_running != null || s.requests_waiting != null)
    out.push({
      label: "Requests",
      value: String(s.requests_running ?? 0),
      sub: [`${s.requests_waiting ?? 0} waiting`],
    });
  return out;
}

/**
 * The URL to reach it from this computer: its own on `local`; on another
 * machine, at that machine's address when it listens on every interface
 * (`selene@172.16.200.16` → `http://172.16.200.16:3000`). None when only its machine can reach it.
 */
export function reachUrl(
  h: Pick<ServicesHost, "alias">,
  s: Pick<Service, "url" | "bind" | "port">,
  address?: string | null,
): string | undefined {
  if (h.alias === "local") return s.url ?? undefined;
  if (loopbackOnly(s) || !s.port || !address) return undefined;
  // `user@host`, `user@host:2222` (its ssh port), `[::1]:22`, a bare IPv6
  let host = address.split("@").pop() ?? "";
  if (host.startsWith("[")) host = host.slice(1, host.indexOf("]"));
  else if ((host.match(/:/g) ?? []).length === 1) host = host.split(":")[0];
  if (!host || /\s/.test(host)) return undefined;
  return `http://${host.includes(":") ? `[${host}]` : host}:${s.port}`;
}

/** Where it answers: from here, its URL, the OpenAI base, where it listens, its process. */
export function serverFacts(
  h: Pick<ServicesHost, "alias">,
  s: Service,
  t: ChartTheme,
  address?: string | null,
): Fact[] {
  const rows: Fact[] = [];
  const here = reachUrl(h, s, address);
  if (here && here !== s.url) rows.push({ label: "From here", parts: [{ text: here, strong: true }] });
  if (s.url) rows.push({ label: "URL", parts: [{ text: s.url }] });
  if (s.openai_base) rows.push({ label: "OpenAI", parts: [{ text: s.openai_base }] });
  if (s.bind || s.ports.length) {
    const only = loopbackOnly(s);
    const remote = h.alias !== "local";
    const parts: Part[] = [
      {
        text: [s.bind, s.ports.join(", ")].filter(Boolean).join(" : "),
        dot: only && remote ? t.state.warn : t.state.done,
      },
    ];
    if (only && remote) parts.push({ text: `${h.alias} only · ⌘T tunnels`, muted: true });
    else if (!only) parts.push({ text: "every interface", muted: true });
    rows.push({ label: "Listens", parts });
  }
  if (s.process)
    rows.push({ label: "Process", parts: [{ text: s.process, note: s.pid ? `pid ${s.pid}` : undefined }] });
  return rows;
}

/** A server's whole detail: the card, a notice when it isn't up, its figures, its models, where it answers. */
export function serverMarkdown(o: {
  h: Pick<ServicesHost, "alias">;
  s: Service;
  t: ChartTheme;
  logo: LogoFn;
  /** the machine's ssh address (`ls --json`), for the URL from here */
  address?: string | null;
  width?: number;
  /** models shown before `+n more` */
  maxModels?: number;
}): string {
  const { h, s, t } = o;
  const w = o.width ?? PANE;
  const tone = healthTone(s.health);
  const tags: Part[] = [{ text: s.health_label || s.health, dot: toneColor(tone, t) }];
  if (s.openai) tags.push({ text: "OpenAI API", muted: true });
  if (s.auth_required) tags.push({ text: "needs a key", muted: true });
  const out = [
    img(
      `${s.name} on ${h.alias}: ${s.health_label || s.health}`,
      hero(
        {
          title: s.name,
          color: s.color,
          logo: o.logo("service", s.id),
          letter: s.name,
          subtitle: [h.alias, kindWord(s.kind), s.port ? `:${s.port}` : ""].filter(Boolean).join(" · "),
          tags,
          aside: s.version ? { value: s.version.replace(/^v/i, ""), label: "version" } : undefined,
        },
        t,
        w,
      ),
    ),
  ];
  if (tone !== "good" && s.health_detail)
    out.push(
      img(
        "Health",
        notice(
          {
            state: tone === "bad" ? "down" : "warn",
            color: toneColor(tone, t),
            title: s.health_label || s.health,
            detail: s.health_detail,
          },
          t,
          w,
        ),
      ),
    );
  const figures = serverTiles(s);
  if (figures.length) out.push(img("KV cache and requests", tiles(figures, t, w)));
  const max = o.maxModels ?? 8;
  if (s.models.length) {
    const rows = modelRows(s, t);
    const known = s.models.some((m) => m.loaded != null);
    out.push(
      img(
        "Models",
        itemRows(rows.slice(0, max), t, w, {
          title: "Models",
          note: known ? `${s.loaded_count} of ${s.model_count || s.models.length} loaded` : `${s.models.length}`,
          more: rows.length > max ? `+${rows.length - max} more (⌘M copies any name)` : undefined,
        }),
      ),
    );
  }
  const where = serverFacts(h, s, t, o.address);
  if (where.length) out.push(img("Where it answers", facts(where, t, w, { title: "Endpoint" })));
  return out.join("\n\n");
}
