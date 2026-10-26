//! The seam between the product and its demo mode. Every call site uses
//! these functions unconditionally; without the `showcase` feature each is
//! the identity (or nothing at all), so a release build carries none of it.
//! With the feature, `src/showcase/` swaps private data for space references
//! at output time and can run a scripted demo world (`aleph showcase`).

use crate::config::Config;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Span;
use serde_json::Value;
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[cfg(feature = "showcase")]
pub(crate) mod capture;
#[cfg(feature = "showcase")]
mod celebrate;
#[cfg(feature = "showcase")]
pub mod cmd;
#[cfg(feature = "showcase")]
mod demo;
#[cfg(feature = "showcase")]
mod dict;
#[cfg(feature = "showcase")]
mod fake;
#[cfg(feature = "showcase")]
mod flair;
#[cfg(feature = "showcase")]
mod fx;
#[cfg(feature = "showcase")]
mod gate;
#[cfg(feature = "showcase")]
mod hud;
#[cfg(feature = "showcase")]
mod json;
#[cfg(feature = "showcase")]
mod palette;
#[cfg(feature = "showcase")]
mod reflow;
#[cfg(feature = "showcase")]
mod safety;
#[cfg(feature = "showcase")]
mod scan;
#[cfg(feature = "showcase")]
mod sound;
#[cfg(feature = "showcase")]
mod state;
#[cfg(feature = "showcase")]
mod sync;
#[cfg(all(test, feature = "showcase"))]
mod tests;
#[cfg(feature = "showcase")]
mod tour;
#[cfg(feature = "showcase")]
mod tui;
#[cfg(feature = "showcase")]
mod universe;
#[cfg(feature = "showcase")]
mod web;
#[cfg(feature = "showcase")]
mod words;

/// Is anything being masked (or a demo running) right now?
#[inline]
pub fn on() -> bool {
    #[cfg(feature = "showcase")]
    {
        state::mode() != state::Mode::Off
    }
    #[cfg(not(feature = "showcase"))]
    {
        false
    }
}

/// Changes whenever the mode does (for caches and subscribers).
#[inline]
pub fn epoch() -> u64 {
    #[cfg(feature = "showcase")]
    {
        state::epoch()
    }
    #[cfg(not(feature = "showcase"))]
    {
        0
    }
}

/// Is showcase mode in this build at all?
#[inline]
pub fn built() -> bool {
    cfg!(feature = "showcase")
}

/// `real`, `masked` or `demo` (what a snapshot's marker says).
#[inline]
#[cfg_attr(not(feature = "showcase"), allow(dead_code))]
pub fn mode_name() -> &'static str {
    #[cfg(feature = "showcase")]
    {
        state::mode().name()
    }
    #[cfg(not(feature = "showcase"))]
    {
        "real"
    }
}

/// The dashboard is starting: in showcase mode every name, folder and
/// address gets a new stand-in (each window follows within a second).
#[inline]
pub fn started() {
    #[cfg(feature = "showcase")]
    state::reshuffle();
}

/// Text on its way out (a printed line, a notification, a title).
#[inline]
pub fn text(s: &str) -> Cow<'_, str> {
    #[cfg(feature = "showcase")]
    if on() {
        return Cow::Owned(scan::scrub(s));
    }
    Cow::Borrowed(s)
}

/// A name the dashboard lays out (a machine's alias in a card or a title),
/// as it should read there: with whole names on (`showcase.names = full`)
/// the stand-in itself, so the layout is made for it; else as it is (the
/// frame's own pass swaps it, width for width).
#[inline]
pub fn shown(s: &str) -> Cow<'_, str> {
    #[cfg(feature = "showcase")]
    if on() {
        let d = dict::get();
        if d.names.tui_full() {
            return Cow::Owned(scan::scrub_as(&d, s, true));
        }
    }
    Cow::Borrowed(s)
}

/// A printed line's spans on their way out, masked across them: the words
/// swapped, each span keeping its own part (a longer name pushes the rest of
/// its run along). None when there's nothing to change.
#[inline]
pub fn spans(parts: &[&str]) -> Option<Vec<String>> {
    #[cfg(feature = "showcase")]
    if on() {
        return scan::spans(parts);
    }
    let _ = parts;
    None
}

/// JSON on its way out (`--json`, the socket, HTTP, SSE): masked by key.
#[inline]
pub fn json(v: Value) -> Value {
    // safety: while paused, nothing real goes out at all (what's left — an
    // error, the marker — still masked)
    #[cfg(feature = "showcase")]
    if paused() {
        let v = safety::pause::neutral(v);
        return if on() { json::mask(v) } else { v };
    }
    #[cfg(feature = "showcase")]
    if on() {
        return json::mask(v);
    }
    v
}

/// A drawn frame, just before it's flushed: every private word swapped for
/// one of the same width, cell for cell.
#[inline]
pub fn buffer(buf: &mut ratatui::buffer::Buffer) {
    // safety: while paused, only the stars
    #[cfg(feature = "showcase")]
    if paused() {
        safety::pause::blank(buf, &safety::settings().panic_keys);
        return;
    }
    #[cfg(feature = "showcase")]
    if on() {
        tui::buffer(buf);
    }
    let _ = buf;
}

/// A name coming back in (a selector typed or sent by a client that saw the
/// masked one): the real one it stands for, when it's one of `reals`.
#[inline]
pub fn unmask<'a>(q: &'a str, reals: &[String]) -> Cow<'a, str> {
    #[cfg(feature = "showcase")]
    if on() {
        if let Some(r) = scan::unmask(q, reals) {
            return Cow::Owned(r);
        }
    }
    let _ = reals;
    Cow::Borrowed(q)
}

/// A folder seen only at runtime — an agent running outside herdr, which no
/// snapshot or history names: its words get the same kind of stand-ins as
/// every herdr session's folder.
#[inline]
pub fn learn_folder(path: &str) {
    #[cfg(feature = "showcase")]
    if on() {
        dict::learn(path);
    }
    let _ = path;
}

