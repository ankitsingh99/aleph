// Notification Settings: which Aleph messages reach you, as Aleph's own
// settings (`aleph settings set`), so the daemon, the dashboard and this
// extension read the same choices:
// * notify.source = both | herdr | agents — who tells you: herdr's view of
//   each agent, the agents' own hooks (`aleph hooks install`), or both;
// * notify.raycast — the daemon refreshes the menu bar the moment something
//   happens, by opening notify.raycast.deeplink (shown only when it isn't
//   this extension's menu bar); notify.raycast.kinds — what the menu bar lists;
// * notify / notify_done / notify.desktop.click — desktop banners;
// * notify.remote_on — what goes to ntfy, Slack and webhooks (the ones set up
//   are named); moshi, moshi.events, moshi.approvals — the Moshi phone app,
//   on builds that have it;
// * do not disturb (`aleph notify dnd`), on builds that have it.
// Only the settings you change are written. Every field says its setting in
// its info (ⓘ), and nothing else.
import {
  Action,
  ActionPanel,
  Form,
  Icon,
  Image,
  launchCommand,
  LaunchType,
  popToRoot,
  showToast,
  Toast,
} from "@raycast/api";
import { pausable } from "./lib/pause";
import { CopyAction } from "./lib/copy";
import { usePromise } from "@raycast/utils";
import { useState } from "react";
import { commands, hasFlag, MENU_BAR_DEEPLINK, setDnd, setting, settingSet, settingsList } from "./lib/cli";
import { ALL_KINDS, dndState } from "./lib/model";
import { dndKeep, MOSHI_KINDS, notifyNow, notifyWrites, NotifyValues, remoteSinks } from "./lib/notify-view";
import { openInTerminal } from "./lib/terminal";
import { ALEPH_ICON, errorText, kindLook, WARN } from "./lib/ui";

const SOURCES: [string, string, Image.ImageLike][] = [
  ["both", "herdr and the agents' hooks", ALEPH_ICON],
  ["herdr", "herdr only", Icon.Terminal],
  ["agents", "The agents' hooks only", Icon.SpeechBubble],
];

const CLICKS: [string, string, Image.ImageLike][] = [
  ["aleph", "The session in Aleph", ALEPH_ICON],
  ["raycast", "Raycast's Aleph Inbox", Icon.RaycastLogoNeg],
  ["warp", "A Warp tab in herdr", Icon.Terminal],
];

const MOSHI: [string, string][] = [
  ["auto", "When it's paired"],
  ["on", "On"],
  ["off", "Off"],
];

const DND: [string, string][] = [
  ["off", "Off"],
  ["30m", "For 30 minutes"],
  ["1h", "For 1 hour"],
  ["2h", "For 2 hours"],
  ["on", "Until I turn it off"],
];

