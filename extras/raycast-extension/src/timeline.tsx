// Timeline: what an agent did, conversation by conversation (`timeline.recent`
// / `timeline.get`, or `aleph timeline … --json`). The list: the latest
// conversations on every machine, each with its project, prompts and cost;
// ↵ opens one as its tree — prompts, turns, tool calls and subagents, each
// with time, tokens and $ — folded the way the dashboard folds it (smart:
// finished prompts to a line, repeats as "Read + 7 more"). ⌃F switches the
// folding, ⌘] opens everything (⌘[ folds it), ⌘⇧P opens or folds one
// prompt, ⌘⇧W shows the words (prompts' first lines, tools' targets: read
// from the machine now, never kept). Names and numbers otherwise.
import { Action, ActionPanel, Detail, Icon, Keyboard, List } from "@raycast/api";
import { useCachedPromise } from "@raycast/utils";
import { useState } from "react";
import { pausable } from "./lib/pause";
import { CopyAction } from "./lib/copy";
import { apiOr } from "./lib/api";
import { runJson } from "./lib/cli";
import { useBrands } from "./lib/hooks";
import { agentLogo } from "./lib/logos";
import { navTitle, withBanner } from "./lib/showcase";
import { openInTerminal } from "./lib/terminal";
import { brand } from "./lib/ui";
import { compact, money } from "./lib/usage";
import { folded, Fold, nextFold, TlRecent, TlTree, took, treeMarkdown } from "./lib/timeline-view";

/** A conversation still being worked on: anything in its last two minutes. */
const LIVE_MS = 120_000;

async function recent(): Promise<TlRecent[]> {
  return apiOr<TlRecent[]>({ method: "timeline.recent", params: { n: 60 }, accept: Array.isArray }, () =>
    runJson<TlRecent[]>(["timeline", "ls", "--json", "-n", "60"]),
  );
}

/** One conversation's tree, folded `fold` with `open`'s rows as you set them; its words when asked. */
async function tree(c: TlRecent, fold: Fold, open: Record<string, boolean>, words: boolean): Promise<TlTree> {
  const running = Date.now() - c.last_ms < LIVE_MS;
  const t = await apiOr<TlTree>(
    {
      method: "timeline.get",
      params: { session: c.session, host: c.host, fold, open, running },
      accept: (v) => Array.isArray(v?.rows),
      timeoutMs: 15_000,
    },
    () => runJson<TlTree>(["timeline", c.session, "--host", c.host, "--fold", fold, "--json"], 30_000),
  );
  if (words && !t.words) {
    t.words = await apiOr<Record<string, string>>(
      { method: "timeline.words", params: { session: c.session, host: c.host }, timeoutMs: 25_000 },
      async () =>
        (await runJson<TlTree>(["timeline", c.session, "--host", c.host, "--words", "--json"], 30_000)).words ?? {},
    ).catch(() => ({}));
  }
  return t;
}

const ago = (ms: number) => {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  return s < 60
    ? "just now"
    : s < 3600
      ? `${Math.floor(s / 60)}m ago`
      : s < 86_400
        ? `${Math.floor(s / 3600)}h ago`
        : `${Math.floor(s / 86_400)}d ago`;
};

const titleOf = (c: TlRecent) => c.project || `${brand(c.agent).name} · ${c.session.slice(0, 8)}`;

function Conversation({ c }: { c: TlRecent }) {
  const [fold, setFold] = useState<Fold>("smart");
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [words, setWords] = useState(false);
  const { data, isLoading, revalidate } = useCachedPromise(
    (cc: TlRecent, f: Fold, o: Record<string, boolean>, w: boolean) => tree(cc, f, o, w),
    [c, fold, open, words],
    { keepPreviousData: true, failureToastOptions: { title: "Couldn't read its timeline" } },
  );
  const rows = data?.rows ?? [];
  const prompts = rows.filter((r) => r.kind === "prompt");
  // everything open: what's folded now, then what that opens, a level at a time
  const openAll = async () => {
    let o = { ...open };
    for (let i = 0; i < 5; i++) {
      const t = await tree(c, fold, o, false);
      const next = folded(t.rows);
      if (!next.length) break;
      o = { ...o, ...Object.fromEntries(next.map((id) => [id, true])) };
    }
    setOpen(o);
  };
  const md = data
    ? treeMarkdown(data, { title: titleOf(c), fold, words, agentName: brand(c.agent).name })
    : `## ${titleOf(c)}\n\n_Reading its timeline…_`;
  return (
    <Detail
      isLoading={isLoading}
      navigationTitle={navTitle(`Timeline · ${titleOf(c)}`)}
      markdown={withBanner(md)}
      actions={
        <ActionPanel>
          <ActionPanel.Section>
            <Action
              title={`Fold: ${nextFold(fold)}`}
              icon={Icon.List}
              shortcut={{ modifiers: ["ctrl"], key: "f" }}
              onAction={() => {
                setOpen({});
                setFold(nextFold(fold));
              }}
            />
            <Action
              title="Open Everything"
              icon={Icon.ChevronDown}
              shortcut={{ modifiers: ["cmd"], key: "]" }}
              onAction={openAll}
            />
            <Action
              title="Fold Everything"
              icon={Icon.ChevronUp}
              shortcut={{ modifiers: ["cmd"], key: "[" }}
              onAction={() => {
                setOpen({});
                setFold("folded");
              }}
            />
            {prompts.length ? (
              <ActionPanel.Submenu
                title="Open or Fold a Prompt"
                icon={Icon.Text}
                shortcut={{ modifiers: ["cmd", "shift"], key: "p" }}
              >
                {prompts.map((p) => (
                  <Action
                    key={p.id}
                    title={`${p.folded ? "Open" : "Fold"} ${p.label}`}
                    icon={p.folded ? Icon.ChevronRight : Icon.ChevronDown}
                    onAction={() => setOpen({ ...open, [p.id]: p.folded })}
                  />
                ))}
              </ActionPanel.Submenu>
            ) : null}
            <Action
              title={words ? "Hide Words" : "Show Words"}
              icon={Icon.Eye}
              shortcut={{ modifiers: ["cmd", "shift"], key: "w" }}
              onAction={() => setWords(!words)}
            />
            <Action
              title="Refresh"
              icon={Icon.ArrowClockwise}
              shortcut={Keyboard.Shortcut.Common.Refresh}
              onAction={revalidate}
            />
          </ActionPanel.Section>
          <ActionPanel.Section>
            <Action
              title="Open in Terminal"
              icon={Icon.Terminal}
              shortcut={{ modifiers: ["cmd", "shift"], key: "t" }}
              onAction={() => openInTerminal(["timeline", c.session, "--host", c.host], "Aleph · timeline")}
            />
            <CopyAction title="Copy Conversation Id" content={c.session} shortcut={Keyboard.Shortcut.Common.Copy} />
          </ActionPanel.Section>
        </ActionPanel>
      }
    />
  );
}

