//! Secrets out of the settings file: a setting can say `secret:NAME`, and the
//! value comes from the OS keystore — the macOS Keychain (`security`), Linux's
//! Secret Service (`secret-tool`) — or, where neither works (a headless
//! server, a locked keychain), `state/secrets.toml` readable only by you.
//! Exports and logs never show them.
//!
//!   aleph secret set NAME [VALUE | -]    (nothing: typed or pasted, never shown; -: stdin)
//!   aleph secret get NAME | rm NAME | ls
//!   aleph secret move                    sink URLs and tokens → secret:…
//!
//! Names Aleph knows itself ([`KNOWN_NAMES`], listed by `ls` before they're
//! stored): `openrouter` — the OpenRouter API key `openrouter.key` points at
//! (`openrouter.rs`). Every value read or stored here is masked in whatever
//! this process writes down from then on (`log::mask`).
//!
//! On macOS a value reaches `security` on its stdin (`security -i`), never on a
//! command line where `ps` would show it; it's read back from `-g`'s report,
//! which gives a value that isn't plain ASCII as hex (`-w` would print that hex
//! as though it were the value). `ALEPH_KEYCHAIN` names a keychain file to use
//! instead of the login one (for trying it against a throwaway keychain).

use crate::config::{state_dir, write_private, Config};
use crate::out::*;
use crate::proc;
use std::process::ExitCode;
use std::time::Duration;

const SERVICE: &str = "aleph";
const T: Duration = Duration::from_secs(8);

/// Settings that are secret by nature (their values move with `secret move`).
pub const SECRET_KEYS: &[&str] = &[
    "notify.ntfy",
    "notify.slack",
    "notify.webhook",
    "openrouter.key",
];

/// Secrets Aleph itself knows by name, and the setting that points at each
/// by default: `aleph secret ls` lists them even before they're stored.
pub const KNOWN_NAMES: &[(&str, &str, &str)] = &[(
    "openrouter",
    "openrouter.key",
    "OpenRouter API key: OPENROUTER_API_KEY in agent sessions, spend in Usage",
)];

/// The secret's name for a setting's value (`secret move`, setup):
/// `openrouter.key` → `openrouter`, else the setting's own name.
pub fn name_for(key: &str) -> &str {
    KNOWN_NAMES
        .iter()
        .find(|(_, k, _)| *k == key)
        .map_or(key, |(n, _, _)| n)
}

fn file() -> std::path::PathBuf {
    state_dir().join("secrets.toml")
}

fn file_get(name: &str) -> Option<String> {
    let t = std::fs::read_to_string(file()).ok()?;
    let doc: toml_edit::DocumentMut = t.parse().ok()?;
    doc.get(name)?.as_str().map(String::from)
}

fn file_set(name: &str, v: Option<&str>) -> Result<(), String> {
    let t = std::fs::read_to_string(file()).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = t.parse().unwrap_or_default();
    match v {
        Some(v) => {
            doc.insert(name, toml_edit::value(v));
        }
        None => {
            doc.remove(name);
        }
    }
    write_private(&file(), doc.to_string().as_bytes()).map_err(|e| e.to_string())
}

fn backend() -> &'static str {
    if cfg!(test) || crate::brand::env_flag("SECRETS_FILE") {
        "file"
    } else if cfg!(target_os = "macos") && proc::which("security").is_some() {
        "keychain"
    } else if proc::which("secret-tool").is_some()
        && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
    {
        "secret-tool"
    } else {
        "file"
    }
}

/// Where `set` puts secrets on this computer, in words.
pub fn store_name() -> &'static str {
    match backend() {
        "keychain" => "the macOS Keychain",
        "secret-tool" => "the Secret Service",
        _ => "a file only you can read (state/secrets.toml)",
    }
}

/// The keychain file to use instead of the login one (`ALEPH_KEYCHAIN`).
fn keychain() -> Option<String> {
    std::env::var("ALEPH_KEYCHAIN")
        .ok()
        .filter(|s| !s.is_empty())
}

/// One argument on a `security -i` line: in double quotes, `"` and `\`
/// escaped. None for what a line can't carry (a line break, a NUL).
fn kc_quote(v: &str) -> Option<String> {
    if v.contains(['\n', '\r', '\0']) {
        return None;
    }
    let mut q = String::with_capacity(v.len() + 2);
    q.push('"');
    for c in v.chars() {
        if c == '"' || c == '\\' {
            q.push('\\');
        }
        q.push(c);
    }
    q.push('"');
    Some(q)
}

