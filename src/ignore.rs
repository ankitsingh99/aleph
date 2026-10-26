//! Folders hidden by default: dot-folders, and folders created by package
//! managers, build tools and caches. Both can be toggled in the UI (ctrl+o).
//!
//! The built-in list can be edited with the `ignore=` setting: a comma list
//! of `+name` (also ignore) and `-name` (never ignore) entries; a bare name
//! means `+`. A leading or trailing `*` makes it a suffix / prefix pattern,
//! e.g. `ignore=+data,-build,+*.ckpt,+tmp*`.

const EXACT: &[&[u8]] = &[
    // JavaScript / TypeScript
    b"node_modules",
    b"bower_components",
    b"jspm_packages",
    b"web_modules",
    b".pnpm-store",
    b".yarn",
    b".npm",
    b".next",
    b".nuxt",
    b".svelte-kit",
    b".parcel-cache",
    b".turbo",
    b".vite",
    b".angular",
    b".expo",
    b".docusaurus",
    b".output",
    // Python
    b"__pycache__",
    b"__pypackages__",
    b".venv",
    b"venv",
    b".tox",
    b".nox",
    b".eggs",
    b".mypy_cache",
    b".pytest_cache",
    b".ruff_cache",
    b".pytype",
    b".pyre",
    b".hypothesis",
    b".ipynb_checkpoints",
    b"site-packages",
    b"dist-packages",
    // Generic build output / coverage / caches
    b"build",
    b"dist",
    b"out",
    b"target",
    b"_build",
    b".build",
    b"coverage",
    b"htmlcov",
    b".nyc_output",
    b".cache",
    // JVM / Android / Apple
    b".gradle",
    b".m2",
    b".ivy2",
    b".kotlin",
    b".cxx",
    b"Pods",
    b"DerivedData",
    b"Carthage",
    b".swiftpm",
    // Native toolchains
    b"CMakeFiles",
    b"zig-cache",
    b".zig-cache",
    b"zig-out",
    // Haskell / Elixir / Dart / Elm / Go / PHP / Ruby
    b".stack-work",
    b"dist-newstyle",
    b"deps",
    b".elixir_ls",
    b".dart_tool",
    b".pub-cache",
    b"elm-stuff",
    b"vendor",
    b".bundle",
    // Infra / env managers
    b".terraform",
    b".serverless",
    b".aws-sam",
    b".direnv",
    b".devenv",
    // VCS / editors
    b".git",
    b".hg",
    b".svn",
    b".bzr",
    b".idea",
    b".vscode",
];
const PREFIX: &[&[u8]] = &[b"cmake-build-", b"bazel-"];
const SUFFIX: &[&[u8]] = &[b".egg-info"];

pub fn hidden(name: &[u8]) -> bool {
    name.first() == Some(&b'.')
}

/// Built-in list only; use `Rules` to honour the user's edits.
fn builtin(name: &[u8]) -> bool {
    EXACT.contains(&name)
        || PREFIX.iter().any(|p| name.starts_with(p))
        || SUFFIX.iter().any(|s| name.ends_with(s))
}

#[derive(Clone, Debug, PartialEq)]
enum Pat {
    Exact(Vec<u8>),
    Prefix(Vec<u8>),
    Suffix(Vec<u8>),
}

impl Pat {
    fn parse(s: &str) -> Option<Pat> {
        let b = s.as_bytes();
        if b.is_empty() || b.contains(&b'/') || b == b"*" {
            return None;
        }
        Some(if let Some(r) = b.strip_prefix(b"*") {
            Pat::Suffix(r.to_vec())
        } else if let Some(r) = b.strip_suffix(b"*") {
            Pat::Prefix(r.to_vec())
        } else {
            Pat::Exact(b.to_vec())
        })
    }
    fn hit(&self, name: &[u8]) -> bool {
        match self {
            Pat::Exact(x) => name == x.as_slice(),
            Pat::Prefix(x) => name.starts_with(x),
            Pat::Suffix(x) => name.ends_with(x),
        }
    }
    /// As a `find -name` glob.
    fn glob(&self) -> Vec<u8> {
        match self {
            Pat::Exact(x) => x.clone(),
            Pat::Prefix(x) => [x.as_slice(), b"*"].concat(),
            Pat::Suffix(x) => [b"*", x.as_slice()].concat(),
        }
    }
}

