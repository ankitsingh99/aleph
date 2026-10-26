//! Wake-on-LAN: a magic packet for a machine that's asleep, sent from another
//! machine in the fleet (one on its network) or this computer. Nothing is
//! installed: it uses what the sender has (`wakeonlan`, `wol`, `socat`, or
//! OpenBSD `nc -b`), and says so when there's none.
//!
//! The MAC is `host.<a>.mac`, else what the agent probe saw while the
//! machine was up (its default route's interface, kept a month).

use crate::config::Config;
use std::time::Duration;

/// `aa:bb:cc:dd:ee:ff` from any common spelling (`AA-BB-…`, `aabb.ccdd.eeff`,
/// `aabbccddeeff`); None when it isn't one (pure — tested).
pub fn normalize(s: &str) -> Option<String> {
    let hex: String = s
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '.' | ' '))
        .collect();
    if hex.len() != 12 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let hex = hex.to_lowercase();
    if hex == "000000000000" || hex == "ffffffffffff" {
        return None;
    }
    Some(
        (0..6)
            .map(|i| &hex[i * 2..i * 2 + 2])
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// A machine's MAC: the setting, else the probe's (seen while it was up).
pub fn mac_of(cfg: &Config, host: &str) -> Option<String> {
    cfg.host_get(host, "mac")
        .and_then(|m| normalize(&m))
        .or_else(|| {
            crate::probe::cached(host, Duration::from_secs(30 * 86_400))
                .and_then(|p| p.mac.map(|m| m.1))
        })
}

/// `mac_of`, remembered a minute (screens ask every frame).
pub fn known(cfg: &Config, host: &str) -> bool {
    type Seen = Vec<(String, std::time::Instant, bool)>;
    static S: std::sync::Mutex<Seen> = std::sync::Mutex::new(Vec::new());
    let mut g = S.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, at, v)) = g.iter().find(|(h, _, _)| h == host) {
        if at.elapsed() < Duration::from_secs(60) {
            return *v;
        }
    }
    let v = mac_of(cfg, host).is_some();
    g.retain(|(h, _, _)| h != host);
    g.push((host.to_string(), std::time::Instant::now(), v));
    v
}

/// The magic packet as `printf` octal escapes: six 0xff, then the MAC 16
/// times (pure — tested).
pub fn packet_escapes(mac: &str) -> String {
    let bytes: Vec<u8> = mac
        .split(':')
        .filter_map(|b| u8::from_str_radix(b, 16).ok())
        .collect();
    let mut s = String::from(&"\\377".repeat(6));
    for _ in 0..16 {
        for b in &bytes {
            s.push_str(&format!("\\{b:03o}"));
        }
    }
    s
}

/// The sender's script: the first tool it has (pure — tested). Prints
/// `sent <tool>`, or `none` and exits 1.
pub fn script(mac: &str) -> String {
    let p = packet_escapes(mac);
    format!(
        "LC_ALL=C; M={mac}; P='{p}'\n\
         if command -v wakeonlan >/dev/null 2>&1 && wakeonlan \"$M\" >/dev/null 2>&1; then echo 'sent wakeonlan'; exit 0; fi\n\
         if command -v wol >/dev/null 2>&1 && wol \"$M\" >/dev/null 2>&1; then echo 'sent wol'; exit 0; fi\n\
         if command -v socat >/dev/null 2>&1 && printf \"$P\" | socat - UDP-DATAGRAM:255.255.255.255:9,broadcast >/dev/null 2>&1; then echo 'sent socat'; exit 0; fi\n\
         if command -v nc >/dev/null 2>&1 && printf \"$P\" | nc -u -b -w1 255.255.255.255 9 >/dev/null 2>&1; then echo 'sent nc'; exit 0; fi\n\
         echo none; exit 1\n"
    )
}

