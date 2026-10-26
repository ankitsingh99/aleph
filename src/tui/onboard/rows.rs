//! The rows a step's body is made of. Steps build them fresh every frame from
//! their state; the wizard draws them, moves the cursor over the ones you can
//! act on (found again by [`Row::ident`] after a rebuild), and handles the
//! setting rows (choices, toggles, text) itself: a change goes into the plan,
//! not the settings file. Buttons, checks, picks and matrix cells go back to
//! the step that made them. Enter acts on every kind of row it can stop at.

use crate::ui::{self, c};
use ratatui::prelude::*;

/// How a check (or a machine, a tool) stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum St {
    Ok,
    Warn,
    Bad,
    Info,
    /// being looked at right now
    Busy,
    /// not looked at yet
    Todo,
}

impl St {
    pub fn mark(self, spin: &'static str) -> (&'static str, Color) {
        match self {
            St::Ok => ("✓", c::GREEN()),
            St::Warn => ("!", c::YELLOW()),
            St::Bad => ("✗", c::RED()),
            St::Info => ("·", c::MUTED()),
            St::Busy => (spin, c::ACCENT()),
            St::Todo => ("○", c::FAINT()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// a section title inside the step
    Head,
    /// text, not selectable
    Info,
    /// a setting with a few values: ←→ (or enter) change it
    Choice {
        key: String,
        opts: Vec<(String, String)>,
        cur: String,
        pending: bool,
    },
    /// an on/off setting: enter (or space) flips it
    Toggle {
        key: String,
        on: bool,
        pending: bool,
    },
    /// a text setting: enter edits it
    Input {
        key: String,
        cur: String,
        hint: String,
        secret: bool,
        pending: bool,
    },
    /// enter runs it
    Button { id: String },
    /// a check; enter carries out its fix when it has one (one without a
    /// fix is only read: the cursor passes it by)
    Check {
        st: St,
        id: String,
        fix: Option<String>,
    },
    /// one of several to pick (enter or space)
    Pick { id: String, on: bool },
    /// a row of on/off cells (machines × agents): ←→ moves, enter flips
    Matrix {
        id: String,
        cols: Vec<String>,
        on: Vec<bool>,
    },
    /// a choice that isn't a setting (the step keeps it): ←→ (or enter)
    /// change it
    Local {
        id: String,
        opts: Vec<(String, String)>,
        cur: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: Kind,
    pub label: String,
    /// shown after the value (faint unless styled)
    pub note: Vec<Span<'static>>,
    /// what `?` says about it
    pub why: String,
}

impl Row {
    fn new(kind: Kind, label: impl Into<String>) -> Row {
        Row {
            kind,
            label: label.into(),
            note: vec![],
            why: String::new(),
        }
    }
    pub fn head(t: impl Into<String>) -> Row {
        Row::new(Kind::Head, t)
    }
    pub fn info(t: impl Into<String>) -> Row {
        Row::new(Kind::Info, t)
    }
    /// An info row made of styled spans (the label stays empty).
    pub fn line(spans: Vec<Span<'static>>) -> Row {
        let mut r = Row::new(Kind::Info, "");
        r.note = spans;
        r
    }
    pub fn blank() -> Row {
        Row::new(Kind::Info, "")
    }
    pub fn button(id: impl Into<String>, label: impl Into<String>) -> Row {
        Row::new(Kind::Button { id: id.into() }, label)
    }
    pub fn check(st: St, id: impl Into<String>, label: impl Into<String>, text: String) -> Row {
        let mut r = Row::new(
            Kind::Check {
                st,
                id: id.into(),
                fix: None,
            },
            label,
        );
        r.note = vec![Span::raw(text)];
        r
    }
    pub fn pick(id: impl Into<String>, label: impl Into<String>, on: bool) -> Row {
        Row::new(Kind::Pick { id: id.into(), on }, label)
    }
    pub fn matrix(
        id: impl Into<String>,
        label: impl Into<String>,
        cols: Vec<String>,
        on: Vec<bool>,
    ) -> Row {
        Row::new(
            Kind::Matrix {
                id: id.into(),
                cols,
                on,
            },
            label,
        )
    }
    pub fn local(
        id: impl Into<String>,
        label: impl Into<String>,
        opts: Vec<(String, String)>,
        cur: usize,
    ) -> Row {
        Row::new(
            Kind::Local {
                id: id.into(),
                opts,
                cur,
            },
            label,
        )
    }
    pub fn choice(
        key: impl Into<String>,
        label: impl Into<String>,
        opts: Vec<(String, String)>,
        cur: String,
        pending: bool,
    ) -> Row {
        Row::new(
            Kind::Choice {
                key: key.into(),
                opts,
                cur,
                pending,
            },
            label,
        )
    }
    pub fn toggle(
        key: impl Into<String>,
        label: impl Into<String>,
        on: bool,
        pending: bool,
    ) -> Row {
        Row::new(
            Kind::Toggle {
                key: key.into(),
                on,
                pending,
            },
            label,
        )
    }
    pub fn input(
        key: impl Into<String>,
        label: impl Into<String>,
        cur: String,
        hint: impl Into<String>,
        pending: bool,
    ) -> Row {
        let key = key.into();
        let secret = crate::configcmd::is_secret(&key);
        Row::new(
            Kind::Input {
                key,
                cur,
                hint: hint.into(),
                secret,
                pending,
            },
            label,
        )
    }

    /// Builder: an input whose secret is shown (you asked to see it).
    pub fn revealed(mut self) -> Row {
        if let Kind::Input { secret, .. } = &mut self.kind {
            *secret = false;
        }
        self
    }

    /// Builder: the `?` text.
    pub fn why(mut self, w: impl Into<String>) -> Row {
        self.why = w.into();
        self
    }
    /// Builder: a note after the value.
    pub fn note(mut self, t: impl Into<String>) -> Row {
        let t = t.into();
        if !t.is_empty() {
            self.note.push(Span::styled(t, ui::faint()));
        }
        self
    }
    pub fn spans(mut self, s: Vec<Span<'static>>) -> Row {
        self.note.extend(s);
        self
    }
    /// Builder (checks): what enter does about it.
    pub fn fix(mut self, label: impl Into<String>) -> Row {
        if let Kind::Check { fix, .. } = &mut self.kind {
            *fix = Some(label.into());
        }
        self
    }

    /// Can the cursor stop here? Not on text, and not on a check there's
    /// nothing to do about.
    pub fn selectable(&self) -> bool {
        match &self.kind {
            Kind::Head | Kind::Info => false,
            Kind::Check { fix, .. } => fix.is_some(),
            _ => true,
        }
    }

    /// What the row is, whatever its label says now: its setting or its id
    /// (text: its words). The cursor finds its row again by this after the
    /// rows are rebuilt.
    pub fn ident(&self) -> String {
        match &self.kind {
            Kind::Choice { key, .. } | Kind::Toggle { key, .. } | Kind::Input { key, .. } => {
                format!("set:{key}")
            }
            Kind::Button { id }
            | Kind::Check { id, .. }
            | Kind::Pick { id, .. }
            | Kind::Matrix { id, .. }
            | Kind::Local { id, .. } => format!("id:{id}"),
            Kind::Head | Kind::Info => format!("text:{}", self.label),
        }
    }

    /// The rows of one matrix share this (`remote:b31`, `remote:gpu2` →
    /// `remote`); None for any other row.
    pub fn matrix_of(&self) -> Option<&str> {
        match &self.kind {
            Kind::Matrix { id, .. } => Some(id.split_once(':').map_or(id.as_str(), |x| x.0)),
            _ => None,
        }
    }

    /// What enter does on this row (the footer's word).
    pub fn enter_word(&self) -> &'static str {
        match &self.kind {
            Kind::Input { .. } => "edit",
            Kind::Button { .. } => "run",
            Kind::Check { fix: Some(_), .. } => "fix",
            Kind::Choice { .. } | Kind::Local { .. } => "change",
            Kind::Toggle { .. } | Kind::Pick { .. } | Kind::Matrix { .. } => "toggle",
            _ => "next",
        }
    }
}

/// The value column of a choice: every option when they fit, else `‹ v ›`.
fn choice_spans(opts: &[(String, String)], cur: &str, sel: bool, w: usize) -> Vec<Span<'static>> {
    let all: usize = opts.iter().map(|(_, l)| ui::width(l) + 3).sum();
    if all <= w {
        let mut v = Vec::new();
        for (i, (val, l)) in opts.iter().enumerate() {
            if i > 0 {
                v.push(Span::styled(" ", ui::faint()));
            }
            if val == cur {
                v.push(Span::styled(
                    format!("[{l}]"),
                    ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD),
                ));
            } else {
                v.push(Span::styled(format!(" {l} "), ui::muted()));
            }
        }
        return v;
    }
    let l = opts
        .iter()
        .find(|(v, _)| v == cur)
        .map(|x| x.1.clone())
        .unwrap_or_else(|| cur.to_string());
    vec![
        Span::styled(if sel { "‹ " } else { "  " }, ui::faint()),
        Span::styled(l, ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD)),
        Span::styled(if sel { " ›" } else { "" }, ui::faint()),
    ]
}

