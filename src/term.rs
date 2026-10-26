//! Terminal lifecycle. One `Tui` owns raw mode + the alternate screen; screens
//! (dashboard, folder browser, launch progress) draw into it in turn, so moving
//! between them — and finally handing the screen to herdr — never flashes the shell.
//!
//! The terminal is restored on panic and on SIGTERM / SIGHUP / SIGQUIT too, so a
//! killed dashboard never leaves the shell in raw mode.

use crossterm::event::{
    DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

pub type Term = Terminal<CrosstermBackend<Out>>;

/// What the full-screen views draw through: the tty. Showcase builds also
/// hand every byte to a recording while one runs (`showcase::tee`, after the
/// lens); other builds write straight through.
pub struct Out(File);

impl Write for Out {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = self.0.write(b)?;
        crate::showcase::tee(&b[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()?;
        crate::showcase::tee_flush();
        Ok(())
    }
}

/// Mouse capture is on while a full-screen view is up (config `mouse`).
static MOUSE: AtomicBool = AtomicBool::new(false);
/// A Tui is currently holding the terminal (raw mode / alt screen).
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Is a full-screen view up? (Children started now get their own session:
/// none of them may take the terminal from it.)
pub fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// While a view is up, a child that takes the terminal over (herdr starting
/// its server did) must not get Aleph stopped by the shell for drawing
/// (SIGTTOU, SIGTTIN): those are ignored, and put back for a hand-off.
fn hold_terminal(on: bool) {
    #[cfg(unix)]
    // SAFETY: setting a signal's disposition
    unsafe {
        let d = if on { libc::SIG_IGN } else { libc::SIG_DFL };
        libc::signal(libc::SIGTTOU, d);
        libc::signal(libc::SIGTTIN, d);
    }
}

pub fn open_tty() -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open("/dev/tty")
}

pub fn set_mouse(on: bool) {
    MOUSE.store(on, Ordering::SeqCst);
}

/// Every key reported as itself (kitty's keyboard protocol, its first
/// level: "disambiguate"), where the terminal has it: ctrl+enter, ctrl+tab,
/// alt+esc, … come in told apart from enter, tab, esc, so Focus passes them
/// to an agent as they were pressed. `ui.keyboard = legacy` leaves it off.
static KEYS_WANTED: AtomicBool = AtomicBool::new(true);
/// Asked of the terminal once: 0 not yet, 1 has it, 2 doesn't.
static KEYS_OK: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// On now (it's popped before the terminal goes to anyone else).
static KEYS_ON: AtomicBool = AtomicBool::new(false);

pub fn set_keyboard(enhanced: bool) {
    KEYS_WANTED.store(enhanced, Ordering::SeqCst);
}

fn keys_on(w: &mut impl Write) {
    use crossterm::event::{KeyboardEnhancementFlags as F, PushKeyboardEnhancementFlags};
    if !KEYS_WANTED.load(Ordering::SeqCst) || KEYS_ON.load(Ordering::SeqCst) {
        return;
    }
    let ok = match KEYS_OK.load(Ordering::SeqCst) {
        0 => {
            let ok = terminal::supports_keyboard_enhancement().unwrap_or(false);
            KEYS_OK.store(if ok { 1 } else { 2 }, Ordering::SeqCst);
            ok
        }
        n => n == 1,
    };
    if ok
        && execute!(
            w,
            PushKeyboardEnhancementFlags(F::DISAMBIGUATE_ESCAPE_CODES)
        )
        .is_ok()
    {
        KEYS_ON.store(true, Ordering::SeqCst);
    }
}

fn keys_off(w: &mut impl Write) {
    if KEYS_ON.swap(false, Ordering::SeqCst) {
        let _ = execute!(w, crossterm::event::PopKeyboardEnhancementFlags);
    }
}

/// Is every key reported as itself now?
pub fn keys_enhanced() -> bool {
    KEYS_ON.load(Ordering::SeqCst)
}

/// Bracketed paste is on (only while keys go straight into an agent: a
/// paste then reaches it as one paste, and every other view keeps getting
/// a paste as typed keys).
static PASTE: AtomicBool = AtomicBool::new(false);

/// Turn bracketed paste on or off (a no-op when it already is).
pub fn bracketed_paste(on: bool) {
    if PASTE.swap(on, Ordering::SeqCst) == on || !ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    if let Ok(mut t) = open_tty() {
        let _ = t.write_all(if on { b"\x1b[?2004h" } else { b"\x1b[?2004l" });
        let _ = t.flush();
    }
}

/// Off before the terminal goes to someone else (it never leaks into a shell).
fn paste_off(w: &mut impl Write) {
    if PASTE.swap(false, Ordering::SeqCst) {
        let _ = w.write_all(b"\x1b[?2004l");
        let _ = w.flush();
    }
}

fn restore_now() {
    if let Ok(mut t) = open_tty() {
        paste_off(&mut t);
        keys_off(&mut t);
        // a frame cut short (an error mid-draw) never leaves the screen held
        let _ = execute!(
            t,
            terminal::EndSynchronizedUpdate,
            DisableMouseCapture,
            DisableFocusChange,
            cursor::Show,
            LeaveAlternateScreen
        );
        pop_title(&mut t);
    }
    let _ = terminal::disable_raw_mode();
}

/// Save the terminal's window title (xterm title stack), so ours can be put
/// back when we leave. Terminals without a stack just ignore it.
fn push_title(w: &mut impl Write) {
    let _ = w.write_all(b"\x1b[22;0t");
    let _ = w.flush();
}

fn pop_title(w: &mut impl Write) {
    let _ = w.write_all(b"\x1b[23;0t");
    let _ = w.flush();
}

/// The terminal window / tab title (OSC 2), e.g. `aleph — Fleet`.
pub fn set_title(title: &str) {
    if !ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    if let Ok(mut t) = open_tty() {
        let title = crate::showcase::text(title);
        let clean: String = title.chars().filter(|c| !c.is_control()).collect();
        let _ = write!(t, "\x1b]2;{clean}\x07");
        let _ = t.flush();
    }
}

/// Restore the terminal if anything panics while a TUI is up, or if we're
/// killed by a signal. Install once.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_now();
        // a closed pipe (`aleph ls | head`) isn't a crash: leave like `cat` does
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if msg.contains("Broken pipe") {
            std::process::exit(141);
        }
        crate::log::crash(info);
        prev(info);
    }));
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGHUP, SIGQUIT, SIGTERM};
        if let Ok(mut sigs) = signal_hook::iterator::Signals::new([SIGTERM, SIGHUP, SIGQUIT]) {
            std::thread::spawn(move || {
                if let Some(sig) = sigs.forever().next() {
                    if ACTIVE.load(Ordering::SeqCst) {
                        restore_now();
                    }
                    std::process::exit(128 + sig);
                }
            });
        }
    }
}

