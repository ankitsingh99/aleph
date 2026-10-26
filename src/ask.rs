//! A question an agent is asking — herdr's `detection` read of a blocked
//! pane (the bottom of its screen) — taken apart for the approval board
//! (`tui/approvals.rs`): what it asks, every option it offers (numbered or
//! not, in a box or not, two or twenty of them), which one its cursor is
//! on, and the keys that choose each. A menu with a cursor is answered the
//! way every agent's menu takes it: the arrows from the cursor to the
//! option, then enter (no agent's own number shortcuts assumed); without a
//! cursor, the option's number.

/// One option of the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opt {
    /// its number on the screen (`2.`), if it has one
    pub n: Option<u32>,
    pub label: String,
}

/// What the agent asks.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Ask {
    /// the question's own lines (above the options; boxes' borders off)
    pub question: Vec<String>,
    pub options: Vec<Opt>,
    /// the option the agent's cursor is on
    pub cursor: Option<usize>,
}

/// What a key on the board asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Approve,
    Deny,
    /// the option at this index
    Option(usize),
}

/// Marks a cursor line (`❯ 1. Yes`, Gemini's chosen radio `●`).
const CURSORS: &[char] = &['❯', '›', '▸', '►', '→', '●', '◉', '>'];
/// Marks an option the cursor isn't on (radio buttons).
const UNMARKED: &[char] = &['○', '◯', '◦'];
/// A box's sides, taken off both ends of a line.
const SIDES: &[char] = &['│', '┃', '║', '|', '╎', '┆'];

/// A line with a box's sides and the indentation off.
fn clean(l: &str) -> &str {
    l.trim()
        .trim_start_matches(SIDES)
        .trim_end_matches(SIDES)
        .trim()
}

/// (on the cursor, the rest) of a cleaned line.
fn marked(t: &str) -> (bool, &str) {
    if let Some(r) = t.strip_prefix(CURSORS) {
        return (true, r.trim_start());
    }
    if let Some(r) = t.strip_prefix(UNMARKED) {
        return (false, r.trim_start());
    }
    (false, t)
}

/// `2. Yes, and don't ask again` → (2, the label).
fn numbered(t: &str) -> Option<(u32, &str)> {
    let (n, rest) = t.split_once(['.', ')'])?;
    let n: u32 = n.trim().parse().ok()?;
    let label = rest.trim();
    ((1..=20).contains(&n) && !label.is_empty()).then_some((n, label))
}

/// A line of one drawing character: a box's top or bottom, a rule.
fn rule(t: &str) -> bool {
    let t = t.trim();
    t.chars().count() >= 3 && t.chars().all(|c| "─━═╌┄┈-╭╮╰╯┌┐└┘┏┓┗┛╔╗╚╝".contains(c))
}

/// Words a "yes" option starts with.
const YES: &[&str] = &[
    "yes", "allow", "approve", "trust", "continue", "proceed", "accept", "ok", "run",
];

fn is_yes(label: &str) -> bool {
    let t = label.trim().to_lowercase();
    YES.iter().any(|w| t.starts_with(w))
}

fn is_no(label: &str) -> bool {
    let t = label.trim().to_lowercase();
    t.starts_with("no")
        || [
            "reject",
            "deny",
            "don't allow",
            "do not allow",
            "cancel",
            "skip",
        ]
        .iter()
        .any(|w| t.starts_with(w))
}

impl Ask {
    /// Take the question apart (herdr's plain `detection` text). No menu
    /// found: no options (the question is the screen's last lines).
    pub fn parse(detection: &str) -> Ask {
        let lines: Vec<&str> = detection.lines().collect();
        let cleaned: Vec<&str> = lines.iter().map(|l| clean(l)).collect();
        let (first, options, cursor) = numbered_menu(&cleaned)
            .or_else(|| arrow_menu(&cleaned))
            .unwrap_or((lines.len(), vec![], None));
        Ask {
            question: question_above(&cleaned[..first]),
            options,
            cursor,
        }
    }

