//! A recording: asciinema's cast format, version 2 — a JSON header line, then
//! one `[seconds, "o", data]` line per flush of the bytes the dashboard sends
//! its terminal (`term::Out` tees them here), `[t, "r", "WxH"]` when the
//! screen changes size and `[t, "m", label]` markers where it paused.
//!
//! What's written is the frame as drawn, after the lens, so nothing private
//! gets in. The first frame — and the first after a hand-off or a resize —
//! is written whole (a keyframe, from the buffer), so the cast starts on a
//! full screen however far into the dashboard's life it begins; the diff
//! ratatui flushes right after it only repaints what's already there.
//! Logos sent as images (kitty, iTerm2) are left out: a player can't show
//! them.

use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::{Buffer, Cell};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// How a recording is labelled and played back.
#[derive(Clone, Debug, Default)]
pub struct Opts {
    /// the player's title
    pub title: String,
    /// the longest pause played back, in seconds (0: as recorded)
    pub idle: f64,
    /// (fg, bg, the 16 colours) as `#rrggbb`, for the player's theme
    pub theme: Option<(String, String, Vec<String>)>,
    /// more of the header's `env` (what its video needs: the mode, the
    /// accent)
    pub env: Vec<(String, String)>,
}

pub struct Recorder {
    pub path: PathBuf,
    out: BufWriter<File>,
    opts: Opts,
    /// when the clock started (the first frame)
    t0: Instant,
    /// time spent away (hand-offs), taken off the clock
    paused: Duration,
    /// away since
    away: Option<Instant>,
    /// bytes since the last flush
    pending: Vec<u8>,
    /// the screen's size (None until the first frame)
    size: Option<(u16, u16)>,
    /// the next frame is written whole
    key: bool,
    /// the time of the last event
    last: f64,
    pub events: u64,
}

impl Recorder {
    pub fn create(path: &Path, opts: Opts) -> io::Result<Recorder> {
        let f = File::create(path)?;
        Ok(Recorder {
            path: path.to_path_buf(),
            out: BufWriter::new(f),
            opts,
            t0: Instant::now(),
            paused: Duration::ZERO,
            away: None,
            pending: Vec::new(),
            size: None,
            key: true,
            last: 0.0,
            events: 0,
        })
    }

    /// Seconds on the recording's clock (time away doesn't count).
    pub fn now(&self) -> f64 {
        if self.size.is_none() {
            return 0.0;
        }
        let away = self.away.map(|a| a.elapsed()).unwrap_or_default();
        self.t0
            .elapsed()
            .saturating_sub(self.paused + away)
            .as_secs_f64()
    }

    #[cfg(test)]
    pub fn is_away(&self) -> bool {
        self.away.is_some()
    }

    /// Bytes on their way to the terminal.
    pub fn bytes(&mut self, b: &[u8]) {
        if self.away.is_none() && self.size.is_some() {
            self.pending.extend_from_slice(b);
        }
    }

