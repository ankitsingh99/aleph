// Local LLM Servers: Ollama, LM Studio, llama.cpp, vLLM, TGI, LiteLLM, … on
// every machine (`aleph services --json`), a section per machine. A row: its
// logo, its models loaded and its health. The detail is SVG cards
// (services-view.ts): health and version, KV cache and requests when it
// reports them, its models (loaded first, params, quant, context, VRAM) and
// where it answers — from here too, at the machine's address (`ls --json`),
// when it listens on every interface. Copy the OpenAI base URL (↵), an ssh
// tunnel, a model's name. Read-only: it never starts, stops or loads anything.
import { Action, ActionPanel, environment, Icon, Keyboard, List } from "@raycast/api";
import { pausable } from "./lib/pause";
import { CopyAction } from "./lib/copy";
import { useState } from "react";
import { explainError, onHost, Service, ServicesHost, ServicesReport, SERVICES_ARGS, servicesApi } from "./lib/cli";
import { chartTheme } from "./lib/charts";
import { notice } from "./lib/cards";
import { useAleph, useFleet } from "./lib/hooks";
import { cardLogos, machineLogo, serviceLogo } from "./lib/logos";
import {
  hostSummary,
  modelsLine,
  reachUrl,
  ServerBadge,
  serverBadges,
  serverMarkdown,
  sortedModels,
  sortedServers,
  tunnel,
} from "./lib/services-view";
import { navTitle } from "./lib/showcase";
import { img } from "./lib/stats-view";
import { DONE, DOWN, MUTED, WARN } from "./lib/ui";

const ALL = "all";

function LlmServers() {
  return <ServicesList />;
}

/** A server's badge as Raycast's accessory. */
function accessory(b: ServerBadge, loaded: boolean): List.Item.Accessory {
  switch (b.kind) {
    case "auth":
      return { icon: { source: Icon.Lock, tintColor: MUTED }, tooltip: b.tooltip };
    case "models":
      return { icon: { source: Icon.Layers, tintColor: loaded ? DONE : MUTED }, text: b.text, tooltip: b.tooltip };
    case "vram":
      return { icon: Icon.MemoryChip, text: b.text, tooltip: b.tooltip };
    case "port":
      return { text: b.text, tooltip: b.tooltip };
    case "health":
      return b.tone === "good"
        ? { icon: { source: Icon.CircleFilled, tintColor: DONE }, tooltip: b.tooltip }
        : {
            tag: { value: b.text, color: b.tone === "bad" ? DOWN : b.tone === "warn" ? WARN : MUTED },
            tooltip: b.tooltip,
          };
  }
}

/** Every machine's LLM servers, or one machine's (`only`, from Machines: scans just that one). */
export function ServicesList({ only }: { only?: string }) {
  const { data, isLoading, errorInfo, stale, revalidate } = useAleph<ServicesReport | undefined>(
    onHost(only, SERVICES_ARGS),
    {
      optional: "services",
      timeoutMs: 90_000,
      failureTitle: "Couldn't scan for LLM servers",
      api: servicesApi(only),
      topics: ["services"],
    },
  );
  // each machine's address, for the URL from here
  const fleet = useFleet();
  const [host, setHost] = useState(only ?? ALL);
  const [showDetail, setShowDetail] = useState(true);
  const dark = environment.appearance !== "light";
  const unsupported = !isLoading && !errorInfo && data === undefined;
  const hosts = (data?.hosts ?? []).filter((h) => host === ALL || h.alias === host);
  const found = hosts.reduce((n, h) => n + h.services.length + (h.reachable ? 0 : 1), 0);
  const address = (alias: string) => fleet.data?.hosts.find((h) => h.alias === alias)?.address;

  return (
    <List
      isLoading={isLoading}
      isShowingDetail={showDetail && found > 0}
      navigationTitle={navTitle(
        [only ? `LLM Servers on ${only}` : "LLM Servers", stale ? "cached" : ""].filter(Boolean).join(" · "),
      )}
      searchBarPlaceholder="Filter by server, model or machine"
      searchBarAccessory={
        only ? undefined : (
          <List.Dropdown tooltip="Machine" value={host} onChange={setHost}>
            <List.Dropdown.Item title="All Machines" value={ALL} icon={Icon.Network} />
            {(data?.hosts ?? []).map((h) => (
              <List.Dropdown.Item
                key={h.alias}
                title={h.alias}
                value={h.alias}
                icon={machineLogo(h.alias)?.icon ?? { source: Icon.CircleFilled, tintColor: h.color }}
              />
            ))}
          </List.Dropdown>
        )
      }
    >
      {unsupported ? (
        <List.EmptyView
          icon={Icon.Info}
          title="This aleph has no `services` command yet"
          description="aleph self-update"
        />
      ) : errorInfo && !data ? (
        <List.EmptyView icon={Icon.Warning} title={errorInfo.title} description={errorInfo.message} />
      ) : (
        <List.EmptyView
          icon={Icon.ComputerChip}
          title={isLoading ? "Looking for LLM servers…" : "No LLM servers found"}
          description={hosts.length ? `Scanned ${hosts.map((h) => h.alias).join(", ")}` : undefined}
          actions={
            <ActionPanel>
              <Action title="Scan Again" icon={Icon.ArrowClockwise} onAction={revalidate} />
            </ActionPanel>
          }
        />
      )}
      {hosts.map((h) => (
        <List.Section key={h.alias} title={h.alias} subtitle={hostSummary(h)}>
          {!h.reachable ? <OfflineItem h={h} dark={dark} refresh={revalidate} /> : null}
          {sortedServers(h.services).map((s) => (
            <ServiceItem
              key={`${h.alias}-${s.id}-${s.port ?? ""}`}
              h={h}
              s={s}
              address={address(h.alias)}
              enabled={!!data?.enabled}
              dark={dark}
              refresh={revalidate}
              showDetail={showDetail}
              toggle={() => setShowDetail((x) => !x)}
            />
          ))}
        </List.Section>
      ))}
    </List>
  );
}

