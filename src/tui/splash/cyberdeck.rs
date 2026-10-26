//! `cyberdeck`: a neural interface booting, in a cyberpunk HUD's language.
//! Cold start: black, a cursor blinking on the boot prompt; the screen
//! powers on, a line of light opening into scanlines, rows torn sideways in
//! red and cyan with a little static. The HUD's frame traces itself in
//! off-red from its cut corner, its labels typing in in capitals beside it.
//! The diagnostics type in, in red (an alarm in crimson that clears); your
//! machines come online as cyan readouts, a bar filling for each; the
//! yellow sync meter climbs, with a hiccup. The sync completes: a flash,
//! the panels' brackets snap tight, a bright yellow scan line sweeps the
//! screen. It ends steady, its readouts live, with one last glitch. With
//! the logo the ℵ assembles out of the HUD in two ghosts, red and cyan,
//! that converge and snap into focus in yellow.

use super::scene::{Frame, GATHER_S};
use super::*;

type Rgb = (u8, u8, u8);

pub(super) const TITLE: &str = "jacking in";
/// Its colours on the way into the ℵ: the chrome's red, then yellow.
pub(super) const COLS: [Rgb; 2] = [RED, YELLOW];

/// The interface: electric yellow (the sync, what needs you), off-red
/// chrome, crimson alarms, cyan data; the chrome's quiet red (rules,
/// leaders, addresses) and the data's (a bar's empty part); the flash.
const YELLOW: Rgb = (252, 238, 10);
const RED: Rgb = (255, 74, 87);
const CRIMSON: Rgb = (255, 0, 60);
const CYAN: Rgb = (94, 246, 255);
const DIM: Rgb = (168, 54, 64);
const DEEP: Rgb = (44, 118, 128);
const WHITE: Rgb = (255, 252, 230);
/// The ℵ in focus: pale, electric, deep yellow, top left to bottom right.
const FOCUS: [Rgb; 3] = [(255, 250, 160), (252, 238, 10), (226, 200, 0)];

/// The acts, as shares of the scene: the boot prompt (the cursor alone,
/// then typing from the first, gone into the power-on's line by the
/// second); the power-on, a line of light opening into scanlines that fade
/// as the frame comes; the frame tracing itself, its labels after.
const PROMPT: (f64, f64) = (0.035, 0.105);
const POWER: (f64, f64) = (0.085, 0.16);
const LINES: (f64, f64) = (0.15, 0.34);
const TRACE: (f64, f64) = (0.12, 0.32);
const LABELS: f64 = 0.2;
/// The diagnostics, the machines coming online, the sync climbing (a
/// hiccup in the middle), the sync complete and its sweep.
const DIAG: (f64, f64) = (0.27, 0.66);
const NODES: (f64, f64) = (0.36, 0.76);
const SYNC: (f64, f64) = (0.36, 0.8);
const HICCUP: (f64, f64) = (0.555, 0.615);
const DONE: f64 = 0.8;
const SWEEP: (f64, f64) = (0.8, 0.9);
/// The glitches: three as it powers on, one at the sync's hiccup, the last.
const TEARS: [(f64, f64); 5] = [
    (0.05, 0.064),
    (0.1, 0.112),
    (0.132, 0.158),
    (0.585, 0.6),
    (0.93, 0.952),
];
/// How fast it types: characters a share of the scene.
const RATE: f64 = 420.0;

/// How far the sync is, 0 → 1: a surge, the hiccup (holding), the rest.
fn sync(u: f64) -> f64 {
    0.47 * smooth(u, SYNC.0, HICCUP.0) + 0.53 * smooth(u, HICCUP.1, SYNC.1)
}

/// `s` as typed by `u`, from `start`.
fn typed(s: &str, u: f64, start: f64) -> String {
    let n = ((u - start) * RATE).max(0.0) as usize;
    s.chars().take(n).collect()
}

/// A box, its corners inclusive.
#[derive(Clone, Copy, Debug)]
struct Bx {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Bx {
    fn w(self) -> i32 {
        self.x1 - self.x0 + 1
    }
}

/// Where everything goes on this screen.
struct Hud {
    /// the outer frame
    out: Bx,
    /// the diagnostics and the uplink (your machines), side by side
    diag: Bx,
    up: Bx,
    /// the sync: a block of its own with big digits (`big`), or a row
    sync: Bx,
    big: bool,
    /// the big digits' height (rows)
    dh: i32,
    /// the cortex trace under the sync (big screens)
    wave: Option<Bx>,
    /// the diagnostics that fit
    checks: Vec<Check>,
}

/// A line of the diagnostics: what's checked, its status (and a short one
/// for a narrow panel), the status's colour.
struct Check {
    what: String,
    status: String,
    short: String,
    col: Rgb,
}

/// The boot's checks (the third raises an alarm first, and clears).
const CHECKS: [(&str, &str, &str); 10] = [
    ("NEURAL LINK", "OK", "OK"),
    ("OPTICS", "CALIBRATED", "OK"),
    ("ICE", "NOMINAL", "OK"),
    ("CORTEX BUS", "OK", "OK"),
    ("SYNAPSE MAP", "LOADED", "OK"),
    ("FIREWALL", "ACTIVE", "OK"),
    ("BIOMONITOR", "STABLE", "OK"),
    ("THERMALS", "41.2°C", "41°C"),
    ("MEMORY", "MAPPED", "OK"),
    ("REFLEXES", "ONLINE", "OK"),
];
const ALARM: usize = 2;

impl Hud {
    fn of(f: &Frame) -> Hud {
        let (w, h) = (f.w as i32, f.h as i32);
        let out = Bx {
            x0: 1,
            y0: 0,
            x1: w - 2,
            y1: h - 4,
        };
        let inner = out.y1 - out.y0 - 1;
        let span = out.x1 - out.x0;
        let big = inner >= 22 && w >= 80;
        // a narrow screen: the panels closer to the frame and each other
        let (margin, gap) = if w >= 80 {
            (3, (span / 10).max(2))
        } else {
            (2, 1)
        };
        let pw = ((span - 2 * margin + 1 - gap) / 2).min(64);
        let dh = match inner {
            i if i >= 48 => 7,
            i if i >= 33 => 5,
            _ => 3,
        };
        // big: the panels, the sync and the trace, the room left over shared
        // out between the trace and the gaps, then centred in the frame
        let (mut g, mut wh) = (2, 5);
        let (top, n) = if big {
            let n = (inner - dh - 15).clamp(4, 12);
            let tot = |g: i32, wh: i32| n + 2 + g + dh + 3 + g + wh;
            let extra = inner - 2 - tot(g, wh);
            if extra > 0 {
                wh += (extra / 2).min(8);
                g += extra / 8;
            }
            (out.y0 + 1 + ((inner - tot(g, wh)) / 2).max(1), n)
        } else {
            let top = out.y0 + 1 + (inner >= 12) as i32;
            (top, (out.y1 - 3 - top - 1).max(1))
        };
        let diag = Bx {
            x0: out.x0 + margin,
            y0: top,
            x1: out.x0 + margin + pw - 1,
            y1: top + n + 1,
        };
        let up = Bx {
            x0: out.x1 - margin - pw + 1,
            y0: top,
            x1: out.x1 - margin,
            y1: top + n + 1,
        };
        let (sync, wave) = if big {
            let sw = ((span as f64 * 0.62) as i32).clamp(44, 96);
            let cx = f.cx as i32;
            let s = Bx {
                x0: cx - sw / 2,
                y0: diag.y1 + g,
                x1: cx - sw / 2 + sw - 1,
                y1: diag.y1 + g + 2 + dh,
            };
            let wv = Bx {
                x0: out.x0 + 3,
                y0: s.y1 + g,
                x1: out.x1 - 3,
                y1: s.y1 + g + wh - 1,
            };
            (s, Some(wv))
        } else {
            let y = out.y1 - 1;
            (
                Bx {
                    x0: out.x0 + 3,
                    y0: y,
                    x1: out.x1 - 3,
                    y1: y,
                },
                None,
            )
        };
        // the checks that fit, the fleet's two kept last
        let info = f.info;
        let rows = n as usize;
        let check = |what: &str, status: String, short: String, col: Rgb| Check {
            what: what.into(),
            status,
            short,
            col,
        };
        let mut checks: Vec<Check> = CHECKS
            .iter()
            .take(rows.saturating_sub(2).max(1))
            .map(|(a, b, c)| check(a, b.to_string(), c.to_string(), CYAN))
            .collect();
        if rows >= 3 {
            checks.push(match info.sessions {
                0 => check("DAEMONS", "IDLE".into(), "IDLE".into(), CYAN),
                n => check("DAEMONS", format!("{n} RUNNING"), n.to_string(), CYAN),
            });
            checks.push(match info.needs {
                0 => check("ATTENTION", "CLEAR".into(), "OK".into(), CYAN),
                n => check(
                    "ATTENTION",
                    format!("{n} NEED YOU"),
                    // the theme's own glyph for what needs you
                    format!("{n} ◆"),
                    YELLOW,
                ),
            });
        }
        Hud {
            out,
            diag,
            up,
            sync,
            big,
            dh,
            wave,
            checks,
        }
    }