/// A secret as it's shown: its first few characters, then dots.
pub fn masked(v: &str) -> String {
    if v.starts_with("secret:") || v.is_empty() {
        return v.to_string();
    }
    let keep: String = v.chars().take(12).collect();
    if v.chars().count() <= 12 {
        "•".repeat(v.chars().count().min(8))
    } else {
        format!("{keep}••••")
    }
}

const PENDING: &str = " •";

/// One row as a line, `w` columns wide. `cell`: the matrix cell under the
/// cursor (when this row is selected). `lw`: the label column.
pub fn draw(
    r: &Row,
    sel: bool,
    cell: usize,
    w: usize,
    lw: usize,
    spin: &'static str,
) -> Line<'static> {
    let bar = if sel {
        Span::styled("┃ ", ui::fg(c::ACCENT()))
    } else {
        Span::raw("  ")
    };
    let label_style = if sel { ui::bold() } else { Style::new() };
    let pend = |p: bool| {
        if p {
            Span::styled(PENDING, ui::fg(c::ACCENT()))
        } else {
            Span::raw("")
        }
    };
    let mut v: Vec<Span<'static>> = vec![bar];
    let rest = w.saturating_sub(2 + lw + 2);
    match &r.kind {
        Kind::Head => {
            let t = format!("{} ", r.label);
            // every rule ends in the same column (at most 72 wide)
            let fill = w.saturating_sub(3).min(72).saturating_sub(ui::width(&t));
            return Line::from(vec![
                Span::raw("  "),
                Span::styled(t, ui::bold().fg(c::TEXT())),
                Span::styled("─".repeat(fill), ui::fg(c::LINE())),
            ]);
        }
        Kind::Info => {
            let mut l = vec![Span::raw("  ")];
            if !r.label.is_empty() {
                l.push(Span::styled(r.label.clone(), ui::muted()));
                if !r.note.is_empty() {
                    l.push(Span::raw(" "));
                }
            }
            l.extend(r.note.iter().cloned());
            return Line::from(l);
        }
        Kind::Choice {
            opts, cur, pending, ..
        } => {
            v.push(Span::styled(
                ui::pad(&ui::trunc(&r.label, lw), lw),
                label_style,
            ));
            v.push(Span::raw("  "));
            v.extend(choice_spans(opts, cur, sel, rest.saturating_sub(2)));
            v.push(pend(*pending));
        }
        Kind::Local { opts, cur, .. } => {
            let curv = opts.get(*cur).map(|x| x.0.clone()).unwrap_or_default();
            v.push(Span::styled(
                ui::pad(&ui::trunc(&r.label, lw), lw),
                label_style,
            ));
            v.push(Span::raw("  "));
            v.extend(choice_spans(opts, &curv, sel, rest.saturating_sub(2)));
        }
        Kind::Toggle { on, pending, .. } => {
            v.push(Span::styled(
                ui::pad(&ui::trunc(&r.label, lw), lw),
                label_style,
            ));
            v.push(Span::raw("  "));
            v.push(if *on {
                Span::styled("☑ on", ui::fg(c::GREEN()).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("☐ off", ui::muted())
            });
            v.push(pend(*pending));
        }
        Kind::Input {
            cur,
            hint,
            secret,
            pending,
            ..
        } => {
            v.push(Span::styled(
                ui::pad(&ui::trunc(&r.label, lw), lw),
                label_style,
            ));
            v.push(Span::raw("  "));
            if cur.is_empty() {
                v.push(Span::styled(ui::trunc(hint, rest), ui::faint()));
            } else {
                let shown = if *secret { masked(cur) } else { cur.clone() };
                v.push(Span::styled(
                    ui::trunc(&shown, rest),
                    Style::new().fg(c::TEXT()),
                ));
            }
            v.push(pend(*pending));
        }
        Kind::Button { .. } => {
            let st = if sel && c::mono() {
                Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else if sel {
                // the accent as the button, in the theme's ink
                Style::new()
                    .fg(c::BASE())
                    .bg(c::ACCENT())
                    .add_modifier(Modifier::BOLD)
            } else {
                ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD)
            };
            v.push(Span::styled(format!(" {} ", r.label), st));
        }
        Kind::Check { st, fix, .. } => {
            let (m, col) = st.mark(spin);
            v.push(Span::styled(
                format!("{m} "),
                ui::fg(col).add_modifier(Modifier::BOLD),
            ));
            v.push(Span::styled(
                ui::pad(
                    &ui::trunc(&r.label, lw.saturating_sub(2)),
                    lw.saturating_sub(2),
                ),
                label_style,
            ));
            v.push(Span::raw("  "));
            v.extend(r.note.iter().cloned());
            if let (Some(fx), true) = (fix, sel) {
                v.push(Span::styled(format!("  ↵ {fx}"), ui::fg(c::ACCENT())));
            }
            return fit(v, w);
        }
        Kind::Pick { on, .. } => {
            v.push(if *on {
                Span::styled("◉ ", ui::fg(c::ACCENT()).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("○ ", ui::faint())
            });
            v.push(Span::styled(
                ui::pad(
                    &ui::trunc(&r.label, lw.saturating_sub(2)),
                    lw.saturating_sub(2),
                ),
                label_style,
            ));
            v.push(Span::raw("  "));
        }
        Kind::Matrix { cols, on, .. } => {
            v.push(Span::styled(
                ui::pad(&ui::trunc(&r.label, lw), lw),
                label_style,
            ));
            v.push(Span::raw("  "));
            for (i, (name, o)) in cols.iter().zip(on).enumerate() {
                let here = sel && i == cell;
                let col = if *o { c::GREEN() } else { c::FAINT() };
                let st = match (here, c::mono()) {
                    (false, _) => ui::fg(col),
                    (true, true) => Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD),
                    // the cell's colour as its background, in the theme's ink
                    (true, false) => Style::new()
                        .fg(c::BASE())
                        .bg(col)
                        .add_modifier(Modifier::BOLD),
                };
                v.push(Span::styled(
                    format!("{} {name}", if *o { "☑" } else { "☐" }),
                    st,
                ));
                v.push(Span::raw("  "));
            }
        }
    }
    // a gap between the value and what's said about it
    let joined = v.last().is_some_and(|s| s.content.ends_with("  "));
    if !joined && r.note.first().is_some_and(|s| !s.content.starts_with(' ')) {
        v.push(Span::raw("  "));
    }
    v.extend(r.note.iter().cloned().map(|s| {
        if s.style == Style::default() {
            Span::styled(s.content, ui::faint())
        } else {
            s
        }
    }));
    fit(v, w)
}

