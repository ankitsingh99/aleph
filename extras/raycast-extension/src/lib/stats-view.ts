// Machine Stats, drawn: what one machine's detail shows (`aleph stats
// --json` with its fleet and agents answers, and the samples kept while the
// view is open) as Markdown with SVG cards — a hero card, gauges, the GPUs,
// a heat grid of the cores, live trend rows, the facts, the disks, the top
// processes. No Raycast imports: plain node tests it, and the previews
// render it from the fake world.
import type { AgentsHost, Gpu, Host, MachineStats } from "./cli";
import { cardUri, ChartTheme, fit } from "./charts";
import {
  coreGrid,
  Fact,
  facts,
  Gauge,
  gauges,
  hero,
  Meter,
  meters,
  PANE,
  Part,
  severity,
  tempSeverity,
  Trend,
  trends,
} from "./dash";
import { bytes, duration, pct, rate } from "./fmt";
import { gpuId, osId, shortGpu } from "./logo-ids";
import { explainError } from "./model";
import { seriesOf, spanText, StatSample } from "./samples";

export type LogoKind = "agent" | "os" | "gpu" | "service";
/** A logo for an SVG card: (kind, id) → data URI, undefined when there's none. */
export type LogoFn = (kind: LogoKind, id: string | null | undefined) => string | undefined;
export interface BrandLike {
  id: string;
  name: string;
  color: string;
}

export const img = (alt: string, svg: string) => `![${alt.replace(/[[\]]/g, "")}](${cardUri(svg)})`;

const frac = (used?: number | null, total?: number | null) =>
  used != null && total ? Math.min(1, Math.max(0, used / total)) : undefined;

const sum = (v: number[]) => v.reduce((s, x) => s + x, 0);

/** What one refresh says about a machine, for the trend rows. */
export function statSample(h: MachineStats, t: number): StatSample {
  const gpus = h.gpus ?? [];
  const io = h.disk_io ?? [];
  const temps = [h.cpu_temp, ...gpus.map((g) => g.temp)].filter((x): x is number => typeof x === "number");
  return {
    t,
    cpu: h.cpu ?? null,
    mem: frac(h.mem_used, h.mem_total) ?? null,
    gpu: gpus.length ? sum(gpus.map((g) => g.util ?? 0)) / gpus.length : null,
    vram: gpus.length
      ? (frac(sum(gpus.map((g) => g.mem_used ?? 0)), sum(gpus.map((g) => g.mem_total ?? 0))) ?? null)
      : null,
    load: h.load?.[0] ?? null,
    rx: h.net_rx ?? null,
    tx: h.net_tx ?? null,
    rd: io.length ? sum(io.map((d) => d.read ?? 0)) : null,
    wr: io.length ? sum(io.map((d) => d.write ?? 0)) : null,
    temp: temps.length ? Math.max(...temps) : null,
  };
}

/** `2× RTX 4090`, `RTX 4090 + A100`, or what `agents` saw (`Apple GPU`). */
export function gpuSummary(h: Pick<MachineStats, "gpus">, a?: Pick<AgentsHost, "gpu">): string {
  const names = (h.gpus ?? []).map((g) => shortGpu(g.name)).filter(Boolean);
  if (names.length) {
    const same = names.every((n) => n === names[0]);
    return names.length === 1 ? names[0] : same ? `${names.length}× ${names[0]}` : `${names.length} GPUs`;
  }
  return a?.gpu ?? "";
}

/** herdr's version there, and what's off about it. */
export function herdrText(host?: Host, a?: AgentsHost): { text: string; warn?: string } | undefined {
  const v = host?.herdr_version ?? a?.herdr?.version;
  if (!v) return undefined;
  if (host?.herdr_restart_needed) return { text: `herdr ${v}`, warn: "restart needed (aleph update)" };
  if (host?.herdr_drift) return { text: `herdr ${v}`, warn: "differs from this computer's (aleph update)" };
  return { text: `herdr ${v}` };
}

/** The GPU's vendor (for its logo): the stats' vendor field, else its name, else what `agents` saw. */
export function gpuVendor(h: Pick<MachineStats, "gpus">, a?: Pick<AgentsHost, "gpu" | "gpus">): string | undefined {
  return gpuId(...(h.gpus ?? []).flatMap((g) => [g.vendor, g.name]), a?.gpu, ...(a?.gpus ?? []))?.id;
}

