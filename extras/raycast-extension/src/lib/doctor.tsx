// The doctor (`aleph doctor --json`, `aleph --host H doctor --json`): this
// computer, the daemon, every machine's ssh, login, herdr and agents, and
// the shortcuts.
//
// The Doctor command (doctor-view.ts) keeps it to one level at a time: a row
// per machine (problems first; This Mac with the daemon and the shortcuts) —
// or, from the dropdown, a row per agent — with its counts as coloured tags
// (failing, warnings; else how many are ok) and every check in its detail as
// an aligned checklist. ↵ on a row opens its checks; typing finds any check.
// A check: a mark (✕ failing, ! warning, a bulb for a tip, ✓ ok; a note
// shows what it's about), a short title, the key fact as a tag in its colour.
// Its detail: a card, why it matters, its fix; ↵ runs a fix that is an Aleph
// command in a terminal tab, the others are copied.
// Machines › Doctor shows one machine's report as cards.
import { Action, ActionPanel, Color, Detail, environment, Icon, Image, Keyboard, List } from "@raycast/api";
import { CopyAction } from "./copy";
import { useCachedState } from "@raycast/utils";
import { ReactNode, useCallback, useMemo, useState } from "react";
import { request } from "./api";
import { chartTheme } from "./charts";
import { DOCTOR_ARGS, DOCTOR_TIMEOUT_MS, onHost, doctorApi } from "./cli";
import {
  AgentDoctor,
  agentDoctorMarkdown,
  agentGroups,
  CheckKind,
  checkMarkdown,
  CheckRow,
  countsText,
  doctorGroups,
  DoctorGroup,
  FixStep,
  groupFacts,
  groupMarkdown,
  KIND_WORD,
  reportMarkdown,
  rowSubtitle,
  shortFact,
} from "./doctor-view";
import { useAleph, useBrands, useFleet } from "./hooks";
import { agentLogo, cardLogos, machineLogo, osLogo } from "./logos";
import { doctorMarkdown, DoctorReport, doctorTally } from "./model";
import { openInTerminal } from "./terminal";
import { brands, DONE, DOWN, MUTED, WARN, WORKING } from "./ui";

const KIND_COLOR: Record<CheckKind, Color.ColorLike> = {
  fail: DOWN,
  warn: WARN,
  tip: WORKING,
  ok: DONE,
  note: MUTED,
};

/** A note shows what it's about (muted); the rest their state. */
const NOTE_ICONS: Record<string, Image.Source> = {
  system: Icon.Monitor,
  sessions: Icon.List,
  "http api": Icon.Globe,
  exposure: Icon.Network,
  wrapper: Icon.Terminal,
  quicklinks: Icon.Link,
  "tab config": Icon.Window,
  Raycast: Icon.CommandSymbol,
  hub: Icon.Bolt,
  socket: Icon.Plug,
  service: Icon.Gear,
  "moshi-hook": Icon.Mobile,
  "ssh-copy-id": Icon.Key,
  "ssh key": Icon.Key,
  "ssh agent": Icon.Key,
  agents: Icon.Person,
  sidebar: Icon.Swatch,
};

function kindIcon(row: CheckRow): Image.ImageLike {
  switch (row.kind) {
    case "fail":
      return { source: Icon.XMarkCircle, tintColor: DOWN };
    case "warn":
      return { source: Icon.Warning, tintColor: WARN };
    case "tip":
      return { source: Icon.LightBulb, tintColor: WORKING };
    case "ok":
      return { source: Icon.CheckCircle, tintColor: DONE };
    default:
      return { source: NOTE_ICONS[row.check.label] ?? Icon.Dot, tintColor: MUTED };
  }
}

/** A group's own icon: a machine's OS logo, this Mac's (its daemon and shortcuts are part of it). */
function groupIcon(g: DoctorGroup, color?: string): Image.ImageLike {
  if (g.machine) return (osLogo(g.os) ?? machineLogo(g.name))?.icon ?? { source: Icon.Desktop, tintColor: color };
  if (g.name === "This Mac") return osLogo("macOS")?.icon ?? Icon.Desktop;
  return { source: Icon.Heartbeat, tintColor: MUTED };
}

