// The Inbox's rows grouped and sorted: by day (the timeline), by machine, by
// agent or by what happened; within a group newest first, or by machine,
// agent or kind. Groups follow the sort when it's on the same thing (machines
// in the fleet's order, kinds by urgency), else the one with the newest row
// comes first. No Raycast imports: plain node tests it.
import type { TimelineItem } from "./model";

export type GroupBy = "day" | "machine" | "agent" | "kind";
export type SortBy = "time" | "machine" | "agent" | "kind";

export const GROUPS: { id: GroupBy; title: string }[] = [
  { id: "machine", title: "Machine" },
  { id: "agent", title: "Agent" },
  { id: "kind", title: "Kind" },
  { id: "day", title: "Day" },
];

export const SORTS: { id: SortBy; title: string }[] = [
  { id: "machine", title: "Machine" },
  { id: "agent", title: "Agent" },
  { id: "kind", title: "Kind" },
  { id: "time", title: "Newest First" },
];

/** What blocks work first, then what happened, then housekeeping. */
export const KIND_ORDER = ["blocked", "limit", "error", "remote", "auto", "task", "done", "new", "gone", "heal", "up"];

const kindRank = (k: string) => {
  const i = KIND_ORDER.indexOf(k);
  return i < 0 ? KIND_ORDER.length : i;
};

/** The agent a row is about ("" for a machine's own events). */
export const agentOf = (it: TimelineItem) => it.e.agent || it.launch?.agent || "";

/** What a row says happened: a launch on its own counts as started. */
export const kindOf = (it: TimelineItem) => (it.launch && !it.logged ? "new" : it.e.kind);

export interface GroupOpts {
  /** the machines in the fleet's order (others after them, by name) */
  hosts: string[];
  agentName: (id: string) => string;
  /** `needs you`, `finished`, … */
  kindWord: (kind: string) => string;
  /** a day's title: `Today`, `Yesterday`, `Monday, Oct 20` */
  dayOf: (t: number) => string;
}

export interface Group {
  key: string;
  title: string;
  items: TimelineItem[];
}

function hostRank(hosts: string[], h: string): [number, string] {
  const i = hosts.indexOf(h);
  return [i < 0 ? hosts.length : i, h];
}

const byHost = (hosts: string[]) => (a: string, b: string) => {
  const [x, y] = [hostRank(hosts, a), hostRank(hosts, b)];
  return x[0] - y[0] || x[1].localeCompare(y[1]);
};

/** Agents by name; a machine's own events (no agent) last. */
const byAgent = (name: (id: string) => string) => (a: string, b: string) =>
  (a ? 0 : 1) - (b ? 0 : 1) || name(a).localeCompare(name(b));

function compare(sort: SortBy, o: GroupOpts): (a: TimelineItem, b: TimelineItem) => number {
  const newest = (a: TimelineItem, b: TimelineItem) => b.e.t - a.e.t;
  switch (sort) {
    case "machine":
      return (a, b) => byHost(o.hosts)(a.e.host, b.e.host) || newest(a, b);
    case "agent":
      return (a, b) => byAgent(o.agentName)(agentOf(a), agentOf(b)) || newest(a, b);
    case "kind":
      return (a, b) => kindRank(kindOf(a)) - kindRank(kindOf(b)) || newest(a, b);
    default:
      return newest;
  }
}

/** The rows in groups, each group's rows sorted; see the file's comment for the groups' order. */
export function groupItems(items: TimelineItem[], group: GroupBy, sort: SortBy, o: GroupOpts): Group[] {
  const sorted = [...items].sort(compare(sort, o));
  const keyOf = (it: TimelineItem) =>
    group === "machine" ? it.e.host : group === "agent" ? agentOf(it) : group === "kind" ? kindOf(it) : o.dayOf(it.e.t);
  const map = new Map<string, TimelineItem[]>();
  for (const it of sorted) {
    const k = keyOf(it);
    map.set(k, [...(map.get(k) ?? []), it]);
  }
  const latest = (g: TimelineItem[]) => Math.max(...g.map((it) => it.e.t));
  const groups = [...map.entries()];
  const order: (a: [string, TimelineItem[]], b: [string, TimelineItem[]]) => number =
    group === "machine" && sort === "machine"
      ? (a, b) => byHost(o.hosts)(a[0], b[0])
      : group === "agent" && sort === "agent"
        ? (a, b) => byAgent(o.agentName)(a[0], b[0])
        : group === "kind" && sort === "kind"
          ? (a, b) => kindRank(a[0]) - kindRank(b[0])
          : (a, b) => latest(b[1]) - latest(a[1]);
  return groups.sort(order).map(([key, v]) => ({
    key,
    title:
      group === "agent" ? (key ? o.agentName(key) : "Machine events") : group === "kind" ? cap(o.kindWord(key)) : key,
    items: v,
  }));
}

const cap = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);