/** The gauges: CPU and memory, then the GPUs (utilisation, VRAM) or swap and the fullest disk, and the hottest sensor. */
export function statGauges(h: MachineStats, t: ChartTheme): Gauge[] {
  const out: Gauge[] = [];
  const ncpu = h.ncpu ? `${h.ncpu} cores` : "";
  out.push({ label: "CPU", frac: h.cpu, sub: h.load ? `load ${h.load[0].toFixed(2)}` : ncpu });
  out.push({
    label: "Memory",
    frac: frac(h.mem_used, h.mem_total),
    sub: `${bytes(h.mem_used)} / ${bytes(h.mem_total)}`,
  });
  const gpus = h.gpus ?? [];
  if (gpus.length) {
    const util = sum(gpus.map((g) => g.util ?? 0)) / gpus.length;
    const used = sum(gpus.map((g) => g.mem_used ?? 0));
    const total = sum(gpus.map((g) => g.mem_total ?? 0));
    out.push({ label: gpus.length > 1 ? `GPU ×${gpus.length}` : "GPU", frac: util, sub: shortGpu(gpus[0].name) });
    out.push({ label: "VRAM", frac: frac(used, total), sub: `${bytes(used)} / ${bytes(total)}` });
  } else {
    if (h.swap_total)
      out.push({
        label: "Swap",
        frac: frac(h.swap_used, h.swap_total),
        sub: `${bytes(h.swap_used)} / ${bytes(h.swap_total)}`,
      });
    const disk = fullestDisk(h);
    if (disk)
      out.push({
        label: `Disk ${shortMount(disk.mount)}`,
        frac: frac(disk.used, disk.total),
        sub: `${bytes(disk.used)} / ${bytes(disk.total)}`,
      });
  }
  const hot = hottest(h);
  if (hot) {
    out.push({
      label: hot.label,
      // 20 °C at the start of the arc, 100 °C at its end
      frac: Math.min(1, Math.max(0, (hot.c - 20) / 80)),
      value: `${Math.round(hot.c)}°`,
      sub: hot.sub,
      color: tempSeverity(hot.c, t),
    });
  }
  return out.slice(0, 5);
}

/** The hottest reading: the GPUs' and the CPU's. */
function hottest(h: MachineStats): { label: string; c: number; sub: string } | undefined {
  const gpus = (h.gpus ?? []).filter((g) => typeof g.temp === "number");
  const g = gpus.reduce<Gpu | undefined>((b, x) => (!b || (x.temp ?? 0) > (b.temp ?? 0) ? x : b), undefined);
  const cpu = typeof h.cpu_temp === "number" ? h.cpu_temp : undefined;
  if (g && (cpu === undefined || (g.temp ?? 0) >= cpu))
    return {
      label: "Temp",
      c: g.temp ?? 0,
      sub: gpus.length > 1 ? `GPU ${g.index}` : cpu !== undefined ? `GPU · CPU ${Math.round(cpu)}°` : "GPU",
    };
  if (cpu !== undefined) return { label: "Temp", c: cpu, sub: "CPU" };
  return undefined;
}

/** The disk to show in a gauge: the fullest real one (not a tiny boot or snapshot volume). */
function fullestDisk(h: MachineStats) {
  const big = (h.disks ?? []).filter((d) => d.total > 4 * 1024 ** 3);
  return big.reduce<(typeof big)[number] | undefined>(
    (b, d) => (!b || (frac(d.used, d.total) ?? 0) > (frac(b.used, b.total) ?? 0) ? d : b),
    undefined,
  );
}

/** `/System/Volumes/Data` → `Data`, `/home` stays. */
const shortMount = (m: string) => (m.length > 8 ? (m.split("/").filter(Boolean).pop() ?? m) : m);

/** One meter row per GPU (utilisation · temperature · power) and its VRAM, when there's more than the gauges say. */
function gpuMeters(gpus: Gpu[], t: ChartTheme): Meter[] {
  const rows: Meter[] = [];
  for (const g of gpus) {
    const bits = [
      pct(g.util),
      typeof g.temp === "number" ? `${Math.round(g.temp)}°` : "",
      typeof g.power === "number" ? `${Math.round(g.power)} W` : "",
    ].filter(Boolean);
    rows.push({ label: gpus.length > 1 ? `GPU ${g.index}` : "Utilisation", frac: g.util, value: bits.join(" · ") });
    rows.push({
      label: gpus.length > 1 ? `  VRAM` : "VRAM",
      frac: frac(g.mem_used, g.mem_total),
      value: `${bytes(g.mem_used)} / ${bytes(g.mem_total)}`,
    });
    if (gpus.length === 1 && typeof g.power === "number" && g.power_limit)
      rows.push({
        label: "Power",
        frac: frac(g.power, g.power_limit),
        value: `${Math.round(g.power)} / ${Math.round(g.power_limit)} W`,
        color: severity(frac(g.power, g.power_limit), t),
      });
  }
  return rows;
}