/** An agent's id from its name in the report (`Claude Code` → `claude`). */
const agentId = (name: string) => brands().find((b) => b.name === name)?.id;

/** `aleph [--host H] doctor --json`: JSON even when a check fails (it exits 1 then). */
function useDoctor(host?: string) {
  const r = useAleph<DoctorReport | undefined>(onHost(host, DOCTOR_ARGS), {
    optional: "doctor",
    lenient: true,
    timeoutMs: DOCTOR_TIMEOUT_MS,
    // the daemon's last report (its sections arrive as a new run finishes them)
    api: doctorApi(host),
    topics: ["doctor"],
  });
  // a run under way shows as loading; Refresh asks the daemon for a new one
  const { revalidate } = r;
  const again = useCallback(() => {
    request("doctor.get", { ...(host ? { host } : {}), refresh: true }).catch(() => undefined);
    revalidate();
  }, [host, revalidate]);
  return { ...r, isLoading: r.isLoading || !!r.data?.running, revalidate: again };
}

/** Run a fix that is an Aleph command in a terminal tab (you see it run); copy any of them. */
function FixActions({ fixes }: { fixes: FixStep[] }) {
  const runs = fixes.filter((f) => f.run);
  return (
    <>
      {runs.map((f, i) => (
        <Action
          key={`run-${i}`}
          title={runs.length > 1 ? `Run Fix ${i + 1} in Terminal` : "Run Fix in Terminal"}
          icon={Icon.Hammer}
          onAction={() => openInTerminal(f.run!, "Aleph · doctor")}
        />
      ))}
      {fixes.map((f, i) => (
        <CopyAction
          key={`copy-${i}`}
          title={fixes.length > 1 ? `Copy Fix ${i + 1}` : "Copy Fix"}
          content={f.cmd}
          shortcut={i === 0 ? Keyboard.Shortcut.Common.Copy : undefined}
        />
      ))}
    </>
  );
}

/** The first fix Aleph can run for a group's problems. */
const firstRunnable = (rows: CheckRow[]) =>
  rows
    .filter((r) => r.kind === "fail" || r.kind === "warn")
    .flatMap((r) => r.fixes)
    .find((f) => f.run);

/** One machine's (or everything's) report as cards. */
export function DoctorDetail({ host }: { host?: string }) {
  const brandsSeen = useBrands();
  const fleet = useFleet();
  const { data, isLoading, errorInfo, revalidate } = useDoctor(host);
  const title = host ? `Doctor · ${host}` : "Doctor";
  const dark = environment.appearance !== "light";
  const groups = useMemo(() => (data ? doctorGroups(data, agentId) : []), [data, brandsSeen]);
  const text = data ? doctorMarkdown(data, title) : "";
  const md = data
    ? reportMarkdown(groups, {
        t: chartTheme(dark),
        logo: cardLogos(dark),
        color: (name) => fleet.data?.hosts.find((h) => h.alias === name)?.color,
      })
    : errorInfo
      ? `# ${title}\n\n**${errorInfo.title}**\n\n${errorInfo.message ?? ""}`
      : isLoading
        ? `# ${title}\n\n_checking ${host ?? "every machine"} — ssh, login, herdr and the agents…_`
        : `# ${title}\n\n_this aleph has no \`doctor --json\` yet (aleph self-update)_`;
  const first = firstRunnable(groups.flatMap((g) => g.rows));
  return (
    <Detail
      isLoading={isLoading}
      navigationTitle={data ? `${title} · ${doctorTally(data)}` : title}
      markdown={md}
      actions={
        <ActionPanel>
          {first?.run ? (
            <Action
              title="Run the First Fix in Terminal"
              icon={Icon.Hammer}
              onAction={() => openInTerminal(first.run!, "Aleph · doctor")}
            />
          ) : null}
          <Action
            title="Run Doctor in Terminal"
            icon={Icon.Terminal}
            onAction={() => openInTerminal([...(host ? ["--host", host] : []), "doctor"], "Aleph · doctor")}
          />
          {text ? <CopyAction title="Copy Report" content={text} shortcut={Keyboard.Shortcut.Common.Copy} /> : null}
          <Action
            title="Check Again"
            icon={Icon.ArrowClockwise}
            shortcut={Keyboard.Shortcut.Common.Refresh}
            onAction={revalidate}
          />
        </ActionPanel>
      }
    />
  );
}

