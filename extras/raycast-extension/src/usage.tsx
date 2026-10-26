// Usage: tokens, time and cost from the agents' own logs on every machine
// (aleph usage), as a list you move through: the overview first (today, 7
// and 30 days; the range per day stacked by agent; by agent and machine),
// then every agent, model, machine and project — each with its share of the
// range as a ring and its own detail (its figures, its days, where it ran and
// with what). The dropdown narrows everything to one agent or one machine;
// projects come a page at a time as you scroll (typing finds any of them).
// The dropdown also takes one account or one project. "Cost breakup &
// trends" puts the cost under the total by account (⌘G: by agent, machine,
// model, project), this period against the one before, the days by that
// split, the week's hours and 12 weeks; "Insights" per prompt and per
// session, the costliest prompts, plans, tools and the streaks
// (`usage.explore`). The numbers come from the daemon's `usage.query` (the
// daemon keeps reading the logs) or `aleph usage --json`, again every 30 s
// while the view is open. ⌘T switches cost and tokens, ⌘L shows the numbers
// as tables, ⌘1–9 the period, ⌘0 your own dates.
import {
  Action,
  ActionPanel,
  Color,
  environment,
  Form,
  Icon,
  Image,
  Keyboard,
  LaunchProps,
  List,
  useNavigation,
} from "@raycast/api";
import { pausable } from "./lib/pause";
import { CopyAction } from "./lib/copy";
import { getProgressIcon, useCachedPromise, useCachedState } from "@raycast/utils";
import { useEffect, useMemo, useRef, useState } from "react";
import { apiOr } from "./lib/api";
import { isLive, listen } from "./lib/live";
import { CliError, runJson, setting, usageQuery, UsageQuery, UsageReport, UsageRow } from "./lib/cli";
import { chartTheme, fit } from "./lib/charts";
import { useBrands, useFleet, useSettings } from "./lib/hooks";
import { agentLogo, agentLogoId, cardLogos, machineLogo, machineOsText } from "./lib/logos";
import { osId } from "./lib/logo-ids";
import { mode as showcaseMode, navTitle, withBanner, type Mode } from "./lib/showcase";
import { openInTerminal } from "./lib/terminal";
import { ALEPH_ICON, brand, errorText } from "./lib/ui";
import {
  compact,
  dayKey,
  keyColors,
  metricOf,
  Metric,
  money,
  RangeId,
  RANGES,
  span,
  table,
  vendorAgent,
} from "./lib/usage";
import { Explore, exploreMarkdown, insightsMarkdown, splitName } from "./lib/explore-view";
import {
  figure,
  itemMarkdown,
  Kind,
  limitsMarkdown,
  limitSummary,
  Loaded,
  notes,
  overviewMarkdown,
  projectName,
  rowColor,
  shareOf,
  Sources,
  sourcesMarkdown,
  tablesMarkdown,
} from "./lib/usage-view";

/** Each agent's fullest plan window, its logo and the share in a colour by how full it is. */
function limitAccessory(l: Loaded["limits"]): List.Item.Accessory[] {
  return limitSummary(l).map((x) => ({
    icon: agentLogo(x.agent),
    text: {
      value: `${Math.round(x.top)}%`,
      color: x.top >= 90 ? Color.Red : x.top >= 70 ? Color.Orange : Color.Green,
    },
    tooltip: `${brand(x.agent).name}: ${x.windows.join(" · ")} used`,
  }));
}

/** What `*` means, in a figure's tooltip. */
const ESTIMATED = " · * part of it from Aleph's price table";

/** Agents whose days are fetched with the rest (the others' when you select them). */
const AGENT_DAYS = 8;
/** Projects per page. */
const PAGE = 25;
/** Read again this long after the last answer. */
const LIVE_MS = 30_000;

/** Run `jobs`, at most `n` at a time (the CLI fallback is one process each). */
async function pool<T>(jobs: (() => Promise<T>)[], n = 4): Promise<T[]> {
  const out: T[] = new Array(jobs.length);
  let next = 0;
  const worker = async () => {
    while (next < jobs.length) {
      const i = next++;
      out[i] = await jobs[i]();
    }
  };
  await Promise.all(Array.from({ length: Math.min(n, jobs.length) }, worker));
  return out;
}

type Scope = Pick<UsageQuery, "agent" | "host" | "account" | "project">;

