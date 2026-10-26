# Aleph for Raycast

Your coding-agent sessions across every machine, from Raycast. It is a thin front end over Aleph: it asks the
daemon on its socket when one runs, otherwise it runs the `aleph` binary.

## Install

You need Node 20+, Raycast and a working `aleph` (`aleph setup`).

```sh
cd extras/raycast-extension
npm install
npm run dev        # imports the extension into Raycast; it stays installed after you stop it
```

Open **Aleph Menu Bar** once to put it in the menu bar.

## Commands

| Command | What it does |
|---|---|
| **Aleph Sessions** | Every session by machine, its live screen and question; open, approve, reply, queue, close |
| **Aleph Start Session** | Pick a machine, agent and folder, with an optional first prompt |
| **Aleph Send Message** | Prompt a session now, or queue it for when it's free |
| **Aleph Menu Bar** | What needs you, what's working, the Inbox and your machines |
| **Aleph Inbox** | What happened, newest first; reply, approve or start again |
| **Aleph Machines** | Each machine live: OS, GPU, link, sessions, agents and LLM servers |
| **Aleph Usage** | Tokens, time and cost by account, agent, model, machine and project, with charts |
| **Aleph Timeline** | What each agent did, conversation by conversation |
| **Aleph Presets** | Saved starting points; launch one |
| **Aleph Agents** · **Local LLM Servers** · **Message Queue** · **Daemon** · **Doctor** · **Notification Settings** | The same views as the dashboard's |

**Preferences:** the `aleph` binary (default: looked up in the usual places), whether sessions open in
Aleph's Focus or in herdr, and the terminal to use (Warp, iTerm2, Terminal, Ghostty, kitty, WezTerm).

## Develop

```sh
npm run lint · npm run typecheck · npm test · npm run build
```
