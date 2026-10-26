// Closing a session after Aleph's grace (`close.grace`, 5 s): the agent
// keeps running until then, and Undo (`session.close.undo`) keeps it. Pure:
// what a close answered, as the toast says it.

/** What `session.close` (and `aleph close --json`) said. */
export interface CloseAnswer {
  /** closed at once (`close.grace` = 0, or an aleph from before the grace) */
  closed?: boolean;
  /** waiting out the grace: the daemon closes it at `deadline` */
  pending?: boolean;
  /** an undo kept it while the CLI waited */
  kept?: boolean;
  /** it ended by itself during the grace */
  ended?: boolean;
  /** unix seconds */
  deadline?: number;
  deadline_ms?: number;
  grace_s?: number;
}

/** A parsed answer, if it is one. */
export function asClose(v: unknown): CloseAnswer | undefined {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as CloseAnswer) : undefined;
}

/** The answer's JSON, or undefined (an aleph that prints none: it closed). */
export function parseClose(out: string): CloseAnswer | undefined {
  try {
    return asClose(JSON.parse(out.trim()));
  } catch {
    return undefined;
  }
}

/** Whole seconds until it closes (0: due). */
export function secondsLeft(a: CloseAnswer, nowMs = Date.now()): number {
  const ms = a.deadline_ms ?? (a.deadline ?? 0) * 1000;
  return Math.max(0, Math.ceil((ms - nowMs) / 1000));
}

/** The toast after a close was asked, and whether its Undo still helps. */
export function closeToast(
  label: string,
  a: CloseAnswer | undefined,
  nowMs = Date.now(),
): { title: string; message?: string; undo: boolean } {
  if (a?.pending) {
    const left = secondsLeft(a, nowMs);
    return left > 0
      ? { title: `Closing ${label} in ${left}s`, message: "The agent runs until then · Undo keeps it", undo: true }
      : { title: `Closing ${label}…`, undo: false };
  }
  if (a?.kept) return { title: `Kept ${label}`, message: "It won't close", undo: false };
  if (a?.ended) return { title: `${label} had ended`, message: "Nothing to close", undo: false };
  return { title: `Closed ${label}`, undo: false };
}