/** The dropdown: arrange by machine or by agent, or only one machine (`host:b31`). */
type View = "machine" | "agent" | `host:${string}`;

/** The doctor as a list: what needs attention, then a row per machine (or agent); typing finds any check. */
export function DoctorList() {
  const brandsSeen = useBrands();
  const fleet = useFleet();
  const [view, setView] = useCachedState<View>("doctor.view", "machine");
  const host = view.startsWith("host:") ? view.slice(5) : undefined;
  const { data, isLoading, errorInfo, stale, revalidate } = useDoctor(host);
  const [showDetail, setShowDetail] = useState(true);
  const [search, setSearch] = useState("");
  const unsupported = !isLoading && !errorInfo && data === undefined;
  const groups = useMemo(() => (data ? doctorGroups(data, agentId) : []), [data, brandsSeen]);
  const agents = useMemo(
    () => agentGroups(groups, (id) => brands().find((b) => b.id === id)?.name ?? id),
    [groups, brandsSeen],
  );
  const title = data ? `Doctor · ${doctorTally(data)}${stale ? " · cached" : ""}` : "Doctor";
  const report = data ? doctorMarkdown(data, host ? `Aleph doctor · ${host}` : "Aleph doctor") : "";
  const dark = environment.appearance !== "light";
  const colorOf = (name: string) => fleet.data?.hosts.find((h) => h.alias === name)?.color || undefined;
  const colors = (fleet.data?.hosts ?? []).map((h) => `${h.alias}=${h.color}`).join();
  const look = useMemo(() => ({ t: chartTheme(dark), logo: cardLogos(dark) }), [dark]);
  // every detail drawn once per answer (and theme), not on every render
  const details = useMemo(() => {
    const m = new Map<string, string>();
    for (const g of groups) {
      m.set(`group/${g.name}`, groupMarkdown(g, { ...look, color: colorOf(g.name) }));
      for (const row of g.rows) m.set(row.key, checkMarkdown(row, look));
    }
    for (const a of agents) m.set(`agent/${a.agent}`, agentDoctorMarkdown(a, look));
    return m;
  }, [groups, agents, look, colors]);

  const common = (
    <ActionPanel.Section>
      <Action
        title={showDetail ? "Hide Details" : "Show Details"}
        icon={Icon.Sidebar}
        shortcut={{ modifiers: ["cmd", "shift"], key: "d" }}
        onAction={() => setShowDetail((x) => !x)}
      />
      {report ? <CopyAction title="Copy Report" content={report} /> : null}
      <Action
        title="Run Doctor in Terminal"
        icon={Icon.Terminal}
        shortcut={{ modifiers: ["cmd", "shift"], key: "t" }}
        onAction={() => openInTerminal([...(host ? ["--host", host] : []), "doctor"], "Aleph · doctor")}
      />
      <Action
        title="Check Again"
        icon={Icon.ArrowClockwise}
        shortcut={Keyboard.Shortcut.Common.Refresh}
        onAction={revalidate}
      />
    </ActionPanel.Section>
  );
  const checkItem = (row: CheckRow, where?: string) => (
    <CheckItem
      key={row.key}
      row={row}
      where={where}
      showDetail={showDetail}
      common={common}
      markdown={details.get(row.key) ?? ""}
    />
  );
  const checksOf = (name: string, rows: CheckRow[], where = false) => (
    <ChecksList title={name} rows={rows} where={where} details={details} common={common} />
  );

  return (
    <List
      isLoading={isLoading}
      isShowingDetail={showDetail && groups.length > 0}
      navigationTitle={title}
      searchBarPlaceholder="Filter checks by name, machine or state"
      onSearchTextChange={setSearch}
      searchBarAccessory={
        <List.Dropdown tooltip="Arrange" value={view} onChange={(v) => setView(v as View)}>
          <List.Dropdown.Section title="Arrange">
            <List.Dropdown.Item title="By Machine" value="machine" icon={Icon.Network} />
            <List.Dropdown.Item title="By Agent" value="agent" icon={Icon.Person} />
          </List.Dropdown.Section>
          <List.Dropdown.Section title="Only One Machine">
            {(fleet.data?.hosts ?? []).map((h) => (
              <List.Dropdown.Item
                key={h.alias}
                title={h.alias}
                value={`host:${h.alias}`}
                icon={{ source: Icon.CircleFilled, tintColor: h.color || Color.SecondaryText }}
              />
            ))}
          </List.Dropdown.Section>
        </List.Dropdown>
      }
    >
      {unsupported ? (
        <List.EmptyView
          icon={Icon.Info}
          title="This aleph has no `doctor --json` yet"
          description="Update Aleph (aleph self-update)."
        />
      ) : errorInfo && !data ? (
        <List.EmptyView icon={Icon.Warning} title={errorInfo.title} description={errorInfo.message} />
      ) : (
        <List.EmptyView
          icon={Icon.Heartbeat}
          title={isLoading ? "Checking — ssh, login, herdr and the agents on every machine…" : "Nothing to check"}
        />
      )}
      {search ? (
        // typing: every check, by machine
        groups.map((g) => (
          <List.Section key={g.name} title={g.name} subtitle={countsText(g.counts)}>
            {g.rows.map((row) => checkItem(row))}
          </List.Section>
        ))
      ) : (
        <>
          {view === "agent" ? (
            // one level at a time: the agents, each its counts; its machines in its detail, ↵ its checks
            <List.Section title="Agents" subtitle={String(agents.length)}>
              {agents.map((a) => (
                <AgentItem
                  key={`agent/${a.agent}`}
                  a={a}
                  showDetail={showDetail}
                  common={common}
                  markdown={details.get(`agent/${a.agent}`) ?? ""}
                  checks={checksOf(a.name, a.rows)}
                />
              ))}
            </List.Section>
          ) : (
            // the machines (This Mac with the daemon and the shortcuts), each its counts; its checks in its detail
            <List.Section title="Machines" subtitle={String(groups.length)}>
              {groups.map((g) => (
                <GroupItem
                  key={g.name}
                  g={g}
                  color={colorOf(g.name)}
                  showDetail={showDetail}
                  common={common}
                  markdown={details.get(`group/${g.name}`) ?? ""}
                  checks={checksOf(g.name, g.rows)}
                />
              ))}
            </List.Section>
          )}
        </>
      )}
    </List>
  );
}

