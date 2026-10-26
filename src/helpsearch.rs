//! Finding a key or a command by what it does (the `?` overlay's search,
//! `aleph help <words>`): every word typed must match the entry's key
//! exactly, start one of its words — synonyms count, so "max" finds zoom
//! (maximise) and "kill" finds close — or fuzzy-match its description.

/// Words the descriptions use, and what people type for them.
const SYNONYMS: &[(&str, &[&str])] = &[
    (
        "zoom",
        &[
            "maximise",
            "maximize",
            "fullscreen",
            "enlarge",
            "expand",
            "max",
            "bigger",
        ],
    ),
    ("fullscreen", &["maximise", "maximize", "zoom"]),
    (
        "close",
        &["kill", "quit", "exit", "stop", "end", "terminate", "remove"],
    ),
    (
        "new",
        &["start", "launch", "create", "spawn", "run", "open"],
    ),
    ("start", &["launch", "create", "spawn", "run", "new"]),
    (
        "approve",
        &["yes", "accept", "allow", "confirm", "ok", "permit"],
    ),
    ("deny", &["no", "reject", "refuse"]),
    ("attach", &["open", "connect", "join", "enter"]),
    (
        "send",
        &["message", "prompt", "reply", "type", "tell", "write"],
    ),
    ("prompt", &["message", "send", "tell", "write"]),
    ("reply", &["answer", "respond", "message"]),
    ("archive", &["dismiss", "hide", "clear", "remove"]),
    ("dismiss", &["clear", "archive", "hide", "purge"]),
    ("delete", &["purge", "remove", "erase", "forget", "clear"]),
    (
        "split",
        &["divide", "tile", "side", "pane", "vertical", "horizontal"],
    ),
    ("queue", &["later", "schedule", "defer", "when"]),
    (
        "undo",
        &["keep", "cancel", "revert", "restore", "back", "oops"],
    ),
    (
        "mark",
        &["select", "multi", "batch", "bulk", "several", "pick"],
    ),
    (
        "shell",
        &["terminal", "bash", "zsh", "command", "console", "cli"],
    ),
    ("rename", &["name", "label", "title"]),
    ("search", &["find", "filter", "look", "grep"]),
    ("filter", &["search", "narrow", "find"]),
    ("copy", &["clipboard", "yank"]),
    ("refresh", &["reload", "reconnect", "update", "poll"]),
    (
        "resize",
        &[
            "size", "grow", "shrink", "width", "height", "bigger", "smaller",
        ],
    ),
    ("mute", &["silence", "quiet", "notifications", "snooze"]),
    ("note", &["tag", "comment", "annotate", "memo"]),
    ("preset", &["template", "recipe", "workspace", "save"]),
    ("browse", &["folders", "files", "directory", "dir", "cd"]),
    ("tile", &["grid", "wall", "side", "layout"]),
    ("broadcast", &["all", "everyone", "every"]),
    ("herdr", &["server", "heal", "restart"]),
    ("palette", &["commands", "jump", "goto", "everything"]),
    ("keys", &["shortcuts", "keystrokes", "bindings", "press"]),
    ("machine", &["host", "server", "box", "computer"]),
    ("session", &["agent", "pane", "tab"]),
    ("move", &["navigate", "go", "focus"]),
    ("read", &["seen", "unread"]),
    (
        "usage",
        &[
            "tokens", "cost", "spend", "billing", "money", "limits", "quota",
        ],
    ),
    (
        "doctor",
        &["health", "check", "diagnose", "problems", "fix"],
    ),
    ("settings", &["config", "preferences", "options"]),
    ("group", &["sort", "organise", "organize", "arrange"]),
    ("layout", &["arrange", "view", "grid"]),
];

fn tokens(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
}

/// The words an entry answers to: its own, then their synonyms.
pub fn words(text: &str, extra: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = tokens(text).collect();
    v.extend(extra.iter().flat_map(|e| tokens(e)));
    let own = v.clone();
    for (w, syn) in SYNONYMS {
        if own.iter().any(|o| o == w) {
            v.extend(syn.iter().map(|s| s.to_string()));
        }
    }
    v.sort();
    v.dedup();
    v
}

/// How well `query` finds an entry with these `keys` and description
/// (`extra`: more words it answers to); None when some word doesn't match.
/// A key typed exactly wins, then words that start with what was typed
/// (shorter words first: "max" → "max" over "maximise"), then fuzzy.
pub fn rank(query: &str, keys: &[&str], text: &str, extra: &[&str]) -> Option<i32> {
    let q: Vec<&str> = query.split_whitespace().collect();
    if q.is_empty() {
        return Some(0);
    }
    // its own words count a little more than their synonyms
    let own: Vec<String> = tokens(text)
        .chain(extra.iter().flat_map(|e| tokens(e)))
        .collect();
    let ws = words(text, extra);
    let lower = text.to_lowercase();
    let mut total = 0;
    for w in q {
        let lw = w.to_lowercase();
        if keys.contains(&w) {
            total += 1000;
            continue;
        }
        let prefix = |v: &[String], base: i32| {
            v.iter()
                .filter(|x| x.starts_with(&lw))
                .map(|x| base - (x.len() - lw.len()).min(100) as i32)
                .max()
        };
        if let Some(best) = prefix(&own, 450).or_else(|| prefix(&ws, 400)) {
            total += best;
            continue;
        }
        match crate::fuzzy::score(&lw, &lower) {
            Some((s, _)) if lw.len() >= 2 => total += s.min(250),
            _ => return None,
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_by_what_people_type() {
        let zoom = rank("max", &["z"], "zoom the pane", &[]);
        let mark = rank("max", &["space"], "mark a session (or fold a machine)", &[]);
        assert!(zoom.is_some() && zoom > mark, "{zoom:?} {mark:?}");
        assert!(rank("kill", &["X"], "close the session", &[]).is_some());
        assert!(rank("z", &["z"], "zoom the pane", &[]) > rank("z", &["s"], "a size", &[]));
        assert!(rank("terminal", &["S"], "new shell", &[]).is_some());
        assert!(rank("split pane", &["v"], "split side by side", &[]).is_some());
        assert_eq!(rank("qqq", &["x"], "archive", &[]), None);
        assert_eq!(rank("", &["x"], "archive", &[]), Some(0));
    }
}
