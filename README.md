# ℵ Aleph

One dashboard and CLI for coding agents — Claude Code, Codex, OpenCode, Pi, Gemini, Cursor, Copilot and every
other agent [herdr](https://herdr.dev) supports — running in herdr across your machines.

`aleph` (or `al`) opens the dashboard: every machine, every session on it (in herdr or outside it), what
needs you, live machine stats and token usage. From there you start, watch, answer, tile and close agents,
and a background daemon keeps watching — answering what you've allowed, queuing work, and telling your
phone, desktop or Raycast when something needs you.

It's one Rust binary with no runtime dependencies. Nothing is installed on the remote machines: everything
there is a plain `sh` script over ssh. (Formerly *claude-herdr*; the old name and its settings still work.)

## Requirements

* **Here (macOS or Linux):** Rust ≥ 1.89 to build ([rustup.rs](https://rustup.rs)), herdr, OpenSSH.
* **Each machine:** POSIX `sh`, herdr and at least one agent. `nvidia-smi` adds GPU stats.
* **ssh key login** to each machine (`aleph host key` sets it up once; your password only ever goes to ssh).

`aleph doctor` checks all of this, on every machine, and offers fixes.

### Adding a machine

`aleph host add box3 me@box3.lan` (or `+` in the dashboard, or `aleph host include` for a Host already in
`~/.ssh/config`) looks at the machine first. One with no herdr or no agent gets its setup in a **new
terminal tab** (Warp, iTerm2, Terminal, Ghostty, kitty, WezTerm), and Aleph stays where it is:

* **herdr** — installed (or updated to this computer's version) and the machine saved in herdr;
* **agents** — a checklist of every agent Aleph knows, the ones there ticked with their versions (on a
  bare machine: the default agent and Codex). Each machine's choice is its own. The ticked ones install one
  after another with their output; each is checked (on your PATH, `--version` runs, herdr sees it). A
  failure says why — no Node.js, the network, permission, a timeout — with its last lines, and asks
  retry / skip / show the log; it never stops the rest. An agent that installs with npm gets **Node.js**
  first when the machine has none: the current LTS from nodejs.org, sha256-checked, in your home folder, no
  sudo (`agents.node = off` turns that off);
* **herdr's integrations** for them, if you want them.

Nothing is installed without your yes. When it's done the running Aleph is told (the daemon's
`hosts.refresh`, or a note an open dashboard reads): it looks at the machine again and says
`box3 set up · herdr 0.9.5 · 2 agents`. `aleph host setup <machine>` runs the same thing in this terminal
(`--agents a,b --yes` without questions); `M` in the dashboard opens it for any machine.

## Install

```sh
./install.sh b31            # build + install `aleph`, add the `al` function to ~/.zshrc, default machine b31
source ~/.zshrc
aleph setup                 # machines, herdr, agents, notifications (also runs by itself the first time)
```

On a new computer from the zip (`tools/package.sh` makes one: a clean git repository, no build output or
installed dependencies):

```sh
unzip aleph-2.0.0.zip && cd aleph-2.0.0
./install.sh --defaults     # no questions: installs Rust and herdr if they're missing, builds Aleph, then
                            # setup's quick start installs what's missing here and opens the dashboard
```

Without `--defaults`, `./install.sh` asks before each install (Rust with rustup, as you; herdr with Homebrew
or its own script; Apple's command line tools when there's no linker), and `aleph setup` walks through every
step — its first step has the same **⚡ quick start** (`aleph setup --defaults`). `--showcase --raycast`
add showcase mode and the Raycast extension.

* `./install.sh --name cc` picks another function name; `--wrap-claude` makes `claude` the function
  (Claude Code's own flags, subcommands and piped input still reach Claude Code); `--showcase` builds
  showcase mode in; `--raycast` builds the Raycast extension too (needs Node.js).
* Themes: `ctrl+k › Theme gallery` (each paints its own background and has its own frames, spinner,
  glyphs and living details; `ui.ground`, `ui.panels`, `ui.ambience`); splashes: `ctrl+k › Splash gallery`
  or `aleph splash NAME --loop`.
* `aleph self-update` updates from GitHub releases; `aleph update` updates herdr, its integrations and the
  agents across the fleet.

## The dashboard

`aleph` — four tabs (`1`–`4`); `Tab` / `Shift+Tab` step through the sections of the one you're on
(Monitor's views, Inbox's kinds, Settings' sections — the same key in each), or to the next tab where
there are none:

| Tab | What's there |
|---|---|
| **Fleet** | Every machine as a card (OS, link quality, GPU, load, today's tokens), its sessions under it as a tree — herdr sessions and agents running outside herdr — and a panel for what's selected (the session's screen, or the machine: hardware, agents, servers, usage). |
| **Monitor** | Machine stats in depth (CPU, memory, GPU, processes, network), **Services** (LLM servers and gateways: Ollama, vLLM, LM Studio, LiteLLM, …) and **Usage** (tokens, time and cost by day, agent, model, machine, project; heatmap; plan limits). |
| **Inbox** | What happened: needed you, finished, started, ended, matched a watch rule, acted on from the phone — merged with the sessions you started (start one again, save it as a preset). |
| **Settings** | Every setting by section with search, plus the doctor's checks for that section (`f` fixes). |

Keys you'll use most (the full map: `?`; rebind any of them in `[keys]`):

* `enter` opens a session in **Focus** (fullscreen here: send messages, queue them, answer, split); `a`
  attaches in herdr, `O` opens herdr in a new terminal tab; `t` in Focus shows the session's timeline.
* `n` **new session**: pick where (a workspace, a recent folder, a worktree, or *Browse folders*) and which
  agent — `enter` chooses, **`i` starts** (`o`: and opens it in Focus).
* `i` the **folder browser**: `enter` / `→` open a folder (`..` goes up), **`i` starts the agent there**,
  `/` searches (fuzzy; `/path` or `~/` jumps), `n` makes a folder, `r` lists it again (it also refreshes
  itself every 5 s).
* `w` tiles marked sessions (or a machine's) side by side; `y` approves, `K` sends keys, `s` sends a
  prompt, `x` closes, `D` duplicates, `P` presets, `W` wakes a sleeping machine (Wake-on-LAN).
* `ctrl+k` the palette; `:` a command line (`:split b31:ignite`, `:preset launch ml`).

Logos: real pictures in terminals that show images (kitty, Ghostty), the OS chip and brand glyphs
elsewhere (`ui.logos` picks: auto, images, thumbnails, ascii, glyphs, text; `aleph icons` shows them).

## From the shell

```sh
aleph new --agent codex --dir ~/work/x         # start one (in a new herdr tab; --detach to stay here)
aleph ls                                       # every session (TSV when piped, --json)
aleph send ignite "run the tests"              # prompt one; --after idle | 30m | 14:30 queues it
aleph approve b31:temper                       # answer its question with its own "yes" option
aleph peek ignite · aleph wait ignite --until done · aleph close ignite
aleph usage 7d --by model                      # tokens, active time and $ from the agents' own logs
aleph preset launch ml                         # a saved layout of sessions across machines
aleph restore                                  # recreate sessions lost to a reboot (same conversation)
```

`aleph help` lists everything; `aleph help <command>` explains one.

## Notifications and your phone

The daemon tells you when a session needs you, finishes, hits a limit, or a machine goes down — wherever
you've asked: desktop banners, [ntfy](https://ntfy.sh) (with Approve / Deny buttons), Moshi, Slack or a
webhook, herdr's own toasts, and Raycast. Routes (`[[notify.route]]`) pick which events go where, with
quiet hours, escalation and do-not-disturb (`aleph notify dnd 1h`). See
[docs/notifications.md](docs/notifications.md).

* **Answering from the phone** is off until you allow it per machine and agent (`remote.allow`); every
  remote action lands in the Inbox.
* **Watch rules** notify you when a session's output says something:
  `aleph notify watch add tests 'FAILED|Traceback' --regex`.
* **Agent hooks** (opt-in per machine, `aleph hooks install`) let agents report in their own words.

## The daemon

`aleph daemon install` runs Aleph in the background (launchd / systemd `--user`): the hub (auto-approve
rules, the task queue, notifications, self-heal), usage reading, and an API on `state/daemon.sock`
(`aleph api …`, `ssh vps aleph api --stdio`). Run it near your machines with
`aleph daemon deploy <host>`, and reach that dashboard with `aleph connect <host>` (it reconnects by
itself). See [docs/daemon.md](docs/daemon.md).

## Usage and cost

`aleph usage` reads the agents' own logs on each machine — Claude Code, Codex, OpenCode, Pi, Gemini,
Qwen — and keeps a local store. Only ids, times, models and numbers leave the machines, never what was
said. Costs are what the agent reported, or an estimate from a price table (marked *est.*). See
[docs/analytics.md](docs/analytics.md).

## OpenRouter

One [OpenRouter](https://openrouter.ai) key for hundreds of models: give it to Aleph once — setup's
Agents step, or `aleph secret set openrouter` — and it's kept in the Keychain (Secret Service on Linux),
never in a settings file (`openrouter.key = "secret:openrouter"`).

* Sessions Aleph starts for the agents that read `OPENROUTER_API_KEY` themselves — OpenCode, Kilo, Pi,
  OMP, Crush, Hermes, Maki (`openrouter.agents`) — get it in the new pane's own environment, on this
  computer and your machines. It travels only inside herdr's socket request (`layout.apply`), never on a
  command line, on disk or in a log; when the socket can't be reached the key is left out and the start
  says so.
* Monitor › Usage has an OpenRouter card (spent today / this month / in all, credits left, your key's
  limit), the header shows today's OpenRouter spend, and `aleph usage` (and `--json`) has it too — read
  from OpenRouter's API every few minutes.
* For OpenCode, Kilo, Pi and OMP, the new-session form (`n`) picks any OpenRouter model (type to search;
  context length and $/MTok in and out beside each); from the shell,
  `aleph new --agent opencode --openrouter qwen/qwen3-coder`.

## Configuration

Settings live in `~/.config/aleph/aleph.toml` — every one is documented in
[docs/config.md](docs/config.md) (`aleph settings` lists them with current values). Secrets go in the
Keychain (`aleph secret set ntfy`, paste it when asked — it's never shown — then `notify.ntfy =
"secret:ntfy"`; `aleph secret ls` lists them, `openrouter` among the names Aleph knows). On macOS the value
reaches the Keychain on `security`'s stdin, never on a command line. `aleph config check|edit|undo`
validates, edits with a check and steps back. Presets: [docs/presets.md](docs/presets.md).

## Troubleshooting

* `aleph doctor` (fleet-wide; `--fix` repairs what it can), `aleph logs -f`, `aleph daemon doctor`.
* A machine that's down shows why (refused, auth, host key, timeout) and a hint; `r` on it reconnects.
* `aleph --debug …` logs every ssh/herdr call to the debug log.

## Development

`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
`tests/e2e-fake/smoke.sh` (CLI checks against fake machines — run it only from there).
The guides are in `docs/`: every setting (`config.md`), the daemon, notifications, presets,
usage analytics and the timeline.