/** One machine's (or agent's) checks, pushed from its row: problems first, each with its detail. */
function ChecksList({
  title,
  rows,
  where,
  details,
  common,
}: {
  title: string;
  rows: CheckRow[];
  where: boolean;
  details: Map<string, string>;
  common: ReactNode;
}) {
  const [showDetail, setShowDetail] = useState(true);
  return (
    <List
      isShowingDetail={showDetail}
      navigationTitle={`Doctor · ${title}`}
      searchBarPlaceholder={`Filter ${title}'s checks`}
    >
      {rows.map((row) => (
        <CheckItem
          key={row.key}
          row={row}
          where={where ? row.group : undefined}
          showDetail={showDetail}
          common={
            <>
              <ActionPanel.Section>
                <Action
                  title={showDetail ? "Hide Details" : "Show Details"}
                  icon={Icon.Sidebar}
                  shortcut={{ modifiers: ["cmd", "shift"], key: "d" }}
                  onAction={() => setShowDetail((x) => !x)}
                />
              </ActionPanel.Section>
              {common}
            </>
          }
          markdown={details.get(row.key) ?? ""}
        />
      ))}
    </List>
  );
}

/** An agent across the machines: its logo, its counts; its detail a row per machine; ↵ its checks. */
function AgentItem({
  a,
  showDetail,
  common,
  markdown,
  checks,
}: {
  a: AgentDoctor;
  showDetail: boolean;
  common: ReactNode;
  markdown: string;
  checks: ReactNode;
}) {
  const fix = firstRunnable(a.rows);
  const g = { counts: a.counts } as DoctorGroup;
  return (
    <List.Item
      id={`agent/${a.agent}`}
      icon={agentLogo(a.agent)}
      title={a.name}
      subtitle={showDetail ? undefined : a.rows.map((r) => `${r.brief.title} ${KIND_WORD[r.kind]}`).join(" · ")}
      keywords={[a.agent, ...a.rows.map((r) => r.group)]}
      accessories={countTags(g, !showDetail)}
      detail={<List.Item.Detail markdown={markdown} />}
      actions={
        <ActionPanel>
          <ActionPanel.Section title={a.name}>
            <Action.Push title="Show Checks" icon={Icon.List} target={checks} />
            {fix?.run ? (
              <Action
                title="Run the First Fix in Terminal"
                icon={Icon.Hammer}
                shortcut={{ modifiers: ["cmd"], key: "return" }}
                onAction={() => openInTerminal(fix.run!, "Aleph · doctor")}
              />
            ) : null}
          </ActionPanel.Section>
          {common}
        </ActionPanel>
      }
    />
  );
}

