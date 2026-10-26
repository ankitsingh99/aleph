//! An agent's screen with its words in alien script (`showcase.screens =
//! redact`). The agent's own frame stays readable — box borders, gutters
//! and bullets, its status and footer lines, spinners, token counts, model
//! names, the prompt's frame, its questions and their options, the names of
//! its tools — and everything it or you wrote becomes symbols of the same
//! widths: spaces, punctuation and line shapes stay, and a word always gets
//! the same symbols, so a live screen doesn't flicker.
//!
//! Pure: a line in, the character ranges to swap out ([`swaps`]); text,
//! plain or ANSI ([`text`]), and drawn cells ([`cells`]) both go through it.
//! `hide` draws a card instead ([`card`], [`HIDDEN`]).

use crate::ui::{self, c};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// The alien script: one-cell symbols, none of them a letter or a digit
/// (so the lens never takes them for words).
pub const GLYPHS: [char; 26] = [
    '⏃', '⏚', '☊', '⎅', '⟒', '⎎', '☌', '⊑', '⟟', '⟊', '☍', '⌰', '⋔', '⋏', '⍜', '⌿', '⍾', '⍀', '⌇',
    '⏁', '⎍', '⎐', '⍙', '⌖', '⊬', '⋉',
];

/// What a hidden screen says, as text.
pub const HIDDEN: &str = "◌ screen hidden for the demo";

/// Box drawing, blocks and braille: the screen's frame, never words.
fn frame(c: char) -> bool {
    matches!(c as u32, 0x2500..=0x259F | 0x2800..=0x28FF)
}

/// A mark at a run's start that's the agent's, not words: bullets, prompts,
/// gutters, spinners, a diff's signs.
fn marker(c: char) -> bool {
    matches!(
        c,
        '⏺' | '●'
            | '•'
            | '◦'
            | '∙'
            | '·'
            | '›'
            | '❯'
            | '>'
            | '⎿'
            | '↳'
            | '→'
            | '▸'
            | '▶'
            | '△'
            | '▲'
            | '■'
            | '□'
            | '☐'
            | '☒'
            | '✓'
            | '✔'
            | '✗'
            | '✘'
            | '✕'
            | '⚠'
            | '✻'
            | '✳'
            | '✶'
            | '✢'
            | '✽'
            | '✺'
            | '✹'
            | '✷'
            | '✸'
            | '⏵'
            | '⏸'
            | '*'
            | '$'
            | '#'
            | '?'
            | '!'
            | '-'
            | '+'
            | '%'
            | '❮'
            | '◆'
            | '◇'
            | '○'
            | '◉'
            | '◐'
            | '◓'
            | '◑'
            | '◒'
    )
}

/// Lines with one of these are the agent's own (status, footers, welcome
/// boxes, questions, empty prompts), whatever else they say.
const PHRASES: &[&str] = &[
    // status, spinner and footer lines
    "esc to interrupt",
    "esc to cancel",
    "ctrl+c to",
    "to interrupt",
    "for shortcuts",
    "context left",
    "accept edits",
    "bypass permissions",
    "plan mode",
    "auto-accept",
    "auto-compact",
    "shift+tab",
    "press enter",
    "to confirm",
    "worked for",
    "interrupted by user",
    "tokens used",
    // welcome boxes and banners
    "welcome to",
    "for help",
    "/help",
    "/status",
    "/model",
    "/init",
    "claude code",
    "openai codex",
    "gemini cli",
    "opencode",
    "tips for getting started",
    // questions
    "do you want to",
    "would you like to",
    "permission required",
    "don't ask again",
    "what to do differently",
    "allow once",
    "allow always",
    "allow execution",
    "bash command",
    "edit file",
    "create file",
    "read file",
    "run this command",
    // an empty prompt's placeholder
    "ask codex",
    "ask anything",
    "type your message",
    "send a message",
    "try \"",
];

/// `label: …` lines of a status block or a welcome box.
const LABELS: &[&str] = &[
    "model",
    "directory",
    "cwd",
    "workdir",
    "provider",
    "account",
    "session",
    "version",
    "mode",
    "approval",
    "sandbox",
    "effort",
    "context",
    "agent",
];

/// A question's options start with one of these (`❯ 1. Yes`, `3. No, and …`).
const OPTIONS: &[&str] = &[
    "yes", "no", "allow", "reject", "deny", "cancel", "always", "approve", "skip", "don't",
    "abort", "continue",
];