    /// The sync meter's row.
    fn bar_row(&self) -> i32 {
        if self.big {
            self.sync.y0 + self.dh + 2
        } else {
            self.sync.y0
        }
    }

    /// The node rows that fit (your machines; none: this one).
    fn nodes(&self, f: &Frame) -> Vec<(String, bool, Vec<Rgb>)> {
        let rows = (self.up.y1 - self.up.y0 - 1).max(1) as usize;
        let mut v: Vec<(String, bool, Vec<Rgb>)> = f
            .info
            .fleet
            .iter()
            .map(|m| (m.name.clone(), m.up, m.sessions.clone()))
            .collect();
        if v.is_empty() {
            v.push(("local".into(), true, Vec::new()));
        }
        if v.len() > rows {
            let more = v.len() - rows + 1;
            v.truncate(rows - 1);
            v.push((format!("+{more} MORE"), true, Vec::new()));
        }
        v
    }
}

/// The scene's light, its flash at the sync and its sweep: everything
/// drawn passes through here.
struct Ink {
    fade: f64,
    flash: f64,
    sweep: Option<f64>,
}

impl Ink {
    fn of(f: &Frame) -> Ink {
        let u = f.u();
        let sweep = (SWEEP.0..SWEEP.1).contains(&u).then(|| {
            let q = (u - SWEEP.0) / (SWEEP.1 - SWEEP.0);
            -2.0 + q * (f.h - 2.0)
        });
        Ink {
            fade: f.fade(),
            flash: window(u, DONE, DONE + 0.004, DONE + 0.012, DONE + 0.05),
            sweep,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn put(&self, cv: &mut Canvas, x: i32, y: i32, g: &str, col: Rgb, k: f64, bold: bool) {
        let mut col = mix(col, WHITE, self.flash * 0.75);
        let mut k = (k + self.flash * 0.5).min(1.0);
        if let Some(s) = self.sweep {
            // brighter, yellower as the scan passes (more just behind it)
            let d = y as f64 - s;
            let near = if d > 0.0 {
                (-d * d / 0.8).exp()
            } else {
                (-d * d / 9.0).exp()
            };
            col = mix(col, YELLOW, near * 0.85);
            k = (k + near * 0.4).min(1.0);
        }
        cv.put_with(x, y, g, col, k * self.fade, bold);
    }

    /// A label over a line: its spaces cleared, so the line doesn't show
    /// through.
    #[allow(clippy::too_many_arguments)]
    fn label(&self, cv: &mut Canvas, x: i32, y: i32, s: &str, col: Rgb, k: f64, bold: bool) {
        if k * self.fade < 0.05 {
            return;
        }
        for (x, ch) in (x..).zip(s.chars()) {
            if ch == ' ' {
                cv.clear(x, y);
            }
        }
        self.text(cv, x, y, s, col, k, bold);
    }

    #[allow(clippy::too_many_arguments)]
    fn text(&self, cv: &mut Canvas, x: i32, y: i32, s: &str, col: Rgb, k: f64, bold: bool) {
        let mut b = [0u8; 4];
        for (x, ch) in (x..).zip(s.chars()) {
            if ch != ' ' {
                self.put(cv, x, y, ch.encode_utf8(&mut b), col, k, bold);
            }
        }
    }
}

pub(super) fn draw(cv: &mut Canvas, f: &Frame) {
    let ink = Ink::of(f);
    if ink.fade <= 0.0 {
        return;
    }
    let hud = Hud::of(f);
    prompt(cv, f, &ink);
    power(cv, f, &ink, &hud);
    frame(cv, f, &ink, &hud);
    diagnostics(cv, f, &ink, &hud);
    nodes(cv, f, &ink, &hud);
    meter(cv, f, &ink, &hud);
    trace(cv, f, &ink, &hud);
    sweep(cv, f, &ink);
    tears(cv, f, &hud);
}

/// The cold start: a cursor blinking alone in the dark, then the boot
/// prompt typing after it, until the screen powers on.
fn prompt(cv: &mut Canvas, f: &Frame, ink: &Ink) {
    let u = f.u();
    if u >= PROMPT.1 {
        return;
    }
    let line = "> NEURAL DECK // COLD BOOT";
    let n = line.chars().count() as i32;
    let (x, y) = (f.cx as i32 - n / 2, f.cy as i32);
    let s = typed(line, u, PROMPT.0);
    // collapsing into the power-on's line
    let k = 1.0 - smooth(u, POWER.0, PROMPT.1);
    ink.text(cv, x, y, &s, RED, 0.9 * k, false);
    // steady while it types, blinking otherwise
    let typing = u > PROMPT.0 && s.chars().count() < n as usize;
    let on = typing || ((f.t / 130.0) as u64).is_multiple_of(2);
    if on {
        let cx = x + s.chars().count() as i32;
        ink.put(cv, cx, y, "▮", YELLOW, k, true);
    }
}

/// The power-on: a line of light across the middle, opening up and down
/// into scanlines, which flicker and fade as the frame comes.
fn power(cv: &mut Canvas, f: &Frame, ink: &Ink, hud: &Hud) {
    let u = f.u();
    if !(POWER.0..LINES.1).contains(&u) {
        return;
    }
    let (w, cy) = (f.w as i32, f.cy);
    // the line: out from the middle, then opening
    let wide = smooth(u, POWER.0, POWER.0 + 0.025);
    let open = smooth(u, POWER.0 + 0.02, POWER.1);
    let half = wide * f.w / 2.0;
    let reach = open * (cy - hud.out.y0 as f64).max(hud.out.y1 as f64 - cy);
    let line = 1.0 - smooth(u, POWER.0 + 0.02, POWER.1 + 0.01);
    let lines = smooth(u, POWER.0 + 0.03, POWER.1) * (1.0 - smooth(u, LINES.0, LINES.1));
    let tick = (f.t / 40.0) as u64;
    for x in 0..w {
        let dx = (x as f64 - f.cx).abs();
        if dx > half {
            continue;
        }
        // the line of light, thick and white-hot (its two edges, opening
        // apart, thinner)
        let edge = 1.0 - dx / half.max(1.0);
        for (y, k, g) in [
            (cy - reach, line * 0.8, "━"),
            (cy + reach, line * 0.8, "━"),
            (cy, line, "▀"),
        ] {
            if k > 0.0 && (g == "▀" || reach >= 1.0) {
                let col = mix(CYAN, WHITE, edge.sqrt());
                ink.put(
                    cv,
                    x,
                    y.round() as i32,
                    g,
                    col,
                    k * (0.45 + 0.55 * edge),
                    true,
                );
            }
        }
        // scanlines behind them, every other row, broken and flickering
        if lines <= 0.0 {
            continue;
        }
        for y in hud.out.y0..=hud.out.y1 {
            if (y as f64 - cy).abs() > reach || y % 2 != 0 {
                continue;
            }
            let fl = unit(
                f.seed ^ 0x5ca7,
                (y as u64) << 16 ^ (x / 7) as u64 ^ tick << 32,
            );
            if fl > 0.3 {
                let col = if fl > 0.93 { CYAN } else { DIM };
                ink.put(cv, x, y, "─", col, lines * (0.12 + 0.25 * fl), false);
            }
        }
    }
}

/// A box's bracket corners, `loose` cells out, in `col`.
fn brackets(cv: &mut Canvas, ink: &Ink, b: Bx, loose: i32, col: Rgb, k: f64) {
    let (x0, y0, x1, y1) = (
        b.x0 - loose,
        b.y0 - loose / 2,
        b.x1 + loose,
        b.y1 + loose / 2,
    );
    for (x, y, g, dx) in [
        (x0, y0, "┌", 1),
        (x1, y0, "┐", -1),
        (x0, y1, "└", 1),
        (x1, y1, "┘", -1),
    ] {
        ink.put(cv, x, y, g, col, k, true);
        ink.put(cv, x + dx, y, "─", col, k * 0.8, false);
    }
}

/// The outer frame's cells in order round it from its cut corner (top
/// left), clockwise: where and what.
fn ring(b: Bx) -> Vec<(i32, i32, &'static str)> {
    let mut v = vec![(b.x0, b.y0 + 1, "╱"), (b.x0 + 1, b.y0, "╱")];
    v.extend((b.x0 + 2..b.x1).map(|x| (x, b.y0, "─")));
    v.push((b.x1, b.y0, "┐"));
    v.extend((b.y0 + 1..b.y1 - 1).map(|y| (b.x1, y, "│")));
    v.push((b.x1, b.y1 - 1, "╱"));
    v.push((b.x1 - 1, b.y1, "╱"));
    v.extend((b.x0 + 1..b.x1 - 1).rev().map(|x| (x, b.y1, "─")));
    v.push((b.x0, b.y1, "└"));
    v.extend((b.y0 + 2..b.y1).rev().map(|y| (b.x0, y, "│")));
    v
}

/// The frame: traced from its cut corner both ways round, a bright head
/// on each line; its labels typing in; the panels' brackets, loose until
/// the sync snaps them tight.
fn frame(cv: &mut Canvas, f: &Frame, ink: &Ink, hud: &Hud) {
    let u = f.u();
    let p = f.act(TRACE.0, TRACE.1);
    if p <= 0.0 {
        return;
    }
    let cells = ring(hud.out);
    let n = cells.len().max(1) as f64;
    let snap = window(u, DONE, DONE + 0.004, DONE + 0.02, DONE + 0.05);
    for (i, (x, y, g)) in cells.iter().enumerate() {
        let d = (i as f64).min(n - i as f64) / (n / 2.0);
        if d > p {
            continue;
        }
        let head = (1.0 - (p - d) / 0.06).max(0.0) * (p < 1.0) as u8 as f64;
        let col = mix(mix(RED, WHITE, head), YELLOW, snap);
        ink.put(cv, *x, *y, g, col, 0.75 + 0.25 * head, head > 0.5);
    }
    // the labels in the top edge, after it has passed
    let la = smooth(u, LABELS, LABELS + 0.02);
    if la > 0.0 {
        let (x0, y0, x1) = (hud.out.x0, hud.out.y0, hud.out.x1);
        let left = if f.w >= 90.0 {
            " ℵ NEURAL INTERFACE // DECK 9.4 "
        } else {
            " ℵ NEURAL INTERFACE "
        };
        let s = typed(left, u, LABELS);
        if !s.is_empty() {
            ink.put(cv, x0 + 3, y0, "┤", RED, la, false);
            ink.label(cv, x0 + 4, y0, &s, RED, la, true);
            let n = s.chars().count() as i32;
            ink.put(cv, x0 + 4 + n, y0, "├", RED, la, false);
        }
        // its boot counter, ticking over
        let tick = (f.t / 90.0) as u64;
        let hh = hash(f.seed ^ 0xb007, tick);
        let right = format!(" {:04X}·{:04X} ", hh & 0xffff, (hh >> 16) & 0xffff);
        let s = typed(&right, u, LABELS + 0.03);
        let nr = right.chars().count() as i32;
        if !s.is_empty() && x1 - nr - 4 > x0 + 30 {
            let rx = x1 - nr - 3;
            ink.put(cv, rx - 1, y0, "┤", RED, la, false);
            ink.label(cv, rx, y0, &s, CYAN, la * 0.9, false);
            ink.put(cv, rx + nr, y0, "├", RED, la, false);
            // the cursor, blinking on
            if ((f.t / 260.0) as u64).is_multiple_of(2) {
                ink.put(cv, rx + nr + 1, y0, "▮", YELLOW, la, true);
            }
        }
        // the bottom edge: a status once synced
        if u >= DONE {
            let full = " LINK ESTABLISHED ";
            let s = typed(full, u, DONE + 0.02);
            let n = s.chars().count() as i32;
            if n > 0 && x1 - x0 > 40 {
                let bx = x1 - 4 - full.chars().count() as i32;
                ink.put(cv, bx - 1, hud.out.y1, "┤", RED, 1.0, false);
                ink.label(cv, bx, hud.out.y1, &s, YELLOW, 1.0, true);
                ink.put(cv, bx + n, hud.out.y1, "├", RED, 1.0, false);
            }
        }
    }
    // the panels: their brackets flicker on, loose, and snap tight at the sync
    let on = smooth(u, LABELS - 0.02, LABELS + 0.02);
    if on <= 0.0 {
        return;
    }
    let flick = if u < LABELS + 0.04 {
        unit(f.seed ^ 0xf11c, (f.t / 30.0) as u64) * 0.8 + 0.2
    } else {
        1.0
    };
    let loose = match (u < DONE, hud.big) {
        (false, _) => 0,
        (true, true) => 2,
        // a small screen: loose only sideways (no room above)
        (true, false) => 1,
    };
    let col = mix(RED, YELLOW, snap);
    let mut boxes = vec![(hud.diag, " DIAGNOSTICS "), (hud.up, " UPLINK // NODES ")];
    if hud.big {
        boxes.push((hud.sync, " NEURAL SYNC "));
    }
    if let Some(wv) = hud.wave {
        boxes.push((wv, " CORTEX ACTIVITY "));
    }
    for (b, label) in boxes {
        brackets(cv, ink, b, loose, col, on * flick);
        let s = typed(label, u, LABELS + 0.02);
        ink.text(cv, b.x0 + 2, b.y0, &s, RED, on, true);
    }
}

/// `label ····· status` to `w` columns (the label cut short if it must).
/// (its short status when the long one doesn't fit whole beside it).
fn leader(c: &Check, w: usize) -> (String, String, String) {
    let fits = |s: &str| c.what.chars().count() + s.chars().count() + 3 <= w;
    let status = if fits(&c.status) { &c.status } else { &c.short };
    let sn = status.chars().count();
    let label: String = c.what.chars().take(w.saturating_sub(sn + 2)).collect();
    let dots = w.saturating_sub(label.chars().count() + sn + 2);
    (
        label,
        " ".to_string() + &"·".repeat(dots) + " ",
        status.clone(),
    )
}

/// The diagnostics typing in, a line at a time: the check in red, a dotted
/// leader, its status in cyan (the alarm in crimson first, blinking, and
/// what needs you in yellow); each line led by its address.
fn diagnostics(cv: &mut Canvas, f: &Frame, ink: &Ink, hud: &Hud) {
    let u = f.u();
    if u < DIAG.0 {
        return;
    }
    let b = hud.diag;
    let n = hud.checks.len().max(1);
    let step = (DIAG.1 - DIAG.0 - 0.06) / n as f64;
    let addr = b.w() >= 34;
    let x = b.x0 + 2;
    let w = (b.w() - 3 - if addr { 7 } else { 0 }).max(4) as usize;
    for (i, c) in hud.checks.iter().enumerate() {
        let y = b.y0 + 1 + i as i32;
        if y >= b.y1 {
            break;
        }
        let start = DIAG.0 + step * i as f64;
        if u < start {
            break;
        }
        let mut x = x;
        if addr {
            let a = format!(
                "{:04X}",
                0x0a00 + i as u64 * 0x40 + (hash(f.seed, i as u64) & 0x3f)
            );
            ink.text(cv, x, y, &typed(&a, u, start), DIM, 0.8, false);
            x += 7;
        }
        let (label, dots, status) = leader(c, w);
        let typed_label = typed(&label, u, start + 0.004);
        ink.text(cv, x, y, &typed_label, RED, 0.95, false);
        let lx = x + label.chars().count() as i32;
        let td = typed(
            &dots,
            u * 2.0,
            (start + 0.004) * 2.0 + label.chars().count() as f64 / RATE,
        );
        if typed_label.chars().count() < label.chars().count() {
            continue;
        }
        ink.text(cv, lx, y, &td, DIM, 0.7, false);
        let dn = dots.chars().count();
        if td.chars().count() < dn {
            continue;
        }
        let sx = lx + dn as i32;
        let at = start + 0.004 + (label.chars().count() as f64 + dn as f64 / 2.0) / RATE;
        // the alarm: crimson, blinking, until it clears
        if i == ALARM && u < at + 0.06 {
            if !((f.t / 70.0) as u64).is_multiple_of(3) {
                ink.text(cv, sx, y, "WARN", CRIMSON, 1.0, true);
            }
            continue;
        }
        ink.text(
            cv,
            sx,
            y,
            &typed(&status, u, at),
            c.col,
            1.0,
            c.col == YELLOW,
        );
    }
}

/// Your machines coming online, one after another: the name typing in in
/// cyan, its bar filling, then ONLINE (or a machine that doesn't answer:
/// its bar stalling, NO SIGNAL in crimson); its sessions after, in their
/// states' colours.
fn nodes(cv: &mut Canvas, f: &Frame, ink: &Ink, hud: &Hud) {
    let u = f.u();
    if u < NODES.0 {
        return;
    }
    let b = hud.up;
    let nodes = hud.nodes(f);
    let m = nodes.len().max(1);
    let x = b.x0 + 2;
    let inner = (b.w() - 3).max(8);
    let nw = nodes.iter().map(|n| n.0.chars().count()).max().unwrap_or(4) as i32;
    // the word after the bar: ONLINE and its round trip, LINK, or none
    let tail = match inner {
        i if i >= 40 => 9,
        i if i >= 26 => 5,
        _ => 0,
    };
    let dots = if inner >= 44 { 8 } else { 0 };
    let nw = nw.min(16).min(inner - 7 - tail - dots).max(3);
    let bw = (inner - 2 - nw - 1 - tail - dots).max(3);
    let step = (NODES.1 - NODES.0 - 0.14) / m as f64;
    let mut up = 0;
    for (i, (name, alive, sess)) in nodes.iter().enumerate() {
        let y = b.y0 + 1 + i as i32;
        if y >= b.y1 {
            break;
        }
        let start = NODES.0 + step * i as f64;
        if u < start {
            break;
        }
        let more = name.starts_with('+');
        let name: String = name.chars().take(nw as usize).collect();
        ink.put(cv, x, y, "▸", if more { DIM } else { CYAN }, 0.9, false);
        ink.text(cv, x + 2, y, &typed(&name, u, start), CYAN, 1.0, !more);
        if more {
            continue;
        }
        // the bar, filling (a machine that doesn't answer stalls)
        let fill = smooth(u, start + 0.02, start + 0.11);
        let lvl = if *alive { fill } else { fill * 0.34 };
        let bx = x + 2 + nw + 1;
        let full = (lvl * bw as f64).round() as i32;
        let lost = !*alive && fill >= 1.0;
        for j in 0..bw {
            let (g, col, k) = if j < full {
                ("▰", if lost { CRIMSON } else { CYAN }, 0.95)
            } else {
                ("▱", DEEP, 0.75)
            };
            if u >= start + 0.015 {
                ink.put(cv, bx + j, y, g, col, k, false);
            }
        }
        let tx = bx + bw + 1;
        up += (fill >= 1.0 && *alive) as usize;
        let word = if tail == 0 {
            String::new()
        } else if fill >= 1.0 {
            if *alive {
                if tail >= 9 {
                    // live: its round trip, ticking
                    let ms = 4 + hash(f.seed ^ 0x9a1, (i as u64) << 20 ^ (f.t / 240.0) as u64) % 40;
                    format!("ONLINE {ms:>2}")
                } else {
                    "LINK".into()
                }
            } else if tail >= 9 {
                "NO SIGNAL".into()
            } else {
                "LOST".into()
            }
        } else if u >= start + 0.02 {
            format!("{:>3}%", (lvl * 100.0) as u32)
        } else {
            String::new()
        };
        let wc = if lost { CRIMSON } else { CYAN };
        ink.text(cv, tx, y, &word, wc, 1.0, fill >= 1.0);
        // its sessions, in their states' colours
        if dots > 0 && fill >= 1.0 && *alive {
            for (j, c) in sess.iter().take(dots as usize - 1).enumerate() {
                ink.put(cv, tx + tail + 1 + j as i32, y, "▪", *c, 0.95, false);
            }
        }
    }
    stream(
        cv,
        f,
        ink,
        b,
        nodes.len().min((b.y1 - b.y0 - 1) as usize) as i32,
    );
    // how many are up, on the panel's top bracket
    if u >= NODES.0 + 0.03 && b.w() >= 30 {
        let total = f.info.fleet.len().max(1);
        let s = format!(" {up}/{total} ");
        let n = s.chars().count() as i32;
        ink.text(cv, b.x1 - 2 - n, b.y0, &s, CYAN, 0.95, true);
    }
}

/// Under the nodes, the traffic: rows of hex streaming up the panel, a byte
/// lit here and there, a rule between.
fn stream(cv: &mut Canvas, f: &Frame, ink: &Ink, b: Bx, used: i32) {
    let u = f.u();
    let y0 = b.y0 + 1 + used + 1;
    if u < NODES.0 + 0.04 || y0 + 1 >= b.y1 {
        return;
    }
    let on = smooth(u, NODES.0 + 0.04, NODES.0 + 0.1);
    let x0 = b.x0 + 2;
    let w = b.w() - 4;
    for x in x0..x0 + w {
        ink.put(cv, x, y0, "╌", DEEP, on * 0.8, false);
    }
    // a line a tick, coming in from the bottom
    let tick = (f.t / 90.0) as u64;
    let rows = b.y1 - y0 - 1;
    let came = ((u - NODES.0 - 0.04) * 260.0) as i32;
    for r in 0..rows {
        let y = y0 + 1 + r;
        if rows - r > came {
            continue;
        }
        let line = tick + r as u64;
        for i in 0..(w + 1) / 3 {
            let h = hash(f.seed ^ 0x57e4, line << 12 ^ i as u64);
            let (col, k, bold) = match h % 100 {
                0..=1 => (YELLOW, 1.0, true),
                2..=8 => (CYAN, 0.95, true),
                _ => (DEEP, 0.75, false),
            };
            let byte = format!("{:02X}", (h >> 8) & 0xff);
            ink.text(cv, x0 + i * 3, y, &byte, col, k * on, bold);
        }
    }
}

/// The segments each digit lights (a top, b c the right, d the bottom, e f
/// the left, g the middle: bits 0–6).
const SEGS: [u8; 10] = [
    0b011_1111, 0b000_0110, 0b101_1011, 0b100_1111, 0b110_0110, 0b110_1101, 0b111_1101, 0b000_0111,
    0b111_1111, 0b110_1111,
];

/// A big digit, `cols` by `rows`: its seven segments a half cell thick, in
/// quadrant blocks (so never a whole one), top to bottom.
fn big_digit(d: u8, cols: i32, rows: i32) -> Vec<String> {
    const QUAD: [char; 16] = [
        ' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█',
    ];
    let s = SEGS[d as usize % 10];
    let (pw, ph) = (cols * 2, rows * 2);
    let mid = ph / 2 - 1;
    let on = |px: i32, py: i32| {
        let lit = |b: u8| s & (1 << b) != 0;
        (lit(0) && py == 0)
            || (lit(1) && px == pw - 1 && py <= mid)
            || (lit(2) && px == pw - 1 && py >= mid)
            || (lit(3) && py == ph - 1)
            || (lit(4) && px == 0 && py >= mid)
            || (lit(5) && px == 0 && py <= mid)
            || (lit(6) && py == mid)
    };
    (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    let (x, y) = (c * 2, r * 2);
                    let q = on(x, y) as usize
                        | (on(x + 1, y) as usize) << 1
                        | (on(x, y + 1) as usize) << 2
                        | (on(x + 1, y + 1) as usize) << 3;
                    QUAD[q]
                })
                .collect()
        })
        .collect()
}

