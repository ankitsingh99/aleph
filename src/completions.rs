//! `aleph completions zsh|bash|fish [--name al]` — shell completion.
//!
//! The scripts are thin: the word lists come from `aleph __complete …`
//! (commands, hosts, sessions, presets, settings), which is fast and offline,
//! plus the static flags/sub-commands below. Load with
//!   zsh:  source <(aleph completions zsh)       (after compinit)
//!   bash: source <(aleph completions bash)
//!   fish: aleph completions fish | source

use crate::cli::Args;
use crate::out::*;
use std::process::ExitCode;

/// Static words per command (flags and sub-commands).
const WORDS: &[(&str, &str)] = &[
    (
        "new",
        "--dir --name --agent --openrouter --worktree --tile -m --prompt",
    ),
    ("ls", "--json --plain --watch"),
    ("status", "--short --json --watch --live"),
    ("watch", "--once --interval --install --uninstall --dry-run"),
    ("close", "--yes --now --undo --pending --worktree --json"),
    ("history", "-n --json"),
    ("recent", "--json"),
    ("preset", "ls add rm show --json"),
    ("host", "list pick set add key include exclude color --json"),
    ("doctor", "--json"),
    (
        "agents",
        "install setup --host --local --all --json --refresh --yes --dry-run",
    ),
    ("services", "--json --host"),
    (
        "usage",
        "today yesterday 7d 30d all sources script sync on off prune --by --host --agent --model --since --json --csv --no-sync --full --until-done --keep --dry-run",
    ),
    ("theme", "show herdr machines --apply --json"),
    ("shortcuts", "install ls --dry-run --json"),
    ("completions", "zsh bash fish --name"),
    ("self-update", "--check --json"),
];
/// Commands whose first argument is a session name.
const ONE_SESSION: &[&str] = &["resume", "send", "keys", "rename", "close"];
/// Commands that take any number of session names.
const MANY_SESSIONS: &[&str] = &["tile", "wall"];
/// `--agent` values: every registry agent, then a plain shell.
fn agent_words() -> String {
    let mut v: Vec<&str> = crate::agents::IDS.to_vec();
    v.push(crate::agents::SHELL);
    v.join(" ")
}
/// `new` flags that take a value we can't complete.
const NEW_VALUE: &str = "--dir --name --tile -m --prompt";

pub fn cmd(a: Args) -> ExitCode {
    let mut name = crate::brand::BIN.to_string();
    let mut shell = None;
    let mut i = 0;
    while i < a.rest.len() {
        match a.rest[i].as_str() {
            "--name" => {
                i += 1;
                match a.rest.get(i) {
                    Some(n) => name = n.clone(),
                    None => return fail("--name needs the command name to complete"),
                }
            }
            s if s.starts_with("--name=") => name = s["--name=".len()..].to_string(),
            s if !s.starts_with('-') && shell.is_none() => shell = Some(s.to_string()),
            s => return fail(&format!("unknown option {s}")),
        }
        i += 1;
    }
    if !valid_name(&name) {
        return fail(&format!("bad command name '{name}'"));
    }
    let script = match shell.as_deref() {
        Some("zsh") => zsh(&name),
        Some("bash") => bash(&name),
        Some("fish") => fish(&name),
        Some(s) => {
            err(&format!("no completions for '{s}'"));
            hint("aleph completions zsh | bash | fish");
            return ExitCode::from(2);
        }
        None => {
            err("which shell?");
            hint("zsh:  source <(aleph completions zsh)   (after compinit, e.g. in ~/.zshrc)");
            hint("bash: source <(aleph completions bash)");
            hint("fish: aleph completions fish > ~/.config/fish/completions/claude.fish");
            return ExitCode::from(2);
        }
    };
    print!("{script}");
    ExitCode::SUCCESS
}

fn valid_name(n: &str) -> bool {
    let mut c = n.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_' || x == '-')
}

/// Shell-function-safe identifier for `name`.
fn ident(name: &str) -> String {
    name.replace('-', "_")
}

/// `case` arms for the static words, in sh/bash/zsh syntax.
fn sh_arms(var: &str) -> String {
    WORDS
        .iter()
        .filter(|(c, _)| *c != "new")
        .map(|(c, w)| format!("      {c}) {var}=\"{w}\" ;;\n"))
        .collect()
}