/// An agent's tools, as their calls start (`⏺ Bash(…)`, `• Ran …`).
const TOOLS: &[&str] = &[
    "Bash",
    "Read",
    "Write",
    "Edit",
    "Update",
    "MultiEdit",
    "Search",
    "Grep",
    "Glob",
    "LS",
    "List",
    "Task",
    "Agent",
    "WebFetch",
    "WebSearch",
    "Fetch",
    "TodoWrite",
    "NotebookEdit",
    "Ran",
    "Explored",
    "Edited",
    "Added",
    "Deleted",
    "Created",
    "Updated",
    "Running",
    "Called",
    "Shell",
    "Skill",
    "Plan",
];

/// A model's name starts with one of these (`claude-opus-4-1`, `gpt-5.1-codex`, `Sonnet`).
const MODELS: &[&str] = &[
    "claude", "opus", "sonnet", "haiku", "gpt", "o1", "o3", "o4", "codex", "gemini", "qwen",
    "llama", "deepseek", "mistral", "grok", "kimi", "glm",
];

/// What a run of a line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    /// the agent's own: kept whole
    Keep,
    /// a tool call: its name (this many characters) kept, the rest words
    Tool(usize),
    /// words (model names and token counts among them kept)
    Words,
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The run `t` (its leading marks gone; `led`: there were some) decided.
fn classify(t: &[char], led: bool, keep: &[String]) -> Part {
    let s: String = t.iter().collect();
    let low = s.to_lowercase();
    if PHRASES.iter().any(|p| low.contains(p)) || keep.iter().any(|k| low.contains(k.as_str())) {
        return Part::Keep;
    }
    let first: String = t.iter().take_while(|c| word_char(**c)).collect();
    let rest = &t[first.chars().count()..];
    // a spinner's status: `✻ Pondering…`, `◦ Working…`
    if led
        && !first.is_empty()
        && (rest.first() == Some(&'…') || rest.starts_with(&['.', '.', '.']))
    {
        return Part::Keep;
    }
    // `model: gpt-5.1-codex high`, `cwd: ~/x`
    if rest.first() == Some(&':') && LABELS.contains(&first.to_lowercase().as_str()) {
        return Part::Keep;
    }
    // a question's option: `1. Yes`, `2) No, and tell …`
    if !first.is_empty() && first.len() <= 2 && first.chars().all(|c| c.is_ascii_digit()) {
        if let Some(r) = rest
            .strip_prefix(&['.'])
            .or_else(|| rest.strip_prefix(&[')']))
        {
            let w: String = r
                .iter()
                .skip_while(|c| c.is_whitespace())
                .take_while(|c| word_char(**c) || **c == '\'')
                .collect();
            if OPTIONS.contains(&w.to_lowercase().as_str()) {
                return Part::Keep;
            }
            // a numbered list: its number stays, like a bullet
            return Part::Tool(first.len() + 1);
        }
    }
    // a tool call: `Bash(cargo test)`, `• Ran cargo test`
    if TOOLS.contains(&first.as_str()) {
        match rest.first() {
            Some('(') => return Part::Tool(first.chars().count()),
            None => return Part::Keep,
            Some(' ') if led => return Part::Tool(first.chars().count()),
            _ => {}
        }
    }
    Part::Words
}

/// A model's name (kept among words).
fn model(w: &str) -> bool {
    let l = w.to_lowercase();
    MODELS.iter().any(|m| {
        l.strip_prefix(m).is_some_and(|r| {
            r.is_empty()
                || r.starts_with(['-', '.', '_'])
                || r.starts_with(|c: char| c.is_ascii_digit())
        })
    })
}

/// A number a counter shows (`1.2k`, `↑ 812`, `45%`).
fn count(w: &[char]) -> bool {
    let w: Vec<char> = w
        .iter()
        .copied()
        .filter(|c| !matches!(c, '↑' | '↓'))
        .collect();
    !w.is_empty()
        && w[0].is_ascii_digit()
        && w.iter()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | 'k' | 'K' | 'M' | 'B' | '%'))
}