/// The sync climbing: the percentage in big digits beside its status, the
/// meter under them filling in yellow (on a small screen, one row of it);
/// the hiccup in crimson; complete, bright.
fn meter(cv: &mut Canvas, f: &Frame, ink: &Ink, hud: &Hud) {
    let u = f.u();
    if u < SYNC.0 - 0.03 {
        return;
    }
    let s = sync(u);
    let on = smooth(u, SYNC.0 - 0.03, SYNC.0);
    let hic = (HICCUP.0 + 0.01..HICCUP.1).contains(&u);
    let done = u >= DONE;
    let col = if hic && ((f.t / 60.0) as u64).is_multiple_of(2) {
        CRIMSON
    } else {
        YELLOW
    };
    let pct = (s * 100.0).round() as u32;
    let status = if done {
        "SYNC COMPLETE"
    } else if hic {
        "RESYNC"
    } else {
        "SYNCHRONISING"
    };
    let b = hud.sync;
    if hud.big {
        // the digits, centred over the meter, the status beside them
        let digits = format!("{pct:03}");
        let (dc, dr) = (hud.dh, hud.dh);
        let dw = 3 * (dc + 1) + 1;
        let sw = status.chars().count() as i32;
        let x0 = (b.x0 + b.x1) / 2 - (dw + 3 + sw) / 2;
        for (i, d) in digits.bytes().enumerate() {
            for (r, line) in big_digit(d - b'0', dc, dr).iter().enumerate() {
                ink.text(
                    cv,
                    x0 + i as i32 * (dc + 1),
                    b.y0 + 1 + r as i32,
                    line,
                    col,
                    on,
                    true,
                );
            }
        }
        ink.text(cv, x0 + dw - 1, b.y0 + dr, "%", col, on, true);
        let sy = b.y0 + 1 + dr / 2;
        ink.text(
            cv,
            x0 + dw + 3,
            sy,
            status,
            col,
            on * if done { 1.0 } else { 0.8 },
            done,
        );
        if !done && ((f.t / 260.0) as u64).is_multiple_of(2) {
            ink.put(cv, x0 + dw + 3 + sw + 1, sy, "▮", col, on, true);
        }
        bar(cv, ink, b.x0 + 2, b.x1 - 2, hud.bar_row(), s, col, on);
    } else {
        let label = "SYNC ";
        ink.text(cv, b.x0, b.y0, label, YELLOW, on, true);
        let tail = format!(" {pct:>3}%");
        let x1 = b.x1 - tail.chars().count() as i32;
        bar(cv, ink, b.x0 + 5, x1, b.y0, s, col, on);
        ink.text(cv, x1 + 1, b.y0, &tail, col, on, true);
    }
}

