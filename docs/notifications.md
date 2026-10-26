# Notifications

Aleph notices when a session needs you, finishes, hits a limit or loses its machine, and sends it where you've
asked. The daemon does this; with no daemon, the open dashboard does.

| Where | Set up with |
|---|---|
| Desktop | on by default: questions, limits, problems, long finished turns |
| [ntfy](https://ntfy.sh) | `notify.ntfy = "secret:ntfy"`: questions come with Approve / Deny buttons |
| Slack, a webhook | `notify.slack`, `notify.webhook` |
| Raycast | `notify.raycast = true`: the menu bar updates at once |
| herdr | `notify.herdr = true`: a toast in herdr's window |

Nothing is sent about a session you're looking at. `notify.include_text` (off · summary · full) decides how
much of the agent's words leave the machine.

**Routes** pick which events go where, with quiet hours and escalation:

```toml
[[notify.route]]
when = ["needs_input", "limit", "error"]
to = ["ntfy", "desktop"]
quiet = "23:00-08:00"
escalate_after = "10m"
escalate_to = ["ntfy"]
```

```sh
aleph notify test --to ntfy,desktop     # a test that looks like a real question
aleph notify dnd 1h                     # hold everything back for an hour
aleph notify mute b31:ignite 1h
aleph notify watch add tests 'FAILED|Traceback' --regex    # tell me when output says this
```

**Answering from your phone** is off until you allow it per machine and agent (`remote.allow`). An answer
always uses the question's own "yes" option, only while that question still waits, and is recorded in the
Inbox.

**Agent hooks** (`aleph hooks install`, opt-in) let Claude Code, Codex and OpenCode report in their own words:
"Claude needs your permission", or a finished turn's last message. Each change is shown as a diff first and
backed up; `aleph hooks remove` takes them out.
