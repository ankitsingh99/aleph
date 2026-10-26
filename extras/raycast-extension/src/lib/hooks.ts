import { Cache } from "@raycast/api";
import { useCachedPromise } from "@raycast/utils";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ApiCall, apiOr, request } from "./api";
import { isLive, listen, Topic } from "./live";
import { parseHistory, pushSample, Sampled } from "./samples";
import {
  AgentsReport,
  agentsApi,
  AGENTS_ARGS,
  daemonStatus,
  Fleet,
  FLEET_API,
  hasFlag,
  onHost,
  optionalJson,
  queueRuns,
  runJson,
  SETTINGS_API,
  SETTINGS_ARGS,
  watchInterval,
} from "./cli";
import { rememberMachines } from "./logos";
import { following, type Mode, mode as showcaseMode, parsePresenter, type Presenter } from "./showcase";
import { brandsFresh, errorText, rememberBrands } from "./ui";

export interface Options<T = unknown> {
  /** per-call timeout (ms) */
  timeoutMs?: number;
  /** re-run every N ms while the view is open */
  pollMs?: number;
  /**
   * live: re-run this long after each answer while the view is open (never
   * two at once, so a slow machine can't pile them up). The view's loading
   * bar stays quiet for these; it shows for the first answer and for
   * `revalidate` (a Refresh you asked for).
   */
  liveMs?: number;
  /** the subcommand may not exist in this aleph: resolve to `undefined` instead of failing */
  optional?: string;
  /** skip running (e.g. nothing selected yet) */
  execute?: boolean;
  /** toast title when it fails (none when omitted and the view shows the error itself) */
  failureTitle?: string;
  /** JSON on stdout counts even when the command exits non-zero (`doctor --json`) */
  lenient?: boolean;
  /** ask the daemon's API first; the CLI command answers when no daemon does */
  api?: ApiCall<T>;
  /**
   * ask again when the daemon says these changed (`events.subscribe`); while
   * it's telling us, the timers above only run once a minute as a safety net
   */
  topics?: Topic[];
}

/** While the daemon pushes changes, a timer only checks once a minute. */
const SAFETY_MS = 60_000;

/**
 * `aleph <args…>` → parsed JSON (with `api`: the daemon's answer first),
 * cached across launches so lists show instantly (marked loading while fresh
 * data arrives), with friendly errors. `stale` is true when the last refresh
 * failed and the data shown is the cached one.
 */