function Timeline() {
  useBrands();
  const { data, isLoading, revalidate } = useCachedPromise(recent, [], {
    keepPreviousData: true,
    failureToastOptions: { title: "Couldn't list the conversations" },
  });
  const list = data ?? [];
  // by day: today, yesterday, then the date
  const dayOf = (ms: number) => {
    const d = new Date(ms);
    const today = new Date();
    const y = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
    if (d.toDateString() === today.toDateString()) return "Today";
    if (d.toDateString() === y.toDateString()) return "Yesterday";
    return d.toLocaleDateString("en", { weekday: "short", month: "short", day: "numeric" });
  };
  const days: [string, TlRecent[]][] = [];
  for (const c of list) {
    const k = dayOf(c.last_ms);
    const g = days.find((x) => x[0] === k);
    if (g) g[1].push(c);
    else days.push([k, [c]]);
  }
  return (
    <List
      isLoading={isLoading}
      navigationTitle={navTitle("Aleph Timeline")}
      searchBarPlaceholder="Find a conversation by project, machine or agent"
    >
      {!isLoading && !list.length ? (
        <List.EmptyView
          icon={Icon.List}
          title="No conversations read yet"
          description="Aleph reads each machine's agent logs about once a minute (aleph usage sync reads now)."
        />
      ) : null}
      {days.map(([day, cs]) => (
        <List.Section key={day} title={day} subtitle={String(cs.length)}>
          {cs.map((c) => {
            const live = Date.now() - c.last_ms < LIVE_MS;
            const acc: List.Item.Accessory[] = [];
            if (live) acc.push({ tag: { value: "working", color: "#3B9EFF" } });
            if (c.prompts)
              acc.push({
                text: `${c.prompts} prompt${c.prompts === 1 ? "" : "s"}`,
                tooltip: `${c.requests ?? 0} requests · ${compact(c.tokens ?? 0)} tokens`,
              });
            if (c.cost !== undefined && c.cost !== null)
              acc.push({ text: `${money(c.cost)}${c.estimated ? "*" : ""}` });
            acc.push({
              text: ago(c.last_ms),
              tooltip: c.first
                ? `since ${new Date(c.first * 1000).toLocaleString()} · ${took(c.last_ms - c.first * 1000)}`
                : undefined,
            });
            return (
              <List.Item
                key={`${c.host}|${c.agent}|${c.session}`}
                icon={agentLogo(c.agent)}
                title={titleOf(c)}
                subtitle={[c.host, c.account_label].filter(Boolean).join(" · ")}
                keywords={[c.host, c.agent, brand(c.agent).name, c.session, c.account_label ?? ""]}
                accessories={acc}
                actions={
                  <ActionPanel>
                    <Action.Push title="Show Timeline" icon={Icon.List} target={<Conversation c={c} />} />
                    <Action
                      title="Open in Terminal"
                      icon={Icon.Terminal}
                      shortcut={{ modifiers: ["cmd", "shift"], key: "t" }}
                      onAction={() => openInTerminal(["timeline", c.session, "--host", c.host], "Aleph · timeline")}
                    />
                    <Action
                      title="Refresh"
                      icon={Icon.ArrowClockwise}
                      shortcut={Keyboard.Shortcut.Common.Refresh}
                      onAction={revalidate}
                    />
                    <CopyAction
                      title="Copy Conversation Id"
                      content={c.session}
                      shortcut={Keyboard.Shortcut.Common.Copy}
                    />
                  </ActionPanel>
                }
              />
            );
          })}
        </List.Section>
      ))}
    </List>
  );
}

// blank while showcase mode pauses every window
export default pausable(Timeline);