/// A version a daemon reported, which showcase mode masked on its way out:
/// this build's own when it's this build's stand-in (so comparing the two
/// still tells a stale daemon from a current one).
#[inline]
pub fn own_version(seen: &str) -> Cow<'_, str> {
    #[cfg(feature = "showcase")]
    if on() && seen == fake::version(env!("CARGO_PKG_VERSION")) {
        return Cow::Borrowed(env!("CARGO_PKG_VERSION"));
    }
    Cow::Borrowed(seen)
}

/// What's new in showcase mode (`docs/whats-new-showcase.toml`): notes only
/// this build carries, added to the product's own entries.
#[inline]
pub fn whats_new() -> &'static str {
    #[cfg(feature = "showcase")]
    {
        include_str!("../docs/whats-new-showcase.toml")
    }
    #[cfg(not(feature = "showcase"))]
    {
        ""
    }
}

/// `aleph showcase …` (the command exists only with the feature).
#[inline]
pub fn command(name: &str, rest: &[String]) -> Option<std::process::ExitCode> {
    #[cfg(feature = "showcase")]
    {
        cmd::run(name, rest)
    }
    #[cfg(not(feature = "showcase"))]
    {
        let _ = (name, rest);
        None
    }
}

/// What the app can switch to from inside (the palette): (mode, label,
/// what it does). None in a build without the feature.
#[inline]
pub fn modes() -> &'static [(&'static str, &'static str, &'static str)] {
    #[cfg(feature = "showcase")]
    {
        cmd::MODES
    }
    #[cfg(not(feature = "showcase"))]
    {
        &[]
    }
}

/// Switch from inside the app (`off`, `masked`, `demo`): what to tell you.
#[inline]
pub fn switch(mode: &str) -> Result<String, String> {
    #[cfg(feature = "showcase")]
    {
        cmd::switch_to(mode)
    }
    #[cfg(not(feature = "showcase"))]
    {
        let _ = mode;
        Err(String::new())
    }
}

/// A JSON answer that says which mode it was made in (the web console's
/// banner, Raycast): a marker added while showcase mode is on.
#[inline]
pub fn mark(v: &mut Value) {
    #[cfg(feature = "showcase")]
    if on() && v.is_object() {
        v["showcase"] = Value::from(mode_name());
    }
    // safety: what the clients follow (the pause, money, copies, screens)
    #[cfg(feature = "showcase")]
    if (on() || paused()) && v.is_object() {
        v["showcase_safety"] = safety::marker();
    }
    let _ = v;
}

/// Is this setting showcase mode's own? (Left out of docs/config.md, so the
/// page reads the same from either build.)
#[inline]
pub fn own_key(key: &str) -> bool {
    // every `showcase.*` setting (Settings › Showcase)
    #[cfg(feature = "showcase")]
    {
        key.starts_with("showcase.")
    }
    #[cfg(not(feature = "showcase"))]
    {
        let _ = key;
        false
    }
}

// ------------------------------------------------------------------ flair

/// The header's start (`left` begins with the wordmark and the gap after
/// it): a shimmer across the wordmark and a chip naming the mode.
#[inline]
pub fn header(cfg: &Config, left: &mut Vec<Span<'static>>) {
    #[cfg(feature = "showcase")]
    if on() {
        flair::header(cfg, left);
    }
    let _ = (cfg, left);
}

/// The footer, right of its first `used` cells: the keys just pressed.
#[inline]
pub fn footer(cfg: &Config, buf: &mut Buffer, r: Rect, used: u16) {
    #[cfg(feature = "showcase")]
    if on() {
        flair::footer(cfg, buf, r, used);
    }
    let _ = (cfg, buf, r, used);
}

/// Spare room on the tab row: a tagline.
#[inline]
pub fn spare(cfg: &Config, buf: &mut Buffer, r: Rect) {
    #[cfg(feature = "showcase")]
    if on() {
        flair::spare(cfg, buf, r);
    }
    let _ = (cfg, buf, r);
}