/// A meter from `x0` to `x1` on row `y`, `s` of it full; its leading edge
/// bright.
#[allow(clippy::too_many_arguments)]
fn bar(cv: &mut Canvas, ink: &Ink, x0: i32, x1: i32, y: i32, s: f64, col: Rgb, k: f64) {
    let n = (x1 - x0).max(1);
    let full = s * n as f64;
    for j in 0..n {
        let x = x0 + j;
        if (j as f64) < full.floor() {
            let lead = (1.0 - (full - j as f64 - 1.0) / 3.0).max(0.0) * (s < 1.0) as u8 as f64;
            ink.put(cv, x, y, "▰", mix(col, WHITE, lead * 0.7), k, true);
        } else {
            ink.put(cv, x, y, "▱", mix(col, (60, 56, 10), 0.6), k * 0.6, false);
        }
    }
}

/// The cortex's activity under the sync (big screens): a trace in braille
/// running across, calm while it boots and livelier once synced, a quieter
/// red one under it.
fn trace(cv: &mut Canvas, f: &Frame, ink: &Ink, hud: &Hud) {
    let Some(b) = hud.wave else { return };
    let u = f.u();
    let on = smooth(u, LABELS + 0.04, LABELS + 0.12);
    if on <= 0.0 {
        return;
    }
    let mid = (b.y0 + b.y1) as f64 / 2.0;
    let amp = (b.y1 - b.y0) as f64 / 2.0 - 0.7;
    let live = 0.6 + 0.4 * smooth(u, DONE - 0.05, DONE + 0.05);
    let t = f.t / 1000.0;
    // how far it has run in from the left
    let reach = b.x0 as f64 + 2.0 + (b.x1 - b.x0 - 3) as f64 * smooth(u, LABELS + 0.04, 0.5);
    for (lane, col, gain, k) in [(1u64, DIM, 0.6, 0.7), (0, CYAN, 1.0, 1.0)] {
        let mut d = Dots::default();
        let mut last: Option<f64> = None;
        let mut x = b.x0 as f64 + 2.0;
        let ph = lane as f64 * 1.7;
        while x <= reach {
            let s = x - t * 38.0;
            let mut v = 0.45 * (s * 0.23 + ph).sin() + 0.25 * (s * 0.071 - ph).sin();
            // spikes now and then, like a pulse
            let cell = (s / 9.0).floor();
            let pk = unit(f.seed ^ 0x5b1e ^ lane, cell as i64 as u64);
            if pk < 0.45 {
                let c = (cell + 0.5) * 9.0;
                v += (1.0 - ((s - c).abs() / 1.1)).max(0.0) * (if pk < 0.2 { -1.4 } else { 1.6 });
            }
            let y = mid - (v * live * gain).clamp(-1.0, 1.0) * amp;
            if let Some(py) = last {
                let steps = ((y - py).abs() * 4.0).ceil().max(1.0) as i32;
                for k in 0..=steps {
                    d.add(x, py + (y - py) * k as f64 / steps as f64, 1.0);
                }
            } else {
                d.add(x, y, 1.0);
            }
            last = Some(y);
            x += 0.5;
        }
        let mut b2 = [0u8; 4];
        for ((x, y), (bits, _)) in d.0 {
            let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
            if lane == 0 || cv.blank(x, y) {
                ink.put(cv, x, y, ch.encode_utf8(&mut b2), col, k * on, lane == 0);
            }
        }
    }
    // its reading, live
    if b.w() >= 40 {
        let hz = 8.0 + 6.0 * live + 1.5 * unit(f.seed ^ 0x42, (f.t / 300.0) as u64);
        let s = format!(" {hz:.1} HZ ");
        let n = s.chars().count() as i32;
        ink.text(cv, b.x1 - 2 - n, b.y0, &s, CYAN, on * 0.95, true);
    }
}