/// The built-in ignore list plus the user's `ignore=` edits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rules {
    add: Vec<Pat>,
    remove: Vec<Pat>,
}

impl Rules {
    /// Parse `ignore=` edits: `+data,-build,*.ckpt`. Unknown junk is skipped.
    pub fn parse(edits: &str) -> Rules {
        let mut r = Rules::default();
        for item in edits
            .split([',', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let (add, name) = match item.as_bytes()[0] {
                b'-' => (false, &item[1..]),
                b'+' => (true, &item[1..]),
                _ => (true, item),
            };
            if let Some(p) = Pat::parse(name.trim()) {
                // a later edit for the same pattern wins
                r.add.retain(|x| *x != p);
                r.remove.retain(|x| *x != p);
                if add {
                    r.add.push(p);
                } else {
                    r.remove.push(p);
                }
            }
        }
        r
    }

    pub fn ignored(&self, name: &[u8]) -> bool {
        if self.remove.iter().any(|p| p.hit(name)) {
            return false;
        }
        builtin(name) || self.add.iter().any(|p| p.hit(name))
    }

    /// `find -name` globs to prune on the server for deep search (dot-folders
    /// are pruned separately). A removed built-in is left out, so it's indexed.
    pub fn prune_globs(&self) -> Vec<Vec<u8>> {
        let built = EXACT
            .iter()
            .map(|x| Pat::Exact(x.to_vec()))
            .chain(PREFIX.iter().map(|x| Pat::Prefix(x.to_vec())))
            .chain(SUFFIX.iter().map(|x| Pat::Suffix(x.to_vec())));
        let mut v: Vec<Vec<u8>> = Vec::new();
        for p in built.chain(self.add.iter().cloned()) {
            let g = p.glob();
            // dot-names are covered by `.*`; removed patterns stay searchable
            if g.first() == Some(&b'.') || self.remove.contains(&p) || v.contains(&g) {
                continue;
            }
            // a pattern that would also catch a removed exact name
            let wild = !matches!(p, Pat::Exact(_));
            if wild
                && self
                    .remove
                    .iter()
                    .any(|r| matches!(r, Pat::Exact(n) if p.hit(n)))
            {
                continue;
            }
            v.push(g);
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches() {
        for n in [
            &b"node_modules"[..],
            b"__pycache__",
            b"target",
            b"foo.egg-info",
            b"cmake-build-debug",
            b"bazel-out",
            b".venv",
        ] {
            assert!(builtin(n), "{}", String::from_utf8_lossy(n));
        }
        for n in [
            &b"src"[..],
            b"models",
            b"node_modules2",
            b"my-build",
            b"AuroraMix_3.1",
        ] {
            assert!(!builtin(n), "{}", String::from_utf8_lossy(n));
        }
        assert!(hidden(b".config") && !hidden(b"config"));
    }

    #[test]
    fn edits() {
        let r = Rules::parse("+data, -build,*.ckpt,+tmp*,-,+,*,-bazel-out, junk/x");
        assert!(r.ignored(b"data") && r.ignored(b"node_modules"));
        assert!(!r.ignored(b"build"), "removed built-in");
        assert!(r.ignored(b"model.ckpt") && r.ignored(b"tmp_42"));
        assert!(!r.ignored(b"bazel-out") && r.ignored(b"bazel-bin"));
        assert!(!r.ignored(b"src") && !r.ignored(b"junk/x"));
        assert_eq!(Rules::parse(""), Rules::default());
        // later edits win
        assert!(!Rules::parse("+data,-data").ignored(b"data"));
        assert!(Rules::parse("-data,+data").ignored(b"data"));

        let g = r.prune_globs();
        let has = |x: &[u8]| g.iter().any(|y| y == x);
        assert!(has(b"node_modules") && has(b"data") && has(b"*.ckpt") && has(b"tmp*"));
        assert!(has(b"*.egg-info"));
        assert!(!has(b"build"), "removed names stay searchable");
        assert!(
            !has(b"bazel-*"),
            "a removed name under a built-in prefix keeps it unpruned"
        );
        assert!(!has(b".venv"), "dot-names are pruned by .*");
        assert!(g.iter().all(|x| !x.contains(&b'/')));
    }
}
