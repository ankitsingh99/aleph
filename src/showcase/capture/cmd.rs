//! `aleph showcase snap` (a poster of a view, drawn offscreen) and
//! `aleph showcase casts` (the recordings made so far: casts, MP4s, GIFs) and
//! `aleph showcase render` (a cast as MP4 and GIF).

use super::{cast, dir, poster, shown, video};
use crate::config::Config;
use crate::out::{err, head, hint, ok, out, warn};
use crate::ui::{self, c};
use ratatui::prelude::*;
use serde_json::{json, Value};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

/// `--name value` or `--name=value`.
fn opt(rest: &[String], name: &str) -> Option<String> {
    let eq = format!("{name}=");
    rest.iter().enumerate().find_map(|(i, a)| {
        if a == name {
            Some(rest.get(i + 1).cloned().unwrap_or_default())
        } else {
            a.strip_prefix(&eq).map(String::from)
        }
    })
}

fn has(rest: &[String], name: &str) -> bool {
    rest.iter().any(|a| a == name)
}

/// `160x48` (or `160×48`) → (columns, rows), within what a poster can be.
pub fn parse_size(s: &str) -> Result<(u16, u16), String> {
    let (w, h) = s
        .trim()
        .split_once(['x', 'X', '×'])
        .ok_or_else(|| format!("--size {s}: columns x rows, like 160x48"))?;
    let w: u16 = w
        .trim()
        .parse()
        .map_err(|_| format!("--size {s}: columns x rows"))?;
    let h: u16 = h
        .trim()
        .parse()
        .map_err(|_| format!("--size {s}: columns x rows"))?;
    if !(40..=400).contains(&w) || !(12..=200).contains(&h) {
        return Err(format!(
            "--size {s}: 40–400 columns and 12–200 rows make a poster"
        ));
    }
    Ok((w, h))
}

const SNAP_USAGE: &str = "aleph showcase snap [--view fleet|monitor|usage|inbox|settings|focus|tile|help|palette|…] [--size 160x48] [--frame cockpit|plain] [--out FILE.svg] [--png auto|rsvg-convert|qlmanage|off | --no-png] [--wait SECS] [--json]";