/// The sync's sweep: a bright yellow line running down the screen.
fn sweep(cv: &mut Canvas, f: &Frame, ink: &Ink) {
    let Some(s) = ink.sweep else { return };
    let y = s.round() as i32;
    for x in 0..f.w as i32 {
        let edge = ((x as f64 - f.cx).abs() / (f.w / 2.0)).powi(3);
        cv.put_with(
            x,
            y,
            "━",
            mix(WHITE, YELLOW, 0.4 + 0.6 * edge),
            ink.fade,
            true,
        );
    }
}

/// The glitches: bands of rows torn sideways, tinted red one way and cyan
/// the other, a ghost of the row behind and a little static; a new tear
/// every few frames while one is on.
fn tears(cv: &mut Canvas, f: &Frame, hud: &Hud) {
    let u = f.u();
    let Some(i) = TEARS.iter().position(|(a, b)| (*a..*b).contains(&u)) else {
        return;
    };
    let rows = hud.out.y1 + 1;
    let tick = (f.t / 35.0) as u64;
    let h = hash(f.seed ^ 0x7ea4, tick ^ (i as u64) << 32);
    // the ones as it powers on tear across the middle, where the light is
    let cold = i < 3;
    let bands = 2 + h % 3;
    for k in 0..bands {
        let hb = hash(h, k);
        let y0 = if cold {
            f.cy as i32 + ((hb % 9) as i32 - 4)
        } else {
            (hb % rows as u64) as i32
        };
        let n = 1 + (hb >> 8) % 3;
        let dx = (1 + ((hb >> 16) % 5) as i32) * if k % 2 == 0 { -1 } else { 1 };
        let tint = if dx < 0 { RED } else { CYAN };
        for y in y0..(y0 + n as i32).min(rows) {
            tear(cv, f, y, dx, tint, 0.6, hb ^ y as u64, cold);
        }
    }
}