pub struct Tui {
    pub term: Term,
    active: bool,
}

impl Tui {
    pub fn enter() -> io::Result<Tui> {
        let tty = open_tty()?;
        terminal::enable_raw_mode()?;
        let mut out = tty.try_clone()?;
        execute!(out, EnterAlternateScreen, cursor::Hide)?;
        keys_on(&mut out);
        let _ = execute!(out, EnableFocusChange);
        push_title(&mut out);
        if MOUSE.load(Ordering::SeqCst) {
            let _ = execute!(out, EnableMouseCapture);
        }
        ACTIVE.store(true, Ordering::SeqCst);
        hold_terminal(true);
        let backend = CrosstermBackend::new(Out(tty));
        // some ptys report 0×0 — fall back to a fixed 80×24 viewport
        let term = match terminal::size() {
            Ok((w, h)) if w > 0 && h > 0 => Terminal::new(backend)?,
            _ => Terminal::with_options(
                backend,
                TerminalOptions {
                    viewport: Viewport::Fixed(Rect::new(0, 0, 80, 24)),
                },
            )?,
        };
        Ok(Tui { term, active: true })
    }

    /// Back to the normal screen in cooked mode (password prompts, setup wizards).
    pub fn suspend(&mut self) -> io::Result<()> {
        if self.active {
            paste_off(self.term.backend_mut());
            keys_off(self.term.backend_mut());
            execute!(
                self.term.backend_mut(),
                DisableMouseCapture,
                DisableFocusChange,
                cursor::Show,
                LeaveAlternateScreen
            )?;
            terminal::disable_raw_mode()?;
            self.active = false;
            hold_terminal(false);
        }
        Ok(())
    }

    /// Cooked mode but stay on the alternate screen — for a full-screen child
    /// (herdr, `agent attach`) that will draw over us.
    pub fn suspend_keep_screen(&mut self) -> io::Result<()> {
        if self.active {
            paste_off(self.term.backend_mut());
            keys_off(self.term.backend_mut());
            execute!(
                self.term.backend_mut(),
                DisableMouseCapture,
                DisableFocusChange,
                cursor::Show
            )?;
            terminal::disable_raw_mode()?;
            self.active = false;
            hold_terminal(false);
        }
        Ok(())
    }

