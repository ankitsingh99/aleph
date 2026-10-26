// History for the live sparklines: each refresh adds a sample per machine,
// kept across refreshes and launches (the commands store it in Raycast's
// Cache) for a quarter of an hour. No Raycast imports: plain node tests it.

export interface Sampled {
  /** unix seconds */
  t: number;
}

/** One Machine Stats sample (fractions 0..1, bytes per second, °C). */
export interface StatSample extends Sampled {
  cpu: number | null;
  mem: number | null;
  gpu: number | null;
  vram: number | null;
  load: number | null;
  rx: number | null;
  tx: number | null;
  rd: number | null;
  wr: number | null;
  temp: number | null;
}

/** One link reading (Machines): the poll's round trip, or down. */
export interface LinkSample extends Sampled {
  ms: number | null;
  up: boolean;
}

export const KEEP = { points: 300, ageS: 15 * 60 };

/**
 * `hist` with `s` added: a sample no newer than the last one is dropped (the
 * same answer read twice), and so is everything older than `keep.ageS`
 * before `now` or past the last `keep.points`.
 */
export function pushSample<T extends Sampled>(hist: T[], s: T, now = s.t, keep = KEEP): T[] {
  const last = hist[hist.length - 1];
  const next = last && s.t <= last.t ? hist : [...hist, s];
  return next.filter((x) => now - x.t <= keep.ageS).slice(-keep.points);
}

/** A stored history (JSON), or nothing when it's missing or not one. */
export function parseHistory<T extends Sampled>(raw: string | undefined): T[] {
  if (!raw) return [];
  try {
    const v = JSON.parse(raw);
    return Array.isArray(v) ? v.filter((x) => x && typeof x.t === "number") : [];
  } catch {
    return [];
  }
}

/** One measure over time, for a sparkline. */
export function seriesOf<T extends Sampled>(hist: T[], key: keyof T): [number, number | null][] {
  return hist.map((s) => {
    const v = s[key];
    return [s.t, typeof v === "number" && Number.isFinite(v) ? v : null];
  });
}

/** `last 40 s`, `last 4 min`, "" with fewer than two samples. */
export function spanText(hist: Sampled[]): string {
  if (hist.length < 2) return "";
  const s = hist[hist.length - 1].t - hist[0].t;
  if (s < 90) return `last ${Math.max(1, Math.round(s))} s`;
  return `last ${Math.round(s / 60)} min`;
}

/** Signal bars (0..4) for a link: 4 under 150 ms, 3 under 400, 2 under the slow mark, 1 when slow, 0 down. */
export function bars(ms: number | null | undefined, up: boolean, slowMs = 800): number {
  if (!up) return 0;
  if (ms == null) return 3;
  return ms < 150 ? 4 : ms < 400 ? 3 : ms <= slowMs ? 2 : 1;
}
