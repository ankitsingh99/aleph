//! fzf-style fuzzy matching for folder names: the query's characters must
//! appear in order; scores reward matches at word starts (`_ - . space`,
//! camelCase, letter→digit), consecutive runs and early positions, and
//! penalise gaps. Smart case: lowercase queries ignore case.
//! Returns the best score plus the matched char positions (for highlighting).

const MATCH: i32 = 16;
const GAP_START: i32 = 3;
const GAP_EXT: i32 = 1;
const B_START: i32 = 10;
const B_BOUNDARY: i32 = 9;
const B_CAMEL: i32 = 8;
const B_DIGIT: i32 = 6;
const B_CONSECUTIVE: i32 = 5;
const NONE: i32 = i32::MIN / 4;

fn bonus(prev: Option<char>, cur: char) -> i32 {
    match prev {
        None => B_START,
        Some(p) if !p.is_alphanumeric() => B_BOUNDARY,
        Some(p) if p.is_lowercase() && cur.is_uppercase() => B_CAMEL,
        Some(p) if p.is_alphabetic() && cur.is_ascii_digit() => B_DIGIT,
        _ => 0,
    }
}

/// `Some((score, positions))` if every query char appears in order in `cand`.
pub fn score(query: &str, cand: &str) -> Option<(i32, Vec<usize>)> {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some((0, vec![]));
    }
    let c: Vec<char> = cand.chars().collect();
    let (m, n) = (q.len(), c.len());
    if m > n {
        return None;
    }
    let smart = q.iter().any(|ch| ch.is_uppercase());
    let eq = |a: char, b: char| {
        if smart {
            a == b
        } else {
            a.to_lowercase().eq(b.to_lowercase())
        }
    };
    // quick reject: subsequence?
    let mut k = 0;
    for &ch in &c {
        if k < m && eq(q[k], ch) {
            k += 1;
        }
    }
    if k < m {
        return None;
    }

    // dp[i][j]: best score with q[i] matched at c[j]; from[i][j]: where q[i-1] matched
    let mut dp = vec![vec![NONE; n]; m];
    let mut from = vec![vec![usize::MAX; n]; m];
    for i in 0..m {
        // running best over k ≤ j-2 of dp[i-1][k] minus gap penalty to reach j
        let (mut g, mut gk) = (NONE, usize::MAX);
        for j in i..n {
            if i > 0 && j >= 2 {
                let cand_k = dp[i - 1][j - 2];
                let aged = if g == NONE { NONE } else { g - GAP_EXT };
                if cand_k != NONE && cand_k - GAP_START >= aged {
                    g = cand_k - GAP_START;
                    gk = j - 2;
                } else {
                    g = aged;
                }
            }
            if !eq(q[i], c[j]) {
                continue;
            }
            let b = bonus(if j == 0 { None } else { Some(c[j - 1]) }, c[j]);
            let s = MATCH + if i == 0 { b * 2 } else { b } + if q[i] == c[j] { 1 } else { 0 };
            if i == 0 {
                // prefer early first matches a little
                dp[0][j] = s - (j as i32).min(12);
                continue;
            }
            let consec = if j >= 1 && dp[i - 1][j - 1] != NONE {
                dp[i - 1][j - 1] + B_CONSECUTIVE
            } else {
                NONE
            };
            let (best, pk) = if consec >= g {
                (consec, j.wrapping_sub(1))
            } else {
                (g, gk)
            };
            if best != NONE {
                dp[i][j] = s + best;
                from[i][j] = pk;
            }
        }
    }
    let (mut j, best) = (0..n)
        .map(|j| (j, dp[m - 1][j]))
        .max_by_key(|&(j, s)| (s, std::cmp::Reverse(j)))?;
    if best == NONE {
        return None;
    }
    let mut pos = vec![0; m];
    for i in (0..m).rev() {
        pos[i] = j;
        if i > 0 {
            j = from[i][j];
        }
    }
    // shorter names win ties
    Some((best - (n as i32) / 8, pos))
}

/// Extra credit for each matched char that starts a path component.
const B_COMPONENT: i32 = 4;
/// The last component is exactly the query.
const B_EXACT: i32 = 8;
/// Matching across components instead of within the last one.
const P_SPREAD: i32 = 12;
/// Per extra level below the first.
const P_DEPTH: i32 = 3;