export function useAleph<T>(args: string[], o: Options<T> = {}) {
  const { api } = o;
  const r = useCachedPromise(
    // (the last two arguments are only the cache key's: two hosts' answers
    // are two, and what was cached unmasked never shows in showcase mode)
    (
      a: string[],
      timeout?: number,
      optional?: string,
      lenient?: boolean,
      method?: string,
      _params?: string,
      _mode?: Mode,
    ) => {
      const cli = () => (optional ? optionalJson<T>(optional, a, timeout, lenient) : runJson<T>(a, timeout, lenient));
      return api && method ? apiOr<T | undefined>(api as ApiCall<T | undefined>, cli) : cli();
    },
    // the params are part of what's asked (two hosts' stats are two answers)
    [args, o.timeoutMs, o.optional, o.lenient, api?.method, JSON.stringify(api?.params ?? null), showcaseMode()],
    {
      keepPreviousData: true,
      execute: o.execute ?? true,
      failureToastOptions: o.failureTitle ? { title: o.failureTitle } : undefined,
      onError: o.failureTitle ? undefined : () => undefined,
    },
  );
  const { revalidate } = r;
  // pushed: ask again when the daemon says it changed (a burst is one ask)
  const topics = o.topics?.join(",") ?? "";
  const [pushed, setPushed] = useState(false);
  useEffect(() => {
    if (!topics || o.execute === false) return;
    let t: ReturnType<typeof setTimeout> | undefined;
    const stop = listen(topics.split(",") as Topic[], () => {
      if (t) clearTimeout(t);
      t = setTimeout(revalidate, 200);
    });
    const k = setInterval(() => setPushed(isLive()), 2000);
    setPushed(isLive());
    return () => {
      stop();
      clearInterval(k);
      if (t) clearTimeout(t);
    };
  }, [revalidate, topics, o.execute]);
  const pollMs = o.pollMs && pushed ? Math.max(o.pollMs, SAFETY_MS) : o.pollMs;
  const liveMs = o.liveMs && pushed ? Math.max(o.liveMs, SAFETY_MS) : o.liveMs;
  useEffect(() => {
    if (!pollMs || o.execute === false) return;
    const t = setInterval(revalidate, pollMs);
    return () => clearInterval(t);
  }, [revalidate, pollMs, o.execute]);
  // live: the next refresh once this one has answered
  useEffect(() => {
    if (!liveMs || o.execute === false || r.isLoading) return;
    const t = setTimeout(revalidate, liveMs);
    return () => clearTimeout(t);
  }, [revalidate, liveMs, o.execute, r.isLoading, r.data]);
  const [asked, setAsked] = useState(false);
  useEffect(() => {
    if (!r.isLoading) setAsked(false);
  }, [r.isLoading]);
  const refresh = useCallback(() => {
    setAsked(true);
    revalidate();
  }, [revalidate]);
  // the same answer again is the same object: what's drawn from it isn't drawn again
  const last = useRef<{ json: string; data: T } | null>(null);
  const data = useMemo(() => {
    if (r.data === undefined) return r.data;
    const json = JSON.stringify(r.data);
    if (last.current?.json === json) return last.current.data;
    last.current = { json, data: r.data as T };
    return r.data;
  }, [r.data]);
  return {
    ...r,
    data,
    revalidate: refresh,
    // refreshes you didn't ask for (timers, pushes) keep the loading bar quiet
    isLoading: o.liveMs || o.topics ? r.isLoading && (r.data === undefined || asked) : r.isLoading,
    errorInfo: r.error ? errorText(r.error) : undefined,
    stale: !!r.error && r.data !== undefined,
  };
}

const histories = new Cache({ namespace: "history" });

/**
 * Each machine's samples over time (the live sparklines): `samples` adds one
 * per machine each time it changes. Kept in Raycast's Cache, so a view opened
 * again within a quarter of an hour starts with what it saw before.
 */
export function useHistory<T extends Sampled>(
  kind: string,
  samples: { alias: string; s: T }[] | undefined,
): Record<string, T[]> {
  const ref = useRef<Record<string, T[]>>({});
  const [hist, setHist] = useState<Record<string, T[]>>({});
  useEffect(() => {
    if (!samples?.length) return;
    const now = Date.now() / 1000;
    const next = { ...ref.current };
    for (const { alias, s } of samples) {
      const key = `${kind}.${alias}`;
      const before = next[alias] ?? parseHistory<T>(histories.get(key));
      const after = pushSample(before, s, now);
      next[alias] = after;
      if (after !== before) histories.set(key, JSON.stringify(after));
    }
    ref.current = next;
    setHist(next);
  }, [kind, samples]);
  return hist;
}

/** Every machine and its sessions: the daemon's snapshot, else `ls --json`. */
export function useFleet(o: Omit<Options<Fleet>, "optional" | "api"> = {}) {
  const r = useAleph<Fleet>(["ls", "--json"], { timeoutMs: 45_000, api: FLEET_API, topics: ["state"], ...o });
  // each machine's OS as the fleet knows it (its herdr check), before this render draws its logo:
  // a machine added a minute ago shows its own logo, not a dot
  useMemo(() => rememberMachines((r.data?.hosts ?? []).map((h) => ({ alias: h.alias, os: h.os }))), [r.data]);
  return r;
}

/**
 * `agents --json`: installed agents per machine (aleph reuses its answers
 * for 10 minutes, so this is cheap). Remembers each agent's brand for the other commands.
 * `extra`: more flags (`--local`: this computer too).
 */
