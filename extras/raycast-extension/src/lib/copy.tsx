// Copying, as showcase mode allows it (`showcase.clipboard`): what Raycast
// shows is already masked by the daemon, so a copy takes that; with copies
// off (or every window paused) the copy actions aren't there and a copy
// from code says why. In a build without showcase mode, plain copying.
import { Action, Clipboard, showHUD, showToast, Toast } from "@raycast/api";
import { canCopy } from "./showcase";

/** Action.CopyToClipboard, unless showcase mode keeps copies off. */
export function CopyAction(props: Action.CopyToClipboard.Props) {
  return canCopy() ? <Action.CopyToClipboard {...props} /> : null;
}

/**
 * Clipboard.copy, unless showcase mode keeps copies off: false then, and a
 * toast says so (`hud`: a HUD, for the menu bar, which has no toasts).
 */
export async function copyText(text: string, hud = false): Promise<boolean> {
  if (!canCopy()) {
    if (hud) await showHUD("Copying is off for the demo (showcase.clipboard)");
    else
      await showToast({
        style: Toast.Style.Failure,
        title: "Copying is off for the demo",
        message: "showcase.clipboard",
      });
    return false;
  }
  await Clipboard.copy(text);
  return true;
}