/** A group's counts as coloured tags: its problems (or, when there are none, how many are ok). */
function countTags(g: DoctorGroup, all: boolean): List.Item.Accessory[] {
  const n = g.counts;
  const tags: List.Item.Accessory[] = [];
  if (n.fail) tags.push({ tag: { value: `${n.fail} failing`, color: DOWN } });
  if (n.warn) tags.push({ tag: { value: `${n.warn} warning${n.warn === 1 ? "" : "s"}`, color: WARN } });
  if (all && n.tip) tags.push({ tag: { value: `${n.tip} tip${n.tip === 1 ? "" : "s"}`, color: WORKING } });
  if (all || !tags.length) tags.push({ tag: { value: `${n.ok} ok`, color: DONE } });
  return tags.map((x) => ({ ...x, tooltip: countsText(n) }));
}

/** A machine (or This Mac, the daemon, the shortcuts) as a row: its logo, its counts; its detail a card. */
function GroupItem({
  g,
  color,
  showDetail,
  common,
  markdown,
  checks,
}: {
  g: DoctorGroup;
  color?: string;
  showDetail: boolean;
  common: ReactNode;
  markdown: string;
  checks: ReactNode;
}) {
  const fix = firstRunnable(g.rows);
  return (
    <List.Item
      id={`group/${g.name}`}
      icon={{ value: groupIcon(g, color), tooltip: [g.os, g.name].filter(Boolean).join(" · ") }}
      title={g.name}
      subtitle={showDetail ? undefined : groupFacts(g)}
      keywords={[g.address ?? "", g.os ?? "", KIND_WORD[g.worst]].filter(Boolean)}
      accessories={countTags(g, !showDetail)}
      detail={<List.Item.Detail markdown={markdown} />}
      actions={
        <ActionPanel>
          <ActionPanel.Section title={g.name}>
            <Action.Push title="Show Checks" icon={Icon.List} target={checks} />
            {fix?.run ? (
              <Action
                title="Run the First Fix in Terminal"
                icon={Icon.Hammer}
                shortcut={{ modifiers: ["cmd"], key: "return" }}
                onAction={() => openInTerminal(fix.run!, "Aleph · doctor")}
              />
            ) : null}
            {g.machine ? (
              <Action
                title="Run Doctor on This Machine"
                icon={Icon.Terminal}
                onAction={() => openInTerminal(["--host", g.name, "doctor"], `Aleph · doctor · ${g.name}`)}
              />
            ) : null}
            <CopyAction
              title="Copy Summary"
              content={`${g.name}: ${countsText(g.counts)}${groupFacts(g) ? ` (${groupFacts(g)})` : ""}`}
            />
          </ActionPanel.Section>
          {common}
        </ActionPanel>
      }
    />
  );
}

