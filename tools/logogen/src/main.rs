//! logogen — render Aleph's SVG sources into the committed files the app and
//! the Raycast extension use. Dev-only: building aleph never needs it (or
//! resvg), and the aleph binary never fetches anything.
//!
//!   cargo run --release --manifest-path tools/logogen/Cargo.toml -- [command] [repo root]
//!
//! Commands (the repo root defaults to the current folder):
//!
//!   all (default)  `logos`, then `brand`
//!   logos          from assets/logos/<kind>/<id>.svg (kinds: os agent gpu server) and the
//!                  table in src/sources.rs, which every SVG must have a row in:
//!                    assets/logos/<kind>/<id>.svg      monograms and drawn logos (re-written)
//!                    src/logo_data.rs                  bitmaps the terminal renderers draw:
//!                                                      16×16 and 32×32 palette indices, RLE
//!                    assets/logos/png/<id>@{64,128}.png         in brand colours; indexed,
//!                                                      with alpha steps (the app embeds the
//!                                                      128s and recolours their palettes)
//!                    assets/logos/png/<id>-{dark,light}@64.png  for dark / light backgrounds
//!                                                      where the brand colour wouldn't show
//!                    assets/logos/SOURCES.md           where each came from, licence, colour
//!   brand          assets/brand/*.svg → the app icon PNGs, the ℵ mask, the Raycast icon
//!   fetch [--force]  download the Simple Icons SVGs in the table that are missing (all of
//!                  them with --force) with curl, from cdn.jsdelivr.net at the pinned versions
//!   show <id>      print a logo's two bitmaps as text, to check a new one
//!   sheet <out.png>  every logo at 64 px on the app's dark and light backgrounds
//!
//! To add a logo: add a row to src/sources.rs (a Simple Icons slug, or the letters of a
//! monogram), run `fetch` for a Simple Icons one, then `all`, and commit what changed.
//! The outputs are deterministic: re-running it changes nothing unless a source did.

mod mono;
mod pack;
mod sources;

use pack::Rgb;
use resvg::tiny_skia::Pixmap;
use sources::{From, Src};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const KINDS: &[&str] = &["os", "agent", "gpu", "server"];

fn render_data(data: &[u8], name: &str, w: u32, h: u32) -> Pixmap {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(data, &opt).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    let s = tree.size();
    let scale = (w as f32 / s.width()).min(h as f32 / s.height());
    let (dx, dy) = (
        (w as f32 - s.width() * scale) / 2.0,
        (h as f32 - s.height() * scale) / 2.0,
    );
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy),
        &mut pm.as_mut(),
    );
    pm
}

fn render(svg: &Path, w: u32, h: u32) -> Pixmap {
    let data = fs::read(svg).unwrap_or_else(|e| panic!("{}: {e}", svg.display()));
    render_data(&data, &svg.display().to_string(), w, h)
}

fn save(pm: &Pixmap, out: &Path) {
    if let Some(d) = out.parent() {
        fs::create_dir_all(d).unwrap();
    }
    pm.save_png(out)
        .unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    println!("  {}", out.display());
}