/// Row `y` torn `dx` cells sideways, what moved tinted `tint` by `by`.
#[allow(clippy::too_many_arguments)]
fn tear(cv: &mut Canvas, f: &Frame, y: i32, dx: i32, tint: Rgb, by: f64, h: u64, cold: bool) {
    if y < 0 || y >= cv.area.height as i32 {
        return;
    }
    let w = cv.area.width as i32;
    let (ax, ay) = (cv.area.x, cv.area.y + y as u16);
    let row: Vec<ratatui::buffer::Cell> = (0..w)
        .map(|x| cv.buf[(ax + x as u16, ay)].clone())
        .collect();
    let tc = cv.colour(tint, 1.0);
    let tinted = |c: Color| match (c, tc) {
        (Color::Rgb(r, g, b), Color::Rgb(tr, tg, tb)) => {
            let (r, g, b) = mix((r, g, b), (tr, tg, tb), by);
            Color::Rgb(r, g, b)
        }
        (c, _) => c,
    };
    // the torn stretch: from somewhere along the row, a good way across
    let a = (unit(h, 1) * w as f64 * 0.6) as i32;
    let len = (w as f64 * (0.3 + 0.6 * unit(h, 2))) as i32;
    const NOISE: [&str; 7] = ["░", "▒", "▓", "▌", "▐", "▀", "▄"];
    for x in a.max(0)..(a + len).min(w) {
        let from = x - dx;
        let src = if (0..w).contains(&from) {
            row[from as usize].clone()
        } else {
            ratatui::buffer::Cell::default()
        };
        let pos = (ax + x as u16, ay);
        if src.symbol() != " " {
            let mut c = src;
            c.fg = tinted(c.fg);
            cv.buf[pos] = c;
        } else if row[x as usize].symbol() != " " {
            // the ghost of what was there
            let mut c = row[x as usize].clone();
            c.fg = tinted(c.fg);
            c.modifier.remove(Modifier::BOLD);
            cv.buf[pos] = c;
        } else {
            let hn = hash(h ^ 0x5717, x as u64 ^ (f.t / 35.0) as u64);
            let p = if cold { 3 } else { 9 };
            if hn.is_multiple_of(p) {
                let g = NOISE[(hn >> 8) as usize % NOISE.len()];
                cv.put_with(x, y, g, tint, 0.35 + 0.4 * unit(hn, 3), false);
            }
        }
    }
}

