// Numbers and states as text, the way aleph prints them. No Raycast imports
// (ui.ts and cli.ts re-export these; the SVG views and plain node use them).

/** `512B`, `3.4K`, `12M`, `1.5G` (binary units, like aleph). */
export function bytes(n: number | null | undefined): string {
  if (n == null || !Number.isFinite(n)) return "–";
  const u = ["B", "K", "M", "G", "T", "P"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 10 || i === 0 ? Math.round(v) : v.toFixed(1)}${u[i]}`;
}

/** `196K/s`. */
export const rate = (n: number | null | undefined) => (n == null ? "–" : `${bytes(n)}/s`);

export const pct = (frac: number | null | undefined) => `${Math.round((frac ?? 0) * 100)}%`;

/** `40s`, `16m`, `2h`, `3d`. */
export function duration(secs: number): string {
  const d = Math.max(0, Math.floor(secs));
  if (d < 60) return `${d}s`;
  if (d < 3600) return `${Math.floor(d / 60)}m`;
  if (d < 86400) return `${Math.floor(d / 3600)}h`;
  return `${Math.floor(d / 86400)}d`;
}

/** "4m", "2h", "3d" since a unix timestamp. */
export function since(t: number | null | undefined, now = Date.now() / 1000): string {
  if (!t) return "";
  return duration(now - t);
}

export const STATUS_LABEL: Record<string, string> = {
  working: "working",
  blocked: "needs you",
  done: "done",
  idle: "idle",
  limit: "at limit",
};

/** A session's state for display: `limit` when it stopped at its usage limit, else herdr's status. */
export const stateOf = (s: { status: string; limit?: string | null }) => (s.limit != null ? "limit" : s.status);

/** The order sessions are shown in: what needs you first. */
export const STATE_ORDER = ["blocked", "working", "limit", "done", "idle"];