/// Write `text` unless the file already holds it (keeps mtimes, and git, quiet).
fn write(path: &Path, text: &str) {
    if fs::read_to_string(path).ok().as_deref() == Some(text) {
        return;
    }
    if let Some(d) = path.parent() {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("  {}", path.display());
}

// ------------------------------------------------------------------ brand

fn brand(root: &Path) {
    let brand = root.join("assets/brand");
    let png = brand.join("png");
    println!("brand:");
    for n in [16, 32, 64, 128, 180, 192, 256, 512, 1024] {
        save(
            &render(&brand.join("aleph-app.svg"), n, n),
            &png.join(format!("aleph-app-{n}.png")),
        );
    }
    for n in [192, 512] {
        save(
            &render(&brand.join("aleph-maskable.svg"), n, n),
            &png.join(format!("aleph-maskable-{n}.png")),
        );
    }
    save(
        &render(&brand.join("aleph.svg"), 512, 512),
        &png.join("aleph-ink-512.png"),
    );
    // the terminal renderer's mask: alpha only, 64×64, row-major
    let m = render(&brand.join("aleph-mark.svg"), 64, 64);
    let alpha: Vec<u8> = m.pixels().iter().map(|p| p.alpha()).collect();
    fs::write(brand.join("aleph-mask-64.bin"), &alpha).unwrap();
    println!("  {}", brand.join("aleph-mask-64.bin").display());
    let ray = root.join("extras/raycast-extension/assets/extension-icon.png");
    if ray.parent().is_some_and(|d| d.is_dir()) {
        fs::copy(png.join("aleph-app-512.png"), &ray).unwrap();
        println!("  {}", ray.display());
    }
}

// ------------------------------------------------------------------ fetch

fn si_url(ver: &str, slug: &str) -> String {
    format!("https://cdn.jsdelivr.net/npm/simple-icons@{ver}/icons/{slug}.svg")
}

fn svg_path(root: &Path, s: &Src) -> PathBuf {
    root.join("assets/logos")
        .join(s.kind)
        .join(format!("{}.svg", s.id))
}

fn fetch(root: &Path, force: bool) {
    println!("fetch:");
    for s in sources::ALL {
        let From::Si(ver, slug) = s.from else {
            continue;
        };
        let path = svg_path(root, s);
        if path.exists() && !force {
            continue;
        }
        let url = si_url(ver, slug);
        let out = std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "30", &url])
            .output()
            .unwrap_or_else(|e| panic!("curl: {e}"));
        let body = String::from_utf8_lossy(&out.stdout).to_string();
        // a Simple Icons file is one <svg> with a 24×24 viewBox and a path, nothing active
        if !out.status.success()
            || !body.starts_with("<svg")
            || !body.contains("viewBox=\"0 0 24 24\"")
            || body.contains("<script")
            || body.contains("href")
        {
            panic!("{url}: not a Simple Icons SVG ({})", out.status);
        }
        write(&path, &body);
    }
}

// ------------------------------------------------------------------ logos

/// One logo, packed.
struct Packed {
    src: &'static Src,
    pal: Vec<Rgb>,
    x16: String,
    x32: String,
}

fn kind_name(k: &str) -> &'static str {
    match k {
        "os" => "Os",
        "agent" => "Agent",
        "gpu" => "Gpu",
        "server" => "Server",
        _ => panic!("unknown kind {k}"),
    }
}

/// A colour for a background where `c` would barely show, else `c` itself.
fn for_bg(c: Rgb, bg: Rgb, text: Rgb) -> Rgb {
    if pack::contrast(c, bg) >= 3.0 {
        c
    } else {
        pack::fit(c, bg, text)
    }
}

fn pack_one(root: &Path, s: &'static Src) -> Packed {
    let path = svg_path(root, s);
    let brand = pack::parse_hex(s.color);
    let big = render(&path, 128, 128);
    let found = pack::palette(&big, [0, 0, 0]);
    // no colours of its own (Simple Icons): its ink is black, drawn in the brand colour
    let (from, pal) = if found == [[0, 0, 0]] {
        (vec![[0, 0, 0]], vec![brand])
    } else {
        (found.clone(), found)
    };
    let x = |n: u32| {
        let px = pack::indices(&render(&path, n, n), &render(&path, n * 4, n * 4), &from);
        let code = pack::rle(&px, pal.len());
        assert_eq!(pack::unrle(&code, pal.len(), px.len()), px, "{}", s.id);
        code
    };
    let dark: Vec<Rgb> = pal
        .iter()
        .map(|&c| for_bg(c, pack::DARK_BG, pack::DARK_TEXT))
        .collect();
    let light: Vec<Rgb> = pal
        .iter()
        .map(|&c| for_bg(c, pack::LIGHT_BG, pack::LIGHT_TEXT))
        .collect();
    let png = root.join("assets/logos/png");
    for n in [64u32, 128] {
        let pm = render(&path, n, n);
        save_once(
            &pack::indexed_png(&pm, &from, &pal),
            &png.join(format!("{}@{n}.png", s.id)),
        );
        // the app recolours the 128 at run time; the variants are for Raycast
        if n == 64 && dark != pal {
            save_once(
                &pack::indexed_png(&pm, &from, &dark),
                &png.join(format!("{}-dark@{n}.png", s.id)),
            );
        }
        if n == 64 && light != pal {
            save_once(
                &pack::indexed_png(&pm, &from, &light),
                &png.join(format!("{}-light@{n}.png", s.id)),
            );
        }
    }
    let (x16, x32) = (x(16), x(32));
    Packed {
        src: s,
        x16,
        x32,
        pal,
    }
}