pub fn snap(rest: &[String]) -> ExitCode {
    if has(rest, "-h") || has(rest, "--help") {
        println!("{SNAP_USAGE}");
        for (v, what) in crate::tui::snap::VIEWS {
            println!("  {v:<10} {what}");
        }
        return ExitCode::SUCCESS;
    }
    let mut cfg = Config::load();
    if let Err(e) = super::allowed(&cfg) {
        err(&e);
        hint("aleph showcase on (masked) or aleph showcase on --demo, then again");
        return ExitCode::from(1);
    }
    // the dashboard's look, set up as main.rs does before it opens one
    // (`aleph showcase …` runs ahead of that): theme and accent, logos,
    // spacing, spinner, cost, the machines' colours
    crate::ui::c::init(
        &cfg.get_or("theme"),
        cfg.get("accent").and_then(|a| crate::ui::parse_color(&a)),
    );
    crate::icons::init(&cfg.get_or("icons"));
    crate::ui::set_scale(&cfg.get_or("ui_scale"));
    crate::ui::set_spinner(&cfg.get_or("ui.spinner"));
    crate::usage::dash::show_cost(cfg.get_or("analytics.cost") != "off");
    crate::theme::assign_machine_colors(&mut cfg);
    let view = opt(rest, "--view").unwrap_or_else(|| "fleet".into());
    let size = opt(rest, "--size").unwrap_or_else(|| cfg.get_or("showcase.poster_size"));
    let (w, h) = match parse_size(&size) {
        Ok(x) => x,
        Err(e) => {
            err(&e);
            return ExitCode::from(2);
        }
    };
    let frame = match opt(rest, "--frame") {
        Some(f) => match poster::Frame::parse(&f) {
            Some(x) => Some(x),
            None => {
                err(&format!("--frame {f}: cockpit or plain"));
                return ExitCode::from(2);
            }
        },
        None => None,
    };
    let how = if has(rest, "--no-png") {
        "off".to_string()
    } else {
        opt(rest, "--png").unwrap_or_else(|| cfg.get_or("showcase.poster_png"))
    };
    if !matches!(how.as_str(), "auto" | "rsvg-convert" | "qlmanage" | "off") {
        err(&format!("--png {how}: auto, rsvg-convert, qlmanage or off"));
        return ExitCode::from(2);
    }
    let wait = match opt(rest, "--wait").map(|s| s.parse::<f64>()) {
        Some(Ok(s)) if (0.0..=120.0).contains(&s) => Duration::from_secs_f64(s),
        Some(_) => {
            err("--wait needs seconds (0–120)");
            return ExitCode::from(2);
        }
        None => Duration::from_secs(8),
    };
    let json_out = has(rest, "--json");
    let sp = crate::out::Spinner::start(format!("Drawing {view} at {w}×{h}"));
    let r = crate::tui::snap::frame(cfg.clone(), &view, w, h, wait);
    sp.stop();
    let (buf, title) = match r {
        Ok(x) => x,
        Err(e) => {
            err(&e);
            return ExitCode::from(2);
        }
    };
    let look = super::look(&cfg, &title, frame);
    let saved = match opt(rest, "--out") {
        Some(o) => {
            let p = crate::config::expand(&o);
            // a .png asked for: the SVG beside it, and the PNG a must
            let svg = if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
                p.with_extension("svg")
            } else {
                p
            };
            if let Some(parent) = svg.parent().filter(|d| !d.as_os_str().is_empty()) {
                let _ = std::fs::create_dir_all(parent);
            }
            super::save_poster_at(&buf, &look, &svg, &how)
        }
        None => super::save_poster(&buf, &look, &dir(&cfg, "showcase.poster_dir"), &how),
    };
    let (svg, png) = match saved {
        Ok(x) => x,
        Err(e) => {
            err(&e);
            return ExitCode::from(1);
        }
    };
    let wants_png = opt(rest, "--out").is_some_and(|o| o.to_ascii_lowercase().ends_with(".png"));
    let (png_path, tool, png_err) = match &png {
        Ok(Some((p, t))) => (Some(p.clone()), Some(*t), None),
        Ok(None) => (None, None, None),
        Err(e) => (None, None, Some(e.clone())),
    };
    if json_out {
        // the user's own files: real paths, not masked
        let v = json!({
            "view": title,
            "svg": svg,
            "png": png_path,
            "png_tool": tool,
            "png_error": png_err,
            "columns": w,
            "rows": h,
        });
        println!("{v}");
    } else if !std::io::stdout().is_terminal() {
        println!("svg\t{}", svg.display());
        if let Some(p) = &png_path {
            println!("png\t{}", p.display());
        }
    } else {
        ok(&format!("poster of {title} → {}", shown(&svg)));
        match (&png_path, tool, &png_err) {
            (Some(p), Some(t), _) => ok(&format!("PNG ({t}, 2×) → {}", shown(p))),
            (_, _, Some(e)) => warn(&format!("no PNG: {e}")),
            _ if how == "off" => {}
            _ => hint("no PNG: no converter here (rsvg-convert — brew install librsvg —, or qlmanage on macOS); the SVG opens in any browser"),
        }
    }
    if wants_png && png_path.is_none() {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

// ------------------------------------------------------------------ casts

/// A video file: (where, its size, its length and picture size).
pub type Media = (PathBuf, u64, Option<(f64, u32, u32)>);

/// One recording: its .cast, .mp4 and .gif (any of them), by name.
#[derive(Clone, Debug, Default)]
pub struct Rec {
    /// the name they share (`aleph-20261024-003254`)
    pub name: String,
    pub cast: Option<cast::Info>,
    /// (the file, its size, its length and picture size)
    pub mp4: Option<Media>,
    pub gif: Option<Media>,
    /// when it began (unix)
    pub at: u64,
    newest: Option<std::time::SystemTime>,
}

impl Rec {
    /// Its main file: the cast, else the MP4, else the GIF.
    pub fn path(&self) -> PathBuf {
        self.cast
            .as_ref()
            .map(|c| c.path.clone())
            .or_else(|| self.mp4.as_ref().map(|m| m.0.clone()))
            .or_else(|| self.gif.as_ref().map(|g| g.0.clone()))
            .unwrap_or_default()
    }

    /// The video's picture size, if there's a video.
    fn pixels(&self) -> Option<(u32, u32)> {
        [&self.mp4, &self.gif]
            .into_iter()
            .flatten()
            .find_map(|(_, _, i)| i.map(|(_, w, h)| (w, h)).filter(|(w, _)| *w > 0))
    }

    /// How long it plays: the video's length, else the cast's.
    pub fn secs(&self) -> f64 {
        [&self.mp4, &self.gif]
            .into_iter()
            .flatten()
            .find_map(|(_, _, i)| i.map(|(s, _, _)| s))
            .or_else(|| self.cast.as_ref().map(|c| c.secs))
            .unwrap_or(0.0)
    }

    pub fn bytes(&self) -> u64 {
        self.cast.as_ref().map(|c| c.bytes).unwrap_or(0)
            + self.mp4.as_ref().map(|m| m.1).unwrap_or(0)
            + self.gif.as_ref().map(|g| g.1).unwrap_or(0)
    }
}

/// Days since 1970-01-01 of a date.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `aleph-20261024-003254` → when that was (local time, as the name was
/// made).
fn stamp_of(name: &str) -> Option<u64> {
    let s = name.strip_prefix("aleph-")?;
    let (d, t) = s.get(..15)?.split_once('-')?;
    if d.len() != 8 || t.len() != 6 || !d.bytes().chain(t.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = |s: &str| s.parse::<i64>().ok();
    let (y, m, dd) = (n(&d[..4])?, n(&d[4..6])? as u32, n(&d[6..])? as u32);
    let (h, mi, se) = (n(&t[..2])?, n(&t[2..4])?, n(&t[4..])?);
    let secs = days_from_civil(y, m, dd) * 86_400 + h * 3600 + mi * 60 + se - ui::utc_offset();
    u64::try_from(secs).ok()
}

/// The recordings in `d`, newest first.
pub fn list(d: &Path) -> Vec<Rec> {
    let mut by: std::collections::BTreeMap<String, Rec> = Default::default();
    for p in std::fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
    {
        let (Some(stem), Some(ext)) = (
            p.file_stem().map(|s| s.to_string_lossy().to_string()),
            p.extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase()),
        ) else {
            continue;
        };
        if stem.starts_with('.') || !matches!(ext.as_str(), "cast" | "mp4" | "gif") {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&p) else {
            continue;
        };
        let r = by.entry(stem.clone()).or_insert_with(|| Rec {
            name: stem.clone(),
            ..Default::default()
        });
        let m = meta.modified().ok();
        r.newest = r.newest.max(m);
        match ext.as_str() {
            "cast" => r.cast = cast::info(&p),
            "mp4" => r.mp4 = Some((p.clone(), meta.len(), video::mp4_info(&p))),
            _ => r.gif = Some((p.clone(), meta.len(), video::gif_info(&p))),
        }
    }
    let mut v: Vec<Rec> = by
        .into_values()
        .filter(|r| r.cast.is_some() || r.mp4.is_some() || r.gif.is_some())
        .map(|mut r| {
            let mtime = r
                .newest
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            r.at = r
                .cast
                .as_ref()
                .map(|c| c.at)
                .filter(|a| *a > 0)
                .or_else(|| stamp_of(&r.name))
                .unwrap_or(mtime);
            r
        })
        .collect();
    v.sort_by_key(|r| std::cmp::Reverse((r.at, r.newest)));
    v
}

fn size_of(b: u64) -> String {
    match b {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} B"),
    }
}

