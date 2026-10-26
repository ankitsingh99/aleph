//! Byte-exact remote path helpers. Remote names are not guaranteed to be UTF-8,
//! so paths are kept as raw bytes and only converted (lossily) for display.

/// Strip trailing slashes, but keep a lone "/".
pub fn normalize(p: &[u8]) -> Vec<u8> {
    trim(p).to_vec()
}

fn trim(p: &[u8]) -> &[u8] {
    let mut end = p.len();
    while end > 1 && p[end - 1] == b'/' {
        end -= 1;
    }
    &p[..end]
}

pub fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    let mut p = trim(dir).to_vec();
    if !p.ends_with(b"/") {
        p.push(b'/');
    }
    p.extend_from_slice(name);
    p
}

pub fn parent(p: &[u8]) -> Vec<u8> {
    let t = trim(p);
    match t.iter().rposition(|&b| b == b'/') {
        None | Some(0) => b"/".to_vec(),
        Some(i) => t[..i].to_vec(),
    }
}

pub fn basename(p: &[u8]) -> &[u8] {
    let t = trim(p);
    match t.iter().rposition(|&b| b == b'/') {
        Some(i) => &t[i + 1..],
        None => t,
    }
}

/// True if `p` is `root` or lies underneath it.
pub fn is_within(p: &[u8], root: &[u8]) -> bool {
    let root = trim(root);
    p == root || (p.starts_with(root) && (root == b"/" || p.get(root.len()) == Some(&b'/')))
}

/// Lossy, single-line display form (control characters become '?').
pub fn show(b: &[u8]) -> String {
    String::from_utf8_lossy(b)
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// ASCII-only escaping for the cache file: '%', control bytes and non-ASCII → %XX.
pub fn esc(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len());
    for &c in b {
        if c == b'%' || !(0x20..0x7f).contains(&c) {
            s.push_str(&format!("%{c:02X}"));
        } else {
            s.push(c as char);
        }
    }
    s
}

pub fn unesc(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_and_basename() {
        assert_eq!(parent(b"/a/b/c"), b"/a/b");
        assert_eq!(parent(b"/a/b/"), b"/a");
        assert_eq!(parent(b"/a"), b"/");
        assert_eq!(parent(b"/"), b"/");
        assert_eq!(basename(b"/a/b/c/"), b"c");
        assert_eq!(basename(b"/"), b"");
    }

    #[test]
    fn join_paths() {
        assert_eq!(join(b"/", b"x"), b"/x");
        assert_eq!(join(b"/a/", b"x"), b"/a/x");
        assert_eq!(join(b"/a", b"x y"), b"/a/x y");
    }

    #[test]
    fn within() {
        assert!(is_within(b"/a/b", b"/a"));
        assert!(is_within(b"/a", b"/a"));
        assert!(!is_within(b"/ab", b"/a"));
        assert!(is_within(b"/x/y", b"/"));
    }

    #[test]
    fn escape_roundtrip() {
        let raw: &[u8] = b"we%ird\tname\n\xff\xfe caf\xc3\xa9";
        let e = esc(raw);
        assert!(e.is_ascii() && !e.contains('\t') && !e.contains('\n'));
        assert_eq!(unesc(&e).unwrap(), raw);
        assert!(unesc("%zz").is_none());
    }
}