/// The PNGs this run wrote, with their bytes.
static PNGS: std::sync::Mutex<Vec<(PathBuf, Vec<u8>)>> = std::sync::Mutex::new(Vec::new());

/// Save a PNG; two logos may share an id across kinds (Apple: an OS and a GPU
/// vendor) only when they're the same picture.
fn save_once(bytes: &[u8], out: &Path) {
    let bytes = bytes.to_vec();
    let mut done = PNGS.lock().unwrap();
    if let Some((_, b)) = done.iter().find(|(p, _)| p == out) {
        assert!(
            *b == bytes,
            "{} would hold two different pictures: give one of the logos another id",
            out.display()
        );
        return;
    }
    done.push((out.to_path_buf(), bytes.clone()));
    if fs::read(out).ok().as_deref() == Some(&bytes[..]) {
        return;
    }
    if let Some(d) = out.parent() {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(out, bytes).unwrap();
    println!("  {}", out.display());
}

/// Remove PNGs no logo makes any more (a renamed or dropped logo).
fn prune_pngs(dir: &Path) {
    let done = PNGS.lock().unwrap();
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "png") && !done.iter().any(|(d, _)| *d == p) {
            fs::remove_file(&p).unwrap();
            println!("  removed {}", p.display());
        }
    }
}

fn rgb_expr(c: Rgb) -> String {
    format!("Color::Rgb(0x{:02x}, 0x{:02x}, 0x{:02x})", c[0], c[1], c[2])
}

/// A long code as `concat!` pieces of at most 76 characters, one per line.
fn code_expr(code: &str, indent: &str) -> String {
    if code.len() <= 76 {
        return format!("\"{code}\"");
    }
    let mut s = String::from("concat!(\n");
    for chunk in code.as_bytes().chunks(76) {
        let _ = writeln!(
            s,
            "{indent}    \"{}\",",
            std::str::from_utf8(chunk).unwrap()
        );
    }
    s.push_str(indent);
    s.push(')');
    s
}

fn rust(all: &[Packed]) -> String {
    let mut s = String::new();
    s.push_str(concat!(
        "//! @generated by tools/logogen from assets/logos (sources, licences and colours:\n",
        "//! assets/logos/SOURCES.md). Don't edit: change the SVGs or the table in\n",
        "//! tools/logogen/src/sources.rs and run\n",
        "//! `cargo run --release --manifest-path tools/logogen/Cargo.toml`.\n",
        "//!\n",
        "//! Each logo: its brand colour, its palette (bitmap index k is `palette[k - 1]`,\n",
        "//! 0 is transparent) and two bitmaps, 16×16 and 32×32, row-major, run-length\n",
        "//! encoded: one character of `RUNS` per run, `index · span + (length − 1)` with\n",
        "//! `span = 64 / (palette.len() + 1)`. The PNG (the image tier's, 128 px) is\n",
        "//! indexed: `256 / palette.len()` alpha steps per colour, so it's recoloured by\n",
        "//! rewriting its PLTE chunk.\n",
        "\n",
        "use super::{Kind, Logo};\n",
        "use ratatui::style::Color;\n",
        "\n",
        "pub const RUNS: &[u8; 64] = b\"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-_\";\n",
        "\n",
        "pub const ALL: &[Logo] = &[\n",
    ));
    for p in all {
        let pal: Vec<String> = p.pal.iter().map(|&c| rgb_expr(c)).collect();
        let _ = write!(
            s,
            concat!(
                "    Logo {{\n",
                "        id: \"{id}\",\n",
                "        kind: Kind::{kind},\n",
                "        brand: {brand},\n",
                "        palette: &[{pal}],\n",
                "        x16: {x16},\n",
                "        x32: {x32},\n",
                "        png: include_bytes!(\"../assets/logos/png/{id}@128.png\"),\n",
                "    }},\n"
            ),
            id = p.src.id,
            kind = kind_name(p.src.kind),
            brand = rgb_expr(pack::parse_hex(p.src.color)),
            pal = pal.join(", "),
            x16 = code_expr(&p.x16, "        "),
            x32 = code_expr(&p.x32, "        "),
        );
    }
    s.push_str("];\n");
    s
}