/// A key the dashboard acted on: only ever a command key, never typed text.
/// `does` says what it does there — (its key map action's name, the words)
/// — and is asked only when the keycast names keys (`showcase.keycast`).
#[inline]
pub fn key(cfg: &Config, k: &str, does: impl FnOnce() -> Option<(&'static str, String)>) {
    #[cfg(feature = "showcase")]
    if on() {
        flair::key(cfg, k, does);
        return;
    }
    let _ = (cfg, k, does);
}

/// The dashboard switched to this view.
#[inline]
pub fn switched(view: &str) {
    #[cfg(feature = "showcase")]
    if on() {
        flair::switched(view);
    }
    let _ = view;
}

/// Last on the frame (before the lens): the caption of the view just opened
/// (held back while a popup is up: `quiet`).
#[inline]
pub fn overlay(cfg: &Config, buf: &mut Buffer, area: Rect, quiet: bool) {
    // (demo flow: while the autopilot tour runs, its step's caption instead)
    #[cfg(feature = "showcase")]
    if on() && !tour::overlay(cfg, buf, area, quiet) {
        flair::overlay(cfg, buf, area, quiet);
    }
    let _ = (cfg, buf, area, quiet);
}

/// An empty state's blank rows, after its text is drawn.
#[inline]
pub fn starfield(cfg: &Config, buf: &mut Buffer, r: Rect) {
    #[cfg(feature = "showcase")]
    if on() {
        flair::starfield(cfg, buf, r);
    }
    let _ = (cfg, buf, r);
}

/// May the dashboard's splash play? (Showcase mode's own switch: the
/// `splash` part of its flair.)
#[inline]
pub fn splash_wanted(cfg: &Config) -> bool {
    #[cfg(feature = "showcase")]
    if on() {
        return flair::wants(cfg, flair::Part::Splash);
    }
    let _ = cfg;
    true
}

/// The splash's extras in showcase mode: the mode's chip and a tagline.
#[inline]
pub fn splash_extras(ms: u128) -> Option<(Span<'static>, &'static str)> {
    #[cfg(feature = "showcase")]
    if on() {
        return Some((flair::chip(state::mode(), ms), flair::tagline(ms)));
    }
    let _ = ms;
    None
}

// ---- delight
//
// Celebrations (`showcase/celebrate.rs`, the `celebrate` part of the flair),
// the cockpit HUD (`showcase/hud.rs`, its `hud` part), sound cues
// (`showcase/sound.rs`) and the cockpit palette (`showcase/palette.rs`).

/// The fleet's transitions as the dashboard sees them (`fresh`: machines
/// whose first poll this was, their state no news): a supernova where a
/// session finished, a pulsar's flash where a question was answered.
#[inline]
pub fn transitions(cfg: &Config, tr: &[crate::fleet::Transition], fresh: &[String]) {
    #[cfg(feature = "showcase")]
    if on() {
        celebrate::transitions(cfg, tr, fresh);
    }
    let _ = (cfg, tr, fresh);
}

/// Do showcase's own celebrations play when a session finishes? (The
/// dashboard's light motion then leaves the finish to them.) Never without
/// the feature.
pub fn celebrating(cfg: &Config) -> bool {
    #[cfg(feature = "showcase")]
    if on() {
        return flair::wants(cfg, flair::Part::Celebrate);
    }
    let _ = cfg;
    false
}

/// The dashboard's content area (between the rule under the tabs and the
/// footer), once its view and footer are drawn and before any popup: the
/// cockpit's corner brackets, and the celebrations this frame plays, staged
/// for [`celebrations`] (`quiet`: a popup is up, none play; `places`: each
/// session's Fleet row or tile, asked for only while one plays).
#[inline]
pub fn content(
    cfg: &Config,
    buf: &mut Buffer,
    body: Rect,
    quiet: bool,
    places: impl FnOnce() -> Vec<(String, Rect)>,
) {
    #[cfg(feature = "showcase")]
    if on() {
        hud::brackets(cfg, buf, body);
        celebrate::stage(cfg, body, quiet, places);
    }
    #[cfg(not(feature = "showcase"))]
    let _ = (cfg, buf, body, quiet, places);
}

/// After the lens ([`buffer`]): the celebrations staged by [`content`],
/// laid into the masked frame's open sky (so they never take the room a
/// whole-name stand-in moves into).
#[inline]
pub fn celebrations(buf: &mut Buffer) {
    // (safety: none over the pause's stars)
    #[cfg(feature = "showcase")]
    if !paused() {
        celebrate::finish(buf);
    }
    let _ = buf;
}

/// What the cockpit HUD reads out: the dashboard's own counts.
#[cfg_attr(not(feature = "showcase"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gauges {
    /// machines answering, of `machines`
    pub up: usize,
    pub machines: usize,
    pub working: usize,
    /// sessions waiting for you
    pub needs: usize,
}

/// How many cells at the status bar's right end the HUD's readouts want
/// kept for them (0 when they're off): the key hints fit in the rest.
#[inline]
pub fn status_room(cfg: &Config, gauges: &Gauges) -> u16 {
    #[cfg(feature = "showcase")]
    if on() {
        return hud::room(cfg, gauges);
    }
    let _ = (cfg, gauges);
    0
}

/// The bottom status bar `r`, past its first `used` cells (the key hints):
/// the HUD's live readouts at its right end (`gauges` asked for only when
/// they're shown).
#[inline]
pub fn status(cfg: &Config, buf: &mut Buffer, r: Rect, used: u16, gauges: impl FnOnce() -> Gauges) {
    #[cfg(feature = "showcase")]
    if on() {
        hud::readouts(cfg, buf, r, used, gauges);
    }
    #[cfg(not(feature = "showcase"))]
    let _ = (cfg, buf, r, used, gauges);
}

/// Each hub tick (`Router::tick`: only the process running the hub ticks a
/// router): a sound cue for what the event log says just happened, once
/// per event (`showcase.sounds`; never in tests or the fake world).
#[inline]
pub fn sounds() {
    #[cfg(feature = "showcase")]
    sound::tick();
}

/// The palette `ui::c::init` is building: the cockpit's deep-space tones
/// while the dashboard asks for them ([`repaint`]).
#[inline]
pub fn tint(p: &mut crate::ui::c::Pal) {
    #[cfg(feature = "showcase")]
    palette::tint(p);
    let _ = p;
}

/// Tests that draw pictures (and only those, the palette being the whole
/// process's): the cockpit palette on or off.
#[cfg(all(test, feature = "showcase"))]
pub(crate) fn force_palette(on: bool) {
    palette::force(on);
}

/// Every dashboard frame: its palette follows showcase mode and
/// `showcase.palette` (built again when either changes; never in tests,
/// the palette being the whole process's).
#[inline]
pub fn repaint(cfg: &Config) {
    #[cfg(feature = "showcase")]
    if !cfg!(test) {
        palette::repaint(cfg);
    }
    let _ = cfg;
}

// ---- capture
//
// Recording the dashboard (an asciinema cast of the bytes as drawn, after
// the lens), posters (a frame as an SVG card), your own watermark and the
// keycast's names: `showcase/capture/`. Without the feature each is nothing.

/// Bytes a full-screen view sent the terminal (`term::Out`): a copy for the
/// recording, while one runs.
#[inline]
pub fn tee(bytes: &[u8]) {
    #[cfg(feature = "showcase")]
    capture::tee(bytes);
    let _ = bytes;
}

/// That writer flushed: what it sent since is one event of the recording.
#[inline]
pub fn tee_flush() {
    #[cfg(feature = "showcase")]
    capture::tee_flush();
}

