// Start Session: the new-session form. A launch context fills it in —
// {"host": "b31", "dir": "/srv/px", "agent": "codex", "prompt": "…", "worktree": true, "preset": "px"}.
import { LaunchProps } from "@raycast/api";
import { pausable } from "./lib/pause";
import { StartForm, StartInitial } from "./lib/start-form";

function StartSession(props: LaunchProps<{ launchContext?: StartInitial }>) {
  return <StartForm initial={props.launchContext} />;
}

// blank while showcase mode pauses every window
export default pausable(StartSession);