    /// The first option that says yes.
    pub fn yes(&self) -> Option<usize> {
        self.options.iter().position(|o| is_yes(&o.label))
    }

    /// The first option that says no.
    pub fn no(&self) -> Option<usize> {
        self.options.iter().position(|o| is_no(&o.label))
    }

    /// The keys that choose option `i`: the arrows from the cursor, then
    /// enter; with no cursor, its number (one digit).
    pub fn keys_for(&self, i: usize) -> Option<Vec<String>> {
        if i >= self.options.len() {
            return None;
        }
        if let Some(c) = self.cursor {
            let (dir, n) = if i >= c {
                ("down", i - c)
            } else {
                ("up", c - i)
            };
            let mut k = vec![dir.to_string(); n];
            k.push("enter".into());
            return Some(k);
        }
        self.options[i]
            .n
            .filter(|n| *n <= 9)
            .map(|n| vec![n.to_string()])
    }

    /// The keys for a choice. Deny with no "no" option: esc (what every
    /// agent's menu takes as no). None: nothing to send that surely means it.
    pub fn keys(&self, c: Choice) -> Option<Vec<String>> {
        match c {
            Choice::Approve => self.keys_for(self.yes()?),
            Choice::Deny => match self.no() {
                Some(i) => self.keys_for(i),
                None => Some(vec!["esc".into()]),
            },
            Choice::Option(i) => self.keys_for(i),
        }
    }

    /// The same question (another read of it): its words and options, the
    /// cursor aside.
    pub fn same(&self, other: &Ask) -> bool {
        self.question == other.question && self.options == other.options
    }

    /// Its text, for the deny list and the log.
    pub fn text(&self) -> String {
        let mut t = self.question.join("\n");
        for o in &self.options {
            t.push('\n');
            t.push_str(&o.label);
        }
        t
    }
}

/// Options numbered 1, 2, … in a row (up to two wrapped or description
/// lines between them): the run with the cursor in it, else the last one.
/// (index of its first line, its options, the cursor).
fn numbered_menu(lines: &[&str]) -> Option<(usize, Vec<Opt>, Option<usize>)> {
    // (line index, number, label, on the cursor)
    let mut runs: Vec<Vec<(usize, u32, String, bool)>> = Vec::new();
    let mut cur: Vec<(usize, u32, String, bool)> = Vec::new();
    let mut gap = 0;
    for (i, l) in lines.iter().enumerate() {
        let (on, rest) = marked(l);
        match numbered(rest) {
            Some((1, label)) => {
                if cur.len() >= 2 {
                    runs.push(std::mem::take(&mut cur));
                }
                cur = vec![(i, 1, label.to_string(), on)];
                gap = 0;
            }
            Some((n, label)) if cur.last().is_some_and(|x| x.1 + 1 == n) => {
                cur.push((i, n, label.to_string(), on));
                gap = 0;
            }
            Some(_) => {
                if cur.len() >= 2 {
                    runs.push(std::mem::take(&mut cur));
                }
                cur.clear();
            }
            // a wrapped option goes on: the label carries on with it
            None if !cur.is_empty() && gap < 2 && !l.is_empty() && !rule(l) => {
                if let Some(x) = cur.last_mut() {
                    x.2.push(' ');
                    x.2.push_str(l);
                }
                gap += 1;
            }
            None => {
                if cur.len() >= 2 {
                    runs.push(std::mem::take(&mut cur));
                }
                cur.clear();
                gap = 0;
            }
        }
    }
    if cur.len() >= 2 {
        runs.push(cur);
    }
    let run = runs
        .iter()
        .rev()
        .find(|r| r.iter().any(|x| x.3))
        .or(runs.last())?;
    let cursor = run.iter().position(|x| x.3);
    Some((
        run[0].0,
        run.iter()
            .map(|x| Opt {
                n: Some(x.1),
                label: x.2.clone(),
            })
            .collect(),
        cursor,
    ))
}

