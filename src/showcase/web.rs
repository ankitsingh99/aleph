//! The web console in showcase mode: a ribbon (`ℵ showcase` / `ℵ demo`)
//! with a rotating tagline, shown while the state snapshot (SSE, socket,
//! `state.snapshot`) or `/api/v1/me` carries the `showcase` marker
//! (`showcase::mark`). Its two files live only in this build;
//! the page's tags are added as index.html is served (the CSP allows
//! same-origin scripts and styles).

pub const ASSETS: &[(&str, &str, &[u8])] = &[
    (
        "/showcase/banner.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../../web/showcase/banner.js"),
    ),
    (
        "/showcase/banner.css",
        "text/css; charset=utf-8",
        include_bytes!("../../web/showcase/banner.css"),
    ),
];

const TAGS: &str = "<link rel=\"stylesheet\" href=\"/showcase/banner.css\">\n<script src=\"/showcase/banner.js\" defer></script>\n";

/// index.html with the banner's tags at the end of its head.
pub fn index(html: String) -> String {
    match html.find("</head>") {
        Some(i) => {
            let mut s = html;
            s.insert_str(i, TAGS);
            s
        }
        None => html,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_is_served_and_tagged() {
        let page = index("<html><head><title>x</title></head><body></body></html>".into());
        let head_end = page.find("</head>").unwrap();
        let css = page.find("/showcase/banner.css").unwrap();
        let js = page.find("/showcase/banner.js").unwrap();
        assert!(css < head_end && js < head_end, "{page}");
        assert!(page.contains("defer"));
        assert_eq!(index("no head".into()), "no head");
        for (p, ty, body) in ASSETS {
            assert!(p.starts_with("/showcase/"), "{p}");
            assert!(ty.starts_with("text/"));
            assert!(!body.is_empty());
        }
        // the script reads the marker the daemon sends, from both places
        let js = String::from_utf8_lossy(ASSETS[0].2);
        assert!(js.contains("s.snap.showcase") && js.contains("s.me.showcase"));
        // your watermark, from /api/v1/me (asked again now and then), as text
        assert!(js.contains("s.me.watermark") && js.contains("mark.textContent"));
        assert!(!js.contains("innerHTML"), "text only, never markup");
        // presenter sync: it asks the daemon, hears its topic, follows
        assert!(js.contains("presenter.get") && js.contains("topics: ['presenter']"));
        assert!(js.contains("following the presenter") && js.contains("sc-presented"));
    }
}