/// The `security -i` line that stores `value` as NAME (in `keychain`, else
/// the login keychain).
fn kc_add_line(name: &str, value: &str, keychain: Option<&str>) -> Option<String> {
    let mut l = format!(
        "add-generic-password -U -a {SERVICE} -s {} -l {} -w {}",
        kc_quote(&format!("{SERVICE}:{name}"))?,
        kc_quote(&format!("Aleph: {name}"))?,
        kc_quote(value)?,
    );
    if let Some(k) = keychain {
        l.push(' ');
        l.push_str(&kc_quote(k)?);
    }
    l.push('\n');
    Some(l)
}

/// The value in `find-generic-password -g`'s report: `password: "plain"`, or
/// `password: 0xC3BC…  "\303\274…"` when it isn't plain ASCII.
fn kc_password(report: &str) -> Option<String> {
    let line = report.lines().find_map(|l| l.strip_prefix("password: "))?;
    if let Some(hex) = line.strip_prefix("0x") {
        let hex = hex.split_whitespace().next()?;
        if hex.len() % 2 != 0 {
            return None;
        }
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .ok()?;
        return String::from_utf8(bytes).ok();
    }
    Some(line.strip_prefix('"')?.strip_suffix('"')?.to_string())
}

/// `security` with the item's account and service, and the keychain when one's named.
fn kc_args(op: &str, name: &str, more: &[&str]) -> Vec<String> {
    let mut a: Vec<String> = [op, "-a", SERVICE, "-s", &format!("{SERVICE}:{name}")]
        .iter()
        .chain(more)
        .map(|s| s.to_string())
        .collect();
    a.extend(keychain());
    a
}

/// Look a secret up (keystore first, then the private file).
pub fn get(name: &str) -> Option<String> {
    let v = match backend() {
        "keychain" => {
            // -g: the attributes on stdout, the value on stderr
            let o = proc::cap(
                "security",
                &kc_args("find-generic-password", name, &["-g"]),
                T,
            );
            o.ok().then(|| kc_password(&o.stderr)).flatten()
        }
        "secret-tool" => {
            let o = proc::cap(
                "secret-tool",
                &["lookup", "service", SERVICE, "name", name],
                T,
            );
            o.ok().then(|| o.text().trim_end_matches('\n').to_string())
        }
        _ => None,
    };
    let v = v.filter(|s| !s.is_empty()).or_else(|| file_get(name));
    // from now on this process masks it wherever text is written down
    if let Some(v) = &v {
        crate::log::mask(v);
    }
    v
}

/// Store a secret where `backend` says; the private file is the fallback.
pub fn set(name: &str, value: &str) -> Result<&'static str, String> {
    // masked before anything runs (the debug log describes `security`'s argv)
    crate::log::mask(value);
    match backend() {
        "keychain" => {
            // the value goes in on stdin, never on `security`'s command line;
            // -U updates an existing item
            let Some(line) = kc_add_line(name, value, keychain().as_deref()) else {
                return Err("a secret can't hold a line break".into());
            };
            let mut c = std::process::Command::new("security");
            c.arg("-i");
            if proc::run(&mut c, T, Some(line.as_bytes())).is_ok_and(|o| o.ok()) {
                return Ok("the macOS Keychain");
            }
        }
        "secret-tool" => {
            let mut c = std::process::Command::new("secret-tool");
            c.args([
                "store",
                "--label",
                &format!("Aleph: {name}"),
                "service",
                SERVICE,
                "name",
                name,
            ]);
            if proc::run(&mut c, T, Some(value.as_bytes())).is_ok_and(|o| o.ok()) {
                return Ok("the Secret Service (secret-tool)");
            }
        }
        _ => {}
    }
    file_set(name, Some(value)).map(|_| "a file only you can read (state/secrets.toml)")
}

pub fn remove(name: &str) {
    match backend() {
        "keychain" => {
            let _ = proc::cap(
                "security",
                &kc_args("delete-generic-password", name, &[]),
                T,
            );
        }
        "secret-tool" => {
            let _ = proc::cap(
                "secret-tool",
                &["clear", "service", SERVICE, "name", name],
                T,
            );
        }
        _ => {}
    }
    let _ = file_set(name, None);
}

/// `secret:NAME` → its value (other values pass through). Cached per process.
pub fn resolve(v: &str) -> Option<String> {
    let Some(name) = v.strip_prefix("secret:") else {
        return Some(v.to_string());
    };
    static C: std::sync::Mutex<Vec<(String, Option<String>)>> = std::sync::Mutex::new(Vec::new());
    let mut c = C.lock().unwrap();
    if let Some((_, val)) = c.iter().find(|(n, _)| n == name) {
        return val.clone();
    }
    let val = get(name);
    if val.is_none() {
        crate::log::warn(
            "secrets",
            &format!("secret:{name} isn't stored (aleph secret set {name})"),
        );
    }
    c.push((name.to_string(), val.clone()));
    val
}