/** `all`, `agent:claude`, `host:b31`, `account:a3f2c1…`, `project:r:github.com/x/y` → the query's filter. */
function parseFilter(f: string): Scope {
  const [k, ...rest] = f.split(":");
  const v = rest.join(":");
  if (!v) return {};
  return k === "agent" || k === "host" || k === "account" || k === "project" ? { [k]: v } : {};
}

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** The splits the breakup cycles through (⌘G). */
const SPLITS = ["account", "agent", "host", "model", "project"] as const;

/** Usage's screens beyond the totals: the daemon's `usage.explore`, else `aleph usage --explore --json`. */
async function explore(range: RangeId, f: Scope, split: string): Promise<Explore | undefined> {
  const set = Object.entries(f).filter(([, v]) => v) as [string, string][];
  return apiOr<Explore | undefined>(
    {
      method: "usage.explore",
      params: { range, split, ...Object.fromEntries(set) },
      accept: (v) => Array.isArray(v?.breakup),
      timeoutMs: 20_000,
    },
    async () =>
      (
        await runJson<{ explore?: Explore }>(
          [
            "usage",
            range,
            "--by",
            split,
            ...set.flatMap(([k, v]) => [`--${k}`, v]),
            "--explore",
            "--no-sync",
            "--json",
          ],
          60_000,
        )
      ).explore,
  );
}

export async function load(range: RangeId, f: Scope): Promise<Loaded> {
  let days: UsageReport;
  try {
    // the first call may read the machines whose logs are due (the CLI; the daemon reads them by itself)
    days = await usageQuery({ range, by: "day", ...f }, true);
  } catch (e) {
    if (e instanceof CliError && e.unsupported) {
      return {
        range,
        days: [],
        agents: [],
        models: [],
        hosts: [],
        projects: [],
        perAgent: {},
        fromDaemon: false,
        unpriced: [],
        errors: [],
        missing: true,
      };
    }
    throw e;
  }
  const first = (r: UsageReport) => r.rows[0];
  const [today, week, month, total, agents, models, hosts, projects, accounts] = await pool([
    () => usageQuery({ range: "today", by: "total", ...f }),
    () => usageQuery({ range: "7d", by: "total", ...f }),
    () => usageQuery({ range: "30d", by: "total", ...f }),
    () => usageQuery({ range, by: "total", ...f }),
    () => usageQuery({ range, by: "agent", ...f }),
    () => usageQuery({ range, by: "model", ...f }),
    () => usageQuery({ range, by: "host", ...f }),
    () => usageQuery({ range, by: "project", ...f }),
    // older builds have no accounts: none, then
    () => usageQuery({ range, by: "account", ...f }).catch(() => ({ rows: [] }) as unknown as UsageReport),
  ]);
  const ids = [...agents.rows]
    .sort((a, b) => b.tokens - a.tokens || b.cost - a.cost)
    .slice(0, AGENT_DAYS)
    .map((r) => r.key);
  const each = await pool(ids.map((id) => () => usageQuery({ range, by: "day", ...f, agent: id })));
  const reports = [days, agents, models, hosts, projects];
  // what each machine's logs gave (older builds: none)
  const sources = await apiOr<Sources | undefined>(
    { method: "usage.sources", accept: (v) => Array.isArray(v?.hosts) },
    () => runJson<Sources>(["usage", "sources", "--json"]),
  ).catch(() => undefined);
  return {
    range,
    today: first(today),
    week: first(week),
    month: first(month),
    total: first(total),
    days: days.rows,
    agents: agents.rows,
    models: models.rows,
    hosts: hosts.rows,
    projects: projects.rows,
    accounts: accounts.rows,
    perAgent: Object.fromEntries(ids.map((id, i) => [id, each[i].rows])),
    fromDaemon: days.enabled === undefined,
    unpriced: [...new Set(reports.flatMap((r) => r.unpriced_models ?? []))],
    errors: days.errors ?? [],
    limits: days.limits ?? total.limits,
    sources,
  };
}

interface Choice {
  key: string;
  label: string;
  agent?: string;
}

/** Every agent, account, machine and project in the range, unfiltered: what the dropdown offers. */
async function choices(
  range: RangeId,
): Promise<{ agents: string[]; hosts: string[]; accounts: Choice[]; projects: Choice[] }> {
  const none = { rows: [] } as unknown as UsageReport;
  const [a, h, c, p] = await Promise.all([
    usageQuery({ range, by: "agent" }),
    usageQuery({ range, by: "host" }),
    usageQuery({ range, by: "account" }).catch(() => none),
    usageQuery({ range, by: "project" }).catch(() => none),
  ]);
  return {
    agents: a.rows.map((r) => r.key),
    hosts: h.rows.map((r) => r.key),
    accounts: c.rows.filter((r) => r.key).map((r) => ({ key: r.key, label: r.label || r.key, agent: r.agents?.[0] })),
    projects: p.rows
      .filter((r) => r.key)
      .slice(0, 40)
      .map((r) => ({ key: r.key, label: r.label || projectName(r.key).name })),
  };
}