const hhmm = (t: number) => new Date(t * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/** What this aleph can do here: `notify dnd`, `hooks`. */
async function features() {
  const [settings, dnd, cmds] = await Promise.all([
    settingsList(),
    hasFlag("notify", "dnd"),
    commands().catch(() => new Set<string>()),
  ]);
  return { settings, dnd, hooks: cmds.has("hooks") };
}

function NotificationSettings() {
  const { data, isLoading, error, revalidate } = usePromise(features);
  const [saving, setSaving] = useState(false);
  const [source, setSource] = useState<string>();

  if (!data) {
    return (
      <Form isLoading={isLoading}>
        {error ? <Form.Description title="Couldn't read Aleph's settings" text={errorText(error).title} /> : null}
      </Form>
    );
  }

  const get = (k: string) => setting(data.settings, k);
  const dnd = dndState(get("notify.dnd"));
  const now = notifyNow(get, MENU_BAR_DEEPLINK);
  const picked = source ?? now.source;
  const sinks = remoteSinks(get);
  const customLink = now.deeplink !== MENU_BAR_DEEPLINK;

  async function save(v: NotifyValues) {
    const writes = notifyWrites(now, v);
    const dndTo = v.dnd && v.dnd !== "keep" ? v.dnd : undefined;
    if (!writes.length && !dndTo) {
      await showToast({ style: Toast.Style.Success, title: "Nothing changed" });
      return;
    }
    setSaving(true);
    const toast = await showToast({ style: Toast.Style.Animated, title: "Saving…" });
    try {
      for (const [k, val] of writes) await settingSet(k, val);
      if (dndTo) await setDnd(dndTo);
      const n = writes.length + (dndTo ? 1 : 0);
      toast.style = Toast.Style.Success;
      toast.title = `Saved ${n} setting${n === 1 ? "" : "s"}`;
      toast.message = [...writes.map(([k]) => k), ...(dndTo ? ["do not disturb"] : [])].join(", ");
      // the menu bar shows the new choices at once
      await launchCommand({ name: "menu-bar", type: LaunchType.Background }).catch(() => undefined);
      await popToRoot();
    } catch (e) {
      toast.style = Toast.Style.Failure;
      Object.assign(toast, errorText(e));
      revalidate();
    } finally {
      setSaving(false);
    }
  }

  const kindItems = (id: string) =>
    ALL_KINDS.map(([k, title]) => (
      <Form.TagPicker.Item key={`${id}.${k}`} value={k} title={title} icon={kindLook(k).icon} />
    ));

  return (
    <Form
      isLoading={isLoading || saving}
      navigationTitle="Notification Settings"
      actions={
        <ActionPanel>
          <Action.SubmitForm title="Save Settings" icon={Icon.Check} onSubmit={save} />
          {data.hooks ? (
            <Action
              title="Set up Agent Hooks in Terminal"
              icon={Icon.SpeechBubble}
              onAction={() => openInTerminal(["hooks", "install"], "Aleph · hooks")}
            />
          ) : null}
          {data.hooks ? (
            <Action
              title="Agent Hooks Status in Terminal"
              icon={Icon.Info}
              onAction={() => openInTerminal(["hooks", "status"], "Aleph · hooks")}
            />
          ) : null}
          <Action
            title="Send a Test Notification"
            icon={Icon.Bell}
            onAction={() => openInTerminal(["notify", "test"], "Aleph · notify test")}
          />
          <CopyAction title="Copy Menu Bar Deeplink" content={MENU_BAR_DEEPLINK} />
        </ActionPanel>
      }
    >
      <Form.Dropdown
        id="source"
        title="Who Tells You"
        defaultValue={now.source}
        onChange={setSource}
        info="notify.source — the agents speak where their hooks are installed (⌘K › Set up Agent Hooks)"
      >
        {SOURCES.map(([v, title, icon]) => (
          <Form.Dropdown.Item key={v} value={v} title={title} icon={icon} />
        ))}
      </Form.Dropdown>
      {picked !== "herdr" && !data.hooks ? (
        <Form.Description text="This aleph has no agent hooks yet: herdr tells you meanwhile." />
      ) : null}
      <Form.Separator />
      <Form.Checkbox
        id="raycast"
        title="Menu Bar"
        label="Refresh the moment something happens"
        defaultValue={now.raycast}
        info="notify.raycast — otherwise every 30 s"
      />
      {customLink ? (
        <Form.TextField
          id="deeplink"
          title="Menu Bar Deeplink"
          defaultValue={now.deeplink}
          info={`notify.raycast.deeplink — this extension's is ${MENU_BAR_DEEPLINK}`}
        />
      ) : null}
      <Form.TagPicker id="kinds" title="Lists" defaultValue={now.kinds} info="notify.raycast.kinds">
        {kindItems("kinds")}
      </Form.TagPicker>
      <Form.Separator />
      <Form.Checkbox
        id="notify"
        title="Desktop"
        label="When a session needs you"
        defaultValue={now.notify}
        info="notify"
      />
      <Form.Checkbox
        id="notify_done"
        label="When a session finishes"
        defaultValue={now.notify_done}
        info="notify_done"
      />
      <Form.Dropdown id="click" title="A Click Opens" defaultValue={now.click} info="notify.desktop.click">
        {CLICKS.map(([v, title, icon]) => (
          <Form.Dropdown.Item key={v} value={v} title={title} icon={icon} />
        ))}
      </Form.Dropdown>
      <Form.Separator />
      <Form.TagPicker
        id="remote_on"
        title="Phone & Webhooks"
        defaultValue={now.remote_on}
        info={`notify.remote_on — ${sinks.length ? `goes to ${sinks.join(", ")}` : "none set up yet (aleph setup)"}`}
      >
        {kindItems("remote")}
      </Form.TagPicker>
      {now.moshi !== undefined ? (
        <>
          <Form.Dropdown id="moshi" title="Moshi" defaultValue={now.moshi} info="moshi — the phone terminal app">
            {MOSHI.map(([v, title]) => (
              <Form.Dropdown.Item key={v} value={v} title={title} icon={Icon.Mobile} />
            ))}
          </Form.Dropdown>
          <Form.TagPicker id="moshi_events" title="Moshi Cards" defaultValue={now.moshi_events} info="moshi.events">
            {MOSHI_KINDS.map(([k, title, look]) => (
              <Form.TagPicker.Item key={k} value={k} title={title} icon={kindLook(look).icon} />
            ))}
          </Form.TagPicker>
          <Form.Checkbox
            id="moshi_approvals"
            label="Questions as buttons on the phone"
            defaultValue={now.moshi_approvals}
            info="moshi.approvals — never for what approve_deny lists"
          />
        </>
      ) : null}
      {data.dnd ? (
        <>
          <Form.Separator />
          <Form.Dropdown
            id="dnd"
            title="Do Not Disturb"
            defaultValue="keep"
            info={`aleph notify dnd — ${get("notify.quiet_pass") || "error"} still comes through`}
          >
            <Form.Dropdown.Item
              value="keep"
              title={dndKeep(dnd, hhmm)}
              icon={dnd.on ? { source: Icon.BellDisabled, tintColor: WARN } : Icon.Bell}
            />
            {DND.filter(([v]) => v !== "off" || dnd.on).map(([v, title]) => (
              <Form.Dropdown.Item key={v} value={v} title={title} icon={v === "off" ? Icon.Bell : Icon.BellDisabled} />
            ))}
          </Form.Dropdown>
        </>
      ) : null}
    </Form>
  );
}

// blank while showcase mode pauses every window
export default pausable(NotificationSettings);