/// The character ranges of `line` to swap for alien script.
pub fn swaps(line: &[char], keep: &[String]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let n = line.len();
    let mut i = 0;
    while i < n {
        // a run: up to the next piece of the frame
        if frame(line[i]) || line[i] == '\0' {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && !frame(line[i]) && line[i] != '\0' {
            i += 1;
        }
        let run = &line[start..i];
        // its leading marks and spaces stay
        let lead = run
            .iter()
            .take_while(|c| c.is_whitespace() || marker(**c))
            .count();
        let led = run[..lead].iter().any(|c| marker(*c)) || start > 0 && frame(line[start - 1]);
        let mut end = run.len();
        while end > lead && run[end - 1].is_whitespace() {
            end -= 1;
        }
        if end <= lead {
            continue;
        }
        let t = &run[lead..end];
        let from = match classify(t, led, keep) {
            Part::Keep => continue,
            Part::Tool(k) => k,
            Part::Words => 0,
        };
        words(t, from, start + lead, &mut out);
    }
    out
}

/// The words of `t` from `from` on (at `base` in the line) into `out`,
/// but for model names and counters.
fn words(t: &[char], from: usize, base: usize, out: &mut Vec<(usize, usize)>) {
    // whitespace-separated tokens, to know a counter by the word after it
    let mut toks: Vec<(usize, usize)> = Vec::new();
    let mut j = from;
    while j < t.len() {
        if t[j].is_whitespace() {
            j += 1;
            continue;
        }
        let a = j;
        while j < t.len() && !t[j].is_whitespace() {
            j += 1;
        }
        toks.push((a, j));
    }
    for (k, &(a, b)) in toks.iter().enumerate() {
        let tok = &t[a..b];
        let core: String = tok
            .iter()
            .collect::<String>()
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_string();
        if model(&core) {
            continue;
        }
        let next: String = toks
            .get(k + 1)
            .map(|&(x, y)| t[x..y].iter().collect::<String>().to_lowercase())
            .unwrap_or_default();
        if count(tok) && (next.starts_with("token") || next.starts_with("tok")) {
            continue;
        }
        let prev_count = k > 0 && {
            let (x, y) = toks[k - 1];
            count(&t[x..y])
        };
        if prev_count && core.to_lowercase().starts_with("tok") {
            continue;
        }
        // its words (runs of letters, digits and _), punctuation kept
        let mut p = a;
        while p < b {
            if !word_char(t[p]) {
                p += 1;
                continue;
            }
            let s = p;
            while p < b && word_char(t[p]) {
                p += 1;
            }
            out.push((base + s, base + p));
        }
    }
}

// ------------------------------------------------------------------ the script

/// A word's seed: the same word, the same symbols (for this salt).
fn seed(word: &[char], key: u64) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ key;
    for c in word {
        h ^= *c as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The symbol for the `n`th cell of a word with seed `h`.
fn glyph(h: u64, n: usize) -> char {
    let mut z = h.wrapping_add((n as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    GLYPHS[((z ^ (z >> 31)) % GLYPHS.len() as u64) as usize]
}

/// A word in alien script, as many cells wide (a wide character gets two
/// symbols, a mark joining the one before none).
pub fn alien(word: &[char], key: u64) -> String {
    let h = seed(word, key);
    let mut n = 0;
    let mut s = String::new();
    for c in word {
        for _ in 0..c.width().unwrap_or(0) {
            s.push(glyph(h, n));
            n += 1;
        }
    }
    s
}

/// The salt's part in the script: another set of symbols for each set of
/// stand-ins (none in tests).
fn key() -> u64 {
    use std::sync::Mutex;
    static K: Mutex<(u64, u64)> = Mutex::new((u64::MAX, 0));
    let ep = super::super::state::epoch();
    let mut k = K.lock().unwrap_or_else(|e| e.into_inner());
    if k.0 != ep {
        let st = super::super::state::get();
        *k = (
            ep,
            seed(&st.salt.iter().map(|b| *b as char).collect::<Vec<_>>(), 7),
        );
    }
    k.1
}

// ------------------------------------------------------------------ text

/// One line of text, ANSI or plain: its escapes as they are, its words swapped.
fn line(l: &str, keep: &[String], key: u64) -> String {
    // the line's pieces: escapes (kept) and visible characters
    enum Tok<'a> {
        Esc(&'a str),
        Ch(char),
    }
    let mut toks: Vec<Tok> = Vec::new();
    let b = l.as_bytes();
    let mut i = 0;
    while i < l.len() {
        if b[i] == 0x1b {
            let st = i;
            i += 1;
            match b.get(i) {
                Some(b'[') => {
                    i += 1;
                    while i < l.len() && !(0x40..=0x7e).contains(&b[i]) {
                        i += 1;
                    }
                    i = (i + 1).min(l.len());
                }
                Some(b']') => {
                    i += 1;
                    while i < l.len()
                        && b[i] != 0x07
                        && !(b[i] == 0x1b && b.get(i + 1) == Some(&b'\\'))
                    {
                        i += 1;
                    }
                    i = if b.get(i) == Some(&0x07) {
                        i + 1
                    } else {
                        (i + 2).min(l.len())
                    };
                }
                Some(_) => i += 1,
                None => {}
            }
            while !l.is_char_boundary(i) {
                i += 1;
            }
            toks.push(Tok::Esc(&l[st..i]));
            continue;
        }
        let c = l[i..].chars().next().unwrap_or(' ');
        i += c.len_utf8();
        toks.push(Tok::Ch(c));
    }
    let chars: Vec<char> = toks
        .iter()
        .filter_map(|t| match t {
            Tok::Ch(c) if c.is_control() => Some(' '),
            Tok::Ch(c) => Some(*c),
            Tok::Esc(_) => None,
        })
        .collect();
    let sw = swaps(&chars, keep);
    if sw.is_empty() {
        return l.to_string();
    }
    // each visible character: its replacement, when it's in a swapped word
    let mut repl: Vec<Option<String>> = vec![None; chars.len()];
    for (a, b) in sw {
        let al = alien(&chars[a..b], key);
        let mut g = al.chars();
        for k in a..b {
            let w = chars[k].width().unwrap_or(0);
            repl[k] = Some(g.by_ref().take(w).collect());
        }
    }
    let mut out = String::with_capacity(l.len());
    let mut k = 0;
    for t in toks {
        match t {
            Tok::Esc(e) => out.push_str(e),
            Tok::Ch(c) => {
                match &repl[k] {
                    Some(r) => out.push_str(r),
                    None => out.push(c),
                }
                k += 1;
            }
        }
    }
    out
}

/// A screen's text (plain or ANSI, any number of lines) redacted.
pub fn text(s: &str, keep: &[String]) -> String {
    let key = key();
    s.split('\n')
        .map(|l| line(l, keep, key))
        .collect::<Vec<_>>()
        .join("\n")
}

// ------------------------------------------------------------------ cells

/// The screen drawn in `area` redacted, cell for cell: colours and frame
/// as they were. Pictures and the cells wide characters cover are left be.
pub fn cells(buf: &mut Buffer, area: Rect, keep: &[String]) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    let key = key();
    for y in area.top()..area.bottom() {
        let mut chars: Vec<char> = Vec::with_capacity(area.width as usize);
        let mut xs: Vec<Option<u16>> = Vec::with_capacity(area.width as usize);
        let mut any = false;
        for x in area.left()..area.right() {
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            // a picture's cell (kitty's U+10EEEE, iTerm's markers) is never text
            if cell.skip || sym.as_bytes().first().is_some_and(|b| *b >= 0xF3) {
                chars.push('\0');
                xs.push(None);
                continue;
            }
            let c = sym.chars().next().unwrap_or(' ');
            any |= word_char(c);
            chars.push(if c.is_control() { ' ' } else { c });
            xs.push(Some(x));
        }
        if !any {
            continue;
        }
        for (a, b) in swaps(&chars, keep) {
            let al = alien(&chars[a..b], key);
            let mut g = al.chars();
            for k in a..b {
                let w = chars[k].width().unwrap_or(0) as u16;
                let Some(x) = xs[k] else {
                    g.by_ref().take(w as usize).for_each(drop);
                    continue;
                };
                // a wide character's second cell gets a symbol of its own
                for j in 0..w {
                    let Some(c) = g.next() else { break };
                    if x + j < area.right() {
                        let mut s = [0u8; 4];
                        buf[(x + j, y)].set_symbol(c.encode_utf8(&mut s));
                    }
                }
            }
        }
    }
}

/// `area` blanked to a card saying the screen is hidden (the terminal's
/// own background stays).
pub fn card(buf: &mut Buffer, area: Rect) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            cell.reset();
        }
    }
    let words = "screen hidden for the demo";
    let inner = format!(" ◌  {words} ");
    let w = ui::width(&inner) as u16;
    let mid = area.y + area.height.saturating_sub(1) / 2;
    if area.width >= w + 2 && area.height >= 3 {
        let x = area.x + (area.width - (w + 2)) / 2;
        let line = |l: &str| Line::from(Span::styled(l.to_string(), ui::fg(c::LINE())));
        let top = format!("╭{}╮", "─".repeat(w as usize));
        let bot = format!("╰{}╯", "─".repeat(w as usize));
        let y = mid.saturating_sub(1).max(area.y);
        buf.set_line(x, y, &line(&top), w + 2);
        buf.set_line(
            x,
            y + 1,
            &Line::from(vec![
                Span::styled("│", ui::fg(c::LINE())),
                Span::styled(" ◌  ", ui::fg(c::ACCENT())),
                Span::styled(
                    format!("{words} "),
                    ui::muted().add_modifier(Modifier::ITALIC),
                ),
                Span::styled("│", ui::fg(c::LINE())),
            ]),
            w + 2,
        );
        buf.set_line(x, y + 2, &line(&bot), w + 2);
    } else {
        let t = ui::trunc(HIDDEN, area.width as usize);
        let tw = ui::width(&t) as u16;
        let x = area.x + area.width.saturating_sub(tw) / 2;
        buf.set_line(
            x,
            mid,
            &Line::from(Span::styled(t, Style::new().fg(c::MUTED()))),
            area.width,
        );
    }
}