/** Your own dates: from–to (to: today when left as it is). */
function CustomRange({ onPick }: { onPick: (r: RangeId) => void }) {
  const { pop } = useNavigation();
  const now = new Date();
  return (
    <Form
      navigationTitle="Usage · your own dates"
      actions={
        <ActionPanel>
          <Action.SubmitForm
            title="Show These Days"
            icon={Icon.Calendar}
            onSubmit={(v: { from: Date | null; to: Date | null }) => {
              if (!v.from) return;
              const [a, b] = [dayKey(v.from), v.to ? dayKey(v.to) : dayKey(now)];
              onPick(a <= b ? `${a}..${b}` : `${b}..${a}`);
              pop();
            }}
          />
        </ActionPanel>
      }
    >
      <Form.DatePicker
        id="from"
        title="From"
        type={Form.DatePicker.Type.Date}
        defaultValue={new Date(now.getFullYear(), now.getMonth(), 1)}
      />
      <Form.DatePicker id="to" title="To" type={Form.DatePicker.Type.Date} defaultValue={now} />
    </Form>
  );
}

const sortBy = (rows: UsageRow[], m: Metric) =>
  [...rows].sort((a, b) => metricOf(b, m) - metricOf(a, m) || a.key.localeCompare(b.key));

function Usage(props: LaunchProps<{ launchContext?: { agent?: string; host?: string } }>) {
  useBrands();
  const [range, setRange] = useCachedState<RangeId>("usage.range", "30d");
  const [chosen, setMetric] = useCachedState<Metric | "">("usage.metric", "");
  const [asTable, setTable] = useCachedState("usage.table", false);
  const [filter, setFilter] = useCachedState<string>("usage.filter", "all");
  const [split, setSplit] = useCachedState<string>("usage.split", "account");
  // the menu bar or Agents opens it on one agent (or machine)
  const launched = props.launchContext;
  useEffect(() => {
    if (launched?.agent) setFilter(`agent:${launched.agent}`);
    else if (launched?.host) setFilter(`host:${launched.host}`);
  }, [launched?.agent, launched?.host]);
  const [search, setSearch] = useState("");
  const [shown, setShown] = useState(PAGE);
  const [selected, setSelected] = useState<string | null>(null);
  const [asked, setAsked] = useState(false);
  const f = parseFilter(filter);
  // showcase mode is part of each cache key: what was cached unmasked never shows masked
  const sm = showcaseMode();
  const { data, isLoading, error, revalidate } = useCachedPromise(
    (r: RangeId, scope: string, _m: Mode) => load(r, parseFilter(scope)),
    [range, filter, sm],
    {
      keepPreviousData: true,
      failureToastOptions: { title: "Couldn't read usage" },
    },
  );
  // the dropdown's choices; the main load already says when usage can't be read
  const options = useCachedPromise((r: RangeId, _m: Mode) => choices(r), [range, sm], {
    keepPreviousData: true,
    onError: () => undefined,
  });
  // what an older build cached held agents and machines only: the lists added since start empty
  const picks = options.data && {
    ...options.data,
    accounts: options.data.accounts ?? [],
    projects: options.data.projects ?? [],
  };
  // the breakup and the insights: read once one of them is selected
  const wantsMore = selected === "breakup" || selected === "insights";
  const more = useCachedPromise(
    (r: RangeId, scope: string, sp: string, _m: Mode) => explore(r, parseFilter(scope), sp),
    [range, filter, split, sm],
    { execute: wantsMore, keepPreviousData: true, onError: () => undefined },
  );
  const settings = useSettings();
  const fleet = useFleet();
  const hostColor = (h: string) => fleet.data?.hosts.find((x) => x.alias === h)?.color || undefined;
  const analyticsOn = setting(settings.data, "analytics.enabled") !== "false";
  // cost when anything had a price, else tokens (until you pick one)
  const metric: Metric = chosen || ((data?.month?.cost ?? 0) > 0 ? "cost" : "tokens");
  const dark = environment.appearance !== "light";
  const t = chartTheme(dark);
  const logo = cardLogos(dark);

  // pushed: again as soon as the daemon's minute-by-minute numbers move
  useEffect(() => listen(["usage"], () => revalidate()), [revalidate]);
  // live: again LIVE_MS after each answer (a minute while the daemon pushes);
  // the loading bar only for the first and for a Refresh you asked for
  useEffect(() => {
    if (isLoading) return;
    setAsked(false);
    const tm = setTimeout(revalidate, isLive() ? 60_000 : LIVE_MS);
    return () => clearTimeout(tm);
  }, [isLoading, data, revalidate]);
  // a new range or filter starts at the first page
  useEffect(() => setShown(PAGE), [range, filter]);

  // the selected row's days, when they weren't read with the rest
  const sel = selected && selected !== "overview" ? (selected.split(/:(.*)/s) as [Kind, string]) : undefined;
  const needDays =
    !!sel && !!data && !(sel[0] === "agent" && data.perAgent[sel[1]]) && !(sel[0] === "project" && !sel[1]);
  const itemDays = useCachedPromise(
    async (r: RangeId, scope: string, kind: string, key: string, _m: Mode) =>
      (
        await usageQuery({
          range: r,
          by: "day",
          ...parseFilter(scope),
          [kind]: key,
        })
      ).rows,
    [range, filter, sel?.[0] ?? "", sel?.[1] ?? "", sm],
    { execute: needDays, keepPreviousData: false, onError: () => undefined },
  );

  const accountName = (id: string) =>
    picks?.accounts.find((a) => a.key === id)?.label ??
    data?.accounts?.find((a) => a.key === id)?.label ??
    id.slice(0, 6);
  const filterName = f.agent
    ? brand(f.agent).name
    : f.host
      ? f.host
      : f.account
        ? accountName(f.account)
        : f.project
          ? (picks?.projects.find((p) => p.key === f.project)?.label ?? projectName(f.project).name)
          : "";
  const rangeTitle = span(range).title;
  const projectOrder = useMemo(() => sortBy(data?.projects ?? [], metric).map((r) => r.key), [data, metric]);
  // every model and project its own colour, the same in the list, the charts and the chips
  const colors = useMemo(
    () => ({
      model: keyColors(
        (data?.models ?? []).map((r) => r.key),
        dark,
      ),
      project: keyColors(
        (data?.projects ?? []).map((r) => projectName(r.key).name),
        dark,
      ),
      account: keyColors(
        (data?.accounts ?? []).map((r) => r.key),
        dark,
      ),
    }),
    [data, dark],
  );
  const modelColor = (k: string) => colors.model.get(k);
  const projectColor = (name: string) => colors.project.get(name);
  const accountColor = (id: string) => colors.account.get(id);
  const opts = { m: metric, t, logo, brand, hostColor, projectOrder, modelColor, projectColor, accountColor };
  // a part of the breakup: an account's own colour, an agent's brand, a machine's, a model's, a project's
  const partColor = (key: string, label: string) =>
    split === "account"
      ? (accountColor(key) ?? t.other)
      : split === "agent"
        ? brand(key).color
        : split === "host"
          ? (hostColor(key) ?? t.other)
          : split === "model"
            ? (modelColor(key) ?? t.other)
            : (projectColor(label) ?? t.muted);
  const breakupMd = useMemo(
    () =>
      selected === "breakup"
        ? more.data
          ? exploreMarkdown(more.data, { t, colorOf: partColor })
          : more.isLoading
            ? "_Reading the breakup…_"
            : "_This aleph can't break the cost up yet: update it (`aleph self-update`)._"
        : "",
    [selected === "breakup", more.data, more.isLoading, dark, split],
  );
  const insightsMd = useMemo(
    () =>
      selected === "insights"
        ? more.data
          ? insightsMarkdown(more.data, { t, colorOf: partColor, brandName: (id) => brand(id).name })
          : more.isLoading
            ? "_Working the insights out…_"
            : "_This aleph has no insights yet: update it (`aleph self-update`)._"
        : "",
    [selected === "insights", more.data, more.isLoading, dark],
  );
  const cards = { t, logo, brand, hostColor };
  // the two cards are drawn once per answer, and only while their row is selected
  const limitsMd = useMemo(
    () => (selected === "limits" ? limitsMarkdown(data?.limits, cards) : ""),
    [selected === "limits", data, dark],
  );
  const sourcesMd = useMemo(
    () => (selected === "sources" ? sourcesMarkdown(data?.sources, cards) : ""),
    [selected === "sources", data, dark],
  );

  const rows = useMemo(() => {
    if (!data || data.missing) return undefined;
    return {
      agents: f.agent ? [] : sortBy(data.agents, metric),
      models: sortBy(data.models, metric),
      hosts: f.host ? [] : sortBy(data.hosts, metric),
      projects: sortBy(data.projects, metric),
      // one account that isn't known says nothing: shown when there's more to tell
      accounts: f.account
        ? []
        : (data.accounts ?? []).length > 1 || (data.accounts ?? []).some((r) => r.key)
          ? sortBy(data.accounts ?? [], metric)
          : [],
    };
  }, [data, metric, f.agent, f.host, f.account]);

  function overviewMd(): string {
    if (!data)
      return error ? `## Couldn't read usage\n\n${errorText(error).title}\n\n${errorText(error).message ?? ""}` : "";
    if (data.missing) return "## This aleph has no `usage` command yet\n\nUpdate Aleph (`aleph self-update`).";
    if (!data.month?.tokens && !data.agents.length && !data.total?.tokens) {
      return [
        filterName ? `## Nothing for ${filterName} in this range` : "## No usage yet",
        "`aleph usage sync` reads every machine's logs now.",
        notes(data, analyticsOn),
      ]
        .filter(Boolean)
        .join("\n\n");
    }
    const body = asTable
      ? tablesMarkdown(data, metric, (id) => brand(id).name, filterName)
      : overviewMarkdown(data, { ...opts, filter: filterName });
    const todo = notes(data, analyticsOn);
    return todo ? `${body}\n\n---\n\n${todo}` : body;
  }

  // Details are drawn for the selected row and its neighbours only (every
  // project's SVGs on every render would be megabytes for Raycast), and kept
  // until the numbers, the metric, the range, the filter or the theme change.
  const drawn = useRef<{ key: unknown; md: Map<string, string> }>({ key: undefined, md: new Map() });
  const generation = useMemo(() => ({}), [data, metric, range, filter, dark, asTable]);
  if (drawn.current.key !== generation) drawn.current = { key: generation, md: new Map() };
  const projects = rows?.projects ?? [];
  const visible = search ? projects : projects.slice(0, shown);
  const order = [
    "overview",
    ...(rows
      ? [
          ...rows.agents.map((r) => `agent:${r.key}`),
          ...rows.accounts.map((r) => `account:${r.key}`),
          ...rows.models.map((r) => `model:${r.key}`),
          ...rows.hosts.map((r) => `host:${r.key}`),
        ]
      : []),
    ...visible.map((r) => `project:${r.key}`),
  ];
  const at = Math.max(0, order.indexOf(selected ?? "overview"));
  const near = new Set(order.slice(Math.max(0, at - 1), at + 2));

  function draw(id: string, kind: Kind | "overview", row?: UsageRow): string {
    if (kind === "overview" || !row || !data) return overviewMd();
    if (asTable) {
      const head = kind === "host" ? "Machine" : kind[0].toUpperCase() + kind.slice(1);
      return table(head, [row], () => title(kind, row), metric, true);
    }
    const mine = id === selected;
    const days =
      kind === "agent" && data.perAgent[row.key]
        ? data.perAgent[row.key]
        : kind === "project" && !row.key
          ? undefined
          : mine && itemDays.data
            ? itemDays.data
            : "loading";
    return itemMarkdown({ ...opts, kind, row, total: data.total, range, days, logoId: heroLogo(kind, row.key, row) });
  }

  function detailFor(id: string, kind: Kind | "overview", row?: UsageRow): string {
    const kept = drawn.current.md.get(id);
    if (kept !== undefined && !(id === selected && kept.includes("_Reading its days…_") && itemDays.data)) return kept;
    if (!near.has(id)) return "";
    const md = draw(id, kind, row);
    // a neighbour still waiting for its days is drawn again once it's selected
    if (!md.includes("_Reading its days…_") || id === selected) drawn.current.md.set(id, md);
    return md;
  }

  const actions = (kind: Kind | "overview", row?: UsageRow) => (
    <ActionPanel>
      <ActionPanel.Section>
        <Action
          title={metric === "cost" ? "Show Tokens" : "Show Cost"}
          icon={metric === "cost" ? Icon.Hashtag : Icon.Coins}
          shortcut={{ modifiers: ["cmd"], key: "t" }}
          onAction={() => setMetric(metric === "cost" ? "tokens" : "cost")}
        />
        {kind === "overview" ? (
          <Action
            title={`Split by ${cap(splitName(SPLITS[(SPLITS.indexOf(split as (typeof SPLITS)[number]) + 1) % SPLITS.length]))}`}
            icon={Icon.PieChart}
            shortcut={{ modifiers: ["cmd"], key: "g" }}
            onAction={() => setSplit(SPLITS[(SPLITS.indexOf(split as (typeof SPLITS)[number]) + 1) % SPLITS.length])}
          />
        ) : null}
        <Action
          title={asTable ? "Show Charts" : "Show Numbers"}
          icon={asTable ? Icon.BarChart : Icon.List}
          shortcut={{ modifiers: ["cmd"], key: "l" }}
          onAction={() => setTable(!asTable)}
        />
        <Action
          title="Refresh"
          icon={Icon.ArrowClockwise}
          shortcut={Keyboard.Shortcut.Common.Refresh}
          onAction={() => {
            setAsked(true);
            revalidate();
            options.revalidate();
          }}
        />
      </ActionPanel.Section>
      <ActionPanel.Section title="Show">
        {kind === "agent" && row ? (
          <Action
            title={`Only ${brand(row.key).name}`}
            icon={Icon.Filter}
            shortcut={{ modifiers: ["cmd", "shift"], key: "f" }}
            onAction={() => setFilter(`agent:${row.key}`)}
          />
        ) : null}
        {kind === "host" && row ? (
          <Action
            title={`Only ${row.key}`}
            icon={Icon.Filter}
            shortcut={{ modifiers: ["cmd", "shift"], key: "f" }}
            onAction={() => setFilter(`host:${row.key}`)}
          />
        ) : null}
        {kind === "account" && row?.key ? (
          <Action
            title={`Only ${row.label || "This Account"}`}
            icon={Icon.Filter}
            shortcut={{ modifiers: ["cmd", "shift"], key: "f" }}
            onAction={() => setFilter(`account:${row.key}`)}
          />
        ) : null}
        {kind === "project" && row?.key ? (
          <Action
            title={`Only ${projectName(row.key).name}`}
            icon={Icon.Filter}
            shortcut={{ modifiers: ["cmd", "shift"], key: "f" }}
            onAction={() => setFilter(`project:${row.key}`)}
          />
        ) : null}
        {filter !== "all" ? (
          <Action
            title="Everything"
            icon={Icon.XMarkCircle}
            shortcut={{ modifiers: ["cmd", "shift"], key: "a" }}
            onAction={() => setFilter("all")}
          />
        ) : null}
      </ActionPanel.Section>
      <ActionPanel.Section title="Range">
        {RANGES.map((r, i) => (
          <Action
            key={r.id}
            title={r.title}
            icon={r.id === range ? Icon.CheckCircle : Icon.Circle}
            shortcut={{ modifiers: ["cmd"], key: (["1", "2", "3", "4", "5", "6", "7", "8", "9"] as const)[i] }}
            onAction={() => setRange(r.id)}
          />
        ))}
        <Action.Push
          title={range.includes("..") ? `Your Own Dates (${rangeTitle})…` : "Your Own Dates…"}
          icon={range.includes("..") ? Icon.CheckCircle : Icon.Calendar}
          shortcut={{ modifiers: ["cmd"], key: "0" }}
          target={<CustomRange onPick={setRange} />}
        />
      </ActionPanel.Section>
      <ActionPanel.Section>
        {data && !data.missing ? (
          <CopyAction
            title="Copy as Tables"
            content={tablesMarkdown(data, metric, (id) => brand(id).name, filterName)}
            shortcut={Keyboard.Shortcut.Common.Copy}
          />
        ) : null}
        {kind === "project" && row?.key ? (
          <CopyAction title="Copy Path" content={row.key} shortcut={Keyboard.Shortcut.Common.CopyPath} />
        ) : null}
        <Action
          title="Open in Terminal"
          icon={Icon.Terminal}
          shortcut={{ modifiers: ["cmd", "shift"], key: "t" }}
          onAction={() =>
            openInTerminal(
              [
                "usage",
                range,
                ...(f.agent ? ["--agent", f.agent] : []),
                ...(f.host ? ["--host", f.host] : []),
                ...(f.account ? ["--account", f.account] : []),
                ...(f.project ? ["--project", f.project] : []),
                ...(kind === "project" && row?.key ? ["--project", row.key] : []),
                ...(kind === "model" && row ? ["--model", row.key] : []),
              ],
              "Aleph · usage",
            )
          }
        />
      </ActionPanel.Section>
    </ActionPanel>
  );

  const item = (kind: Kind, r: UsageRow) => {
    const id = `${kind}:${r.key}`;
    const share = shareOf(r, data?.total, metric);
    const color = fit(rowColor(kind, r.key, opts), t);
    const named = kind === "project" ? projectName(r.key) : undefined;
    return (
      <List.Item
        key={id}
        id={id}
        icon={rowIcon(kind, r.key, opts.hostColor, color, r)}
        title={title(kind, r)}
        subtitle={
          named ? [named.worktree ? `⑂ ${named.worktree}` : "", named.parent].filter(Boolean).join(" · ") : undefined
        }
        keywords={kind === "project" ? [r.key, ...(named?.parent ?? "").split("/")] : [r.key, r.label]}
        accessories={[
          {
            icon: getProgressIcon(share, color, { background: t.grid, backgroundOpacity: 1 }),
            text: figure(r, metric),
            tooltip: `${Math.round(share * 100)}% of the ${metric === "cost" ? "cost" : "tokens"} · ${money(r.cost, r.unpriced_requests)} · ${compact(r.tokens)} tokens · ${r.sessions} sessions${r.estimated ? ESTIMATED : ""}`,
          },
        ]}
        detail={<List.Item.Detail markdown={withBanner(detailFor(id, kind, r))} />}
        actions={actions(kind, r)}
      />
    );
  };

  return (
    <List
      isLoading={(isLoading && (!data || asked)) || (needDays && itemDays.isLoading)}
      isShowingDetail
      filtering
      onSearchTextChange={setSearch}
      onSelectionChange={(id) => setSelected(id ?? null)}
      navigationTitle={navTitle(["Aleph Usage", rangeTitle, filterName].filter(Boolean).join(" · "))}
      searchBarPlaceholder="Find an agent, model, machine or project"
      searchBarAccessory={
        <List.Dropdown tooltip="Only one agent, account, machine or project" value={filter} onChange={setFilter}>
          <List.Dropdown.Item title="Everything" value="all" icon={ALEPH_ICON} />
          <List.Dropdown.Section title="Agent">
            {(picks?.agents ?? (f.agent ? [f.agent] : [])).map((a) => (
              <List.Dropdown.Item key={a} title={brand(a).name} value={`agent:${a}`} icon={agentLogo(a)} />
            ))}
          </List.Dropdown.Section>
          {picks?.accounts.length ? (
            <List.Dropdown.Section title="Account">
              {picks.accounts.map((a) => (
                <List.Dropdown.Item
                  key={a.key}
                  title={a.label}
                  value={`account:${a.key}`}
                  icon={a.agent ? agentLogo(a.agent) : Icon.Person}
                />
              ))}
            </List.Dropdown.Section>
          ) : null}
          <List.Dropdown.Section title="Machine">
            {(picks?.hosts ?? (f.host ? [f.host] : [])).map((h) => (
              <List.Dropdown.Item key={h} title={h} value={`host:${h}`} icon={rowIcon("host", h, hostColor)} />
            ))}
          </List.Dropdown.Section>
          {picks?.projects.length ? (
            <List.Dropdown.Section title="Project">
              {picks.projects.map((p) => (
                <List.Dropdown.Item key={p.key} title={p.label} value={`project:${p.key}`} icon={Icon.Folder} />
              ))}
            </List.Dropdown.Section>
          ) : null}
        </List.Dropdown>
      }
      pagination={
        search || !rows
          ? undefined
          : { pageSize: PAGE, hasMore: projects.length > shown, onLoadMore: () => setShown((n) => n + PAGE) }
      }
    >
      <List.Section title="Overview">
        <List.Item
          id="overview"
          icon={ALEPH_ICON}
          title={rangeTitle}
          subtitle={filterName || undefined}
          keywords={["overview", "total", "today"]}
          accessories={
            data?.total
              ? [
                  {
                    text: figure(data.total, metric),
                    tooltip: `${compact(data.total.tokens)} tokens · ${data.total.sessions} sessions${data.total.estimated ? ESTIMATED : ""}`,
                  },
                ]
              : []
          }
          detail={<List.Item.Detail markdown={withBanner(detailFor("overview", "overview"))} />}
          actions={actions("overview")}
        />
        <List.Item
          id="breakup"
          icon={Icon.PieChart}
          title="Cost breakup & trends"
          subtitle={`by ${splitName(split)}`}
          keywords={["breakup", "split", "accounts", "compare", "trend", "heatmap", "calendar", "week"]}
          detail={
            <List.Item.Detail
              isLoading={more.isLoading}
              markdown={selected === "breakup" ? withBanner(breakupMd) : ""}
            />
          }
          actions={actions("overview")}
        />
        <List.Item
          id="insights"
          icon={Icon.LightBulb}
          title="Insights"
          subtitle="per prompt · per session · tools · plans"
          keywords={["insights", "stats", "prompts", "sessions", "tools", "plans", "streak", "costliest"]}
          detail={
            <List.Item.Detail
              isLoading={more.isLoading}
              markdown={selected === "insights" ? withBanner(insightsMd) : ""}
            />
          }
          actions={actions("overview")}
        />
        <List.Item
          id="limits"
          icon={Icon.Gauge}
          title="Plan limits"
          keywords={["limits", "quota", "plan", "reset"]}
          accessories={limitAccessory(data?.limits)}
          detail={<List.Item.Detail markdown={selected === "limits" ? withBanner(limitsMd) : ""} />}
          actions={actions("overview")}
        />
        <List.Item
          id="sources"
          icon={Icon.HardDrive}
          title="Sources"
          keywords={["sources", "logs", "files", "read"]}
          accessories={
            data?.sources
              ? [
                  {
                    text: `${data.sources.hosts.length} machines · ${data.sources.hosts.reduce((n, h) => n + h.files_tracked, 0)} files`,
                  },
                ]
              : []
          }
          detail={<List.Item.Detail markdown={selected === "sources" ? withBanner(sourcesMd) : ""} />}
          actions={actions("overview")}
        />
      </List.Section>
      {rows?.agents.length ? (
        <List.Section title="Agents" subtitle={String(rows.agents.length)}>
          {rows.agents.map((r) => item("agent", r))}
        </List.Section>
      ) : null}
      {rows?.accounts.length ? (
        <List.Section title="Accounts" subtitle={String(rows.accounts.length)}>
          {rows.accounts.map((r) => item("account", r))}
        </List.Section>
      ) : null}
      {rows?.models.length ? (
        <List.Section title="Models" subtitle={String(rows.models.length)}>
          {rows.models.map((r) => item("model", r))}
        </List.Section>
      ) : null}
      {rows?.hosts.length ? (
        <List.Section title="Machines" subtitle={String(rows.hosts.length)}>
          {rows.hosts.map((r) => item("host", r))}
        </List.Section>
      ) : null}
      {projects.length ? (
        <List.Section
          title="Projects"
          subtitle={
            search || projects.length <= shown
              ? String(projects.length)
              : `${Math.min(shown, projects.length)} of ${projects.length}`
          }
        >
          {visible.map((r) => item("project", r))}
        </List.Section>
      ) : null}
    </List>
  );
}