/// A menu without numbers: the lines around the last cursor line (a mark
/// that can't be a quote: not `>`), 2 to 12 short ones, no gaps.
fn arrow_menu(lines: &[&str]) -> Option<(usize, Vec<Opt>, Option<usize>)> {
    let ci = lines.iter().rposition(|l| {
        let t = l.trim_start();
        !t.starts_with('>') && marked(t).0
    })?;
    let item = |l: &str| !l.is_empty() && !rule(l) && l.chars().count() <= 80;
    let mut a = ci;
    while a > 0 && item(lines[a - 1]) && ci - (a - 1) < 12 {
        a -= 1;
    }
    let mut b = ci;
    while b + 1 < lines.len() && item(lines[b + 1]) && b + 1 - a < 12 {
        b += 1;
    }
    // the block above the options is the question when it ends in one
    let opts: Vec<&str> = lines[a..=b].to_vec();
    let q_end = opts
        .iter()
        .position(|l| l.trim_end().ends_with('?') || l.trim_end().ends_with(':'))
        .filter(|i| *i < ci - a)
        .map_or(0, |i| i + 1);
    let opts = &opts[q_end..];
    if opts.len() < 2 {
        return None;
    }
    Some((
        a + q_end,
        opts.iter()
            .map(|l| Opt {
                n: None,
                label: marked(l).1.to_string(),
            })
            .collect(),
        Some(ci - a - q_end),
    ))
}