pub fn zsh(name: &str) -> String {
    let f = format!("_{}", ident(name));
    let agents = agent_words();
    format!(
        r#"#compdef {name}
# aleph completion for zsh — source <(aleph completions zsh)
{f}() {{
  local i=2 cmd npos w
  while (( i < CURRENT )); do
    case ${{words[i]}} in
      --host|-H) (( i += 2 )) ;;
      --host=*|--debug) (( i++ )) ;;
      *) break ;;
    esac
  done
  local prev=${{words[CURRENT-1]}}
  if [[ $prev == --host || $prev == -H ]]; then
    w=$({name} __complete hosts 2>/dev/null)
  elif (( i >= CURRENT )); then
    w="$({name} __complete commands 2>/dev/null) --host --debug"
  else
    cmd=${{words[i]}}
    (( npos = CURRENT - i - 1 ))
    case $cmd in
      {one})
        (( npos == 0 )) && w=$({name} __complete sessions 2>/dev/null) ;;
      {many})
        w=$({name} __complete sessions 2>/dev/null) ;;
      new)
        case $prev in
          --agent) w="{agents}" ;;
          --openrouter) w=$({name} __complete openrouter-models 2>/dev/null) ;;
          {new_value}) return 1 ;;
          *) w="$({name} __complete presets 2>/dev/null) {new_words}" ;;
        esac ;;
      settings)
        if (( npos == 0 )); then w="list get set unset edit path"
        elif (( npos == 1 )) && [[ ${{words[i+1]}} == (get|set|unset) ]]; then w=$({name} __complete settings 2>/dev/null)
        fi ;;
      help)
        (( npos == 0 )) && w=$({name} __complete commands 2>/dev/null) ;;
{arms}    esac
  fi
  [[ -z $w ]] && return 1
  compadd -- ${{=w}}
}}
if (( $+functions[compdef] )); then
  compdef {f} {name}{also}
fi
"#,
        also = if name == crate::brand::BIN {
            String::new()
        } else {
            format!(" {}", crate::brand::BIN)
        },
        one = ONE_SESSION.join("|"),
        many = MANY_SESSIONS.join("|"),
        new_value = NEW_VALUE.replace(' ', "|"),
        new_words = words_for("new"),
        arms = sh_arms("w"),
    )
}

pub fn bash(name: &str) -> String {
    let f = format!("_{}", ident(name));
    let agents = agent_words();
    format!(
        r#"# aleph completion for bash — source <(aleph completions bash)
{f}() {{
  local cur=${{COMP_WORDS[COMP_CWORD]}} prev=${{COMP_WORDS[COMP_CWORD-1]}} i=1 cmd npos w=
  while (( i < COMP_CWORD )); do
    case ${{COMP_WORDS[i]}} in
      --host|-H) (( i += 2 )) ;;
      --host=*|--debug) (( i++ )) ;;
      *) break ;;
    esac
  done
  if [[ $prev == --host || $prev == -H ]]; then
    w=$({name} __complete hosts 2>/dev/null)
  elif (( i >= COMP_CWORD )); then
    w="$({name} __complete commands 2>/dev/null) --host --debug"
  else
    cmd=${{COMP_WORDS[i]}}
    npos=$(( COMP_CWORD - i - 1 ))
    case $cmd in
      {one})
        (( npos == 0 )) && w=$({name} __complete sessions 2>/dev/null) ;;
      {many})
        w=$({name} __complete sessions 2>/dev/null) ;;
      new)
        case $prev in
          --agent) w="{agents}" ;;
          --openrouter) w=$({name} __complete openrouter-models 2>/dev/null) ;;
          {new_value}) return 0 ;;
          *) w="$({name} __complete presets 2>/dev/null) {new_words}" ;;
        esac ;;
      settings)
        if (( npos == 0 )); then w="list get set unset edit path"
        elif (( npos == 1 )); then
          case ${{COMP_WORDS[i+1]}} in get|set|unset) w=$({name} __complete settings 2>/dev/null) ;; esac
        fi ;;
      help)
        (( npos == 0 )) && w=$({name} __complete commands 2>/dev/null) ;;
{arms}    esac
  fi
  COMPREPLY=($(compgen -W "$w" -- "$cur"))
}}
complete -o default -F {f} {name}{also}
"#,
        also = if name == crate::brand::BIN {
            String::new()
        } else {
            format!(" {}", crate::brand::BIN)
        },
        one = ONE_SESSION.join("|"),
        many = MANY_SESSIONS.join("|"),
        new_value = NEW_VALUE.replace(' ', "|"),
        new_words = words_for("new"),
        arms = sh_arms("w"),
    )
}

pub fn fish(name: &str) -> String {
    let f = format!("__{}_complete", ident(name));
    let agents = agent_words();
    let arms: String = WORDS
        .iter()
        .filter(|(c, _)| *c != "new")
        .map(|(c, w)| format!("        case {c}\n            printf '%s\\n' {w}\n"))
        .collect();
    format!(
        r#"# aleph completion for fish — aleph completions fish | source
function {f}
    set -l t (commandline -opc)
    set -l i 2
    while test $i -le (count $t)
        switch $t[$i]
            case --host -H
                set i (math $i + 2)
            case '--host=*' --debug
                set i (math $i + 1)
            case '*'
                break
        end
    end
    set -l prev $t[-1]
    if test (count $t) -gt 1; and contains -- $prev --host -H
        {name} __complete hosts 2>/dev/null
        return
    end
    if test $i -gt (count $t)
        {name} __complete commands 2>/dev/null
        printf '%s\n' --host --debug
        return
    end
    set -l cmd $t[$i]
    set -l npos (math (count $t) - $i)
    switch $cmd
        case {one}
            test $npos -eq 0; and {name} __complete sessions 2>/dev/null
        case {many}
            {name} __complete sessions 2>/dev/null
        case new
            switch $prev
                case --agent
                    printf '%s\n' {agents}
                case --openrouter
                    {name} __complete openrouter-models 2>/dev/null
                case {new_value}
                    return
                case '*'
                    {name} __complete presets 2>/dev/null
                    printf '%s\n' {new_words}
            end
        case settings
            if test $npos -eq 0
                printf '%s\n' list get set unset edit path
            else if test $npos -eq 1; and contains -- $t[(math $i + 1)] get set unset
                {name} __complete settings 2>/dev/null
            end
        case help
            test $npos -eq 0; and {name} __complete commands 2>/dev/null
{arms}    end
end
complete -c {name} -f -a '({f})'
"#,
        one = ONE_SESSION.join(" "),
        many = MANY_SESSIONS.join(" "),
        new_value = NEW_VALUE,
        new_words = words_for("new"),
    )
}