/// Last on the dashboard's frame, after the lens: your watermark (yours, so
/// never masked), a recording's first frame, a poster the palette asked for.
#[inline]
pub fn finish(cfg: &Config, buf: &mut Buffer, view: &str) {
    #[cfg(feature = "showcase")]
    capture::finish(cfg, buf, view);
    let _ = (cfg, buf, view);
}

/// The header, after its wordmark: `● rec 0:42` while recording.
#[inline]
pub fn rec_chip(cfg: &Config, left: &mut Vec<Span<'static>>) {
    #[cfg(feature = "showcase")]
    capture::rec_chip(cfg, left);
    let _ = (cfg, left);
}

/// A key in the dashboard's normal mode: true when it was the record key
/// (a recording started or stopped; it isn't keycast).
#[inline]
pub fn capture_key(cfg: &Config, k: &str) -> bool {
    #[cfg(feature = "showcase")]
    {
        capture::key(cfg, k)
    }
    #[cfg(not(feature = "showcase"))]
    {
        let _ = (cfg, k);
        false
    }
}

/// Does a footer hint's key (`↑↓`, `[ ]`, `hjkl`, `ctrl+k`) name the key
/// pressed? (The keycast's names, from the hints.)
#[inline]
pub fn hint_has(hint: &str, k: &str) -> bool {
    #[cfg(feature = "showcase")]
    {
        capture::names::hint_has(hint, k)
    }
    #[cfg(not(feature = "showcase"))]
    {
        hint == k
    }
}

/// The palette's showcase entries: (id, label, what it does) — capture's
/// (record, poster) and the demo flow's (the autopilot tour).
#[inline]
pub fn captures(cfg: &Config) -> Vec<(&'static str, String, String)> {
    #[cfg(feature = "showcase")]
    {
        let mut v = capture::items(cfg);
        v.extend(tour::items(cfg));
        v
    }
    #[cfg(not(feature = "showcase"))]
    {
        let _ = cfg;
        Vec::new()
    }
}

/// One of them chosen.
#[inline]
pub fn capture(cfg: &Config, id: &str) {
    #[cfg(feature = "showcase")]
    if id == "tour" {
        tour::start();
    } else {
        capture::run(cfg, id);
    }
    let _ = (cfg, id);
}

/// What a recording or a poster has to say: (ok, words), for a toast that
/// stays long enough to read a path.
#[inline]
pub fn capture_note() -> Option<(bool, String)> {
    #[cfg(feature = "showcase")]
    {
        capture::note()
    }
    #[cfg(not(feature = "showcase"))]
    {
        None
    }
}

// ---- video
//
// Stopping a recording renders it as an MP4 and a GIF in the background
// (`showcase/capture/video.rs`, frames drawn by `raster.rs`; encoded by the
// ffmpeg already here). Where they went comes back through `capture_note`;
// what can't wait to be read — no ffmpeg, so only the .cast was saved — as
// an alert.

/// The warnings a recording has to hold up, each handed to `f` (the
/// dashboard shows it as an alert).
#[inline]
pub fn capture_alerts(f: impl FnMut(String)) {
    #[cfg(feature = "showcase")]
    capture::alerts(f);
    #[cfg(not(feature = "showcase"))]
    let _ = f;
}

/// The dashboard hands the terminal over (`quit`: for good): a recording
/// pauses or stops. Some: what to print once the screen is gone.
#[inline]
pub fn cast_away(cfg: &Config, quit: bool) -> Option<String> {
    #[cfg(feature = "showcase")]
    {
        capture::away(cfg, quit)
    }
    #[cfg(not(feature = "showcase"))]
    {
        let _ = (cfg, quit);
        None
    }
}

/// The dashboard has the terminal back: a paused recording goes on.
#[inline]
pub fn cast_back() {
    #[cfg(feature = "showcase")]
    capture::back();
}

/// A JSON answer for the web console (`/api/v1/me`): your watermark, while
/// showcase mode shows it there.
#[inline]
pub fn stamp(cfg: &Config, v: &mut Value) {
    #[cfg(feature = "showcase")]
    if on() {
        capture::stamp(cfg, v);
    }
    let _ = (cfg, v);
}

// ------------------------------------------------------------------ attach

/// A hand-off to a screen the lens can't reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outside {
    /// `a` / enter: attach in herdr
    Attach,
    /// herdr's own screen some other way (`A`, `O`, the hub)
    Herdr,
    /// the settings file in `$EDITOR`
    Editor,
}

/// What to do instead of handing the terminal over.
#[cfg_attr(not(feature = "showcase"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// go ahead
    Open,
    /// open it in Aleph's Focus (masked) instead; the note for when it's
    /// already there
    Focus(&'static str),
    /// ask first
    Ask {
        title: &'static str,
        text: &'static str,
    },
    /// don't (the note says why)
    Refuse(&'static str),
}

/// May this hand-off go ahead? Always, unless showcase mode is on.
#[inline]
pub fn gate(what: Outside) -> Gate {
    #[cfg(feature = "showcase")]
    if on() {
        return gate::gate(what);
    }
    let _ = what;
    Gate::Open
}

// ------------------------------------------------------------------ web

/// The web console's extra files: (path, type, bytes).
#[inline]
pub fn web_assets() -> &'static [(&'static str, &'static str, &'static [u8])] {
    #[cfg(feature = "showcase")]
    {
        web::ASSETS
    }
    #[cfg(not(feature = "showcase"))]
    {
        &[]
    }
}

/// The console's index.html as served (with those files' tags).
#[inline]
pub fn web_index(html: String) -> String {
    #[cfg(feature = "showcase")]
    {
        web::index(html)
    }
    #[cfg(not(feature = "showcase"))]
    {
        html
    }
}

