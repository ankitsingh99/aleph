# Changelog

## 2.0.1 — 2026-10-27
- Pausing a machine (`Z`) asks twice, and a paused machine says so everywhere. The daemon and dashboards
  follow a pause or resume at once.
- Codex sessions show their tokens, cost, turns and timeline (Codex 0.164 no longer names its conversation
  to herdr).
- `ctrl+a` opens the approval board while typing into an agent, when something waits.
- Desktop notifications fall back to `osascript` when terminal-notifier isn't allowed.
- Release binaries carry no build paths; `tools/leak-scan.sh` guards the repository and the releases.
- Antigravity CLI: tokens, thinking tokens, cost, turns and timeline, with each request's model.
- OpenCode 2 read in full and installed by default; OpenCode 1 warned.
- Free models count their tokens and show as free; Gemini 3 prices; OpenRouter's prices for paid models
  (with a key).
- `openrouter.models`: all, free or paid only, in the picker and OpenCode's config.
- The model in use in Fleet, Focus and the timeline; `v` folds the timeline by session or prompt.
- Agent installs run inside the dashboard; fresh agents get their herdr integration.
- Usage's `b` drives its charts; `install.sh` puts `aleph` on your PATH.

## 2.0.0 — 2026-10-26
Aleph (formerly claude-herdr): one binary that runs and watches coding agents on all your machines.
- **Dashboard:** Fleet, Focus and Tile (type into agents natively), Monitor, Inbox, Settings, the palette.
- **Approvals:** every waiting question on one board, answered one at a time or all at once; auto-approve
  rules with a deny list.
- **Usage:** tokens, time and cost by account, agent, model, machine and project, from the agents' own logs.
- **Timeline:** what every agent did, as a tree of prompts, turns, tool calls and subagents.
- **Notifications:** desktop, ntfy, Slack, webhooks, Raycast; an Inbox that keeps only what matters.
- **Daemon and API:** a background service, a local API, the web console, deploys to your machines.
- **Looks:** themes, splashes, logos.

## 1.0.0
claude-herdr: a fleet of machines polled in parallel, remote control without attaching (`send`, `keys`,
`close`), live machine stats.

## 0.5.0
A fuzzy folder browser and a session picker.