/// Where its particles are as the ℵ gathers: round the frame, along the
/// meter and the panels' readouts.
pub(super) fn sources(f: &Frame) -> Vec<(f64, f64)> {
    let hud = Hud::of(f);
    let mut v: Vec<(f64, f64)> = ring(hud.out)
        .into_iter()
        .step_by(3)
        .map(|(x, y, _)| (x as f64, y as f64))
        .collect();
    let b = hud.sync;
    let y = hud.bar_row();
    v.extend(
        (b.x0 + 2..b.x1 - 2)
            .step_by(2)
            .map(|x| (x as f64, y as f64)),
    );
    for p in [hud.diag, hud.up] {
        for y in p.y0 + 1..p.y1 {
            v.extend(
                (p.x0 + 2..p.x1 - 1)
                    .step_by(4)
                    .map(|x| (x as f64, y as f64)),
            );
        }
    }
    v
}

/// Is the ℵ there at `(x, y)` (a block or its braille edge)?
fn landed(cv: &Canvas, x: i32, y: i32) -> bool {
    !cv.blank(x, y)
        && cv.buf[(cv.area.x + x as u16, cv.area.y + y as u16)]
            .symbol()
            .chars()
            .next()
            .is_some_and(|c| c == '█' || ('\u{2801}'..='\u{28FF}').contains(&c))
}

/// How far apart the ℵ's two ghosts are at `t` (cells): wide as it lands,
/// converging, none once it snaps into focus.
fn split(f: &Frame) -> f64 {
    let done = f.len + GATHER_S;
    7.0 * (1.0 - smooth(f.t, f.len + GATHER_S * 0.25, done + 80.0))
}

