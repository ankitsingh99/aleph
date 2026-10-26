// Machines, drawn: one machine's detail (`ls --json` with its agents and LLM
// servers, and the link readings kept while the view is open) as Markdown
// with SVG cards — a hero card with its link, pills of its sessions' states,
// the sessions, and chips for its agents, servers and GPU — and the list
// row's badges. No Raycast imports: plain node tests it.
import type { AgentsHost, Host, Service, ServicesHost, Session } from "./cli";
import { ChartTheme } from "./charts";
import { ChipRow, chipRows, hintRow, notice } from "./cards";
import { ChipGroup, chips, hero, PANE, Part, Pill, pills, sessionList, SessionRow } from "./dash";
import { since, STATE_ORDER, stateOf, STATUS_LABEL } from "./fmt";
import { gpuId, osId } from "./logo-ids";
import { explainError, hostLink, Tone } from "./model";
import { bars, LinkSample, seriesOf, spanText } from "./samples";
import { BrandLike, img, LogoFn } from "./stats-view";

/** This poll's link reading. */
export function linkSample(h: Host, t: number): LinkSample {
  return { t: h.fetched_at ?? t, ms: h.reachable ? (h.latency_ms ?? null) : null, up: h.reachable };
}

/** How many sessions are in each state, in the order they matter (needs you first). */
export function sessionCounts(h: Pick<Host, "sessions">): { state: string; n: number }[] {
  return STATE_ORDER.map((state) => ({ state, n: h.sessions.filter((s) => stateOf(s) === state).length }));
}

/** A list row's accessory for a machine, before it becomes Raycast's (at most three). */
export interface MachineBadge {
  kind: "needs" | "working" | "gpu" | "link" | "offline" | "stopped";
  text: string;
  tooltip: string;
  /** signal bars for `link` (0..4) */
  bars?: number;
  tone?: Tone;
}

/** The GPU as `agents --json` saw it, short: `2× RTX 4090`, `Apple GPU`. */
export function machineGpu(a?: Pick<AgentsHost, "gpu" | "gpus">): string {
  const g = a?.gpu || a?.gpus?.[0] || "";
  return g.replace(/(NVIDIA |AMD )?(GeForce |Radeon |Instinct )?/gi, "").trim();
}

/**
 * Machines' row: who needs you (or how many are working), the GPU's vendor,
 * and the link as signal bars. A machine that's down says only that.
 */
export function machineBadges(h: Host, a?: AgentsHost): MachineBadge[] {
  const link = hostLink(h);
  if (link.state === "down") return [{ kind: "offline", text: "offline", tooltip: link.text, tone: "bad" }];
  const out: MachineBadge[] = [];
  const counts = Object.fromEntries(sessionCounts(h).map((c) => [c.state, c.n]));
  if (counts.blocked)
    out.push({
      kind: "needs",
      text: String(counts.blocked),
      tooltip: `${counts.blocked} need${counts.blocked === 1 ? "s" : ""} you`,
      tone: "warn",
    });
  else if (counts.working)
    out.push({ kind: "working", text: String(counts.working), tooltip: `${counts.working} working`, tone: "good" });
  const gpu = machineGpu(a);
  if (gpu && gpuId(a?.gpu, ...(a?.gpus ?? []))) out.push({ kind: "gpu", text: "", tooltip: gpu });
  if (link.state === "stopped") out.push({ kind: "stopped", text: "herdr stopped", tooltip: link.text, tone: "warn" });
  else out.push({ kind: "link", text: "", tooltip: link.text, bars: bars(h.latency_ms, true), tone: link.tone });
  return out;
}

const toneColor = (tone: Tone, t: ChartTheme) =>
  tone === "good" ? t.state.done : tone === "warn" ? t.state.warn : tone === "bad" ? t.state.down : t.other;