/// A value typed or pasted at the terminal, never echoed: raw mode, enter
/// takes it, esc or ctrl+c gives up (None). Says how many characters came.
fn read_hidden(prompt: &str) -> Option<String> {
    use crossterm::event::{read, Event, KeyCode, KeyEventKind, KeyModifiers};
    use std::io::Write;
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    crossterm::terminal::enable_raw_mode().ok()?;
    let mut s = String::new();
    let done = loop {
        match read() {
            Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => match k.code {
                KeyCode::Enter => break true,
                KeyCode::Esc => break false,
                KeyCode::Char('c' | 'd') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    break false
                }
                KeyCode::Backspace => {
                    s.pop();
                }
                KeyCode::Char(c) => s.push(c),
                _ => {}
            },
            Ok(Event::Paste(p)) => s.push_str(&p),
            Ok(_) => {}
            Err(_) => break false,
        }
    };
    let _ = crossterm::terminal::disable_raw_mode();
    if done {
        eprintln!("({} characters)", s.chars().count());
        Some(s.trim().to_string())
    } else {
        eprintln!();
        None
    }
}

fn names(cfg: &Config) -> Vec<(String, String)> {
    cfg.entries()
        .into_iter()
        .filter_map(|(k, v)| v.strip_prefix("secret:").map(|n| (k, n.to_string())))
        .collect()
}