    /// The terminal was flushed: what was sent since is one event (whole
    /// characters only: a cut one waits for the rest).
    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let (text, rest) = utf8_prefix(&self.pending);
        let rest = rest.to_vec();
        let data = clean(&text);
        self.pending = rest;
        if !data.is_empty() && data != HIDE {
            let t = self.now();
            self.event(t, "o", &data);
        }
    }

    /// A frame as drawn (after the lens): the header with the first, a
    /// resize when the size changed, and the whole screen when it's due.
    pub fn frame(&mut self, buf: &Buffer) {
        if self.away.is_some() {
            return;
        }
        let wh = (buf.area.width, buf.area.height);
        match self.size {
            None => {
                self.t0 = Instant::now();
                self.size = Some(wh);
                let h = header(wh, &self.opts);
                let _ = writeln!(self.out, "{h}");
                self.key = true;
            }
            Some(s) if s != wh => {
                self.size = Some(wh);
                let t = self.now();
                self.event(t, "r", &format!("{}x{}", wh.0, wh.1));
                self.key = true;
            }
            _ => {}
        }
        if std::mem::take(&mut self.key) {
            // what was on its way belongs to the screen this one replaces
            self.pending.clear();
            let t = self.now();
            self.event(t, "o", &keyframe(buf));
        }
    }

    /// The terminal goes to something else: the clock stops, a marker says why.
    pub fn pause(&mut self, label: &str) {
        if self.away.is_some() {
            return;
        }
        self.flush();
        self.pending.clear();
        if self.size.is_some() {
            let t = self.now();
            self.event(t, "m", label);
        }
        self.away = Some(Instant::now());
        let _ = self.out.flush();
    }

    /// Back: the clock goes on, the next frame is written whole.
    pub fn resume(&mut self) {
        if let Some(a) = self.away.take() {
            self.paused += a.elapsed();
            self.key = true;
        }
    }

    /// Done: everything written. None when not one frame was (the file goes).
    pub fn finish(mut self) -> Option<(PathBuf, f64)> {
        if self.away.is_none() {
            self.flush();
        }
        let secs = self.last;
        let _ = self.out.flush();
        if self.size.is_none() {
            let _ = std::fs::remove_file(&self.path);
            return None;
        }
        Some((self.path.clone(), secs))
    }

    fn event(&mut self, t: f64, kind: &str, data: &str) {
        // never back in time (a resize between frames, a rounding)
        let t = (t.max(self.last) * 1e6).round() / 1e6;
        self.last = t;
        self.events += 1;
        let _ = writeln!(self.out, "{}", json!([t, kind, data]));
        let _ = self.out.flush();
    }
}

/// The cursor hidden again (every frame does it): no event on its own.
const HIDE: &str = "\x1b[?25l";

/// The header line: v2, the size, when, and how to play it.
pub fn header((w, h): (u16, u16), o: &Opts) -> Value {
    let mut v = json!({
        "version": 2,
        "width": w,
        "height": h,
        "timestamp": crate::cache::now(),
        "env": {"TERM": std::env::var("TERM").ok().filter(|t| !t.is_empty()).unwrap_or_else(|| "xterm-256color".into())},
    });
    for (k, val) in &o.env {
        v["env"][k] = json!(val);
    }
    if o.idle > 0.0 {
        v["idle_time_limit"] = json!(o.idle);
    }
    if !o.title.is_empty() {
        v["title"] = json!(o.title);
    }
    if let Some((fg, bg, pal)) = &o.theme {
        v["theme"] = json!({"fg": fg, "bg": bg, "palette": pal.join(":")});
    }
    v
}

/// The longest prefix of `b` that's whole UTF-8 (bad bytes as U+FFFD), and
/// what's left: the start of a character cut off at the end.
fn utf8_prefix(b: &[u8]) -> (String, &[u8]) {
    let mut out = String::new();
    let mut rest = b;
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                return (out, &[]);
            }
            Err(e) => {
                let ok = e.valid_up_to();
                out.push_str(std::str::from_utf8(&rest[..ok]).unwrap_or_default());
                match e.error_len() {
                    // cut short at the end: it waits for the rest
                    None => return (out, &rest[ok..]),
                    Some(n) => {
                        out.push('\u{fffd}');
                        rest = &rest[ok + n..];
                    }
                }
            }
        }
    }
}