/// Send it from `relay` (a fleet machine, or `local`): which tool did.
pub fn send_from(relay: &str, mac: &str) -> Result<String, String> {
    let mut cmd = crate::sshcfg::shell(relay, "sh -s");
    let o = crate::proc::run(
        &mut cmd,
        Duration::from_secs(15),
        Some(script(mac).as_bytes()),
    )
    .map_err(|e| format!("{relay}: {e}"))?;
    let t = o.text();
    match t.lines().find_map(|l| l.strip_prefix("sent ")) {
        Some(tool) => Ok(tool.trim().to_string()),
        None if t.contains("none") => Err(format!(
            "{relay} has no way to send one (wakeonlan, wol, socat or OpenBSD nc)"
        )),
        None => Err(format!("{relay}: {}", o.reason())),
    }
}

/// Wake `host`: from each sender in turn until one sends it. Ok(where, tool).
pub fn wake(cfg: &Config, host: &str, senders: &[String]) -> Result<(String, String), String> {
    if let Some(e) = crate::showcase::refused() {
        return Err(e);
    }
    let mac = mac_of(cfg, host).ok_or_else(|| {
        format!(
            "no MAC address known for {host}: set host.{host}.mac (aa:bb:cc:dd:ee:ff), or run aleph agents --host {host} while it's up"
        )
    })?;
    let mut errs = Vec::new();
    for s in senders {
        match send_from(s, &mac) {
            Ok(tool) => return Ok((s.clone(), tool)),
            Err(e) => errs.push(e),
        }
    }
    Err(if errs.is_empty() {
        "no machine is up to send it from".to_string()
    } else {
        errs.join(" · ")
    })
}

/// `aleph wake <machine> [--via <machine>]`
pub fn cmd(a: crate::cli::Args) -> std::process::ExitCode {
    use crate::out::{err, hint, ok};
    use std::process::ExitCode;
    let p = a.positional();
    let Some(host) = p.first().map(|s| s.to_string()).or(a.flag.clone()) else {
        return crate::out::fail("usage: aleph wake <machine> [--via <machine>]");
    };
    let via = a
        .rest
        .iter()
        .position(|x| x == "--via")
        .and_then(|i| a.rest.get(i + 1))
        .cloned();
    // this computer first (it may share the network), then the others
    let senders: Vec<String> = match via {
        Some(v) => vec![v],
        None => {
            let mut v = vec![crate::sshcfg::LOCAL.to_string()];
            v.extend(a.cfg.hosts().into_iter().filter(|h| *h != host));
            v
        }
    };
    let sp = crate::out::Spinner::start(format!("Waking {host}"));
    let r = wake(&a.cfg, &host, &senders);
    sp.stop();
    match r {
        Ok((from, tool)) => {
            ok(&format!(
                "sent {host} a wake-up packet from {from} ({tool})"
            ));
            hint("it takes a minute or so to come up; aleph ls shows it then");
            ExitCode::SUCCESS
        }
        Err(e) => {
            err(&e);
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macs() {
        assert_eq!(
            normalize("AA-BB-CC-DD-EE-0F").as_deref(),
            Some("aa:bb:cc:dd:ee:0f")
        );
        assert_eq!(
            normalize("aabb.ccdd.eeff").as_deref(),
            Some("aa:bb:cc:dd:ee:ff")
        );
        assert_eq!(
            normalize("00:11:22:33:44:55").as_deref(),
            Some("00:11:22:33:44:55")
        );
        assert_eq!(normalize("00:00:00:00:00:00"), None);
        assert_eq!(normalize("not a mac"), None);
        assert_eq!(normalize("aa:bb:cc"), None);
    }

    #[test]
    fn packet() {
        let p = packet_escapes("00:11:22:33:44:ff");
        // 6 + 16 × 6 bytes, four characters each
        assert_eq!(p.len(), (6 + 96) * 4);
        assert!(p.starts_with("\\377\\377\\377\\377\\377\\377\\000\\021\\042\\063\\104\\377"));
        let s = script("00:11:22:33:44:ff");
        assert!(s.contains("M=00:11:22:33:44:ff"));
        assert!(s.contains("wakeonlan \"$M\""));
        assert!(!s.contains("python"));
    }
}
