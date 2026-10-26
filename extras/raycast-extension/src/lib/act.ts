import { showToast, Toast } from "@raycast/api";
import { errorText } from "./ui";

/** Run an aleph action with an animated toast; refresh afterwards. */
export async function act(
  title: string,
  f: () => Promise<unknown>,
  refresh?: () => void,
  done?: { title?: string; message?: string },
): Promise<boolean> {
  const toast = await showToast({ style: Toast.Style.Animated, title });
  try {
    await f();
    toast.style = Toast.Style.Success;
    toast.title = done?.title ?? title.replace(/…$/, "") + " ✓";
    if (done?.message) toast.message = done.message;
    refresh?.();
    return true;
  } catch (e) {
    toast.style = Toast.Style.Failure;
    Object.assign(toast, errorText(e));
    return false;
  }
}