/// The question: the lines above the options, from the box's top (or the
/// last rule) down, blank lines at either end off; at most its last 14.
fn question_above(lines: &[&str]) -> Vec<String> {
    let start = lines.iter().rposition(|l| rule(l)).map_or(0, |i| i + 1);
    let mut v: Vec<String> = lines[start..]
        .iter()
        .map(|l| l.to_string())
        .filter(|l| !rule(l))
        .collect();
    while v.first().is_some_and(|l| l.trim().is_empty()) {
        v.remove(0);
    }
    while v.last().is_some_and(|l| l.trim().is_empty()) {
        v.pop();
    }
    let n = v.len();
    v.into_iter().skip(n.saturating_sub(14)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(a: &Ask) -> Vec<&str> {
        a.options.iter().map(|o| o.label.as_str()).collect()
    }

    #[test]
    fn claude_codes_question() {
        let a = Ask::parse(
            "⏺ Bash(cargo bench --bench parse)\n\
             ╭──────────────────────────────────────╮\n\
             │ Bash command                         │\n\
             │                                      │\n\
             │   cargo bench --bench parse_layout   │\n\
             │                                      │\n\
             │ Do you want to proceed?              │\n\
             │ ❯ 1. Yes                             │\n\
             │   2. Yes, and don't ask again for    │\n\
             │      cargo commands in /srv/x        │\n\
             │   3. No, and tell Claude what to do  │\n\
             │      differently (esc)               │\n\
             ╰──────────────────────────────────────╯",
        );
        assert_eq!(
            labels(&a),
            [
                "Yes",
                "Yes, and don't ask again for cargo commands in /srv/x",
                "No, and tell Claude what to do differently (esc)"
            ]
        );
        assert_eq!(a.cursor, Some(0));
        assert_eq!(a.question.first().map(String::as_str), Some("Bash command"));
        assert!(a.question.iter().any(|l| l.contains("cargo bench")));
        assert_eq!(
            a.question.last().map(String::as_str),
            Some("Do you want to proceed?")
        );
        assert_eq!(a.keys(Choice::Approve).unwrap(), ["enter"]);
        assert_eq!(a.keys(Choice::Deny).unwrap(), ["down", "down", "enter"]);
        assert_eq!(a.keys(Choice::Option(1)).unwrap(), ["down", "enter"]);
        assert_eq!(a.keys(Choice::Option(3)), None);
    }

    #[test]
    fn codexs_question_with_the_cursor_further_down() {
        let a = Ask::parse(
            "Would you like to run the following command?\n\
             \n\
             $ cargo test --all\n\
             \n\
               1. Yes, proceed (y)\n\
             › 2. Yes, and don't ask again for this command (a)\n\
               3. No, and tell Codex what to do differently (esc)\n\
             \n\
             Press enter to confirm or esc to cancel",
        );
        assert_eq!(a.options.len(), 3);
        assert_eq!(a.cursor, Some(1));
        assert_eq!(a.keys(Choice::Approve).unwrap(), ["up", "enter"]);
        assert_eq!(a.keys(Choice::Deny).unwrap(), ["down", "enter"]);
        assert!(a.question.iter().any(|l| l.contains("cargo test")));
    }

    #[test]
    fn geminis_boxed_radio_buttons_and_six_options() {
        let a = Ask::parse(
            "╭──────────────────────────────────────────────╮\n\
             │ ?  Shell  rm -rf build/                       │\n\
             │                                               │\n\
             │ Allow execution of: 'rm'?                     │\n\
             │                                               │\n\
             │ ● 1. Allow once                               │\n\
             │ ○ 2. Allow for this session                   │\n\
             │ ○ 3. Allow always for rm                      │\n\
             │ ○ 4. Modify with external editor              │\n\
             │ ○ 5. Suggest changes                          │\n\
             │ ○ 6. No, suggest changes (esc)                │\n\
             ╰──────────────────────────────────────────────╯",
        );
        assert_eq!(a.options.len(), 6);
        assert_eq!(a.options[5].label, "No, suggest changes (esc)");
        assert_eq!(a.cursor, Some(0));
        assert_eq!(a.yes(), Some(0));
        assert_eq!(a.no(), Some(5));
        assert_eq!(
            a.keys(Choice::Option(4)).unwrap(),
            ["down", "down", "down", "down", "enter"]
        );
        assert_eq!(
            a.question.last().map(String::as_str),
            Some("Allow execution of: 'rm'?")
        );
    }

    #[test]
    fn a_menu_without_numbers_and_one_without_a_cursor() {
        let a = Ask::parse(
            "Permission required: edit src/main.rs\n\
             Allow this edit?\n\
             ❯ Allow once\n\
             \u{20} Allow always\n\
             \u{20} Reject",
        );
        assert_eq!(labels(&a), ["Allow once", "Allow always", "Reject"]);
        assert_eq!(
            a.question.last().map(String::as_str),
            Some("Allow this edit?")
        );
        assert_eq!(a.keys(Choice::Deny).unwrap(), ["down", "down", "enter"]);
        // numbered, but no cursor drawn: the number itself
        let a = Ask::parse("Proceed?\n1. Yes\n2. No");
        assert_eq!(a.cursor, None);
        assert_eq!(a.keys(Choice::Approve).unwrap(), ["1"]);
        assert_eq!(a.keys(Choice::Deny).unwrap(), ["2"]);
    }

    #[test]
    fn no_menu_no_options_and_deny_falls_back_to_esc() {
        let a = Ask::parse("Claude needs your attention\n> fix the tests\n");
        assert!(a.options.is_empty());
        assert_eq!(a.keys(Choice::Approve), None);
        assert_eq!(a.keys(Choice::Deny).unwrap(), ["esc"]);
        // a numbered list in the agent's output (no cursor) before the
        // real menu: the menu with the cursor wins
        let a = Ask::parse("Plan:\n1. read the code\n2. fix it\n\nContinue?\n❯ 1. Yes\n  2. No");
        assert_eq!(labels(&a), ["Yes", "No"]);
        assert_eq!(a.question.last().map(String::as_str), Some("Continue?"));
    }

    #[test]
    fn the_same_question_read_again() {
        let one = Ask::parse("Run it?\n❯ 1. Yes\n  2. No");
        let moved = Ask::parse("Run it?\n  1. Yes\n❯ 2. No");
        let other = Ask::parse("Delete it?\n❯ 1. Yes\n  2. No");
        assert!(one.same(&moved));
        assert!(!one.same(&other));
    }
}