/** The live trend rows: what's been sampled while the view was open (and before, for a quarter of an hour). */
export function statTrends(h: MachineStats, hist: StatSample[], t: ChartTheme): Trend[] {
  const c = fit(h.color || t.other, t);
  const rows: Trend[] = [{ label: "CPU", points: seriesOf(hist, "cpu"), max: 1, value: pct(h.cpu), color: c }];
  if (h.gpus?.length) {
    const s = statSample(h, 0);
    rows.push({ label: "GPU", points: seriesOf(hist, "gpu"), max: 1, value: pct(s.gpu), color: c });
  }
  rows.push({
    label: "Memory",
    points: seriesOf(hist, "mem"),
    max: 1,
    value: pct(frac(h.mem_used, h.mem_total)),
    color: c,
  });
  rows.push({
    label: "Load",
    points: seriesOf(hist, "load"),
    max: h.ncpu ?? undefined,
    value: h.load ? `${h.load[0].toFixed(2)}${h.ncpu ? ` / ${h.ncpu}` : ""}` : "–",
    color: c,
  });
  rows.push({ label: "Net in", points: seriesOf(hist, "rx"), value: rate(h.net_rx), color: c });
  rows.push({ label: "Net out", points: seriesOf(hist, "tx"), value: rate(h.net_tx), color: c });
  if (h.disk_io?.length) {
    const s = statSample(h, 0);
    rows.push({ label: "Disk read", points: seriesOf(hist, "rd"), value: rate(s.rd), color: c });
    rows.push({ label: "Disk write", points: seriesOf(hist, "wr"), value: rate(s.wr), color: c });
  }
  return rows;
}

/** The facts, aligned: system, CPU, memory, uptime, herdr, the agents installed and running. */
export function statFacts(
  h: MachineStats,
  o: { host?: Host; agents?: AgentsHost; logo: LogoFn; brand: (id: string) => BrandLike },
): Fact[] {
  const a = o.agents;
  const rows: Fact[] = [];
  const os = osId(h.os, a?.os_name, h.platform);
  const sys: Part[] = [{ text: h.os || a?.os_name || "unknown", logo: o.logo("os", os?.id), strong: true }];
  const kernel = [h.kernel, a?.os?.split(" ")[1]].filter(Boolean).join(" ");
  if (kernel) sys.push({ text: kernel, muted: true });
  rows.push({ label: "System", parts: sys });
  if (h.hostname) rows.push({ label: "Host", parts: [{ text: h.hostname, muted: true }] });
  if (h.cpu_model || h.ncpu)
    rows.push({
      label: "CPU",
      parts: [{ text: h.cpu_model || "CPU", note: h.ncpu ? `${h.ncpu} cores` : undefined }],
    });
  const gpu = gpuSummary(h, a);
  if (gpu || h.gpu_note) {
    rows.push({
      label: "GPU",
      parts: gpu
        ? [
            { text: gpu, logo: o.logo("gpu", gpuVendor(h, a)) },
            ...(h.gpus?.length ? [] : h.gpu_note ? [{ text: "no live numbers", muted: true }] : []),
          ]
        : [{ text: h.gpu_note ?? "", muted: true }],
    });
  }
  rows.push({
    label: "Memory",
    parts: [{ text: bytes(h.mem_total), note: h.swap_total ? `swap ${bytes(h.swap_total)}` : undefined }],
  });
  if (h.uptime_s != null) {
    const procs = [
      h.procs_total ? `${h.procs_total} processes` : "",
      h.users_logged ? `${h.users_logged} user${h.users_logged === 1 ? "" : "s"}` : "",
    ].filter(Boolean);
    rows.push({
      label: "Uptime",
      parts: [{ text: duration(h.uptime_s) }, ...procs.map((p) => ({ text: p, muted: true }))],
    });
  }
  const herdr = herdrText(o.host, a);
  if (herdr)
    rows.push({
      label: "herdr",
      parts: [
        { text: herdr.text.replace(/^herdr /, ""), warn: !!herdr.warn },
        ...(herdr.warn ? [{ text: herdr.warn, muted: true }] : []),
      ],
    });
  const installed = (a?.agents ?? []).filter((x) => x.installed);
  if (installed.length)
    rows.push({
      label: "Agents",
      parts: installed.map((x) => ({
        text: x.name,
        note: x.version ?? undefined,
        logo: o.logo("agent", x.id),
        letter: x.name,
        letterColor: x.color,
      })),
    });
  const running = h.agent_procs ?? [];
  if (running.length)
    rows.push({
      label: "Running",
      parts: running.map((p) => {
        const b = o.brand(p.agent);
        const logo = o.logo("agent", p.agent);
        return { text: `${b.name} ×${p.count}`, note: bytes(p.rss), logo, letter: b.name, letterColor: b.color };
      }),
    });
  else if (h.claude_procs)
    rows.push({
      label: "Running",
      parts: [{ text: `Claude Code ×${h.claude_procs}`, note: bytes(h.claude_rss), logo: o.logo("agent", "claude") }],
    });
  const today = h.sessions_today ?? [];
  if (today.length)
    rows.push({
      label: "Today",
      parts: today.map((x) => ({ text: `${o.brand(x.agent).name} ${x.count}`, logo: o.logo("agent", x.agent) })),
    });
  return rows;
}