/// With the logo, after the scene: the ℵ as it lands with its two ghosts,
/// red to the left and cyan to the right, converging until it snaps into
/// focus in yellow with a flash; held, a yellow scan passing down it and
/// one glitch tearing a band of it sideways.
pub(super) fn paint(cv: &mut Canvas, f: &Frame) {
    let done = f.len + GATHER_S;
    let end = done + f.opts.hold;
    if f.t < f.len || f.t >= end {
        return;
    }
    let (x0, y0, lc, lr, cells) = logo_box(f.w, f.h, f.cx, f.cy);
    let own: std::collections::HashSet<(i32, i32)> = cells.iter().map(|c| (c.0, c.1)).collect();
    let mark: Vec<(i32, i32, String, f64)> = cells
        .into_iter()
        .filter(|&(x, y, _)| landed(cv, x, y))
        .map(|(x, y, _)| {
            let sym = cv.buf[(cv.area.x + x as u16, cv.area.y + y as u16)]
                .symbol()
                .to_string();
            let fpos = ((x - x0) as f64 / lc as f64 + (y - y0) as f64 / lr as f64) / 2.0;
            (x, y, sym, fpos)
        })
        .collect();
    let d = split(f);
    let off = d.round() as i32;
    let snap = window(f.t, done + 80.0, done + 95.0, done + 120.0, done + 260.0);
    // the scan, once held: down the mark and away
    let scan = (f.t > done + 150.0).then(|| {
        let q = (f.t - done - 150.0) / 420.0;
        y0 as f64 - 2.0 + q * (lr as f64 + 4.0)
    });
    // the ghosts, outside the mark's own cells (over the particles still
    // coming in): its fringes, red to the left, cyan to the right
    if off > 0 {
        for (x, y, sym, _) in &mark {
            for (dx, col) in [(-off, RED), (off, CYAN)] {
                if !own.contains(&(x + dx, *y)) {
                    cv.put_with(x + dx, *y, sym, col, 0.95, true);
                }
            }
        }
    }
    let blur = d / 7.0;
    for (x, y, sym, fpos) in &mark {
        let near = scan.map_or(0.0, |s| (-(*y as f64 - s).powi(2) / 1.5).exp());
        let col = mix(ramp(&FOCUS, *fpos), WHITE, snap * 0.85 + near * 0.6);
        cv.put_with(*x, *y, sym, col, 1.0 - 0.35 * blur, true);
    }
    // one glitch while it's held
    let g0 = done + f.opts.hold * 0.55;
    if (g0..g0 + 70.0).contains(&f.t) {
        let h = hash(f.seed ^ 0x9117, (f.t / 35.0) as u64);
        let y = y0 + (h % lr.max(1) as u64) as i32;
        for (k, yy) in (y..y + 2).enumerate() {
            let dx = if k == 0 { 2 } else { -2 };
            tear(
                cv,
                f,
                yy,
                dx,
                if dx < 0 { RED } else { CYAN },
                0.0,
                h ^ k as u64,
                false,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::scene;
    use super::*;

    fn info(names: &[&str]) -> Info {
        Info {
            machines: names.len(),
            sessions: 3,
            needs: 1,
            fleet: names
                .iter()
                .map(|n| Mach {
                    name: n.to_string(),
                    rgb: (120, 200, 255),
                    up: true,
                    sessions: vec![(110, 160, 250), (255, 150, 60)],
                })
                .collect(),
        }
    }

    fn shot(w: u16, h: u16, u: f64, logo: bool, info: &Info) -> (Buffer, Frame<'_>) {
        let ms = (scene::SCENE_MS as f64 * u) as u64;
        let f = scene::at(w, h, ms, logo, info);
        (scene::dark(Kind::Cyberdeck, &f), f)
    }

    fn row(b: &Buffer, y: i32) -> String {
        (0..b.area.width)
            .map(|x| b[(x, y as u16)].symbol())
            .collect()
    }

    fn rgb(b: &Buffer, x: i32, y: i32) -> Option<Rgb> {
        match b[(x as u16, y as u16)].fg {
            Color::Rgb(r, g, bl) => Some((r, g, bl)),
            _ => None,
        }
    }

    /// Is it about this colour (a little either way: the sweep, the light)?
    fn near(c: Option<Rgb>, to: Rgb) -> bool {
        c.is_some_and(|c| {
            (c.0 as i32 - to.0 as i32).abs()
                + (c.1 as i32 - to.1 as i32).abs()
                + (c.2 as i32 - to.2 as i32).abs()
                < 60
        })
    }

    #[test]
    fn it_starts_dark_with_a_cursor_and_the_frame_traces_itself() {
        let info = info(&["normandy"]);
        let (b, _) = shot(120, 32, 0.01, false, &info);
        let lit: Vec<String> = (0..28)
            .map(|y| row(&b, y))
            .filter(|r| r.trim() != "")
            .collect();
        assert_eq!(lit.len(), 1, "{lit:?}");
        assert!(lit[0].contains('▮'));
        // the frame: part way round, then whole
        // its sides and corners (the scanlines are rules across, like its top)
        let sides = |f: &Frame| -> Vec<(i32, i32, &str)> {
            ring(Hud::of(f).out)
                .into_iter()
                .filter(|c| c.2 != "─")
                .collect()
        };
        let count = |u: f64| {
            let (b, f) = shot(120, 32, u, false, &info);
            sides(&f)
                .iter()
                .filter(|&&(x, y, g)| b[(x as u16, y as u16)].symbol() == g)
                .count()
        };
        let (half, whole) = (count(0.22), count(0.34));
        let all = sides(&scene::at(120, 32, 0, false, &info)).len();
        assert!(half > all / 5 && half < all * 4 / 5, "{half} of {all}");
        assert!(whole > all * 9 / 10, "{whole} of {all}");
    }

    #[test]
    fn the_diagnostics_type_in_over_time_in_red() {
        let info = info(&["normandy"]);
        let text = |u: f64| {
            let (b, _) = shot(120, 32, u, false, &info);
            (0..28).map(|y| row(&b, y)).collect::<Vec<_>>().join("\n")
        };
        let (a, b, c) = (text(0.3), text(0.45), text(0.7));
        assert!(a.contains("NEURAL") && !a.contains("SYNAPSE"), "{a}");
        assert!(b.contains("SYNAPSE") || b.contains("CORTEX"), "{b}");
        assert!(
            c.contains("ICE") && c.contains("NOMINAL") && c.contains("1 NEED YOU"),
            "{c}"
        );
        // the checks in the chrome's red
        let (bf, _) = shot(120, 32, 0.7, false, &info);
        let y = (0..28)
            .find(|&y| row(&bf, y).contains("NEURAL LINK"))
            .unwrap();
        let x = row(&bf, y).find("NEURAL LINK").unwrap();
        let x = row(&bf, y)[..x].chars().count() as i32;
        assert!(near(rgb(&bf, x, y), RED), "{:?}", rgb(&bf, x, y));
    }

    #[test]
    fn a_machine_comes_online_as_a_cyan_readout() {
        let info = info(&["normandy", "serenity"]);
        let (b, _) = shot(120, 32, 0.78, false, &info);
        for name in ["normandy", "serenity"] {
            let y = (0..28)
                .find(|&y| row(&b, y).contains(name))
                .unwrap_or_else(|| panic!("{name} isn't there"));
            let r = row(&b, y);
            let x = r[..r.find(name).unwrap()].chars().count() as i32;
            assert!(near(rgb(&b, x, y), CYAN), "{name}: {:?}", rgb(&b, x, y));
            assert!(r.contains("ONLINE") && r.contains('▰'), "{r}");
        }
        // before the uplink: none of them
        let (b, _) = shot(120, 32, 0.3, false, &info);
        assert!(!(0..28).any(|y| row(&b, y).contains("normandy")));
    }

    #[test]
    fn the_sync_meter_climbs_in_yellow_with_a_hiccup() {
        let mut last = -1.0;
        for i in 0..=40 {
            let u = i as f64 / 40.0;
            let s = sync(u);
            assert!(s >= last && (0.0..=1.0).contains(&s), "{u}: {s}");
            last = s;
        }
        assert_eq!(sync(0.2), 0.0);
        assert_eq!(sync(DONE), 1.0);
        // it holds through the hiccup
        assert!((sync(HICCUP.0) - sync(HICCUP.1)).abs() < 1e-9);
        // on screen: the meter's full cells grow, yellow
        let info = info(&["normandy"]);
        let full = |u: f64| {
            let (b, f) = shot(120, 32, u, false, &info);
            let hud = Hud::of(&f);
            let y = hud.bar_row();
            (hud.sync.x0..=hud.sync.x1)
                .filter(|&x| b[(x as u16, y as u16)].symbol() == "▰" && near(rgb(&b, x, y), YELLOW))
                .count()
        };
        let (a, b, c) = (full(0.45), full(0.7), full(0.85));
        assert!(a > 3 && b > a && c > b, "{a} {b} {c}");
        let (bf, _) = shot(120, 32, 0.85, false, &info);
        assert!((0..28).any(|y| row(&bf, y).contains("SYNC COMPLETE")));
        // and on a small screen, a row of its own
        let (bs, f) = shot(50, 14, 0.85, false, &info);
        let y = Hud::of(&f).sync.y0;
        assert!(
            row(&bs, y).contains("SYNC") && row(&bs, y).contains("100%"),
            "{}",
            row(&bs, y)
        );
    }

    #[test]
    fn the_sync_sweeps_a_yellow_line_down_the_screen() {
        let info = info(&[]);
        let line = |u: f64| {
            let (b, _) = shot(120, 32, u, false, &info);
            (0..32).find(|&y| row(&b, y).matches('━').count() > 100)
        };
        let (a, b) = (line(0.82), line(0.87));
        assert!(a.is_some() && b.is_some() && b > a, "{a:?} {b:?}");
        assert_eq!(line(0.95), None);
    }

    #[test]
    fn with_the_logo_its_red_and_cyan_ghosts_converge_into_yellow() {
        let info = info(&["normandy"]);
        let at = |ms: f64| {
            let f = scene::at(120, 32, ms as u64, true, &info);
            scene::dark(Kind::Cyberdeck, &f)
        };
        let len = scene::SCENE_MS as f64;
        let ghosts = |b: &Buffer| {
            let (mut red, mut cyan) = (0, 0);
            for y in 0..32 {
                for x in 0..120 {
                    let s = b[(x as u16, y as u16)].symbol();
                    if s == "█" || s.chars().all(|c| ('\u{2801}'..='\u{28FF}').contains(&c)) {
                        let c = rgb(b, x, y);
                        red += near(c, RED) as usize;
                        cyan += near(c, CYAN) as usize;
                    }
                }
            }
            (red, cyan)
        };
        let (r, c) = ghosts(&at(len + GATHER_S * 0.8));
        assert!(r > 10 && c > 10, "{r} red, {c} cyan");
        // in focus: yellow, no ghosts
        let b = at(len + GATHER_S + 200.0);
        let (r, c) = ghosts(&b);
        assert!(r == 0 && c == 0, "{r} red, {c} cyan");
        let yellow = (0..32)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .filter(|&(x, y)| b[(x as u16, y as u16)].symbol() == "█")
            .filter(|&(x, y)| {
                rgb(&b, x, y).is_some_and(|(r, g, bl)| r > 200 && g > 180 && (bl as i32) < 120)
            })
            .count();
        assert!(yellow > 40, "{yellow}");
    }

    #[test]
    fn any_fleet_none_one_or_many_draws_on_any_screen() {
        let long: Vec<String> = (0..12)
            .map(|i| format!("a-machine-with-a-name-longer-than-a-small-screen-{i}"))
            .collect();
        let long: Vec<&str> = long.iter().map(String::as_str).collect();
        for info in [info(&[]), info(&["normandy"]), info(&long)] {
            for (w, h) in [(50, 14), (80, 24), (200, 60)] {
                for u in [0.05, 0.2, 0.44, 0.65, 0.8, 1.0] {
                    for logo in [false, true] {
                        let (b, _) = shot(w, h, u, logo, &info);
                        let lit = (0..h)
                            .flat_map(|y| (0..w).map(move |x| (x, y)))
                            .filter(|&p| b[p].symbol() != " ")
                            .count();
                        assert!(lit > 10, "{w}×{h} at {u}: {lit}");
                    }
                }
            }
        }
    }
}