fn sources_md(all: &[Packed]) -> String {
    let mut s = String::from(concat!(
        "# Logo sources\n",
        "\n",
        "Logos identify the operating system, GPU, agent or LLM server a session runs on or\n",
        "with. The marks are trademarks of their owners, used here only to name the software\n",
        "or hardware they stand for (nominative use), never to suggest endorsement.\n",
        "\n",
        "* **Simple Icons** files are fetched at the exact package version in the URL. Their SVG\n",
        "  data is CC0 1.0 unless the licence column names another one (Simple Icons records\n",
        "  those per icon); bitmaps and PNGs made from such a file carry its licence.\n",
        "* **Monograms** are drawn here, by `tools/logogen` (its own stroke font, no font files):\n",
        "  the letters cut out of a rounded square for agents and of a circle for servers, in\n",
        "  the colour the app already uses for them. MIT, like the rest of this repo.\n",
        "* **Colours** are Simple Icons' `hex` unless noted. Black, white and grey marks are drawn\n",
        "  in the theme's text colour; a colour that wouldn't show on the background is made\n",
        "  lighter or darker (same hue). The `-dark` / `-light` PNGs are those variants.\n",
        "\n",
        "This file is written by `tools/logogen` from `tools/logogen/src/sources.rs`; add a row\n",
        "there for every new logo.\n",
        "\n",
        "| id | kind | identifies | source | licence | colour | note |\n",
        "|---|---|---|---|---|---|---|\n",
    ));
    for p in all {
        let src = p.src;
        let (from, lic) = match src.from {
            From::Si(ver, slug) => (
                format!("[Simple Icons {ver} `{slug}`]({})", si_url(ver, slug)),
                if src.license.is_empty() {
                    "CC0-1.0".to_string()
                } else {
                    src.license.to_string()
                },
            ),
            From::Mono(t) => (format!("monogram “{t}”, drawn here"), "MIT".into()),
            From::Drawn(_) => ("drawn here".into(), "MIT".into()),
        };
        let _ = writeln!(
            s,
            "| `{}` | {} | {} | {} | {} | `{}` | {} |",
            src.id, src.kind, src.what, from, lic, src.color, src.note
        );
    }
    let n = |k: &str| all.iter().filter(|p| p.src.kind == k).count();
    let _ = writeln!(
        s,
        "\n{} logos: {} systems, {} agents, {} GPU vendors, {} servers and gateways.",
        all.len(),
        n("os"),
        n("agent"),
        n("gpu"),
        n("server")
    );
    s
}

fn logos(root: &Path) {
    println!("logos:");
    let dir = root.join("assets/logos");
    // every row unique, every SVG in a row
    for (i, a) in sources::ALL.iter().enumerate() {
        assert!(KINDS.contains(&a.kind), "{}: unknown kind {}", a.id, a.kind);
        for b in &sources::ALL[..i] {
            assert!(
                !(a.kind == b.kind && a.id == b.id),
                "{}/{} is in the table twice",
                a.kind,
                a.id
            );
        }
    }
    for k in KINDS {
        let Ok(rd) = fs::read_dir(dir.join(k)) else {
            continue;
        };
        let mut names: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".svg"))
            .collect();
        names.sort();
        for n in names {
            let id = n.trim_end_matches(".svg");
            assert!(
                sources::ALL.iter().any(|s| s.kind == *k && s.id == id),
                "assets/logos/{k}/{n} has no row in tools/logogen/src/sources.rs"
            );
        }
    }
    // the logos drawn here
    for s in sources::ALL {
        match s.from {
            From::Mono(text) => write(
                &svg_path(root, s),
                &mono::monogram(s.what, text, s.color, s.kind == "server"),
            ),
            From::Drawn(body) => write(&svg_path(root, s), &mono::drawn(s.what, body)),
            From::Si(..) => assert!(
                svg_path(root, s).exists(),
                "{} is missing: run `logogen fetch`",
                svg_path(root, s).display()
            ),
        }
    }
    let all: Vec<Packed> = sources::ALL.iter().map(|s| pack_one(root, s)).collect();
    prune_pngs(&dir.join("png"));
    let data = root.join("src/logo_data.rs");
    write(&data, &rustfmt(rust(&all)));
    write(&dir.join("SOURCES.md"), &sources_md(&all));
    let size = fs::metadata(&data).map(|m| m.len()).unwrap_or(0);
    println!("  {} logos, logo_data.rs {} KB", all.len(), size / 1024);
}