/// Tests: run as if `aleph showcase on` (`masked`), `on --demo` (`demo`)
/// or `off` (anything else) had been run, on this thread.
#[cfg(all(test, feature = "showcase"))]
pub(crate) fn force(mode: &str) {
    let m = match mode {
        "masked" => state::Mode::Masked,
        "demo" => state::Mode::Demo,
        _ => {
            state::force(None);
            return;
        }
    };
    state::force(Some(state::State {
        mode: m,
        salt: b"0123456789abcdef0123456789abcdef".to_vec(),
        epoch: 1,
        ..Default::default()
    }));
}

/// What a private word is (a machine's name wins over a folder's).
#[cfg_attr(not(feature = "showcase"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Host,
    User,
    Project,
    Session,
    Other,
}

// ------------------------------------------------------------------ the demo world
//
// `aleph showcase on --demo`: a scripted fleet stands in for the real one.
// Each hook below is None (or nothing) unless the demo world is on; then it
// answers from the simulation (`showcase/demo/`), and nothing real — no ssh,
// no herdr, no network — is ever reached.

/// Is the scripted demo world on?
#[inline]
pub fn demo() -> bool {
    #[cfg(feature = "showcase")]
    {
        state::mode() == state::Mode::Demo
    }
    #[cfg(not(feature = "showcase"))]
    {
        false
    }
}

/// The kill switch: what to run instead of ssh or herdr in the demo world
/// (a path that can't exist, so nothing starts).
#[inline]
pub fn dead_bin() -> Option<String> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::DEAD.to_string());
    }
    None
}

/// Why a herdr call doesn't happen in the demo world (None: it does).
#[inline]
pub fn not_here() -> Option<String> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::NOT_HERE.to_string());
    }
    None
}

/// Why the demo world won't do this (heal, wake, deploy, install, …).
#[inline]
pub fn refused() -> Option<String> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::REFUSED.to_string());
    }
    None
}

/// A setting as the demo world has it (its machines, analytics on, …):
/// Some(value) overrides the files, None leaves them.
#[inline]
pub fn setting(key: &str) -> Option<Option<String>> {
    #[cfg(feature = "showcase")]
    if demo::decides(key) && demo() {
        return demo::setting(key);
    }
    let _ = key;
    None
}

/// Why `key` can't be changed now (the demo world's machines are fixed).
#[inline]
pub fn refuse_setting(key: &str) -> Option<String> {
    #[cfg(feature = "showcase")]
    if demo() {
        return demo::refuse_setting(key);
    }
    let _ = key;
    None
}

/// Where the world's own state goes: `dir` itself, or in the demo world its
/// sandbox inside it (so nothing of the real fleet's is touched).
#[inline]
pub fn sandbox(dir: PathBuf) -> PathBuf {
    #[cfg(feature = "showcase")]
    if demo() {
        return demo::sandbox(dir);
    }
    dir
}

/// The demo world went on or off since this process started: it should start
/// again as the other world (the daemon, the dashboard).
#[inline]
pub fn world_changed() -> bool {
    #[cfg(feature = "showcase")]
    {
        demo::world_changed()
    }
    #[cfg(not(feature = "showcase"))]
    {
        false
    }
}

/// Start this process again, same arguments (returns only if that failed).
#[inline]
pub fn restart() {
    #[cfg(feature = "showcase")]
    demo::restart();
}

/// `ssh` config aliases.
#[inline]
pub fn aliases() -> Option<Vec<String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::aliases());
    }
    None
}

/// `ssh -G`'s answer for `host`.
#[inline]
pub fn resolve(host: &str) -> Option<Option<crate::sshcfg::Resolved>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::machines::resolve(host));
    }
    let _ = host;
    None
}

// ---- herdr

/// The agent sessions on `host` (Err: it doesn't answer).
#[inline]
pub fn sessions(
    host: &str,
    f: &crate::agents::Filter,
) -> Option<Result<Vec<crate::herdr::Session>, String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::sessions(host, f));
    }
    let _ = (host, f);
    None
}

/// A pane's text (`source`: visible | recent | detection).
#[inline]
pub fn read(
    host: &str,
    pane: &str,
    source: &str,
    lines: usize,
    ansi: bool,
) -> Option<Result<String, String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::read(host, pane, source, lines, ansi));
    }
    let _ = (host, pane, source, lines, ansi);
    None
}

/// Keys to a session (`1`, `enter`, …).
#[inline]
pub fn keys(host: &str, target: &str, keys: &[&str]) -> Option<Result<(), String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::keys(host, target, keys));
    }
    let _ = (host, target, keys);
    None
}

/// Text to a session: typed (`submit` false) or a prompt (true).
#[inline]
pub fn send(host: &str, target: &str, text: &str, submit: bool) -> Option<Result<(), String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::send(host, target, text, submit));
    }
    let _ = (host, target, text, submit);
    None
}

/// Close a pane or a tab.
#[inline]
pub fn close(host: &str, id: &str) -> Option<Result<(), String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::close(host, id));
    }
    let _ = (host, id);
    None
}

/// Rename a tab (`tab`) or an agent.
#[inline]
pub fn rename(host: &str, target: &str, to: &str, tab: bool) -> Option<Result<(), String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::rename(host, target, to, tab));
    }
    let _ = (host, target, to, tab);
    None
}

/// What starting a tab gives: the sessions started, and its panes.
pub type NewTab = (Vec<crate::herdr::Started>, Vec<String>);

/// A new session's tab: what started, and its panes.
#[inline]
pub fn new_tab(host: &str, o: &crate::herdr::NewOpts) -> Option<Result<NewTab, String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::new_tab(host, o));
    }
    let _ = (host, o);
    None
}

/// A herdr call with nothing to say back (tokens, focus, the sidebar's
/// agent name): fine in the demo world, when `host` answers.
#[inline]
pub fn herdr_ok(host: &str) -> Option<Result<(), String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::up(host));
    }
    let _ = host;
    None
}