    pub fn resume(&mut self) -> io::Result<()> {
        if !self.active {
            terminal::enable_raw_mode()?;
            execute!(self.term.backend_mut(), EnterAlternateScreen, cursor::Hide)?;
            keys_on(self.term.backend_mut());
            let _ = execute!(self.term.backend_mut(), EnableFocusChange);
            if MOUSE.load(Ordering::SeqCst) {
                let _ = execute!(self.term.backend_mut(), EnableMouseCapture);
            }
            self.term.clear()?;
            self.active = true;
            hold_terminal(true);
        }
        Ok(())
    }

    /// Leave the alternate screen up for the program we're about to exec
    /// (herdr), but give it a cooked terminal and a visible cursor to start from.
    pub fn handoff(mut self) {
        paste_off(self.term.backend_mut());
        keys_off(self.term.backend_mut());
        let _ = execute!(
            self.term.backend_mut(),
            DisableMouseCapture,
            DisableFocusChange,
            cursor::Show
        );
        let _ = terminal::disable_raw_mode();
        self.active = false;
        ACTIVE.store(false, Ordering::SeqCst);
        hold_terminal(false);
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        if self.active {
            paste_off(self.term.backend_mut());
            keys_off(self.term.backend_mut());
            let _ = execute!(
                self.term.backend_mut(),
                DisableMouseCapture,
                DisableFocusChange,
                cursor::Show,
                LeaveAlternateScreen
            );
            let _ = terminal::disable_raw_mode();
        }
        pop_title(self.term.backend_mut());
        ACTIVE.store(false, Ordering::SeqCst);
        hold_terminal(false);
    }
}

pub fn interactive() -> bool {
    io::stdout().is_terminal() && io::stdin().is_terminal()
}

/// A terminal something else left in raw mode (a program that died without
/// putting it back): newlines that don't return to the start of the line,
/// every printed line a step further right. Output processing (`OPOST`,
/// `ONLCR`) and the line discipline (`ICANON`, `ECHO`, `ISIG`, `ICRNL`) are put
/// back before anything prints; a full screen sets its own modes after.
#[cfg(unix)]
pub fn unstick() {
    use std::os::fd::AsRawFd;
    let out = io::stdout();
    if !out.is_terminal() {
        return;
    }
    let fd = out.as_raw_fd();
    // SAFETY: a termios filled by tcgetattr on our own stdout, set back on it
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) != 0 {
            return;
        }
        let cooked = t.c_oflag & libc::OPOST != 0 && t.c_lflag & libc::ICANON != 0;
        if cooked {
            return;
        }
        t.c_oflag |= libc::OPOST | libc::ONLCR;
        t.c_lflag |= libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN;
        t.c_iflag |= libc::ICRNL;
        libc::tcsetattr(fd, libc::TCSANOW, &t);
    }
}

#[cfg(not(unix))]
pub fn unstick() {}

/// Colour for printed (non-TUI) output.
pub fn color_out() -> bool {
    io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

pub fn color_err() -> bool {
    io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

// ------------------------------------------------------------------ background

/// The terminal's background as it says itself (OSC 11), asked once a run:
/// None when it isn't asked ([`may_ask`]) or doesn't answer in 120 ms.
pub fn background() -> Option<(u8, u8, u8)> {
    static V: std::sync::OnceLock<Option<(u8, u8, u8)>> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        if cfg!(test) {
            return None;
        }
        may_ask(&|k: &str| std::env::var(k).ok(), interactive()).ok()?;
        ask_background(std::time::Duration::from_millis(120))
    })
}

/// May we ask the terminal and wait for its answer? Only on a real tty
/// (stdin and stdout) with colour, straight to the terminal: not through
/// tmux, screen, zellij or herdr (they answer for themselves, or not), not
/// over ssh (a round trip can outlast the wait, and a late answer would land
/// in the input as keys), never on the Linux console or a dumb terminal
/// (they'd print the query). Err says why not.
pub fn may_ask(env: &dyn Fn(&str) -> Option<String>, tty: bool) -> Result<(), String> {
    let get = |k: &str| env(k).filter(|v| !v.is_empty());
    if !tty {
        return Err("not a terminal".into());
    }
    if get("NO_COLOR").is_some() {
        return Err("NO_COLOR is set".into());
    }
    if let Some(m) = crate::logo::images::multiplexer(env) {
        return Err(format!("inside {m}"));
    }
    if ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|k| get(k).is_some())
    {
        return Err("over ssh".into());
    }
    match get("TERM").as_deref() {
        None | Some("dumb" | "linux") => Err("this terminal can't be asked".into()),
        _ => Ok(()),
    }
}

