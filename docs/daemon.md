# The daemon

`aleph daemon install` runs Aleph in the background, as launchd (`dev.aleph.daemon`) on macOS or systemd
`--user` (`aleph.service`) on Linux. It runs:

- **the hub:** auto-approve rules, the task queue, self-heal and notifications;
- **usage reading;**
- **an API** on `~/.local/state/aleph/daemon.sock`.

With no daemon, an open dashboard does the same.

```sh
aleph daemon status | logs -f | doctor
aleph daemon restart | stop | start | uninstall
```

**The API:** newline-delimited JSON. `aleph api methods` lists the methods.

```sh
aleph api state.snapshot
aleph api session.send '{"session":"b31:ignite","text":"run the tests"}'
ssh vps aleph api --stdio
```

**Near your machines:**

```sh
aleph daemon deploy b31 --service     # build (there, or --build cross here), install, start
aleph connect b31                     # that dashboard, here
```

With daemons on several computers, `hub.machine` picks the one that runs the hub. On Linux, a daemon that
should keep running after you log out needs `loginctl enable-linger $USER`.

**From elsewhere:** `aleph daemon expose tailscale | cloudflare | off` puts the HTTP API behind Tailscale
Serve or a Cloudflare tunnel, after asking.
