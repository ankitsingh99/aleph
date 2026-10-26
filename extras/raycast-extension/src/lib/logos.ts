// Logos: each agent, OS, GPU vendor and LLM server shown with its own logo.
//
// 1. The bundled PNGs, when this build has them: tools/logogen writes them to
//    the repo's assets/logos/png, and `npm run build` / `npm run dev` copy
//    them into this extension's assets/logos (scripts/sync-logos.sh).
// 2. Otherwise the brand's favicon through Raycast's favicon service.
// 3. Offline, or for something without a known site, a dot in the brand
//    colour, so the list never shows a broken image.
//
// A logo whose brand colour wouldn't show on one of Raycast's themes (Apple's
// black, Cursor's white, …) has a `-dark` / `-light` variant (logogen draws
// it lighter or darker); list icons pick per theme, and the SVG cards embed
// the one for the theme in use (`logoUri`).
import { Cache, Color, environment, Icon, Image, List } from "@raycast/api";
import { getFavicon } from "@raycast/utils";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { gpuId, osId } from "./logo-ids";
import { brand } from "./ui";

// ---------------------------------------------------------------- bundled PNGs

export type Kind = "agent" | "os" | "gpu" | "service";

/** Folders a kind's PNGs may sit in (under assets/logos), then the folder itself. */
const KIND_DIRS: Record<Kind, string[]> = {
  agent: ["agents", "agent"],
  os: ["os"],
  gpu: ["gpu", "gpus"],
  service: ["services", "servers", "service", "server"],
};
const SIZES = ["@128", "@64", ""];
const found = new Map<string, string | null>();

function findAsset(kind: Kind, name: string, sizes = SIZES): string | null {
  for (const dir of [...KIND_DIRS[kind].map((k) => `logos/${k}`), "logos"]) {
    for (const size of sizes) {
      const rel = `${dir}/${name}${size}.png`;
      if (existsSync(join(environment.assetsPath, rel))) return rel;
    }
  }
  return null;
}

function lookup(kind: Kind, name: string, sizes?: string[]): string | undefined {
  const key = `${kind}/${name}/${sizes?.join() ?? ""}`;
  if (!found.has(key)) {
    let hit: string | null = null;
    try {
      hit = findAsset(kind, name, sizes);
    } catch {
      hit = null;
    }
    found.set(key, hit);
  }
  return found.get(key) ?? undefined;
}

/** The bundled PNG for `id` (an asset path), when this build has one. Looked up once per id. */
export function bundledLogo(kind: Kind, id: string | null | undefined): string | undefined {
  return id ? lookup(kind, id) : undefined;
}

/** The bundled PNG for `id` as a list icon: its `-dark` / `-light` variant on the theme that needs one. */
export function themedLogo(kind: Kind, id: string | null | undefined): Image.ImageLike | undefined {
  const base = bundledLogo(kind, id);
  if (!base || !id) return undefined;
  const dark = lookup(kind, `${id}-dark`, ["@64"]);
  const light = lookup(kind, `${id}-light`, ["@64"]);
  return dark || light ? { source: { dark: dark ?? base, light: light ?? base } } : { source: base };
}

const uris = new Map<string, string | null>();

/**
 * The bundled PNG for `id` as a data URI, for an SVG card drawn for the
 * theme in use (`dark`): the variant for that theme when there is one.
 */
export function logoUri(kind: Kind, id: string | null | undefined, dark: boolean): string | undefined {
  if (!id) return undefined;
  const key = `${kind}/${id}/${dark}`;
  if (!uris.has(key)) {
    let uri: string | null = null;
    try {
      const rel = lookup(kind, `${id}-${dark ? "dark" : "light"}`, ["@64"]) ?? lookup(kind, id, ["@64", "@128", ""]);
      if (rel) uri = `data:image/png;base64,${readFileSync(join(environment.assetsPath, rel)).toString("base64")}`;
    } catch {
      uri = null;
    }
    uris.set(key, uri);
  }
  return uris.get(key) ?? undefined;
}

// ---------------------------------------------------------------- agents

/** The site whose logo stands for each agent (Aleph's agent registry ids). */
const AGENT_SITES: Record<string, string> = {
  claude: "https://claude.ai",
  codex: "https://openai.com",
  opencode: "https://opencode.ai",
  gemini: "https://gemini.google.com",
  agy: "https://antigravity.google",
  cursor: "https://cursor.com",
  devin: "https://devin.ai",
  copilot: "https://github.com",
  cline: "https://cline.bot",
  mastracode: "https://mastra.ai",
  kimi: "https://www.kimi.com",
  kiro: "https://kiro.dev",
  droid: "https://factory.ai",
  amp: "https://ampcode.com",
  grok: "https://grok.com",
  hermes: "https://nousresearch.com",
  kilo: "https://kilocode.ai",
  qodercli: "https://qoder.com",
  qwen: "https://chat.qwen.ai",
  letta: "https://www.letta.com",
  crush: "https://charm.sh",
  commandcode: "https://commandcode.ai",
};

