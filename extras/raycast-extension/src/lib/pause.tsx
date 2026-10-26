// A command's view, or showcase mode's "be right back" card while every
// window is paused (the panic key, `aleph showcase panic`): looked at twice a
// second, so an open view goes blank (and comes back) by itself. In a build
// without showcase mode it's the view, always.
import { Detail } from "@raycast/api";
import { useEffect, useState, type ReactElement } from "react";
import { paused, pausedMarkdown } from "./showcase";

/** `View` as a command, blanked while paused. */
export function pausable<P extends object>(View: (props: P) => ReactElement | null) {
  return function Pausable(props: P) {
    const [p, setP] = useState(() => paused());
    useEffect(() => {
      const t = setInterval(() => setP(paused()), 500);
      return () => clearInterval(t);
    }, []);
    return p ? <Detail navigationTitle="Aleph" markdown={pausedMarkdown()} /> : <View {...props} />;
  };
}