fn words_for(c: &str) -> &'static str {
    WORDS.iter().find(|w| w.0 == c).map_or("", |w| w.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn have(shell: &str) -> bool {
        crate::proc::which(shell).is_some()
    }

    /// A fake `aleph` answering `__complete` like the real one, so the
    /// scripts can be exercised in a real shell.
    const FAKE: &str = r#"aleph() { case "$2" in hosts) printf 'b31\ngpu2\n';; sessions) printf 'ignite\ntemper\n';; presets) echo @px;; settings) printf 'refresh\ntheme\n';; *) printf 'new\nls\nresume\nsettings\n';; esac; }
"#;

    fn bash_complete(line: &str) -> String {
        let words: Vec<&str> = line.split(' ').collect();
        let arr: Vec<String> = words.iter().map(|w| format!("'{w}'")).collect();
        let script = format!(
            "{FAKE}{}\nCOMP_WORDS=({}); COMP_CWORD={}; _aleph; printf '%s\\n' \"${{COMPREPLY[@]}}\"",
            bash("aleph"),
            arr.join(" "),
            words.len() - 1
        );
        let o = Command::new("bash")
            .args(["--norc", "-c", &script])
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    #[test]
    fn bash_completes_words() {
        if !have("bash") {
            return;
        }
        assert!(bash_complete("aleph ").contains("resume"));
        assert!(bash_complete("aleph --host ").contains("gpu2"));
        assert!(bash_complete("aleph --host b31 re").contains("resume"));
        assert_eq!(bash_complete("aleph resume ig"), "ignite\n");
        assert_eq!(bash_complete("aleph send ignite "), "\n");
        assert!(bash_complete("aleph tile ignite ").contains("temper"));
        let n = bash_complete("aleph new ");
        assert!(n.contains("@px") && n.contains("--worktree"));
        assert!(bash_complete("aleph new --agent ").contains("codex"));
        assert!(bash_complete("aleph settings set ").contains("refresh"));
        assert!(bash_complete("aleph settings ").contains("unset"));
        assert!(bash_complete("aleph theme ").contains("machines"));
        assert!(bash_complete("aleph completions ").contains("fish"));
        assert!(bash_complete("aleph --host b31 status --").contains("--short"));
    }

    #[test]
    fn scripts_parse() {
        for (sh, s) in [
            ("bash", bash("aleph")),
            ("zsh", zsh("al")),
            ("fish", fish("aleph")),
        ] {
            if !have(sh) {
                continue;
            }
            let flag = if sh == "fish" { "--no-execute" } else { "-n" };
            let o = Command::new(sh).args([flag, "-c", &s]).output().unwrap();
            assert!(
                o.status.success(),
                "{sh}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
        }
        let z = zsh("my-claude");
        assert!(z.starts_with("#compdef my-claude"));
        assert!(z.contains("compdef _my_claude my-claude aleph"));
        assert!(z.contains("my-claude __complete sessions"));
        assert!(bash("cc").contains("complete -o default -F _cc cc aleph"));
        assert!(bash("aleph").contains("complete -o default -F _aleph aleph\n"));
        assert!(fish("cc").contains("complete -c cc -f -a '(__cc_complete)'"));
        assert!(valid_name("claude") && valid_name("my-claude"));
        assert!(!valid_name("1x") && !valid_name("a b") && !valid_name("a;b") && !valid_name(""));
    }

    #[test]
    fn zsh_completes_words() {
        if !have("zsh") {
            return;
        }
        // drive the function with a stub compadd that prints its words
        let run = |line: &str| {
            let words: Vec<&str> = line.split(' ').collect();
            let arr: Vec<String> = words.iter().map(|w| format!("'{w}'")).collect();
            let script = format!(
                "{FAKE}compadd() {{ shift; print -l -- \"$@\"; }}\n{}\nwords=({}); CURRENT={}; _aleph",
                zsh("aleph"),
                arr.join(" "),
                words.len()
            );
            let o = Command::new("zsh")
                .args(["-f", "-c", &script])
                .output()
                .unwrap();
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        assert!(run("aleph ").contains("resume"));
        assert!(run("aleph -H ").contains("b31"));
        assert!(run("aleph resume ").contains("temper"));
        assert_eq!(run("aleph send ignite "), "");
        assert!(run("aleph new ").contains("@px"));
        assert!(run("aleph settings get ").contains("theme"));
    }
}
