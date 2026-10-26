// Send Message: prompt a running session now, or queue the message for when it's
// free, after a while or at a time (`aleph send <sel> <text> [--after …]`).
// The menu bar's Reply and Queue open it with the session (and when) filled in,
// and so can a deeplink: launchContext {"sel": "b31:temper", "when": "idle"}
// (`sel` is host:name or host:pane; `when` is now, idle, 30m, 1h, 2h or at).
import { LaunchProps } from "@raycast/api";
import { pausable } from "./lib/pause";
import { SendForm, When } from "./lib/send-form";

interface Context {
  sel?: string;
  host?: string;
  name?: string;
  when?: When;
  text?: string;
}

const WHENS: When[] = ["now", "idle", "30m", "1h", "2h", "at"];

function SendMessage(props: LaunchProps<{ launchContext?: Context }>) {
  const c = props.launchContext ?? {};
  const sel = c.sel ?? (c.host && c.name ? `${c.host}:${c.name}` : undefined);
  const when = c.when && WHENS.includes(c.when) ? c.when : "now";
  return <SendForm root initialSel={sel} initialWhen={when} initialText={c.text ?? ""} />;
}

// blank while showcase mode pauses every window
export default pausable(SendMessage);