/** A check: its mark, a short title, the key fact (a tag in its colour when it's not a pass), its version. */
function CheckItem({
  row,
  where,
  showDetail,
  common,
  markdown,
}: {
  row: CheckRow;
  /** the machine it's on, shown when the list mixes machines (Needs Attention) */
  where?: string;
  showDetail: boolean;
  common: ReactNode;
  markdown: string;
}) {
  const b = row.brief;
  const color = KIND_COLOR[row.kind];
  const flagged = row.kind === "fail" || row.kind === "warn" || row.kind === "tip";
  const said = row.check.text.replace(/`/g, "");
  const factIcon = row.check.label === "system" ? osLogo(row.check.text)?.icon : undefined;
  const fact: List.Item.Accessory | undefined = b.fact
    ? flagged
      ? { tag: { value: shortFact(b.fact, showDetail ? 24 : 32), color }, tooltip: said }
      : { text: shortFact(b.fact, showDetail ? 18 : 40), tooltip: said, ...(factIcon ? { icon: factIcon } : {}) }
    : undefined;
  const version: List.Item.Accessory | undefined = b.version
    ? { text: b.version, tooltip: `${b.title} ${b.version}`, ...(row.agent ? { icon: agentLogo(row.agent) } : {}) }
    : undefined;
  // with the detail open, one accessory: a flagged fact, else the version (or a short fact)
  const place: List.Item.Accessory | undefined = where ? { text: where, icon: machineLogo(where)?.icon } : undefined;
  const accessories = (
    showDetail
      ? where
        ? [place]
        : flagged
          ? [fact]
          : [version ?? (b.fact.length <= 18 ? fact : undefined)]
      : [place, fact, version]
  ).filter((x): x is List.Item.Accessory => !!x);
  const M = List.Item.Detail.Metadata;
  const meta =
    b.version || b.path || b.extra.length ? (
      <M>
        {b.version ? (
          <M.Label title="Version" text={b.version} icon={row.agent ? agentLogo(row.agent) : undefined} />
        ) : null}
        {b.path ? <M.Label title="Path" text={b.path} /> : null}
        {b.extra.length ? (
          <M.TagList title="Also">
            {b.extra.map((x) => (
              <M.TagList.Item key={x} text={x} />
            ))}
          </M.TagList>
        ) : null}
      </M>
    ) : undefined;
  return (
    <List.Item
      id={row.key}
      icon={{ value: kindIcon(row), tooltip: KIND_WORD[row.kind] }}
      title={b.title}
      subtitle={showDetail ? undefined : rowSubtitle(b)}
      keywords={[row.group, row.area, KIND_WORD[row.kind], row.check.label, said, ...row.fixes.map((f) => f.cmd)]}
      accessories={accessories}
      detail={<List.Item.Detail markdown={markdown} metadata={meta} />}
      actions={
        <ActionPanel>
          <ActionPanel.Section title={`${b.title} · ${row.group}`}>
            {row.fixes.length ? <FixActions fixes={row.fixes} /> : null}
            <CopyAction
              title="Copy Check"
              content={`${row.section} · ${row.check.label}: ${row.check.text}${
                row.fixes.length ? `\nfix: ${row.fixes.map((f) => f.text).join("\nfix: ")}` : ""
              }`}
              shortcut={row.fixes.length ? Keyboard.Shortcut.Common.CopyName : Keyboard.Shortcut.Common.Copy}
            />
            {b.path ? (
              <CopyAction
                title="Copy Path"
                content={
                  row.check.text
                    .split("·")
                    .map((p) => p.trim())
                    .find((p) => p.startsWith("/") || p.startsWith("~")) ?? b.path
                }
                shortcut={Keyboard.Shortcut.Common.CopyPath}
              />
            ) : null}
          </ActionPanel.Section>
          {common}
        </ActionPanel>
      }
    />
  );
}