function ServiceItem({
  h,
  s,
  address,
  enabled,
  dark,
  refresh,
  showDetail,
  toggle,
}: {
  h: ServicesHost;
  s: Service;
  address?: string;
  enabled: boolean;
  dark: boolean;
  refresh: () => void;
  showDetail: boolean;
  toggle: () => void;
}) {
  const tun = tunnel(h.alias, s);
  const here = reachUrl(h, s, address);
  const models = sortedModels(s);
  const copyBase = s.openai_base ? (
    <CopyAction
      key="base"
      title="Copy OpenAI Base URL"
      content={s.openai_base}
      shortcut={Keyboard.Shortcut.Common.Copy}
    />
  ) : null;
  const open = here ? <Action.OpenInBrowser key="open" title="Open in Browser" url={here} /> : null;
  // a web UI opens; a server's OpenAI base is what a client needs
  const first = s.kind === "ui" ? [open, copyBase] : [copyBase, open];

  return (
    <List.Item
      icon={{ value: serviceLogo(s), tooltip: s.name }}
      title={s.name}
      subtitle={showDetail ? undefined : modelsLine(s)}
      keywords={[s.id, h.alias, s.kind, String(s.port ?? ""), ...s.models.map((m) => m.name)]}
      accessories={serverBadges(s, !showDetail).map((b) => accessory(b, s.loaded_count > 0))}
      detail={
        <List.Item.Detail markdown={serverMarkdown({ h, s, address, t: chartTheme(dark), logo: cardLogos(dark) })} />
      }
      actions={
        <ActionPanel>
          <ActionPanel.Section title={`${s.name} on ${h.alias}`}>
            {first}
            {s.url ? (
              <CopyAction
                title="Copy URL"
                content={here ?? s.url}
                shortcut={{ modifiers: ["cmd", "shift"], key: "u" }}
              />
            ) : null}
            {tun ? (
              <CopyAction title="Copy SSH Tunnel Command" content={tun} shortcut={{ modifiers: ["cmd"], key: "t" }} />
            ) : null}
            {models.length ? (
              <ActionPanel.Submenu
                title="Copy Model Name"
                icon={Icon.Layers}
                shortcut={{ modifiers: ["cmd"], key: "m" }}
              >
                {models.map((m) => (
                  <CopyAction
                    key={m.name}
                    title={m.name}
                    content={m.name}
                    icon={{
                      source: m.loaded ? Icon.CircleFilled : Icon.Circle,
                      tintColor: m.loaded ? DONE : MUTED,
                    }}
                  />
                ))}
              </ActionPanel.Submenu>
            ) : null}
            {s.homepage ? <Action.OpenInBrowser title="Open Homepage" url={s.homepage} /> : null}
          </ActionPanel.Section>
          <ActionPanel.Section>
            <Action
              title={showDetail ? "Hide Details" : "Show Details"}
              icon={Icon.Sidebar}
              shortcut={{ modifiers: ["cmd", "shift"], key: "d" }}
              onAction={toggle}
            />
            <Action
              title="Scan Again"
              icon={Icon.ArrowClockwise}
              shortcut={Keyboard.Shortcut.Common.Refresh}
              onAction={refresh}
            />
            {!enabled ? (
              <CopyAction
                title="Copy Command to Watch Them Live"
                content="aleph settings set services on"
                icon={Icon.Eye}
              />
            ) : null}
          </ActionPanel.Section>
        </ActionPanel>
      }
    />
  );
}

/** A machine that couldn't be scanned: why, in a card. */
function OfflineItem({ h, dark, refresh }: { h: ServicesHost; dark: boolean; refresh: () => void }) {
  const { head, detail } = explainError(h.error);
  const t = chartTheme(dark);
  return (
    <List.Item
      icon={{ source: Icon.WifiDisabled, tintColor: DOWN }}
      title={h.alias}
      accessories={[{ tag: { value: "offline", color: DOWN }, tooltip: head }]}
      detail={
        <List.Item.Detail
          markdown={img(`${h.alias}: ${head}`, notice({ state: "down", color: t.state.down, title: head, detail }, t))}
        />
      }
      actions={
        <ActionPanel>
          <Action
            title="Scan Again"
            icon={Icon.ArrowClockwise}
            shortcut={Keyboard.Shortcut.Common.Refresh}
            onAction={refresh}
          />
        </ActionPanel>
      }
    />
  );
}

// blank while showcase mode pauses every window
export default pausable(LlmServers);