/// What a player can't show taken out: kitty's graphics (APC `ESC _ G … ESC \`),
/// iTerm2's pictures (OSC 1337), and kitty's placeholder cells (U+10EEEE and
/// the marks after it) — blanks instead.
pub fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let rest = &s[i..];
        if rest.starts_with("\x1b_G") {
            // to the string terminator (ESC \), or the end
            match rest.find("\x1b\\") {
                Some(j) => {
                    let end = i + j + 2;
                    while it.peek().is_some_and(|(k, _)| *k < end) {
                        it.next();
                    }
                }
                None => break,
            }
            continue;
        }
        if rest.starts_with("\x1b]1337;") {
            let bel = rest.find('\x07');
            let st = rest.find("\x1b\\").map(|j| j + 1);
            match bel.into_iter().chain(st).min() {
                Some(j) => {
                    let end = i + j + 1;
                    while it.peek().is_some_and(|(k, _)| *k < end) {
                        it.next();
                    }
                }
                None => break,
            }
            continue;
        }
        if c == '\u{10EEEE}' {
            out.push(' ');
            while it
                .peek()
                .is_some_and(|(_, d)| d.width() == Some(0) && *d != '\x1b')
            {
                it.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// The whole screen as bytes a terminal draws: home, clear, every cell
/// (the ones a wide character covers and image cells left out), styles
/// reset at the end — what ratatui's own backend writes for them.
pub fn keyframe(buf: &Buffer) -> String {
    let a = buf.area;
    let mut cells: Vec<(u16, u16, &Cell)> = Vec::with_capacity(a.area() as usize);
    for y in a.top()..a.bottom() {
        let mut skip = 0usize;
        for x in a.left()..a.right() {
            let c = &buf[(x, y)];
            if skip > 0 {
                skip -= 1;
                continue;
            }
            skip = c.symbol().width().saturating_sub(1);
            if !c.skip {
                cells.push((x, y, c));
            }
        }
    }
    let mut bytes: Vec<u8> = Vec::new();
    {
        let mut be = CrosstermBackend::new(&mut bytes);
        let _ = Backend::draw(&mut be, cells.into_iter());
    }
    let body = String::from_utf8_lossy(&bytes);
    clean(&format!("{HIDE}\x1b[H\x1b[2J{body}"))
}

// ------------------------------------------------------------------ reading

/// What `aleph showcase casts` says about one.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub path: PathBuf,
    pub width: u64,
    pub height: u64,
    /// when it began (unix)
    pub at: u64,
    /// how long it plays
    pub secs: f64,
    pub title: String,
    pub bytes: u64,
}

/// A cast's header and length (the last event's time), or None when it
/// isn't one.
pub fn info(path: &Path) -> Option<Info> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let h: Value = serde_json::from_str(lines.next()?).ok()?;
    if h.get("version").and_then(Value::as_u64) != Some(2) {
        return None;
    }
    let secs = text
        .lines()
        .rev()
        .find_map(|l| {
            serde_json::from_str::<Value>(l)
                .ok()?
                .get(0)
                .and_then(Value::as_f64)
        })
        .unwrap_or(0.0);
    Some(Info {
        path: path.to_path_buf(),
        width: h.get("width").and_then(Value::as_u64).unwrap_or(0),
        height: h.get("height").and_then(Value::as_u64).unwrap_or(0),
        at: h.get("timestamp").and_then(Value::as_u64).unwrap_or(0),
        secs,
        title: h
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        bytes: text.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Style};
    use ratatui::widgets::Paragraph;
    use ratatui::Terminal;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aleph-cast-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d.join("x.cast")
    }

    fn frame(w: u16, h: u16, text: &str) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| {
            f.render_widget(
                Paragraph::new(text).style(Style::new().fg(Color::Rgb(0xe2, 0xb4, 0x51))),
                f.area(),
            )
        })
        .unwrap();
        t.backend().buffer().clone()
    }

    fn lines(p: &Path) -> Vec<Value> {
        std::fs::read_to_string(p)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn a_cast_is_a_header_then_events_starting_on_a_whole_screen() {
        let p = tmp("whole");
        let mut r = Recorder::create(
            &p,
            Opts {
                title: "Aleph".into(),
                idle: 2.0,
                theme: Some((
                    "#eeeeee".into(),
                    "#111111".into(),
                    vec!["#000000".into(); 16],
                )),
                env: vec![("ALEPH_SHOWCASE".into(), "masked".into())],
            },
        )
        .unwrap();
        // bytes before the first frame aren't the recording's
        r.bytes(b"\x1b[2Jbefore");
        r.flush();
        r.frame(&frame(20, 3, "ℵ aleph"));
        r.bytes("\x1b[1;1Hé".as_bytes());
        r.flush();
        // the cursor hidden again, alone: no event
        r.bytes(HIDE.as_bytes());
        r.flush();
        // a character cut in two waits for its other half
        let star = "✦".as_bytes();
        r.bytes(&star[..1]);
        r.flush();
        r.bytes(&star[1..]);
        r.flush();
        let (path, _) = r.finish().unwrap();
        let v = lines(&path);
        let h = &v[0];
        assert_eq!(h["version"], 2);
        assert_eq!(
            (h["width"].as_u64(), h["height"].as_u64()),
            (Some(20), Some(3))
        );
        assert_eq!(h["idle_time_limit"], 2.0);
        assert_eq!(h["title"], "Aleph");
        assert!(h["theme"]["palette"].as_str().unwrap().split(':').count() == 16);
        assert_eq!(h["env"]["ALEPH_SHOWCASE"], "masked");
        assert!(h["env"]["TERM"].is_string());
        // the keyframe: home, clear, the text, styles reset
        let k = v[1][2].as_str().unwrap();
        assert_eq!(v[1][1], "o");
        assert!(k.starts_with("\x1b[?25l\x1b[H\x1b[2J"), "{k:?}");
        assert!(k.contains("ℵ aleph") || k.contains('ℵ'), "{k:?}");
        assert!(
            k.contains("38;2;226;180;81"),
            "the gold, as truecolour: {k:?}"
        );
        assert!(!k.contains("before"));
        assert_eq!(v[2][2], "\x1b[1;1Hé");
        assert_eq!(v[3][2], "✦");
        assert_eq!(v.len(), 4);
        // times never go back
        let ts: Vec<f64> = v[1..].iter().map(|e| e[0].as_f64().unwrap()).collect();
        assert!(ts.windows(2).all(|w| w[0] <= w[1]), "{ts:?}");
        let i = info(&path).unwrap();
        assert_eq!((i.width, i.height, i.title.as_str()), (20, 3, "Aleph"));
    }

    #[test]
    fn away_stops_the_clock_and_back_starts_on_a_whole_screen() {
        let p = tmp("away");
        let mut r = Recorder::create(&p, Opts::default()).unwrap();
        r.frame(&frame(10, 2, "one"));
        r.pause("away");
        assert!(r.is_away());
        // what happens meanwhile (a launch card, the browser) isn't recorded
        r.bytes(b"launch card");
        r.flush();
        r.frame(&frame(10, 2, "card"));
        std::thread::sleep(Duration::from_millis(30));
        let before = r.now();
        std::thread::sleep(Duration::from_millis(30));
        assert!((r.now() - before).abs() < 0.005, "the clock stood still");
        r.resume();
        r.frame(&frame(12, 2, "two"));
        let (path, _) = r.finish().unwrap();
        let v = lines(&path);
        let kinds: Vec<&str> = v[1..].iter().map(|e| e[1].as_str().unwrap()).collect();
        assert_eq!(kinds, ["o", "m", "r", "o"]);
        assert_eq!(v[2][2], "away");
        assert_eq!(v[3][2], "12x2");
        let all = std::fs::read_to_string(&path).unwrap();
        assert!(!all.contains("launch card") && !all.contains("card\""));
        assert!(v[4][2].as_str().unwrap().contains("two"));
    }

    #[test]
    fn a_recording_without_a_frame_leaves_nothing() {
        let p = tmp("empty");
        let r = Recorder::create(&p, Opts::default()).unwrap();
        assert!(p.exists());
        assert!(r.finish().is_none());
        assert!(!p.exists());
        assert!(info(&p).is_none());
    }

    #[test]
    fn pictures_are_left_out() {
        let s = "a\x1b_Ga=T,f=100;QUJD\x1b\\b\x1b]1337;File=inline=1:QUJD\x07c\u{10EEEE}\u{305}\u{30D}d";
        assert_eq!(clean(s), "abc d");
        // cut off: dropped to the end
        assert_eq!(clean("x\x1b_Gabc"), "x");
        assert_eq!(
            clean("plain \x1b[31mred\x1b[0m"),
            "plain \x1b[31mred\x1b[0m"
        );
    }

    #[test]
    fn wide_characters_take_their_cells_once() {
        let k = keyframe(&frame(6, 1, "漢x"));
        // the cell 漢 covers isn't drawn over it: the next is drawn after it
        assert!(k.contains('漢') && k.contains("\x1b[1;3Hx"), "{k:?}");
        let (s, rest) = utf8_prefix(&[b'a', 0xff, b'b', 0xe2, 0x9c]);
        assert_eq!(s, "a\u{fffd}b");
        assert_eq!(rest, &[0xe2, 0x9c]);
    }

    /// A real ratatui terminal whose writer feeds a recorder (as
    /// `term::Out` does), frames drawn before, during, away and after.
    #[test]
    fn a_cast_plays_back_to_the_screen_as_drawn() {
        use ratatui::layout::Rect;
        use ratatui::{TerminalOptions, Viewport};
        use std::cell::RefCell;
        use std::rc::Rc;

        struct Tee(Rc<RefCell<Option<Recorder>>>);
        impl Write for Tee {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                if let Some(r) = self.0.borrow_mut().as_mut() {
                    r.bytes(b);
                }
                Ok(b.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                if let Some(r) = self.0.borrow_mut().as_mut() {
                    r.flush();
                }
                Ok(())
            }
        }
        let rec: Rc<RefCell<Option<Recorder>>> = Rc::new(RefCell::new(None));
        let mut t = Terminal::with_options(
            CrosstermBackend::new(Tee(rec.clone())),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 24, 4)),
            },
        )
        .unwrap();
        let mut draw = |text: &str, col: Color| {
            t.draw(|f| {
                f.render_widget(Paragraph::new(text).style(Style::new().fg(col)), f.area());
                if let Some(r) = rec.borrow_mut().as_mut() {
                    r.frame(f.buffer_mut());
                }
            })
            .unwrap();
        };
        // the dashboard's been up a while
        draw("before\nthe recording", Color::Blue);
        let p = tmp("replay");
        *rec.borrow_mut() = Some(Recorder::create(&p, Opts::default()).unwrap());
        draw("ℵ aleph\nfleet 漢 ok", Color::Yellow);
        draw("ℵ aleph\nfleet 漢 ok\nj next", Color::Yellow);
        rec.borrow_mut().as_mut().unwrap().pause("away");
        // a launch card meanwhile
        draw("launch card", Color::Red);
        rec.borrow_mut().as_mut().unwrap().resume();
        draw("ℵ back\n✦ done", Color::Green);
        let (path, _) = rec.borrow_mut().take().unwrap().finish().unwrap();
        // played back: the screen as last drawn, and nothing of the rest
        let v = lines(&path);
        let mut vt = vt100::Parser::new(4, 24, 0);
        let mut seen = String::new();
        for e in &v[1..] {
            if e[1] == "o" {
                let d = e[2].as_str().unwrap();
                seen.push_str(d);
                vt.process(d.as_bytes());
            }
        }
        let screen: Vec<String> = vt
            .screen()
            .contents()
            .lines()
            .map(|l| l.trim_end().to_string())
            .collect();
        assert_eq!(screen, ["ℵ back", "✦ done", "", ""], "{screen:?}");
        assert!(
            !seen.contains("before") && !seen.contains("launch"),
            "{seen:?}"
        );
        // (a diff writes only what changed: the space between stays as it was)
        assert!(seen.contains("next"));
    }
}