/// OSC 11 (`ESC ] 11 ; ? BEL`) and behind it DA1 (`ESC [ c`), which every
/// terminal answers: once that answer is in, so is the first one (or it
/// isn't coming), and reading stops right there, a byte at a time, so
/// nothing that follows is taken and no reply is left for the shell or the
/// dashboard to read as keys. Raw mode for the wait (unless it's on
/// already); at most `wait`, a little longer for an answer under way.
#[cfg(unix)]
fn ask_background(wait: std::time::Duration) -> Option<(u8, u8, u8)> {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    // our own description of the tty, non-blocking: a read never waits
    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open("/dev/tty")
        .ok()?;
    let fd = tty.as_raw_fd();
    if fd >= libc::FD_SETSIZE as i32 {
        return None;
    }
    let raw = terminal::is_raw_mode_enabled().unwrap_or(false);
    if !raw && terminal::enable_raw_mode().is_err() {
        return None;
    }
    let mut buf: Vec<u8> = Vec::new();
    if tty
        .write_all(b"\x1b]11;?\x07\x1b[c")
        .and_then(|_| tty.flush())
        .is_ok()
    {
        let t0 = std::time::Instant::now();
        while !answered(&buf) && buf.len() < 512 {
            let limit = if buf.is_empty() { wait } else { wait * 4 };
            let left = limit.saturating_sub(t0.elapsed());
            if left.is_zero() {
                break;
            }
            // select(2): macOS's poll(2) doesn't take /dev/tty
            // SAFETY: a zeroed fd_set with our one fd (< FD_SETSIZE) and a
            // timeval, both alive for the call
            let n = unsafe {
                let mut set: libc::fd_set = std::mem::zeroed();
                libc::FD_ZERO(&mut set);
                libc::FD_SET(fd, &mut set);
                let mut tv = libc::timeval {
                    tv_sec: left.as_secs() as _,
                    tv_usec: left.subsec_micros() as _,
                };
                libc::select(
                    fd + 1,
                    &mut set,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut tv,
                )
            };
            if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if n <= 0 {
                break;
            }
            let mut b = [0u8; 1];
            match tty.read(&mut b) {
                Ok(1) => buf.push(b[0]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                _ => break,
            }
        }
        if !answered(&buf) {
            // part of an answer, or none: drop what's queued
            // SAFETY: tcflush on our own open tty
            unsafe { libc::tcflush(fd, libc::TCIFLUSH) };
        }
    }
    if !raw {
        let _ = terminal::disable_raw_mode();
    }
    parse_background(&buf)
}

#[cfg(not(unix))]
fn ask_background(_wait: std::time::Duration) -> Option<(u8, u8, u8)> {
    None
}

/// Has the DA1 answer (`ESC [ ? 6 2 ; … c`) come in?
fn answered(buf: &[u8]) -> bool {
    buf.windows(3).enumerate().any(|(i, w)| {
        w == b"\x1b[?" && {
            let rest = &buf[i + 3..];
            let n = rest
                .iter()
                .take_while(|b| b.is_ascii_digit() || **b == b';')
                .count();
            rest.get(n) == Some(&b'c')
        }
    })
}

/// The colour in an OSC 11 answer (`ESC ] 11 ; rgb:rrrr/gggg/bbbb`, ended by
/// BEL or ST; 1–4 hex digits a channel; `rgba:` and `#rrggbb` too), wherever
/// it is among other bytes; None when there's none or it's malformed.
pub fn parse_background(buf: &[u8]) -> Option<(u8, u8, u8)> {
    let at = buf.windows(5).position(|w| w == b"\x1b]11;")? + 5;
    let rest = &buf[at..];
    let end = rest.iter().position(|&b| b == 0x07 || b == 0x1b)?;
    if rest[end] == 0x1b && rest.get(end + 1) != Some(&b'\\') {
        return None;
    }
    let spec = std::str::from_utf8(&rest[..end]).ok()?.trim();
    let chan = |h: &str| -> Option<u8> {
        if h.is_empty() || h.len() > 4 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        let max = (1u32 << (4 * h.len())) - 1;
        Some(((v * 255 + max / 2) / max) as u8)
    };
    if let Some(hex) = spec.strip_prefix('#') {
        let k = hex.len() / 3;
        if !hex.is_ascii() || k == 0 || hex.len() % 3 != 0 {
            return None;
        }
        return Some((
            chan(&hex[..k])?,
            chan(&hex[k..2 * k])?,
            chan(&hex[2 * k..])?,
        ));
    }
    let (body, n) = match spec.strip_prefix("rgba:") {
        Some(b) => (b, 4),
        None => (spec.strip_prefix("rgb:")?, 3),
    };
    let parts: Vec<&str> = body.split('/').collect();
    if parts.len() != n {
        return None;
    }
    Some((chan(parts[0])?, chan(parts[1])?, chan(parts[2])?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_answers() {
        let p = |s: &[u8]| parse_background(s);
        assert_eq!(p(b"\x1b]11;rgb:ffff/ffff/ffff\x07"), Some((255, 255, 255)));
        // ended by ST, two digits a channel
        assert_eq!(p(b"\x1b]11;rgb:1e/1e/2e\x1b\\"), Some((0x1e, 0x1e, 0x2e)));
        // 1 and 3 digits scale to 8 bits; 4 keep the top byte, rounded
        assert_eq!(p(b"\x1b]11;rgb:f/8/0\x07"), Some((255, 136, 0)));
        assert_eq!(p(b"\x1b]11;rgb:fff/000/800\x07"), Some((255, 0, 128)));
        assert_eq!(
            p(b"\x1b]11;rgb:2828/2a2a/3636\x07"),
            Some((0x28, 0x2a, 0x36))
        );
        assert_eq!(
            p(b"\x1b]11;rgba:fdfd/f6f6/e3e3/ffff\x07"),
            Some((0xfd, 0xf6, 0xe3))
        );
        assert_eq!(p(b"\x1b]11;#fdf6e3\x07"), Some((0xfd, 0xf6, 0xe3)));
        // among other bytes: a key typed ahead, then DA1's answer
        assert_eq!(
            p(b"j\x1b]11;rgb:0000/2b2b/3636\x07\x1b[?62;22c"),
            Some((0, 0x2b, 0x36))
        );
        // malformed or missing
        for bad in [
            &b""[..],
            b"\x1b[?62;22c",
            b"\x1b]11;rgb:ffff/ffff/ffff",
            b"\x1b]11;rgb:ffff/ffff\x07",
            b"\x1b]11;rgb:zz/00/00\x07",
            b"\x1b]11;rgb:fffff/0/0\x07",
            b"\x1b]11;rgb://\x07",
            b"\x1b]11;rgb:ff/ff/ff/ff\x07",
            b"\x1b]11;rgba:ff/ff/ff\x07",
            b"\x1b]11;?\x07",
            b"\x1b]11;#ffff\x07",
            b"\x1b]11;#\xc3\xa9\xc3\xa9\xc3\xa9\x07",
            b"\x1b]11;rgb:ff/ff/ff\x1bX",
            b"\x1b]10;rgb:ffff/ffff/ffff\x07",
            b"\x1b]11;\xff\xfe\x07",
        ] {
            assert_eq!(p(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_last_answer_ends_the_wait() {
        assert!(answered(b"\x1b]11;rgb:0/0/0\x07\x1b[?62;22c"));
        assert!(answered(b"\x1b[?1;2c"));
        assert!(answered(b"\x1b[?c"));
        assert!(!answered(b"\x1b[?62;2"));
        assert!(!answered(b"\x1b]11;rgb:0/0/0\x07"));
        assert!(!answered(b"\x1b[?62;22x"));
        assert!(!answered(b""));
    }

    #[test]
    fn when_the_terminal_may_be_asked() {
        let env = |pairs: &[(&str, &str)]| {
            let v: Vec<(String, String)> = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            move |k: &str| v.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
        };
        let term = ("TERM", "xterm-256color");
        assert_eq!(may_ask(&env(&[term]), true), Ok(()));
        assert!(may_ask(&env(&[term]), false).is_err(), "not a tty");
        for (pairs, why) in [
            (&[term, ("NO_COLOR", "1")][..], "NO_COLOR"),
            (&[term, ("TMUX", "/tmp/t,1,0")][..], "tmux"),
            (&[("TERM", "screen-256color")][..], "screen"),
            (&[term, ("ZELLIJ", "0")][..], "zellij"),
            (&[term, ("HERDR_ENV", "1")][..], "herdr"),
            (
                &[term, ("SSH_CONNECTION", "1.2.3.4 5 6.7.8.9 22")][..],
                "ssh",
            ),
            (&[term, ("SSH_TTY", "/dev/pts/1")][..], "ssh"),
            (&[("TERM", "linux")][..], "can't"),
            (&[("TERM", "dumb")][..], "can't"),
            (&[][..], "can't"),
        ] {
            let e = may_ask(&env(pairs), true).unwrap_err();
            assert!(e.contains(why), "{pairs:?}: {e}");
        }
        // empty values count as unset
        assert_eq!(
            may_ask(&env(&[term, ("TMUX", ""), ("NO_COLOR", "")]), true),
            Ok(())
        );
    }
}
