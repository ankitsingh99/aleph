// Showcase mode as every build of the extension has it: nothing at all.
// scripts/showcase.sh copies this file to src/lib/showcase.ts, or real.ts
// instead for a demo build (ALEPH_SHOWCASE=1). Both export the same names
// and types, so the callers compile against either; here each one is the
// identity or nothing.

export type Mode = "real" | "masked" | "demo";

/** One choice in the menu bar's switch. */
export interface ShowcaseItem {
  title: string;
  subtitle: string;
  /** `aleph …` */
  args: string[];
  /** the HUD once it has switched */
  hud: string;
  current: boolean;
}

export interface ShowcaseMenu {
  title: string;
  items: ShowcaseItem[];
}

/** What every window shows: always the real thing here. Also part of cache keys. */
export function mode(_now?: number): Mode {
  return "real";
}

export function on(): boolean {
  return false;
}

/** A banner card for the top of a detail (markdown), or null. */
export function banner(_now?: number): string | null {
  return null;
}

/** A detail's markdown with the banner on top. */
export function withBanner(md: string): string {
  return md;
}

/** A navigation title. */
export function navTitle(t: string): string {
  return t;
}

/** Your watermark (showcase.watermark): none here. */
export function watermark(_place?: string, _now?: number): string | null {
  return null;
}

/** The menu bar's switch: none. */
export function menu(): ShowcaseMenu | null {
  return null;
}

// ---------------------------------------------------------------- safety

/** Is every window paused (showcase's panic key)? Never here. */
export function paused(_now?: number): boolean {
  return false;
}

/** The pause's card (markdown): none here. */
export function pausedMarkdown(_now?: number): string {
  return "";
}

/** May a copy action copy? Always here. */
export function canCopy(): boolean {
  return true;
}

/** How a $ amount reads when showcase hides money: never hidden here. */
export function moneyText(): string | null {
  return null;
}

// ---------------------------------------------------------------- presenter sync

/** What the presenter's dashboard shows (the daemon's presenter.get). */
export interface Presenter {
  on: boolean;
  view: string;
  host?: string;
  pane_id?: string;
  name?: string;
  gen?: number;
}

/** Does this extension follow the presenter's dashboard (showcase.sync)? Never here. */
export function following(_now?: number): boolean {
  return false;
}

/** presenter.get's answer as a presenter (null: nobody presents): nobody here. */
export function parsePresenter(_v: unknown): Presenter | null {
  return null;
}

/** The list item the presenter has selected: none here. */
export function presenterItem(_p: Presenter | null, _list: "sessions" | "machines"): string | undefined {
  return undefined;
}

/** What a title says while it follows the presenter: nothing here. */
export function followingTitle(_p: Presenter | null): string {
  return "";
}