/// Score a relative path (`a/b/name`) from the deep index. Without a `/` in
/// the query the last component is matched (like a plain name, positions
/// shifted into the full path); failing that, the whole path is tried at a
/// penalty, so `pxsrc` still finds `px_color/src`. A query with `/` always
/// matches the whole path. Matches that start a component earn a bonus;
/// deeper paths lose a little.
pub fn score_path(query: &str, rel: &str) -> Option<(i32, Vec<usize>)> {
    let depth = rel.matches('/').count() as i32;
    let starts: Vec<usize> = std::iter::once(0)
        .chain(
            rel.chars()
                .enumerate()
                .filter(|(_, c)| *c == '/')
                .map(|(i, _)| i + 1),
        )
        .collect();
    let comp_bonus = |pos: &[usize]| -> i32 {
        pos.iter().filter(|p| starts.contains(p)).count() as i32 * B_COMPONENT
    };
    let cut = rel.rfind('/').map_or(0, |i| i + 1);
    let shift = rel[..cut].chars().count();
    if !query.contains('/') {
        let last = &rel[cut..];
        if let Some((s, pos)) = score(query, last) {
            let pos: Vec<usize> = pos.into_iter().map(|p| p + shift).collect();
            // the whole component, not just a prefix of it
            let exact = if last.to_lowercase() == query.to_lowercase() {
                B_EXACT
            } else {
                0
            };
            return Some((s + exact + comp_bonus(&pos) - depth * P_DEPTH, pos));
        }
    }
    let (s, pos) = score(query, rel)?;
    // the match has to reach the folder itself (or the `/` just before it),
    // or every sub-folder of a matching parent would match too
    if pos.last().is_none_or(|&p| p + 1 < shift) {
        return None;
    }
    let spread = if query.contains('/') { 0 } else { P_SPREAD };
    Some((s + comp_bonus(&pos) - spread - depth * P_DEPTH, pos))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank_paths<'a>(q: &str, paths: &[&'a str]) -> Vec<&'a str> {
        let mut v: Vec<(i32, &str)> = paths
            .iter()
            .filter_map(|n| score_path(q, n).map(|(s, _)| (s, *n)))
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
        v.into_iter().map(|x| x.1).collect()
    }

    const DEEP: &[&str] = &[
        "proj/src",
        "proj/resources/scripts",
        "proj/src/bin",
        "ox_color_check/src",
        "ox_color_check/tools/zap_probe",
        "work/blaze/docs",
        "work/models",
        "zap_plugins/srcfiles",
    ];

    #[test]
    fn deep_scoring() {
        // the last component decides, and shallower wins ties
        assert_eq!(
            &rank_paths("src", DEEP)[..2],
            &["proj/src", "ox_color_check/src"]
        );
        assert_eq!(
            rank_paths("trtp", DEEP)[0],
            "ox_color_check/tools/zap_probe"
        );
        assert_eq!(rank_paths("mod", DEEP)[0], "work/models");
        // positions point into the full path
        let (_, pos) = score_path("bin", "proj/src/bin").unwrap();
        assert_eq!(pos, vec![9, 10, 11]);
        // no last-component match: whole path at a penalty, boundaries count
        let (_, pos) = score_path("pxsrc", "ox_color_check/src").unwrap();
        assert_eq!(&pos[2..], &[15, 16, 17]);
        assert_eq!(rank_paths("pxsrc", DEEP)[0], "ox_color_check/src");
        // a query with '/' matches across components without the penalty
        assert_eq!(rank_paths("bl/d", DEEP)[0], "work/blaze/docs");
        assert!(score_path("zzz", "proj/src").is_none());
        // a match only in a parent doesn't drag in all its children
        assert!(score_path("sha", "share/X11").is_none());
        assert!(score_path("work", "work/models").is_none());
        assert!(score_path("proj/", "proj/src").is_some());
        // an exact last component beats a prefix match deeper down
        assert!(
            score_path("src", "proj/src").unwrap().0
                > score_path("src", "zap_plugins/srcfiles").unwrap().0
        );
    }

    fn rank<'a>(q: &str, names: &[&'a str]) -> Vec<&'a str> {
        let mut v: Vec<(i32, &str)> = names
            .iter()
            .filter_map(|n| score(q, n).map(|(s, _)| (s, *n)))
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
        v.into_iter().map(|x| x.1).collect()
    }

    const DIRS: &[&str] = &[
        "wind_sweep",
        "blaze",
        "kite-web-ui",
        "moss-env-check",
        "dev_ox",
        "fern_lab",
        "river_outputs_2026-01-02",
        "heron_zap_try",
        "zap10_outputs_test",
        "zap_bench_box",
        "zap_plugins",
        "zap_probe",
        "ox_2nd_copy",
        "ox_color_check",
        "ox_whistlers",
        "ox_meadow",
        "AuroraMix_3.1",
        "tide_info_probe",
        "leaf_similarity",
        "boot_time_probe",
    ];

    #[test]
    fn subsequence_required() {
        assert!(score("xyz", "blaze").is_none());
        assert!(score("bz", "blaze").is_some());
        assert_eq!(score("", "anything").unwrap().0, 0);
    }

    #[test]
    fn word_starts_beat_scattered() {
        assert_eq!(rank("pxc", DIRS)[0], "ox_color_check");
        assert_eq!(rank("pca", DIRS)[0], "ox_color_check");
        assert_eq!(rank("trtp", DIRS)[0], "zap_probe");
        assert_eq!(rank("mix", DIRS)[0], "AuroraMix_3.1");
        assert_eq!(rank("blaze", DIRS)[0], "blaze");
        assert_eq!(rank("cuda", DIRS)[0], "moss-env-check");
    }

    #[test]
    fn smart_case_and_positions() {
        assert!(score("PX", "ox_meadow").is_none()); // uppercase query is case-sensitive
        assert!(score("px", "AuroraMix_3.1").is_some());
        let (_, pos) = score("pxc", "ox_color_check").unwrap();
        assert_eq!(pos, vec![0, 1, 3]);
        let (_, pos) = score("mix", "AuroraMix_3.1").unwrap();
        assert_eq!(pos, vec![6, 7, 8]);
    }

    #[test]
    fn handles_unicode_and_long_names() {
        assert!(score("nai", "näive-ünïcode").is_none());
        assert!(score("nive", "näive-ünïcode").is_some());
        let long = "a".repeat(300) + "_target";
        assert!(score("tgt", &long).is_some());
    }
}