pub fn casts(rest: &[String]) -> ExitCode {
    let cfg = Config::load();
    let d = dir(&cfg, "showcase.cast_dir");
    let all = list(&d);
    let n = if has(rest, "--all") {
        usize::MAX
    } else {
        match opt(rest, "-n").map(|s| s.parse::<usize>()) {
            Some(Ok(n)) => n,
            Some(Err(_)) => {
                err("-n needs a number");
                return ExitCode::from(2);
            }
            None => 10,
        }
    };
    let shown_ones: Vec<&Rec> = all.iter().take(n).collect();
    let grid = |r: &Rec| r.cast.as_ref().map(|c| (c.width, c.height));
    if has(rest, "--json") {
        let v: Vec<Value> = shown_ones
            .iter()
            .map(|r| {
                let px = r.pixels();
                json!({
                    "name": r.name,
                    "path": r.path(),
                    "started": r.at,
                    "seconds": (r.secs() * 10.0).round() / 10.0,
                    "columns": grid(r).map(|g| g.0),
                    "rows": grid(r).map(|g| g.1),
                    "title": r.cast.as_ref().map(|c| c.title.clone()),
                    "bytes": r.bytes(),
                    "cast": r.cast.as_ref().map(|c| c.path.clone()),
                    "mp4": r.mp4.as_ref().map(|m| m.0.clone()),
                    "gif": r.gif.as_ref().map(|g| g.0.clone()),
                    "width": px.map(|p| p.0),
                    "height": px.map(|p| p.1),
                })
            })
            .collect();
        println!("{}", json!({"dir": d, "casts": v, "total": all.len()}));
        return ExitCode::SUCCESS;
    }
    let size = |r: &Rec| match (r.pixels(), grid(r)) {
        (Some((w, h)), _) => format!("{w}×{h}"),
        (None, Some((c, rr))) => format!("{c}×{rr} cells"),
        _ => String::new(),
    };
    if !std::io::stdout().is_terminal() {
        let p = |o: &Option<Media>| {
            o.as_ref()
                .map(|x| x.0.display().to_string())
                .unwrap_or_default()
        };
        for r in &shown_ones {
            let wh = grid(r)
                .map(|(c, rr)| format!("{c}x{rr}"))
                .or_else(|| r.pixels().map(|(w, h)| format!("{w}x{h}")))
                .unwrap_or_default();
            println!(
                "{}\t{}\t{:.1}\t{}\t{}\t{}\t{}",
                r.path().display(),
                r.at,
                r.secs(),
                wh,
                r.bytes(),
                p(&r.mp4),
                p(&r.gif)
            );
        }
        return ExitCode::SUCCESS;
    }
    if all.is_empty() {
        hint(&format!(
            "no recordings yet in {} — in the dashboard, {} (or the palette's Showcase: record) starts one",
            shown(&d),
            match cfg.get_or("showcase.record_key").trim() {
                "" | "off" | "none" => "the palette".to_string(),
                k => k.to_string(),
            }
        ));
        return ExitCode::SUCCESS;
    }
    head(&format!(
        "{} recording{} in {}",
        all.len(),
        if all.len() == 1 { "" } else { "s" },
        shown(&d)
    ));
    let rows: Vec<[String; 4]> = shown_ones
        .iter()
        .map(|r| {
            [
                super::when(r.at),
                super::clock(r.secs().round() as u64),
                size(r),
                r.name.clone(),
            ]
        })
        .collect();
    let col = |i: usize, min: usize| {
        rows.iter()
            .map(|r| ui::width(&r[i]))
            .max()
            .unwrap_or(0)
            .max(min)
    };
    let (wl, ws, wn) = (col(1, 6), col(2, 4), col(3, 9));
    out(Line::from(vec![
        Span::styled(format!("  {:<16}  ", "WHEN"), ui::faint()),
        Span::styled(format!("{:>wl$}  ", "LENGTH"), ui::faint()),
        Span::styled(format!("{:<ws$}  ", "SIZE"), ui::faint()),
        Span::styled(format!("{:<wn$}  ", "RECORDING"), ui::faint()),
        Span::styled("FILES", ui::faint()),
    ]));
    for (row, r) in rows.iter().zip(&shown_ones) {
        let mut files: Vec<Span> = Vec::new();
        let mut put = |kind: &str, bytes: u64, colour: Style| {
            if !files.is_empty() {
                files.push(Span::styled(" · ", ui::faint()));
            }
            files.push(Span::styled(kind.to_string(), colour));
            files.push(Span::styled(format!(" {}", size_of(bytes)), ui::faint()));
        };
        if let Some(m) = &r.mp4 {
            put("mp4", m.1, ui::fg(c::GREEN()));
        }
        if let Some(g) = &r.gif {
            put("gif", g.1, ui::fg(c::BLUE()));
        }
        if let Some(cs) = &r.cast {
            put("cast", cs.bytes, ui::muted());
        }
        let mut l = vec![
            Span::raw(format!("  {:<16}  ", row[0])),
            Span::styled(format!("{:>wl$}  ", row[1]), ui::fg(c::ACCENT())),
            Span::styled(format!("{:<ws$}  ", row[2]), ui::muted()),
            Span::raw(format!("{:<wn$}  ", row[3])),
        ];
        l.extend(files);
        out(Line::from(l));
    }
    if all.len() > shown_ones.len() {
        hint(&format!("{} more: --all", all.len() - shown_ones.len()));
    }
    let unmade = shown_ones
        .iter()
        .find(|r| r.cast.is_some() && r.mp4.is_none() && r.gif.is_none());
    if let Some(r) = unmade {
        let p = r.path();
        let f = p
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        hint(&format!(
            "aleph showcase render {f} makes a recording's MP4 and GIF (in {})",
            shown(&d)
        ));
    }
    if shown_ones.iter().any(|r| r.cast.is_some()) {
        if crate::proc::which("asciinema").is_some() {
            hint("asciinema play <file.cast> plays a cast here");
        } else {
            hint("a .cast plays with asciinema (asciinema play <file>), or on a page with asciinema-player");
        }
    }
    ExitCode::SUCCESS
}