/** How a row is named: an agent by its brand, a project by its folder, an account by its label. */
function title(kind: Kind, r: UsageRow): string {
  if (kind === "agent") return brand(r.key).name;
  if (kind === "account") return r.label || "account unknown";
  if (kind === "project") return projectName(r.key).name;
  if (kind === "model") return r.label || r.key || "(unknown)";
  return r.key;
}

/** The logo the detail's card opens with. */
function heroLogo(kind: Kind, key: string, row?: UsageRow): { kind: "agent" | "os"; id: string } | undefined {
  if (kind === "agent") return agentLogoId(key) ? { kind: "agent", id: agentLogoId(key)! } : undefined;
  if (kind === "account") {
    const a = row?.agents?.[0];
    return a && agentLogoId(a) ? { kind: "agent", id: agentLogoId(a)! } : undefined;
  }
  if (kind === "model") {
    const a = vendorAgent(key);
    return a && agentLogoId(a) ? { kind: "agent", id: agentLogoId(a)! } : undefined;
  }
  if (kind === "host") {
    const os = osId(machineOsText(key));
    return os ? { kind: "os", id: os.id } : undefined;
  }
  return undefined;
}

function rowIcon(
  kind: Kind,
  key: string,
  hostColor: (h: string) => string | undefined,
  own?: string,
  row?: UsageRow,
): Image.ImageLike {
  if (kind === "agent") return agentLogo(key);
  if (kind === "account") return row?.agents?.[0] ? agentLogo(row.agents[0]) : { source: Icon.Person, tintColor: own };
  if (kind === "model") {
    const a = vendorAgent(key);
    return a ? agentLogo(a) : { source: Icon.Stars, tintColor: "#a0a0a0" };
  }
  if (kind === "host")
    return machineLogo(key)?.icon ?? { source: Icon.CircleFilled, tintColor: hostColor(key) ?? "#a0a0a0" };
  return { source: projectName(key).worktree ? Icon.Tree : Icon.Folder, tintColor: own };
}

// blank while showcase mode pauses every window
export default pausable(Usage);