/// What `secret ls` lists (pure — tested): (setting, secret name, state).
/// The settings that say `secret:NAME`, then the secrets Aleph knows by name
/// ([`KNOWN_NAMES`]) whose setting still points there by default: `stored` /
/// `not set` for those, `stored` / `MISSING` for the rest (a setting refers
/// to it and it isn't there).
pub(crate) fn listing(
    cfg: &Config,
    have: impl Fn(&str) -> bool,
) -> Vec<(String, String, &'static str)> {
    let mut v: Vec<(String, String, &'static str)> = names(cfg)
        .into_iter()
        .map(|(k, n)| {
            let st = if have(&n) { "stored" } else { "MISSING" };
            (k, n, st)
        })
        .collect();
    for (n, k, _) in KNOWN_NAMES {
        if cfg.get_raw(k).is_some() {
            continue;
        }
        let st = if have(n) { "stored" } else { "not set" };
        v.push((k.to_string(), n.to_string(), st));
    }
    v
}

pub fn cmd(a: crate::cli::Args) -> ExitCode {
    let r = a.rest;
    let name = r.get(1).cloned();
    match (r.first().map(String::as_str), name) {
        (Some("set"), Some(n)) => {
            use std::io::IsTerminal;
            let v = match r.get(2).map(String::as_str) {
                // typed or pasted at the terminal: never on screen
                None if std::io::stdin().is_terminal() => {
                    match read_hidden(&format!(
                        "{n}'s value (hidden; enter stores it, esc cancels): "
                    )) {
                        Some(v) => v,
                        None => return fail("cancelled — nothing stored"),
                    }
                }
                Some("-") | None => {
                    let mut s = String::new();
                    let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut s);
                    s.trim_end_matches('\n').to_string()
                }
                Some(v) => {
                    warn(&format!(
                        "the value is in your shell's history now — next time run aleph secret set {n} and paste it when asked"
                    ));
                    v.to_string()
                }
            };
            if v.is_empty() {
                return fail("empty value — nothing stored");
            }
            match set(&n, &v) {
                Ok(where_) => {
                    ok(&format!("stored {n} in {where_}"));
                    let own = format!("secret:{n}");
                    match KNOWN_NAMES.iter().find(|(x, _, _)| *x == n) {
                        Some((_, k, _)) if a.cfg.get_raw(k).is_none_or(|r| r == own) => {
                            hint(&format!("{k} points at it already ({own})"))
                        }
                        Some((_, k, _)) => hint(&format!(
                            "{k} says something else — aleph settings set {k} {own}"
                        )),
                        None => hint(&format!(
                            "use it in a setting as {own}, e.g. aleph settings set notify.ntfy {own}"
                        )),
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
        (Some("get"), Some(n)) => match get(&n) {
            Some(v) => {
                println!("{v}");
                ExitCode::SUCCESS
            }
            None => fail(&format!("no secret {n}")),
        },
        (Some("rm"), Some(n)) => {
            remove(&n);
            ok(&format!("removed {n}"));
            ExitCode::SUCCESS
        }
        (Some("ls") | None, _) => {
            // the list is the command's output (stdout: it pipes); the
            // backend is said beside it
            say(ratatui::text::Line::raw(format!("backend  {}", backend())));
            for (k, n, st) in listing(&a.cfg, |n| get(n).is_some()) {
                out(ratatui::text::Line::raw(format!(
                    "  {k:<22} secret:{n:<18} {st}"
                )));
            }
            ExitCode::SUCCESS
        }
        (Some("move"), _) => {
            let mut cfg = a.cfg;
            let mut n = 0;
            for k in SECRET_KEYS {
                let Some(v) = cfg.get_raw(k) else { continue };
                if v.starts_with("secret:") {
                    continue;
                }
                let name = name_for(k);
                match set(name, &v) {
                    Ok(_) => {
                        if cfg.set(k, &format!("secret:{name}")).is_ok() {
                            n += 1;
                        }
                    }
                    Err(e) => warn(&format!("{k}: {e}")),
                }
            }
            ok(&format!(
                "{n} settings now point at the keystore (secret:…)"
            ));
            ExitCode::SUCCESS
        }
        _ => fail("usage: aleph secret [ls | set NAME [VALUE|-] | get NAME | rm NAME | move]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keychain_lines_carry_any_value_and_never_a_line_break() {
        assert_eq!(
            kc_add_line("openrouter", "sk-or-v1-abc", None).unwrap(),
            "add-generic-password -U -a aleph -s \"aleph:openrouter\" -l \"Aleph: openrouter\" -w \"sk-or-v1-abc\"\n"
        );
        // quotes and backslashes escaped, the rest as it is
        assert_eq!(kc_quote(r#"q"uo te\x"#).unwrap(), r#""q\"uo te\\x""#);
        assert_eq!(
            kc_quote("https://h/x?a=1&b=$HOME #").unwrap(),
            "\"https://h/x?a=1&b=$HOME #\""
        );
        assert_eq!(kc_quote("ünï ✓").unwrap(), "\"ünï ✓\"");
        for bad in ["a\nb", "a\rb", "a\0b"] {
            assert!(kc_quote(bad).is_none());
            assert!(kc_add_line("x", bad, None).is_none());
        }
        // a throwaway keychain goes last
        assert!(kc_add_line("x", "v", Some("/tmp/k k.keychain-db"))
            .unwrap()
            .ends_with(" -w \"v\" \"/tmp/k k.keychain-db\"\n"));
    }

    #[test]
    fn keychain_reports_read_back_plain_and_hex() {
        // what `security find-generic-password -g` writes on stderr
        assert_eq!(kc_password("password: \"c3bc\"\n").as_deref(), Some("c3bc"));
        assert_eq!(
            kc_password(
                "password: 0xC3BC6EC3AF20E29C93  \"\\303\\274n\\303\\257 \\342\\234\\223\"\n"
            )
            .as_deref(),
            Some("ünï ✓")
        );
        assert_eq!(
            kc_password("keychain: \"/x\"\npassword: 0x7122756F2074655C78  \"q\"uo te\\134x\"\n")
                .as_deref(),
            Some(r#"q"uo te\x"#)
        );
        assert_eq!(kc_password("password: \n"), None);
        assert_eq!(kc_password("nothing here"), None);
        assert_eq!(kc_password("password: 0xC3B  \"\""), None);
    }

    #[test]
    fn plain_values_pass_and_secrets_resolve() {
        assert_eq!(resolve("https://x").as_deref(), Some("https://x"));
        let name = format!("t{}", std::process::id());
        set(&name, "s3cret").unwrap();
        assert_eq!(
            resolve(&format!("secret:{name}")).as_deref(),
            Some("s3cret")
        );
        remove(&name);
        assert!(get(&name).is_none());
        // what was read is masked from then on
        assert_eq!(crate::log::redact("got s3cret here"), "got s3cret here");
        let long = format!("value-{name}-long");
        set(&name, &long).unwrap();
        assert!(!crate::log::redact(&format!("got {long}")).contains(&long));
        remove(&name);
    }

    #[test]
    fn known_names_are_listed_before_they_are_stored() {
        let cfg = Config::in_memory(&[]);
        let v = listing(&cfg, |_| false);
        assert!(v.contains(&(
            "openrouter.key".to_string(),
            "openrouter".to_string(),
            "not set"
        )));
        let v = listing(&cfg, |n| n == "openrouter");
        assert!(v
            .iter()
            .any(|(k, _, st)| k == "openrouter.key" && *st == "stored"));
        assert_eq!(name_for("openrouter.key"), "openrouter");
        assert_eq!(name_for("notify.ntfy"), "notify.ntfy");
        assert!(crate::configcmd::is_secret("openrouter.key"));
        assert!(!crate::configcmd::is_secret("openrouter.agents"));
    }
}