/// `code` in rustfmt's layout, so `cargo fmt` leaves the file alone (as it
/// is when rustfmt isn't installed).
fn rustfmt(code: String) -> String {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new("rustfmt")
        .args(["--edition", "2021", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    else {
        return code;
    };
    let mut stdin = child.stdin.take().unwrap();
    let input = code.clone();
    let feed = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let out = child.wait_with_output();
    let _ = feed.join();
    match out {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => {
            String::from_utf8(o.stdout).unwrap_or(code)
        }
        _ => code,
    }
}

/// A contact sheet of every logo at 64 px, on the app's dark background (top)
/// and its light one (bottom), in the colours the app draws them in there.
fn sheet(root: &Path, out: &Path) {
    use resvg::tiny_skia::{Color, PixmapPaint, PremultipliedColorU8, Transform};
    let (cols, cell) = (12u32, 72u32);
    let rows = sources::ALL.len().div_ceil(cols as usize) as u32;
    let mut pm = Pixmap::new(cols * cell, rows * cell * 2).unwrap();
    for (half, bg, text) in [
        (0, pack::DARK_BG, pack::DARK_TEXT),
        (1, pack::LIGHT_BG, pack::LIGHT_TEXT),
    ] {
        let mut fill = Pixmap::new(cols * cell, rows * cell).unwrap();
        fill.fill(Color::from_rgba8(bg[0], bg[1], bg[2], 255));
        pm.draw_pixmap(
            0,
            (half * rows * cell) as i32,
            fill.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
        for (i, s) in sources::ALL.iter().enumerate() {
            let path = svg_path(root, s);
            let mut l = render(&path, 64, 64);
            let found = pack::palette(&render(&path, 128, 128), [0, 0, 0]);
            let to: Vec<Rgb> = if found == [[0, 0, 0]] {
                vec![pack::fit(pack::parse_hex(s.color), bg, text)]
            } else {
                found.iter().map(|&c| pack::fit(c, bg, text)).collect()
            };
            for p in l.pixels_mut() {
                let a = p.alpha();
                if a == 0 {
                    continue;
                }
                let c = p.demultiply();
                let c = [c.red(), c.green(), c.blue()];
                let k = (0..found.len())
                    .min_by_key(|&k| pack::dist(found[k], c))
                    .unwrap_or(0);
                let m = |v: u8| ((v as u16 * a as u16 + 127) / 255) as u8;
                let [r, g, b] = to[k.min(to.len() - 1)];
                *p = PremultipliedColorU8::from_rgba(m(r), m(g), m(b), a).unwrap();
            }
            let (x, y) = (i as u32 % cols, i as u32 / cols + half * rows);
            pm.draw_pixmap(
                (x * cell + 4) as i32,
                (y * cell + 4) as i32,
                l.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
        }
    }
    save(&pm, out);
}

fn show(root: &Path, id: &str) {
    for s in sources::ALL.iter().filter(|s| s.id == id) {
        let path = svg_path(root, s);
        for n in [16u32, 32] {
            let pm = render(&path, n, n);
            let found = pack::palette(&render(&path, 128, 128), [0, 0, 0]);
            let px = pack::indices(&pm, &render(&path, n * 4, n * 4), &found);
            println!("{}/{} {n}×{n} {:?}", s.kind, s.id, found);
            for row in px.chunks(n as usize) {
                let line: String = row
                    .iter()
                    .map(|&v| match v {
                        0 => "· ",
                        1 => "██",
                        2 => "▓▓",
                        3 => "▒▒",
                        _ => "░░",
                    })
                    .collect();
                println!("  {line}");
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmds = ["all", "logos", "brand", "fetch", "show", "sheet"];
    let (cmd, rest) = match args.first() {
        Some(c) if cmds.contains(&c.as_str()) => (c.as_str(), &args[1..]),
        _ => ("all", &args[..]),
    };
    let force = rest.iter().any(|a| a == "--force");
    let mut pos = rest.iter().filter(|a| !a.starts_with("--"));
    let first = pos.next().cloned();
    // `show` and `sheet` take their argument before the root
    let (root, arg) = if matches!(cmd, "show" | "sheet") {
        (pos.next().cloned(), first)
    } else {
        (first, None)
    };
    let root = root
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    match cmd {
        "fetch" => fetch(&root, force),
        "logos" => logos(&root),
        "brand" => brand(&root),
        "show" => show(&root, arg.as_deref().expect("show <id>")),
        "sheet" => sheet(&root, Path::new(arg.as_deref().expect("sheet <out.png>"))),
        _ => {
            logos(&root);
            brand(&root);
        }
    }
}