const dot = (color: string): Image.ImageLike => ({ source: Icon.CircleFilled, tintColor: color });

function favicon(url: string, fallback: Image.Fallback = Icon.Circle): Image.ImageLike {
  return getFavicon(url, { size: 64, fallback, mask: Image.Mask.RoundedRectangle });
}

/** The bundled PNG id that stands for an agent (its own id, or its brand's). */
export function agentLogoId(agent: string | null | undefined): string | undefined {
  const id = agent ?? "";
  if (bundledLogo("agent", id)) return id;
  const b = brand(id);
  return bundledLogo("agent", b.id) ? b.id : undefined;
}

/** An agent's logo (Claude, OpenAI, Gemini, …): bundled, else its favicon, else a dot in its brand colour. */
export function agentLogo(agent: string | null | undefined): Image.ImageLike {
  const id = agent ?? "";
  // a plain terminal (no agent): the terminal symbol
  if (id === "shell") return { source: Icon.Terminal, tintColor: { light: "#646464", dark: "#a0a0a0" } };
  const b = brand(id);
  const png = themedLogo("agent", agentLogoId(id));
  if (png) return png;
  const site = AGENT_SITES[id] ?? AGENT_SITES[b.id];
  return site ? favicon(site) : dot(b.color);
}

// ---------------------------------------------------------------- OS and GPU

/** The OS's logo (Ubuntu, Debian, macOS, …) from any of `os`, `os_name`, `platform`. */
export function osLogo(
  ...names: (string | null | undefined)[]
): { icon: Image.ImageLike; name: string; id: string } | undefined {
  const hit = osId(...names);
  if (!hit) return undefined;
  return { icon: themedLogo("os", hit.id) ?? favicon(hit.site, Icon.Monitor), name: hit.name, id: hit.id };
}

/** The GPU vendor's logo (NVIDIA, AMD, Intel, Apple) from a GPU name or vendor. */
export function gpuLogo(
  ...names: (string | null | undefined)[]
): { icon: Image.ImageLike; name: string; id: string } | undefined {
  const hit = gpuId(...names);
  if (!hit) return undefined;
  return { icon: themedLogo("gpu", hit.id) ?? favicon(hit.site, Icon.ComputerChip), name: hit.name, id: hit.id };
}

/** An LLM server's logo (Ollama, vLLM, LM Studio, …): bundled, else its homepage's favicon, else a dot. */
export function serviceLogo(s: { id?: string; homepage?: string | null; color: string }): Image.ImageLike {
  const png = themedLogo("service", s.id);
  if (png) return png;
  return s.homepage ? favicon(s.homepage, Icon.ComputerChip) : dot(s.color);
}

/** Logos for an SVG card on the theme in use: `(kind, id) → data URI`, undefined when this build has none. */
export const cardLogos =
  (dark: boolean) =>
  (kind: Kind, id: string | null | undefined): string | undefined =>
    kind === "agent" ? logoUri("agent", agentLogoId(id), dark) : logoUri(kind, id, dark);

// ---------------------------------------------------------------- tags

/** An agent as an accessory: its logo and a tag in its brand colour. */
export function agentTag(agent: string | null | undefined): List.Item.Accessory | undefined {
  if (!agent) return undefined;
  const b = brand(agent);
  return { icon: agentLogo(agent), tag: { value: agent, color: b.color }, tooltip: b.name };
}

// Each machine's OS, remembered from `agents` / `stats` answers so every
// command can show its logo without asking the machine again.
const machines = new Cache({ namespace: "machines" });

export function rememberMachines(list: { alias: string; os?: string | null; platform?: string | null }[]) {
  for (const m of list) {
    const os = [m.os, m.platform].filter(Boolean).join(" ");
    if (os && machines.get(m.alias) !== os) machines.set(m.alias, os);
  }
}

/** A machine's OS as some command last saw it (`Ubuntu 24.04.3 LTS linux`). */
export const machineOsText = (alias: string): string | undefined => machines.get(alias);

/** A machine's OS logo: from its OS text when the caller has it (the fleet's), else what some command saw. */
export function machineLogo(
  alias: string,
  osText?: string | null,
): { icon: Image.ImageLike; name: string; id: string } | undefined {
  const os = osText || machines.get(alias);
  return os ? osLogo(os) : undefined;
}

/** A machine as an accessory: a tag in its colour, with its OS logo when known. */
export function hostTag(alias: string, color: Color.ColorLike | undefined, tooltip?: string): List.Item.Accessory {
  const os = machineLogo(alias);
  return {
    tag: { value: alias, color: color || Color.SecondaryText },
    ...(os ? { icon: os.icon } : {}),
    tooltip: [tooltip, os?.name].filter(Boolean).join(" · ") || undefined,
  };
}