/// `herdr status server --json` on `host`.
#[inline]
pub fn server_status(host: &str) -> Option<Option<crate::herdr::ServerStatus>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::server_status(host));
    }
    let _ = host;
    None
}

/// `herdr --version` on `host`.
#[inline]
pub fn herdr_version(host: &str) -> Option<Option<String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::herdr_version(host));
    }
    let _ = host;
    None
}

/// What the fleet keeps of a machine's herdr: (version, OS line).
#[inline]
pub fn herdr_info(host: &str) -> Option<(String, String)> {
    #[cfg(feature = "showcase")]
    if demo() {
        return demo::herdr_info(host);
    }
    let _ = host;
    None
}

/// The workspaces on `host`.
#[inline]
pub fn workspaces(host: &str) -> Option<Result<Vec<crate::herdr::Workspace>, String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::workspaces(host));
    }
    let _ = host;
    None
}

/// The git worktrees of a workspace's repository on `host`.
#[inline]
pub fn worktrees(host: &str) -> Option<Result<Vec<crate::herdr::Worktree>, String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::worktrees(host));
    }
    let _ = host;
    None
}

/// The tab labels on `host`.
#[inline]
pub fn tab_labels(host: &str) -> Option<Vec<String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::tab_labels(host));
    }
    let _ = host;
    None
}

/// `herdr agent wait`: the state once it's one of `until` (or the time is up).
#[inline]
pub fn wait(
    host: &str,
    target: &str,
    until: &[String],
    timeout: Option<std::time::Duration>,
) -> Option<Result<String, String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::wait(host, target, until, timeout));
    }
    let _ = (host, target, until, timeout);
    None
}

// ---- what else a machine tells

/// A live tile's frame: the pane's screen as ANSI text, drawn at `size`.
#[inline]
pub fn screen(host: &str, pane: &str, size: Option<(u16, u16)>) -> Option<Result<String, String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::screen(host, pane, size));
    }
    let _ = (host, pane, size);
    None
}

/// Machine stats as a stream: true when the demo world fed `st` (until
/// `stop`), false to sample the machine for real.
#[inline]
pub fn stats(
    alias: &str,
    interval: u64,
    st: &Arc<Mutex<crate::telemetry::Stats>>,
    stop: &Arc<Mutex<bool>>,
) -> bool {
    #[cfg(feature = "showcase")]
    if demo() {
        demo::stats::feed(alias, interval, st, stop);
        return true;
    }
    let _ = (alias, interval, st, stop);
    false
}

/// What's installed on `host` (agents, herdr, tools).
#[inline]
pub fn probe(host: &str) -> Option<crate::probe::HostProbe> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::machines::probe(host));
    }
    let _ = host;
    None
}

/// The LLM servers on `alias` (only the services in `on`).
#[inline]
pub fn services(
    alias: &str,
    on: &[&'static crate::services::Service],
) -> Option<crate::services::Scan> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::machines::services(alias, on));
    }
    let _ = (alias, on);
    None
}

/// One machine's usage extract: the records since its last good read.
#[inline]
pub fn usage(host: &str) -> Option<Result<crate::usage::extract::Output, String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::usage::run(host));
    }
    let _ = host;
    None
}

/// Codex's plan windows on `host`, as its app server would say them.
#[inline]
pub fn codex_limits(host: &str) -> Option<Option<crate::usage::parse::Limit>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::usage::codex(host));
    }
    let _ = host;
    None
}

/// A demo session's timeline words (`steps::words`), from its steps.
#[inline]
pub fn timeline_words(
    steps: &[crate::usage::steps::Step],
) -> Option<std::collections::HashMap<String, String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::usage::words(steps));
    }
    let _ = steps;
    None
}

/// Which repo each folder is in, as the demo world's machines would say it.
#[inline]
pub fn projects(
    host: &str,
    folders: &[String],
) -> Option<Vec<(String, crate::usage::projects::Folder)>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::usage::projects(host, folders));
    }
    let _ = (host, folders);
    None
}

/// The folder browser's connection: the demo world answers its requests
/// from a made-up folder tree (nothing without the feature).
#[inline]
pub(crate) fn folders(
    host: String,
    rx: std::sync::mpsc::Receiver<crate::remote::Msg>,
    tx: std::sync::mpsc::Sender<crate::remote::Ev>,
) {
    #[cfg(feature = "showcase")]
    demo::browse::serve(host, rx, tx);
    #[cfg(not(feature = "showcase"))]
    let _ = (host, rx, tx);
}

/// A doctor part's lines.
#[inline]
pub fn doctor(p: &crate::doctor::Part) -> Option<Vec<crate::doctor::Line>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::machines::doctor(p));
    }
    let _ = p;
    None
}

/// Is this notification sink kept quiet? The demo world sends only desktop
/// banners and the menu bar's refresh (`remote`: ntfy, Slack, a webhook,
/// Web Push, herdr's own toasts).
#[inline]
pub fn quiet_sink(remote: bool) -> bool {
    // safety: while paused, no banner pops up on the screen being shown
    #[cfg(feature = "showcase")]
    if paused() && !remote {
        return true;
    }
    #[cfg(feature = "showcase")]
    if demo() {
        return remote;
    }
    let _ = remote;
    false
}

// ---- safety
//
// Safety nets for live demos (`showcase/safety/`), each switched in Settings
// › Showcase: agents' screens redacted or hidden, the pause (the panic key),
// what a copy puts on the clipboard, and $ amounts scaled or hidden. All of
// them act at output only; without the feature each is the identity.

/// What a hidden $ amount reads as.
pub const MONEY_HIDDEN: &str = "$—";

