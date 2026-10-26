// The install view (a live log) and the install actions for a missing agent.
// The flow itself (ask first, then install with a toast) is in install-flow.ts.
import { Action, ActionPanel, Detail, Icon, Keyboard, showToast, Toast, useNavigation } from "@raycast/api";
import { copyText, CopyAction } from "./copy";
import { useEffect, useRef, useState } from "react";
import { AgentInfo, stripAnsi } from "./cli";
import { confirmInstall, installBlocker, installDocs, installWithToast } from "./install-flow";
import { agentLogo } from "./logos";
import { StartForm } from "./start-form";
import { openInTerminal } from "./terminal";

/** A live install log. Starts the install when it opens (the Alert came first). */
export function InstallView({ a, host, onDone }: { a: AgentInfo; host: string; onDone?: (ok: boolean) => void }) {
  const [out, setOut] = useState("");
  const [state, setState] = useState<"running" | "ok" | "failed">("running");
  const started = useRef(false);
  const { pop } = useNavigation();

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    installWithToast(a, host, setOut).then((ok) => {
      setState(ok ? "ok" : "failed");
      onDone?.(ok);
    });
  }, [a, host, onDone]);

  const head =
    state === "running"
      ? `### Installing ${a.name} on ${host}…\n\nKeep this open until it finishes (it can take up to 15 minutes).`
      : state === "ok"
        ? `### ✓ ${a.name} is installed on ${host}`
        : `### ✕ ${a.name} didn't install on ${host}`;
  const body = stripAnsi(out).replace(/```/g, "``​`").trimEnd();
  const cmd = a.install_command ?? `aleph agents install ${a.id}`;
  const md = `${head}\n\n\`\`\`\n$ ${cmd}\n${body || "…"}\n\`\`\``;

  return (
    <Detail
      isLoading={state === "running"}
      navigationTitle={`Install ${a.name}`}
      markdown={md}
      actions={
        <ActionPanel>
          {state === "ok" ? (
            <Action.Push
              title={`Start Session with ${a.name}`}
              icon={agentLogo(a.id)}
              target={<StartForm initial={{ host, agent: a.id }} />}
            />
          ) : null}
          {state === "failed" ? (
            <Action
              title="Try in Terminal"
              icon={Icon.Terminal}
              onAction={() => openInTerminal(["--host", host, "agents", "install", a.id], `Install ${a.name}`)}
            />
          ) : null}
          <Action
            title="Copy Output"
            icon={Icon.CopyClipboard}
            shortcut={Keyboard.Shortcut.Common.Copy}
            onAction={async () => {
              if (!(await copyText(stripAnsi(out)))) return;
              await showToast({ style: Toast.Style.Success, title: "Copied the install output" });
            }}
          />
          {state !== "running" ? <Action title="Done" icon={Icon.Check} onAction={pop} /> : null}
        </ActionPanel>
      }
    />
  );
}

/** Actions for an agent that isn't on a machine: install here (asks first), or in the terminal. */
export function InstallActions({ a, host, onDone }: { a: AgentInfo; host: string; onDone?: (ok: boolean) => void }) {
  const { push } = useNavigation();
  const blocked = installBlocker(a);
  const docs = installDocs(a);
  return (
    <>
      {!blocked ? (
        <Action
          title={`Install on ${host}…`}
          icon={{ source: Icon.Download, tintColor: a.color }}
          onAction={async () => {
            if (await confirmInstall(a, host)) push(<InstallView a={a} host={host} onDone={onDone} />);
          }}
        />
      ) : null}
      {docs ? <Action.OpenInBrowser title="Open Install Instructions" url={docs} /> : null}
      {blocked ? (
        <Action
          title="Why It Can't Install"
          icon={Icon.QuestionMarkCircle}
          onAction={() => showToast({ style: Toast.Style.Failure, title: `Can't install ${a.name}`, message: blocked })}
        />
      ) : null}
      <Action
        title="Install in Terminal"
        icon={Icon.Terminal}
        shortcut={{ modifiers: ["cmd", "shift"], key: "i" }}
        onAction={() => openInTerminal(["--host", host, "agents", "install", a.id], `Install ${a.name}`)}
      />
      {a.install_command ? (
        <CopyAction title="Copy Install Command" content={a.install_command} shortcut={Keyboard.Shortcut.Common.Copy} />
      ) : null}
    </>
  );
}
