// The daemon tells this command when something changed, so its views ask
// again then instead of on a timer: one `events.subscribe` connection per
// command (shared by every hook that listens), `{"state": …}` when the fleet
// moved and `{"changed": topic}` when the inbox, queue, notes, usage,
// agents, LLM servers, doctor, settings or history did. It reconnects with a
// backoff; while it's down the hooks fall back to their timers.
//
// No Raycast imports: plain node tests it.
import { createConnection, Socket } from "node:net";
import { socketPath } from "./api";

/** What a listener hears: `state` (the fleet), or a topic's name. */
export type Topic =
  | "state"
  | "inbox"
  | "queue"
  | "notes"
  | "usage"
  | "agents"
  | "services"
  | "doctor"
  | "config"
  | "history"
  // a showcase build's presenter sync (what the presenter's dashboard shows)
  | "presenter";

type Listener = { topics: Set<Topic>; fn: (t: Topic) => void };

const listeners = new Set<Listener>();
let sock: Socket | undefined;
let connected = false;
let retry: ReturnType<typeof setTimeout> | undefined;
let backoff = 500;
let idle: ReturnType<typeof setTimeout> | undefined;
let path: string | undefined;

/** Is the daemon telling us about changes right now? (Else: poll.) */
export const isLive = () => connected;

/** For tests: another socket. */
export function useSocket(p?: string): void {
  path = p;
}

function dispatch(t: Topic) {
  for (const l of listeners) if (l.topics.has(t)) l.fn(t);
}

function open() {
  if (sock || listeners.size === 0) return;
  const s = createConnection(path ?? socketPath());
  sock = s;
  let buf = "";
  s.setEncoding("utf8");
  s.on("connect", () => {
    s.write(
      JSON.stringify({ id: `raycast-live-${process.pid}`, method: "events.subscribe", params: { topics: "all" } }) +
        "\n",
    );
  });
  s.on("data", (chunk: string) => {
    buf += chunk;
    let i;
    while ((i = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, i).trim();
      buf = buf.slice(i + 1);
      if (!line) continue;
      try {
        const m = JSON.parse(line) as { result?: { subscribed?: boolean }; state?: unknown; changed?: string };
        if (m.result?.subscribed) {
          connected = true;
          backoff = 500;
        } else if (m.state !== undefined) {
          dispatch("state");
        } else if (typeof m.changed === "string") {
          dispatch(m.changed as Topic);
        }
      } catch {
        // a line we don't understand: skip it
      }
    }
  });
  const down = () => {
    if (sock !== s) return;
    sock = undefined;
    connected = false;
    s.destroy();
    if (listeners.size === 0) return;
    retry = setTimeout(open, backoff);
    backoff = Math.min(backoff * 2, 30_000);
  };
  s.on("error", down);
  s.on("close", down);
}

function close() {
  if (retry) clearTimeout(retry);
  retry = undefined;
  const s = sock;
  sock = undefined;
  connected = false;
  s?.destroy();
}

/**
 * Hear about `topics` while the view is open; returns the stop. The
 * connection opens with the first listener and closes a moment after the
 * last one leaves.
 */
export function listen(topics: Topic[], fn: (t: Topic) => void): () => void {
  const l: Listener = { topics: new Set(topics), fn };
  listeners.add(l);
  if (idle) clearTimeout(idle);
  idle = undefined;
  open();
  return () => {
    listeners.delete(l);
    if (listeners.size === 0) {
      idle = setTimeout(close, 1500);
    }
  };
}