export function useAgentsReport(host?: string, o: Omit<Options, "optional"> = {}, extra: string[] = []) {
  const r = useAleph<AgentsReport | undefined>(onHost(host, [...AGENTS_ARGS, ...extra]), {
    timeoutMs: 120_000,
    optional: "agents",
    lenient: true,
    // the daemon keeps every machine's answer (AgentWatch); the CLI only without one
    api: agentsApi(host, false, extra.includes("--local")),
    topics: ["agents"],
    ...o,
  });
  useEffect(() => {
    rememberBrands(r.data);
    rememberMachines((r.data?.hosts ?? []).map((h) => ({ alias: h.alias, os: h.os_name || h.os })));
  }, [r.data]);
  return r;
}

/**
 * Agent names, glyphs and colours from the CLI (`agents --json` lists every
 * agent in Aleph's registry), so new agents show up without an extension
 * update. Asked of this computer only (`--host local`: no ssh), at most once a
 * day; the other commands' `agents --json` answers count too. Returns a
 * counter that changes when fresh brands arrive (re-renders the caller).
 */
export function useBrands(): number {
  const [version, setVersion] = useState(0);
  useEffect(() => {
    if (brandsFresh()) return;
    let live = true;
    apiOr(agentsApi("local"), () =>
      optionalJson<AgentsReport>("agents", ["--host", "local", ...AGENTS_ARGS], 60_000, true),
    )
      .then((r) => {
        if (rememberBrands(r) && live) setVersion((v) => v + 1);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);
  return version;
}

/** Does this aleph's `help <cmd>` mention `word`? (cached across launches: features of newer builds) */
export function useHelpHas(cmd: string, word: string): boolean {
  const r = useCachedPromise((c: string, w: string) => hasFlag(c, w), [cmd, word], { keepPreviousData: true });
  return !!r.data;
}

/** Aleph's settings (`settings list --json`; the daemon's `config.list`), cached across launches. */
export function useSettings() {
  return useAleph<Record<string, unknown>>(SETTINGS_ARGS, { api: SETTINGS_API, topics: ["config"] });
}

/**
 * Are queued messages going out right now? The daemon answers on its socket
 * and runs the hub (`daemon.status`; older builds: their status file, with
 * `watch_interval` from the settings). Looked at again every 15 s while the
 * view is open; true until the first answer, so nothing warns for nothing.
 */
export function useWatching(): boolean {
  const settings = useSettings();
  const iv = watchInterval(settings.data);
  const r = useCachedPromise((i: number, _mode: Mode) => daemonStatus(i), [iv, showcaseMode()], {
    keepPreviousData: true,
  });
  const { revalidate } = r;
  useEffect(() => {
    const t = setInterval(revalidate, 15_000);
    return () => clearInterval(t);
  }, [revalidate]);
  return r.data ? queueRuns(r.data, iv) : true;
}

/**
 * Presenter sync (a showcase build, showcase.sync on): what the dashboard
 * you present from shows — asked of the daemon when it says the presenter
 * moved (topic `presenter`), and every 15 s besides. null while nobody
 * presents, with no daemon, or in a build that doesn't follow (the stub).
 */
export function usePresenter(): Presenter | null {
  const want = following();
  const [p, setP] = useState<Presenter | null>(null);
  useEffect(() => {
    if (!want) {
      setP(null);
      return;
    }
    let alive = true;
    const ask = () =>
      request<unknown>("presenter.get")
        .then((v) => parsePresenter(v))
        .catch(() => null)
        .then((n) => {
          // the same presenter: the same object (no re-render, no reselect)
          if (alive) setP((was) => (was && n && was.gen === n.gen ? was : n));
        });
    ask();
    const stop = listen(["presenter"], ask);
    const t = setInterval(ask, 15_000);
    return () => {
      alive = false;
      stop();
      clearInterval(t);
    };
  }, [want]);
  return p;
}