/// Is the pause on (the panic key, `aleph showcase panic`)? Every window
/// shows the stars and the daemon's answers carry nothing real, in any mode.
#[inline]
pub fn paused() -> bool {
    #[cfg(feature = "showcase")]
    {
        state::panic()
    }
    #[cfg(not(feature = "showcase"))]
    {
        false
    }
}

/// A key the dashboard got (`key_str`): true when showcase took it — it
/// was the panic key (the pause switched), or the pause is on (every other
/// key waits until it's off).
#[inline]
pub fn panic_key(key: &str) -> bool {
    #[cfg(feature = "showcase")]
    {
        if safety::settings().panic_keys.iter().any(|k| k == key) {
            safety::pause::toggle();
            return true;
        }
        paused()
    }
    #[cfg(not(feature = "showcase"))]
    {
        let _ = key;
        false
    }
}

/// A daemon answer to `method` on its way out: [`json`], except that while
/// paused what Aleph itself asks (the daemon's pid and status, a refresh)
/// is answered as ever.
#[inline]
pub fn answer(method: &str, v: Value) -> Value {
    #[cfg(feature = "showcase")]
    if paused() && safety::pause::internal(method) {
        return if on() { json::mask(v) } else { v };
    }
    let _ = method;
    json(v)
}

/// JSON masked like [`json`] but never blanked by the pause: what a CLI run
/// prints (`--json`: the shell is the presenter's, and what a command just
/// did — a token it made — must be seen) and a phone's push (the phone's
/// sinks go on while the screen is paused).
#[inline]
pub fn masked(v: Value) -> Value {
    #[cfg(feature = "showcase")]
    if on() {
        return json::mask(v);
    }
    v
}

/// The pictures a frame asked for (logos as images): none while paused,
/// and the ones up taken away (so they're sent again after).
#[inline]
pub fn pictures(p: String) -> String {
    #[cfg(feature = "showcase")]
    if paused() {
        return crate::logo::images::clear();
    }
    p
}

/// An agent's screen on its way out as text (plain or ANSI: `aleph peek`,
/// `screen.read`, a stream, a notification quoting it): redacted or hidden
/// (`showcase.screens`), else as it is (the lens comes after).
#[inline]
pub fn screen_text(s: &str) -> Cow<'_, str> {
    #[cfg(feature = "showcase")]
    if paused() {
        return Cow::Borrowed("");
    }
    #[cfg(feature = "showcase")]
    if on() {
        if let Some(r) = safety::screen_text(s) {
            return Cow::Owned(r);
        }
    }
    Cow::Borrowed(s)
}

/// An agent's screen just drawn into `area` (a Focus or Tile tile, a
/// preview): redacted or hidden there. True when it's hidden (no cursor).
#[inline]
pub fn screen_area(buf: &mut Buffer, area: Rect) -> bool {
    #[cfg(feature = "showcase")]
    if on() {
        return safety::screen_area(buf, area);
    }
    let _ = (buf, area);
    false
}

/// What a copy of `text` puts on the clipboard (`screen`: it's an agent's
/// screen): the masked text, the real one, or None for nothing
/// (`showcase.clipboard`).
#[inline]
pub fn clip(text: &str, screen: bool) -> Option<Cow<'_, str>> {
    #[cfg(feature = "showcase")]
    if paused() {
        return None;
    }
    #[cfg(feature = "showcase")]
    if on() {
        return safety::clip(text, screen);
    }
    let _ = screen;
    Some(Cow::Borrowed(text))
}

/// Why a copy didn't happen, when showcase stopped it.
#[inline]
pub fn clip_refused() -> Option<&'static str> {
    #[cfg(feature = "showcase")]
    if paused() || (on() && clip("", false).is_none()) {
        return Some(safety::CLIP_OFF);
    }
    None
}

/// A $ amount as it shows (`showcase.money`): scaled, or None when hidden
/// ([`MONEY_HIDDEN`]).
#[inline]
pub fn money(v: f64) -> Option<f64> {
    #[cfg(feature = "showcase")]
    if on() {
        return safety::money(v);
    }
    Some(v)
}

// ---- demo flow
//
// The demo world's story and time warp (`showcase/demo/story.rs`,
// `clock.rs`): the hub's rules and self-heal, its limit news and the
// daemon's polls as the world needs them. Without the feature each is
// nothing (or the value it's handed).

/// The hub's auto-approve rules besides yours: the demo world's own (its
/// story's, which answers its soak test and nothing else).
#[inline]
pub fn world_rules() -> Vec<(String, String)> {
    #[cfg(feature = "showcase")]
    if demo() {
        return demo::rules();
    }
    Vec::new()
}

/// Does the hub's self-heal run in the demo world? (Its story's drop: what
/// it restarts is the world's own herdr.)
#[inline]
pub fn world_heals() -> bool {
    #[cfg(feature = "showcase")]
    if demo() {
        return demo::heals();
    }
    false
}

/// Restarting herdr on `host` (self-heal, `S`, the doctor's fix): in the
/// demo world, Some(what happened there); None: not the demo world.
#[inline]
pub fn world_heal(host: &str) -> Option<Result<String, String>> {
    #[cfg(feature = "showcase")]
    if demo() {
        return Some(demo::heal(host));
    }
    let _ = host;
    None
}

/// How long the hub counts a usage limit hit as told (`secs` as ever): the
/// demo world's story hits its own every cycle.
#[inline]
pub fn limit_told_for(secs: u64) -> u64 {
    #[cfg(feature = "showcase")]
    if demo() {
        return demo::told_for(secs);
    }
    secs
}

/// How often the daemon polls the fleet (`d` as ever): the demo world
/// answers at once, so every second — its beats, warped, are short.
#[inline]
pub fn poll_every(d: std::time::Duration) -> std::time::Duration {
    #[cfg(feature = "showcase")]
    if demo() {
        return d.min(std::time::Duration::from_secs(1));
    }
    d
}

// The autopilot tour (`showcase/tour.rs`): a timed walk through the
// dashboard, steered through [`Pilot`] — navigation only.