const cell = (s: string) => s.replace(/\|/g, "\\|").replace(/[<>*_`#[\]]/g, (c) => `\\${c}`);

/** The whole detail of one machine. */
export function statsMarkdown(o: {
  h: MachineStats;
  host?: Host;
  agents?: AgentsHost;
  hist: StatSample[];
  t: ChartTheme;
  logo: LogoFn;
  brand: (id: string) => BrandLike;
  width?: number;
  /** Machines draws its own card (with the link) in place of this one */
  hero?: string;
  /** more cards right after the gauges (Machines: the sessions) */
  afterGauges?: string[];
  /** more cards at the end (Machines: the LLM servers) */
  end?: string[];
  /** the top processes and GPU processes as tables (Machines leaves them out) */
  procs?: boolean;
  /** just the card, the gauges and what comes right after them (while you move between machines) */
  light?: boolean;
}): string {
  const { h, t } = o;
  const w = o.width ?? PANE;
  const a = o.agents;
  const os = osId(h.os, a?.os_name, h.platform);
  if (!h.reachable) {
    const { head, detail } = explainError(h.error);
    return [
      img(
        `${h.alias} is unreachable`,
        hero(
          {
            title: h.alias,
            color: h.color,
            subtitle: head,
            logo: o.logo("os", os?.id),
            link: { bars: 0, text: "offline", color: t.state.down },
          },
          t,
          w,
        ),
      ),
      `**${cell(head)}**`,
      cell(detail),
      "The doctor (⌥⌘D in Machines) says what to do.",
    ].join("\n\n");
  }
  const herdr = herdrText(o.host, a);
  const gpu = gpuSummary(h, a);
  const tags: Part[] = [];
  if (gpu) tags.push({ text: gpu, logo: o.logo("gpu", gpuVendor(h, a)) });
  if (herdr) tags.push({ text: herdr.text, warn: !!herdr.warn });
  const out: string[] = [
    o.hero ??
      img(
        `${h.alias}: ${h.os ?? ""}`,
        hero(
          {
            title: h.alias,
            color: h.color,
            subtitle: [h.os || a?.os_name, h.cpu_model].filter(Boolean).join(" · "),
            logo: o.logo("os", os?.id),
            tags,
            aside: h.uptime_s != null ? { value: duration(h.uptime_s), label: "uptime" } : undefined,
          },
          t,
          w,
        ),
      ),
    img("CPU, memory, GPU and temperature", gauges(statGauges(h, t), t, w)),
    ...(o.afterGauges ?? []),
  ];
  if (o.light) return out.join("\n\n");
  const gpus = h.gpus ?? [];
  if (gpus.length) {
    out.push(
      img(
        "GPUs",
        meters(gpuMeters(gpus, t), t, w, { title: gpus.length > 1 ? `GPUs · ${gpuSummary(h)}` : gpus[0].name }),
      ),
    );
  }
  if (h.cores?.length) out.push(img("Cores", coreGrid(h.cores, t, w)));
  const span = spanText(o.hist);
  const last = o.hist[o.hist.length - 1]?.t ?? 0;
  const first = o.hist[0]?.t ?? 0;
  out.push(
    img(
      "Live",
      trends(
        statTrends(h, o.hist, t),
        { from: first, to: Math.max(last, first + 1), title: "Live", note: span || "filling in while open" },
        t,
        w,
      ),
    ),
  );
  out.push(img("About", facts(statFacts(h, o), t, w, { title: "About" })));
  // real volumes, not a boot partition or a snapshot
  const disks = (h.disks ?? []).filter((d) => d.total >= 1024 ** 3);
  if (disks.length) {
    out.push(
      img(
        "Disks",
        meters(
          disks.slice(0, 8).map((d) => ({
            label: shortMount(d.mount),
            frac: frac(d.used, d.total),
            value: `${pct(frac(d.used, d.total))} · ${bytes(d.used)} / ${bytes(d.total)}`,
          })),
          t,
          w,
          { title: "Disks" },
        ),
      ),
    );
  }
  if (o.procs !== false && h.top?.length) {
    out.push(
      "**Top processes**\n\n| Process | CPU | Memory | User |\n|---|--:|--:|---|\n" +
        h.top
          .slice(0, 6)
          .map((p) => `| ${cell(p.name)} | ${pct(p.cpu)} | ${bytes(p.rss)} | ${cell(p.user)} |`)
          .join("\n"),
    );
  }
  if (o.procs !== false && h.gpu_procs?.length) {
    out.push(
      "**GPU processes**\n\n| Process | GPU | VRAM | User |\n|---|--:|--:|---|\n" +
        h.gpu_procs
          .slice(0, 6)
          .map((p) => `| ${cell(p.name)} | ${p.gpu} | ${bytes(p.mem)} | ${cell(p.user)} |`)
          .join("\n"),
    );
  }
  out.push(...(o.end ?? []));
  return out.join("\n\n");
}

/** A list row's accessory, before it becomes Raycast's: at most three per row, short enough never to overflow. */
export interface Badge {
  /** the tag's text (empty: an icon only) */
  text: string;
  /** 0..1, for the load colour */
  frac?: number;
  /** red / yellow regardless of load */
  tone?: "bad" | "warn";
  /** an icon instead of / before the text */
  icon?: "gpu" | "warn";
  tooltip: string;
}

/** Machine Stats' row: CPU % and GPU % (or memory), coloured by load; offline in red; herdr's warning as an icon. */
export function statsBadges(h: MachineStats, host?: Host, a?: AgentsHost): Badge[] {
  if (!h.reachable) return [{ text: "offline", tone: "bad", tooltip: explainError(h.error).head }];
  const out: Badge[] = [];
  const herdr = herdrText(host, a);
  if (herdr?.warn) out.push({ text: "", icon: "warn", tone: "warn", tooltip: `${herdr.text}: ${herdr.warn}` });
  out.push({
    text: `CPU ${pct(h.cpu)}`,
    frac: h.cpu,
    tooltip: `CPU ${pct(h.cpu)}${h.load ? ` · load ${h.load[0].toFixed(2)}` : ""}`,
  });
  const s = statSample(h, 0);
  if (s.gpu != null) {
    out.push({
      text: pct(s.gpu),
      frac: s.gpu,
      icon: "gpu",
      tooltip: `${gpuSummary(h, a)} · ${pct(s.gpu)} · VRAM ${pct(s.vram)}`,
    });
  } else {
    out.push({
      text: `MEM ${pct(s.mem)}`,
      frac: s.mem ?? 0,
      tooltip: `Memory ${bytes(h.mem_used)} / ${bytes(h.mem_total)}`,
    });
  }
  return out;
}

/** The machine as text (Copy Summary). */
export function statsSummary(h: MachineStats, host?: Host, a?: AgentsHost): string {
  const gpus = (h.gpus ?? []).map((g) => `GPU${g.index} ${pct(g.util)} ${bytes(g.mem_used)}/${bytes(g.mem_total)}`);
  const installed = (a?.agents ?? []).filter((x) => x.installed).map((x) => `${x.id} ${x.version ?? ""}`.trim());
  return [
    `${h.alias}: ${[h.os, h.kernel].filter(Boolean).join(" · ")}`,
    `CPU ${pct(h.cpu)} · MEM ${bytes(h.mem_used)}/${bytes(h.mem_total)}${h.load ? ` · load ${h.load.map((l) => l.toFixed(2)).join(" ")}` : ""}`,
    ...gpus,
    herdrText(host, a)?.text ?? "",
    installed.length ? `agents: ${installed.join(", ")}` : "",
    `claude: ${h.claude_procs ?? 0} procs, ${bytes(h.claude_rss)}`,
  ]
    .filter(Boolean)
    .join("\n");
}