const md = (s: string) => s.replace(/[<>*_`#[\]|\\]/g, (c) => `\\${c}`);

/** The state colour of a session state. */
export function stateColor(state: string, t: ChartTheme): string {
  return state === "blocked"
    ? t.state.blocked
    : state === "working"
      ? t.state.working
      : state === "done"
        ? t.state.done
        : state === "idle"
          ? t.state.idle
          : state === "limit"
            ? t.state.limit
            : t.other;
}

function healthColor(s: Service, t: ChartTheme): string {
  return s.health === "up"
    ? t.state.done
    : s.health === "loading"
      ? t.state.warn
      : s.health === "error" || s.health === "down"
        ? t.state.down
        : t.other;
}

/** The whole detail of one machine. */
export function machineMarkdown(o: {
  h: Host;
  a?: AgentsHost;
  svc?: ServicesHost;
  hist: LinkSample[];
  t: ChartTheme;
  logo: LogoFn;
  brand: (id: string) => BrandLike;
  /** what some command saw of its OS before (`machineLogo`'s text), when `agents` hasn't answered */
  osText?: string;
  /** a session's status in words (cli.ts `statusText`); default: its state and for how long */
  statusText?: (s: Session) => string;
  now?: number;
  width?: number;
}): string {
  const { h, a, t } = o;
  const w = o.width ?? PANE;
  const now = o.now ?? Date.now() / 1000;
  const link = hostLink(h);
  const os = osId(a?.os_name, a?.os, o.osText);
  const gpu = machineGpu(a);
  const herdrWarn = h.herdr_restart_needed
    ? "restart needed (aleph update)"
    : h.herdr_drift
      ? "differs from this computer's (aleph update)"
      : "";
  const tags: Part[] = [];
  if (a?.os_name || os) tags.push({ text: a?.os_name || os?.name || "" });
  if (h.herdr_version) tags.push({ text: `herdr ${h.herdr_version}`, warn: !!herdrWarn });
  const history = seriesOf(o.hist, "ms");
  const span = spanText(o.hist);
  const card = hero(
    {
      title: h.alias,
      color: h.color,
      subtitle: [h.machine && h.machine !== h.alias ? `herdr machine ${h.machine}` : "", h.address]
        .filter(Boolean)
        .join(" · "),
      logo: o.logo("os", os?.id),
      tags,
      link: {
        bars: link.state === "down" ? 0 : link.state === "stopped" ? 1 : bars(h.latency_ms, true),
        text:
          link.state === "down"
            ? "offline"
            : link.state === "stopped"
              ? "herdr stopped"
              : h.latency_ms != null
                ? `${h.latency_ms} ms`
                : "online",
        color: toneColor(link.tone, t),
        history: history.some((p) => p[1] != null) ? history : undefined,
        note: span && link.state !== "down" ? `latency · ${span}` : undefined,
      },
    },
    t,
    w,
  );
  const out = [img(`${h.alias}: ${link.text}`, card)];
  if (!h.reachable) {
    const { head, detail } = explainError(h.error);
    out.push(`**${md(head)}**`, md(detail));
    if (h.fetched_at) out.push(`_Last answered ${since(h.fetched_at, now)} ago._`);
    out.push("The doctor (⌥⌘D) says what to do.");
    return out.join("\n\n");
  }
  const counts = sessionCounts(h);
  const shown = counts.filter((c) => c.n > 0 || c.state !== "limit");
  const pillList: Pill[] = shown.map((c) => ({
    n: c.n,
    label: STATUS_LABEL[c.state] ?? c.state,
    state: c.state,
    color: stateColor(c.state, t),
  }));
  out.push(img("Sessions by state", pills(pillList, t, w)));
  if (h.sessions.length) {
    const order = (s: Session) => STATE_ORDER.indexOf(stateOf(s));
    const sorted = [...h.sessions].sort(
      (x, y) => order(x) - order(y) || (x.display || x.name).localeCompare(y.display || y.name),
    );
    const rows: SessionRow[] = sorted.map((s) => {
      const st = stateOf(s);
      const b = o.brand(s.agent);
      return {
        name: s.name || s.display || s.pane_id,
        title: s.title || (s.cwd && s.cwd !== "-" ? s.cwd.replace(/^\/(home|Users)\/[^/]+/, "~") : ""),
        state: st,
        color: stateColor(st, t),
        status: o.statusText
          ? o.statusText(s)
          : [STATUS_LABEL[st] ?? st, since(s.status_since, now)].filter(Boolean).join(" · "),
        logo: o.logo("agent", s.agent),
        agentLetter: b.name,
        agentColor: b.color,
      };
    });
    out.push(img("Sessions", sessionList(rows, t, w)));
  } else {
    out.push("_No sessions here — Quick Start (⌘N) starts one._");
  }
  const groups: ChipGroup[] = [];
  if (a) {
    const installed = (a.agents ?? []).filter((x) => x.installed);
    groups.push({
      title: "Agents",
      note: installed.length ? `${installed.length} installed` : undefined,
      chips: installed.map((x) => ({
        text: x.name,
        note: x.version ?? undefined,
        logo: o.logo("agent", x.id),
        color: x.color,
      })),
      empty: "none found — ⌘G installs one",
    });
  }
  if (o.svc) {
    const servers = o.svc.services ?? [];
    groups.push({
      title: "LLM servers",
      note: servers.length ? `${servers.filter((s) => s.health === "up").length} of ${servers.length} up` : undefined,
      chips: servers.map((s) => ({
        text: s.name,
        note: s.loaded_count ? `${s.loaded_count} loaded` : s.model_count ? `${s.model_count} models` : undefined,
        logo: o.logo("service", s.id),
        color: s.color,
        dot: healthColor(s, t),
      })),
      empty: "none found",
    });
  }
  if (gpu) {
    groups.push({
      title: "GPU",
      chips: [{ text: gpu, logo: o.logo("gpu", gpuId(a?.gpu, ...(a?.gpus ?? []))?.id), color: t.other }],
    });
  }
  if (groups.length) out.push(img("Agents, LLM servers and GPU", chips(groups, t, w)));
  return out.join("\n\n");
}

// ---------------------------------------------------------------- the detail, cards all the way

/** A machine's list section: `3 online · 5 sessions · 1 needs you`. */
export function machinesSummary(hosts: Host[]): string {
  const sessions = hosts.reduce((n, h) => n + h.sessions.length, 0);
  const needs = hosts.reduce((n, h) => n + h.sessions.filter((s) => stateOf(s) === "blocked").length, 0);
  return [
    `${hosts.length} machine${hosts.length === 1 ? "" : "s"}`,
    sessions ? `${sessions} session${sessions === 1 ? "" : "s"}` : "no sessions",
    needs ? `${needs} need${needs === 1 ? "s" : ""} you` : "",
  ]
    .filter(Boolean)
    .join(" · ");
}

/** `Ubuntu 22.04.5 LTS` → `Ubuntu 22.04` (the card's tags share one line with the GPU and herdr). */
export const shortOs = (s: string) => s.replace(/\s+LTS$/i, "").replace(/(\d+\.\d+)\.\d+/, "$1");

/** A session's second words: its task, else its folder (never just the agent's name again). */
function sessionWords(s: Session, agentName: string): string {
  const folder =
    s.cwd && s.cwd !== "-"
      ? s.cwd
          .replace(/^\/(home|Users)\/[^/]+/, "~")
          .split("/")
          .pop() || "~"
      : "";
  return s.title && s.title !== agentName ? s.title : folder;
}

/**
 * One machine's whole detail, in cards with no prose: the card with its link,
 * its sessions (pills of the states there are, then each session), or a quiet
 * row with Quick Start's keycap when there are none; its agents and LLM
 * servers as chip rows (a faint chip when there are none). Down: why, and the
 * doctor's keycap.
 */
export function machineCards(o: MachineCardOpts): string {
  const p = machineParts(o);
  return [p.hero, ...p.body, ...p.sessions, ...p.chips].join("\n\n");
}

export interface MachineCardOpts {
  h: Host;
  a?: AgentsHost;
  svc?: ServicesHost;
  hist: LinkSample[];
  t: ChartTheme;
  logo: LogoFn;
  brand: (id: string) => BrandLike;
  /** what some command saw of its OS before (`machineLogo`'s text), when `agents` hasn't answered */
  osText?: string;
  /** a session's status in words (cli.ts `statusText`); default: its state and for how long */
  statusText?: (s: Session) => string;
  now?: number;
  width?: number;
  /** leave the agents' chips out (with live stats, the facts list them) */
  noAgents?: boolean;
}

/**
 * A machine's cards in pieces, for Machines to put the live stats between
 * them: the card with its link, why it's down (nothing else then), its
 * sessions (or Quick Start's row), its agents and LLM servers as chips.
 */
export function machineParts(o: MachineCardOpts): {
  hero: string;
  body: string[];
  sessions: string[];
  chips: string[];
} {
  const { h, a, t } = o;
  const w = o.width ?? PANE;
  const now = o.now ?? Date.now() / 1000;
  const link = hostLink(h);
  const os = osId(a?.os_name, a?.os, o.osText);
  const gpu = machineGpu(a);
  const herdrWarn = h.herdr_restart_needed || h.herdr_drift;
  const tags: Part[] = [];
  if (a?.os_name || os) tags.push({ text: shortOs(a?.os_name || os?.name || "") });
  if (gpu) tags.push({ text: gpu, logo: o.logo("gpu", gpuId(a?.gpu, ...(a?.gpus ?? []))?.id) });
  if (h.herdr_version) tags.push({ text: `herdr ${h.herdr_version}`, warn: !!herdrWarn });
  const history = seriesOf(o.hist, "ms");
  const span = spanText(o.hist);
  const heroCard = img(
    `${h.alias}: ${link.text}`,
    hero(
      {
        title: h.alias,
        color: h.color,
        subtitle: [h.machine && h.machine !== h.alias ? `herdr machine ${h.machine}` : "", h.address]
          .filter(Boolean)
          .join(" · "),
        logo: o.logo("os", os?.id),
        tags,
        link: {
          bars: link.state === "down" ? 0 : link.state === "stopped" ? 1 : bars(h.latency_ms, true),
          text:
            link.state === "down"
              ? "offline"
              : link.state === "stopped"
                ? "herdr stopped"
                : h.latency_ms != null
                  ? `${h.latency_ms} ms`
                  : "online",
          color: toneColor(link.tone, t),
          history: history.some((p) => p[1] != null) ? history : undefined,
          note: span && link.state !== "down" ? `latency · ${span}` : undefined,
        },
      },
      t,
      w,
    ),
  );
  const body: string[] = [];
  const sessions: string[] = [];
  const chipCards: string[] = [];
  if (!h.reachable) {
    const { head, detail } = explainError(h.error);
    const seen = h.fetched_at ? `last answered ${since(h.fetched_at, now)} ago` : "";
    // where it tried (`gpu2.lan:2222`); the error's own words only when the head couldn't say more
    const where = detail.includes(" · ") ? detail.split(" · ")[0] : head === "error" ? detail : "";
    const stopped = link.state === "stopped";
    body.push(
      img(
        head,
        notice(
          {
            state: stopped ? "warn" : "down",
            color: stopped ? t.state.warn : t.state.down,
            title: head,
            detail: [where, seen].filter(Boolean).join(" · ") || undefined,
          },
          t,
          w,
        ),
      ),
      img("Doctor", hintRow({ text: "Find out why", keys: "⌥⌘D", action: "Doctor" }, t, w)),
    );
    return { hero: heroCard, body, sessions, chips: chipCards };
  }
  if (h.sessions.length) {
    const pillList: Pill[] = sessionCounts(h)
      .filter((c) => c.n > 0)
      .map((c) => ({ n: c.n, label: STATUS_LABEL[c.state] ?? c.state, state: c.state, color: stateColor(c.state, t) }));
    sessions.push(img("Sessions by state", pills(pillList, t, w)));
    const order = (s: Session) => STATE_ORDER.indexOf(stateOf(s));
    const sorted = [...h.sessions].sort(
      (x, y) => order(x) - order(y) || (x.display || x.name).localeCompare(y.display || y.name),
    );
    const rows: SessionRow[] = sorted.map((s) => {
      const st = stateOf(s);
      const b = o.brand(s.agent);
      return {
        name: s.name || s.display || s.pane_id,
        title: sessionWords(s, b.name),
        state: st,
        color: stateColor(st, t),
        status: o.statusText
          ? o.statusText(s)
          : [STATUS_LABEL[st] ?? st, since(s.status_since, now)].filter(Boolean).join(" · "),
        logo: o.logo("agent", s.agent),
        agentLetter: b.name,
        agentColor: b.color,
      };
    });
    sessions.push(img("Sessions", sessionList(rows, t, w)));
  } else {
    sessions.push(img("No sessions", hintRow({ text: "No sessions", keys: "⌘N", action: "Quick Start" }, t, w)));
  }
  const rows: ChipRow[] = [];
  if (a && !o.noAgents) {
    const installed = (a.agents ?? []).filter((x) => x.installed);
    rows.push({
      label: "Agents",
      chips: installed.map((x) => ({
        text: x.name,
        logo: o.logo("agent", x.id),
        color: x.color,
      })),
      empty: "none found · ⌘G",
    });
  }
  // only when the machine has any: an empty row says nothing
  if (o.svc?.services?.length) {
    rows.push({
      label: "LLM servers",
      chips: (o.svc.services ?? []).map((s) => ({
        text: s.name,
        note: s.loaded_count ? `${s.loaded_count} loaded` : s.model_count ? `${s.model_count} models` : undefined,
        logo: o.logo("service", s.id),
        color: s.color,
        dot: healthColor(s, t),
      })),
      empty: o.svc.reachable ? "none" : "not scanned",
    });
  }
  if (rows.length) chipCards.push(img("Agents and LLM servers", chipRows(rows, t, w)));
  return { hero: heroCard, body, sessions, chips: chipCards };
}