/// What the autopilot tour may do on the dashboard: show, walk, open Focus
/// or Tile. Never approve, send, close, attach or type.
#[cfg_attr(not(feature = "showcase"), allow(dead_code))]
pub trait Pilot {
    /// The dashboard's settings.
    fn cfg(&self) -> &Config;
    /// Free to be driven: no popup, form, filter, typing or layout mode.
    fn free(&self) -> bool;
    /// Show a view or sub-view by its `showcase.tour_views` name (`fleet`,
    /// `monitor`, `usage`, `inbox`, `doctor`, …); false: none such here.
    fn show(&mut self, name: &str) -> bool;
    /// The view's next item — Fleet's next session, the Inbox's next row,
    /// Monitor's next machine (`first`: its first one).
    fn walk(&mut self, first: bool);
    /// Focus on a session, a working one first, the one after `after`: its
    /// key (None: nothing to show).
    fn focus(&mut self, after: Option<&str>) -> Option<String>;
    /// A machine's sessions side by side, the one after `after`: the machine.
    fn tile(&mut self, after: Option<&str>) -> Option<String>;
}

/// Every turn of the dashboard's loop: the autopilot tour starts (asked
/// for, or idle long enough) or carries on.
#[inline]
pub fn autopilot(p: &mut impl Pilot) {
    #[cfg(feature = "showcase")]
    tour::tick(p);
    let _ = p;
}

/// A key or a click reached the dashboard: true when it only stopped the
/// autopilot tour (it does nothing else then).
#[inline]
pub fn tour_input() -> bool {
    #[cfg(feature = "showcase")]
    {
        tour::input()
    }
    #[cfg(not(feature = "showcase"))]
    {
        false
    }
}

/// A key in the dashboard's normal mode: true when it was the tour's key
/// (`showcase.tour_key`) and the tour starts.
#[inline]
pub fn tour_key(cfg: &Config, k: &str) -> bool {
    #[cfg(feature = "showcase")]
    {
        tour::key(cfg, k)
    }
    #[cfg(not(feature = "showcase"))]
    {
        let _ = (cfg, k);
        false
    }
}

/// Is `k` the tour's key? (The keycast's words for it.)
#[inline]
pub fn is_tour_key(cfg: &Config, k: &str) -> bool {
    #[cfg(feature = "showcase")]
    {
        on() && tour::is_key(cfg, k)
    }
    #[cfg(not(feature = "showcase"))]
    {
        let _ = (cfg, k);
        false
    }
}

/// The header, after the mode's chip: `▶ tour 2/5` while the tour runs.
#[inline]
pub fn tour_chip(cfg: &Config, left: &mut Vec<Span<'static>>) {
    #[cfg(feature = "showcase")]
    if on() {
        tour::chip(cfg, left);
    }
    let _ = (cfg, left);
}

// Presenter sync (`showcase/sync.rs`): Raycast and the web console follow
// the dashboard you present from, through the daemon.

/// What the presenter's dashboard shows.
#[cfg_attr(not(feature = "showcase"), allow(dead_code))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presenting {
    /// `fleet`, `focus`, `tile`, `monitor` (and its sub-views by name:
    /// `usage`, `gpu`, …), `inbox`, `settings`
    pub view: String,
    /// the machine selected
    pub host: Option<String>,
    /// the session selected (or in Focus): its pane and name
    pub pane: Option<String>,
    pub name: Option<String>,
}

/// The dashboard's every turn: what it shows goes to the daemon when it
/// changes (`what` asked for only while `showcase.sync` is on).
#[inline]
pub fn present(cfg: &Config, what: impl FnOnce() -> Presenting) {
    #[cfg(feature = "showcase")]
    sync::present(cfg, what);
    #[cfg(not(feature = "showcase"))]
    let _ = (cfg, what);
}

/// A daemon method only a showcase build has (`presenter.get`, `.set`):
/// Some(its answer); None: not one of them.
#[inline]
pub fn method(cfg: &Config, name: &str, p: &Value, remote: bool) -> Option<Result<Value, String>> {
    #[cfg(feature = "showcase")]
    match name {
        "presenter.get" => return Some(Ok(sync::get(cfg))),
        "presenter.set" => return Some(sync::set(p, remote)),
        _ => {}
    }
    let _ = (cfg, name, p, remote);
    None
}

/// Where a subscriber's topic is now, for a showcase build's own topics
/// (`presenter`); 0 for any other.
#[inline]
pub fn topic_stamp(topic: &str) -> u64 {
    #[cfg(feature = "showcase")]
    if topic == "presenter" {
        return sync::stamp();
    }
    let _ = topic;
    0
}

/// Showcase's own keys for the help and its search: (keys, what, the
/// palette entry enter runs — "" for none —, search words).
#[allow(clippy::type_complexity)]
pub fn help_keys(cfg: &Config) -> Vec<(String, String, &'static str, &'static [&'static str])> {
    #[cfg(feature = "showcase")]
    {
        let keys = |k: &str| {
            let k = cfg.get_or(k);
            (!matches!(k.trim(), "" | "off" | "none")).then(|| k.replace(',', " · "))
        };
        let mut v = Vec::new();
        v.extend(tour::help(cfg));
        if let Some(k) = keys("showcase.record_key") {
            v.push((
                k,
                "showcase: start or stop recording the dashboard".to_string(),
                "record",
                &["record", "cast", "video", "capture", "mp4", "gif"][..],
            ));
        }
        if let Some(k) = keys("showcase.panic_key") {
            v.push((
                k,
                "showcase: pause every window, at once (again: back)".to_string(),
                "",
                &["panic", "pause", "blank", "hide", "privacy"][..],
            ));
        }
        v
    }
    #[cfg(not(feature = "showcase"))]
    {
        let _ = cfg;
        Vec::new()
    }
}