/// Cut a line's spans to `w` columns.
pub fn fit(spans: Vec<Span<'static>>, w: usize) -> Line<'static> {
    let mut out = Vec::new();
    let mut used = 0;
    for s in spans {
        let sw = ui::width(&s.content);
        if used + sw <= w {
            used += sw;
            out.push(s);
        } else {
            let room = w.saturating_sub(used);
            if room > 1 {
                out.push(Span::styled(ui::trunc(&s.content, room), s.style));
            }
            break;
        }
    }
    Line::from(out)
}

/// The label column for a set of rows: wide enough for most labels, never
/// more than a third of the line. (Buttons and text don't use it.)
pub fn label_width(rows: &[Row], w: usize) -> usize {
    let mut v: Vec<usize> = rows
        .iter()
        .filter(|r| !matches!(r.kind, Kind::Head | Kind::Info | Kind::Button { .. }))
        .map(|r| ui::width(&r.label) + 2)
        .collect();
    v.sort_unstable();
    let p = v.get(v.len() * 9 / 10).copied().unwrap_or(12);
    p.clamp(8, (w / 3).max(8)).min(26)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn rows_draw_their_values() {
        let opts = vec![
            ("auto".to_string(), "auto".to_string()),
            ("dark".to_string(), "dark".to_string()),
        ];
        let r = Row::choice("theme", "theme", opts.clone(), "dark".into(), true);
        let t = text(&draw(&r, true, 0, 60, 10, "·"));
        assert!(t.starts_with("┃ theme"), "{t}");
        assert!(t.contains("[dark]") && t.contains(" auto "), "{t}");
        assert!(t.ends_with(" •"), "{t}");
        // too narrow for every option: just the current one
        let t = text(&draw(&r, true, 0, 22, 10, "·"));
        assert!(t.contains("‹ dark ›"), "{t}");
        let s = Row::input(
            "notify.ntfy",
            "ntfy",
            "https://ntfy.sh/my-secret-topic".into(),
            "",
            false,
        );
        let t = text(&draw(&s, false, 0, 80, 10, "·"));
        assert!(!t.contains("secret-topic"), "{t}");
        let m = Row::matrix(
            "m",
            "b31",
            vec!["claude".into(), "codex".into()],
            vec![true, false],
        );
        let t = text(&draw(&m, true, 1, 80, 10, "·"));
        assert!(t.contains("☑ claude") && t.contains("☐ codex"), "{t}");
        assert!(!Row::head("x").selectable() && Row::button("b", "Go").selectable());
        assert_eq!(Row::button("b", "Go").enter_word(), "run");
        // a check is a stop only when there's something to do about it
        let ck = Row::check(St::Ok, "git", "git", "2.4".into());
        assert!(!ck.selectable());
        assert!(ck.fix("install it").selectable());
        // enter changes a value row
        assert_eq!(r.enter_word(), "change");
        assert_eq!(
            Row::toggle("mouse", "mouse", true, false).enter_word(),
            "toggle"
        );
        assert_eq!((m.matrix_of(), m.ident().as_str()), (Some("m"), "id:m"));
        assert_eq!(r.ident(), "set:theme");
    }

    #[test]
    fn lines_never_overflow() {
        let r = Row::check(St::Warn, "x", "a very long label indeed", "w".repeat(200)).fix("do it");
        for w in [0, 1, 5, 20, 80] {
            let l = draw(&r, true, 0, w, 12, "·");
            assert!(
                ui::line_width(&l) <= w.max(2),
                "{w}: {}",
                ui::line_width(&l)
            );
        }
    }
}