// ------------------------------------------------------------------ render

const RENDER_USAGE: &str = "aleph showcase render FILE.cast [--mp4] [--gif] [--frame cockpit|plain] [--font FILE] [--json]";

/// The cast named on the line: the first word that isn't an option (or an
/// option's value).
fn file_arg(rest: &[String]) -> Option<String> {
    let takes = ["--frame", "--status", "--font"];
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        if takes.contains(&a.as_str()) {
            i += 2;
            continue;
        }
        if !a.starts_with('-') {
            return Some(a.clone());
        }
        i += 1;
    }
    None
}

/// `aleph showcase render FILE.cast`: its MP4 and GIF, beside it (again,
/// for one made before, or with another look). After a stop the dashboard
/// runs it on its own (`--after-stop --status FILE`): nothing on the
/// screen, how it went in the status file.
pub fn render(rest: &[String]) -> ExitCode {
    if has(rest, "-h") || has(rest, "--help") {
        println!("{RENDER_USAGE}");
        println!("  the MP4 and GIF go beside the cast (showcase.video, showcase.video_frame);");
        println!("  the cast stays (after a stop it goes once they're made, unless showcase.cast_keep is on)");
        return ExitCode::SUCCESS;
    }
    let cfg = Config::load();
    // the accent the card is drawn in, for a cast that doesn't say
    crate::ui::c::init(
        &cfg.get_or("theme"),
        cfg.get("accent").and_then(|a| crate::ui::parse_color(&a)),
    );
    let after = has(rest, "--after-stop");
    let status = opt(rest, "--status")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let Some(file) = file_arg(rest) else {
        err("which cast? aleph showcase render FILE.cast");
        hint("aleph showcase casts lists the recordings");
        return ExitCode::from(2);
    };
    let path = crate::config::expand(&file);
    let mut job = video::Job::of(&cfg, &path);
    if has(rest, "--mp4") || has(rest, "--gif") {
        job.mp4 = has(rest, "--mp4");
        job.gif = has(rest, "--gif");
    }
    if let Some(f) = opt(rest, "--frame") {
        match poster::Frame::parse(&f) {
            Some(x) => job.frame = x,
            None => {
                err(&format!("--frame {f}: cockpit or plain"));
                return ExitCode::from(2);
            }
        }
    }
    // a cast named by hand stays: only the one a stop just made goes
    if !after || has(rest, "--keep-cast") {
        job.keep_cast = true;
    }
    if let Some(f) = opt(rest, "--font") {
        job.font = f;
    }
    let tell = |r: &Result<video::Made, String>| {
        if let Some(s) = &status {
            let _ = crate::config::write_atomic(s, video::status(&job, r).to_string().as_bytes());
        }
    };
    if !job.mp4 && !job.gif {
        err("nothing to make: showcase.video is off (--mp4, --gif)");
        return ExitCode::from(2);
    }
    if !path.is_file() {
        let e = format!("no cast at {}", shown(&path));
        tell(&Err(e.clone()));
        err(&e);
        return ExitCode::from(1);
    }
    let Some(ff) = video::ffmpeg() else {
        let e = video::no_ffmpeg();
        tell(&Err(e.clone()));
        err(&e);
        return ExitCode::from(1);
    };
    if after {
        let r = video::render(&job, &ff, &mut |_| {});
        tell(&r);
        return if r.is_ok() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        };
    }
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let sp = crate::out::Spinner::start(format!("Rendering {name}"));
    let r = video::render(&job, &ff, &mut |s| match s {
        video::Stage::Frames(i, n) => sp.set(format!("Rendering {name}: frame {i} of {n}")),
        video::Stage::Gif => sp.set(format!("Making the GIF of {name}")),
    });
    sp.stop();
    if has(rest, "--json") {
        println!("{}", video::status(&job, &r));
        return if r.is_ok() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        };
    }
    let made = match r {
        Ok(m) => m,
        Err(e) => {
            err(&e);
            if path.is_file() {
                hint(&format!("the .cast is kept: {}", shown(&path)));
            }
            return ExitCode::from(1);
        }
    };
    let bytes = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    if !std::io::stdout().is_terminal() {
        for (k, p) in [("mp4", &made.mp4), ("gif", &made.gif)] {
            if let Some(p) = p {
                println!("{k}\t{}", p.display());
            }
        }
        if made.cast_kept {
            println!("cast\t{}", path.display());
        }
        return ExitCode::SUCCESS;
    }
    if let Some(p) = &made.mp4 {
        ok(&format!(
            "MP4 → {}  {}×{} · {} · {} fps · {}",
            shown(p),
            made.width,
            made.height,
            super::clock(made.secs.round() as u64),
            video::FPS,
            size_of(bytes(p))
        ));
    }
    if let Some(p) = &made.gif {
        let (_, w, h) = video::gif_info(p).unwrap_or((0.0, 0, 0));
        ok(&format!(
            "GIF → {}  {w}×{h} · {} fps · {}",
            shown(p),
            video::GIF_FPS,
            size_of(bytes(p))
        ));
    }
    if made.cast_kept {
        hint(&format!("the .cast is kept: {}", shown(&path)));
    } else {
        hint("the .cast went (showcase.cast_keep is off)");
    }
    hint(&format!(
        "{} frames in {:.1} s, drawn with {}",
        made.frames, made.took, made.font
    ));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_options() {
        assert_eq!(parse_size("160x48"), Ok((160, 48)));
        assert_eq!(parse_size(" 200×60 "), Ok((200, 60)));
        assert!(parse_size("10x5").is_err());
        assert!(parse_size("wide").is_err());
        let a: Vec<String> = ["--view", "usage", "--size=120x40", "--no-png"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(opt(&a, "--view").as_deref(), Some("usage"));
        assert_eq!(opt(&a, "--size").as_deref(), Some("120x40"));
        assert!(has(&a, "--no-png") && opt(&a, "--out").is_none());
        assert_eq!(size_of(512), "512 B");
        assert_eq!(size_of(4096), "4 KB");
    }

    #[test]
    fn casts_are_listed_newest_first() {
        let d = std::env::temp_dir().join(format!("aleph-casts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let h = |w: u32| {
            format!("{{\"version\":2,\"width\":{w},\"height\":24,\"timestamp\":1}}\n[0.0,\"o\",\"x\"]\n[3.5,\"o\",\"y\"]\n")
        };
        std::fs::write(d.join("a.cast"), h(80)).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(d.join("b.cast"), h(100)).unwrap();
        std::fs::write(d.join("note.txt"), "not one").unwrap();
        std::fs::write(d.join("bad.cast"), "{\"version\":1}\n").unwrap();
        let v = list(&d);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].cast.as_ref().unwrap().width, 100);
        assert_eq!(v[1].secs(), 3.5);
        // a recording rendered (its cast gone): its MP4 and GIF together,
        // when from its name; what's still being made isn't one
        let gif = b"GIF89a\x04\x00\x02\x00\x00\x00\x00\x21\xf9\x04\x00\x64\x00\x00\x00\x3b";
        std::fs::write(d.join("aleph-20261024-003254.gif"), gif).unwrap();
        std::fs::write(d.join("aleph-20261024-003254.mp4"), b"not really").unwrap();
        std::fs::write(d.join(".aleph-20261024-003254.mp4.part"), b"half").unwrap();
        let v = list(&d);
        assert_eq!(v.len(), 3);
        let r = &v[0];
        assert_eq!(r.name, "aleph-20261024-003254");
        assert!(r.cast.is_none() && r.mp4.is_some() && r.gif.is_some());
        assert_eq!(r.path(), d.join("aleph-20261024-003254.mp4"));
        assert_eq!(r.secs(), 1.0);
        assert_eq!(r.pixels(), Some((4, 2)));
        assert_eq!(r.bytes(), gif.len() as u64 + 10);
        assert_eq!(super::super::when(r.at), "2026-10-24 00:32");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn names_say_when_and_the_cast_is_found_among_options() {
        assert!(stamp_of("aleph-20261024-003254").is_some());
        assert_eq!(stamp_of("aleph-2026-10"), None);
        assert_eq!(stamp_of("holiday"), None);
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        assert_eq!(
            file_arg(&a(&["--frame", "plain", "x.cast"])).as_deref(),
            Some("x.cast")
        );
        assert_eq!(
            file_arg(&a(&["x.cast", "--gif"])).as_deref(),
            Some("x.cast")
        );
        assert_eq!(file_arg(&a(&["--status", "/s.json", "--after-stop"])), None);
    }
}
