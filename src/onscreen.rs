//! What a dashboard shows now, for the notifier: the sessions on its screen
//! (Focus or Tile, the one selected in Fleet, the questions on the approval
//! board) and whether its terminal has focus. A desktop notification about
//! one of them is skipped (you're looking at it), and a finished turn there
//! is marked seen (`inbox.finished = unseen`). `state/onscreen.json`,
//! written by the dashboard every few seconds; older than 15 s it says
//! nothing (no dashboard up: nothing is on screen).

use serde_json::{json, Value};
use std::path::PathBuf;

fn file() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join(format!("ch-onscreen-{}.json", std::process::id()));
    }
    crate::config::world_state().join("onscreen.json")
}

/// The dashboard's view now: `host|name` of each session on its screen.
pub fn write(keys: &[String], focused: bool) {
    let v = json!({"t": crate::cache::now(), "focused": focused, "keys": keys});
    let _ = crate::config::write_atomic(&file(), v.to_string().as_bytes());
}

/// Is session `name` on `host` on a dashboard's screen now, in a terminal
/// that has focus?
pub fn showing(host: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let Some(v) = std::fs::read_to_string(file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        return false;
    };
    let fresh = v["t"]
        .as_u64()
        .is_some_and(|t| crate::cache::now().saturating_sub(t) <= 15);
    let k = format!("{host}|{name}");
    fresh
        && v["focused"].as_bool().unwrap_or(true)
        && v["keys"]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(k.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_screen_only_while_in_front_and_fresh() {
        write(&["b31|ignite".into()], true);
        assert!(showing("b31", "ignite"));
        assert!(!showing("b31", "temper") && !showing("gpu2", "ignite"));
        assert!(!showing("b31", ""));
        // the terminal in the background: not seen
        write(&["b31|ignite".into()], false);
        assert!(!showing("b31", "ignite"));
        let _ = std::fs::remove_file(file());
        assert!(!showing("b31", "ignite"));
    }
}
