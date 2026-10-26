//! Exact pane layouts: herdr's own tree, a BSP of `pane` and `split` nodes
//! (what `layout.export` returns and `layout.apply` takes). Built from
//! `new --tile N` (an even grid), a preset (`columns`, `rows`, `grid`, or
//! `layout = "herdr"` with a tree saved from a real tab) and the dashboard's
//! Tile; carried out by `Herdr::new_tab` / `Herdr::push_tree` — one socket
//! request when herdr's socket answers, else the same shape by splits or
//! moves (no socket, older herdr).
//!
//! A split's `ratio` is the share of its `first` child (herdr clamps it to
//! 0.1–0.9), the same as `pane split --ratio` and the wall's own tree.

use serde_json::{json, Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// side by side
    Right,
    /// stacked
    Down,
}

impl Dir {
    pub fn name(self) -> &'static str {
        match self {
            Dir::Right => "right",
            Dir::Down => "down",
        }
    }
    pub fn parse(s: &str) -> Option<Dir> {
        match s.trim().to_ascii_lowercase().as_str() {
            "right" | "horizontal" | "side" | "columns" => Some(Dir::Right),
            "down" | "vertical" | "stack" | "rows" => Some(Dir::Down),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tree<L> {
    Pane(L),
    Split {
        dir: Dir,
        /// share of `first`
        ratio: f32,
        first: Box<Tree<L>>,
        second: Box<Tree<L>>,
    },
}

fn clamp(r: f64) -> f32 {
    if r.is_finite() {
        (r as f32).clamp(0.1, 0.9)
    } else {
        0.5
    }
}

impl<L> Tree<L> {
    pub fn split(dir: Dir, ratio: f32, first: Tree<L>, second: Tree<L>) -> Tree<L> {
        Tree::Split {
            dir,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// The panes, in herdr's order (first before second, depth first).
    pub fn leaves(&self) -> Vec<&L> {
        match self {
            Tree::Pane(l) => vec![l],
            Tree::Split { first, second, .. } => {
                let mut v = first.leaves();
                v.extend(second.leaves());
                v
            }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Tree::Pane(_) => 1,
            Tree::Split { first, second, .. } => first.len() + second.len(),
        }
    }

    /// The pane that keeps the node's place when it's split (the first leaf).
    pub fn first(&self) -> &L {
        match self {
            Tree::Pane(l) => l,
            Tree::Split { first, .. } => first.first(),
        }
    }

    pub fn map<M>(&self, f: &mut impl FnMut(&L) -> M) -> Tree<M> {
        match self {
            Tree::Pane(l) => Tree::Pane(f(l)),
            Tree::Split {
                dir,
                ratio,
                first,
                second,
            } => Tree::Split {
                dir: *dir,
                ratio: *ratio,
                first: Box::new(first.map(f)),
                second: Box::new(second.map(f)),
            },
        }
    }

    /// Only the panes `keep` wants: a split that loses one side becomes the
    /// other side (it takes the whole area). None when nothing's left.
    pub fn prune(self, keep: &mut impl FnMut(&L) -> bool) -> Option<Tree<L>> {
        match self {
            Tree::Pane(l) => keep(&l).then_some(Tree::Pane(l)),
            Tree::Split {
                dir,
                ratio,
                first,
                second,
            } => match (first.prune(keep), second.prune(keep)) {
                (Some(a), Some(b)) => Some(Tree::split(dir, ratio, a, b)),
                (Some(x), None) | (None, Some(x)) => Some(x),
                (None, None) => None,
            },
        }
    }
}

/// `items` side by side (or stacked), equal shares: the first keeps 1/n,
/// the rest share what's left the same way.
pub fn chain<L>(mut items: Vec<Tree<L>>, dir: Dir) -> Option<Tree<L>> {
    if items.is_empty() {
        return None;
    }
    let n = items.len();
    let head = items.remove(0);
    Some(match chain(items, dir) {
        None => head,
        Some(rest) => Tree::split(dir, 1.0 / n as f32, head, rest),
    })
}

/// Panes `0..n` in a row (`Right`) or a column (`Down`).
pub fn line(n: usize, dir: Dir) -> Option<Tree<usize>> {
    chain((0..n).map(Tree::Pane).collect(), dir)
}

/// Panes `0..n` as an even grid: ⌈√n⌉ columns, the first ones a row taller
/// when it doesn't divide; numbered down each column, then across.
pub fn grid(n: usize) -> Option<Tree<usize>> {
    if n == 0 {
        return None;
    }
    let cols = (n as f64).sqrt().ceil() as usize;
    let (base, extra) = (n / cols, n % cols);
    let mut next = 0;
    let mut columns = Vec::new();
    for c in 0..cols {
        let rows = base + usize::from(c < extra);
        let col: Vec<Tree<usize>> = (next..next + rows).map(Tree::Pane).collect();
        next += rows;
        columns.extend(chain(col, Dir::Down));
    }
    chain(columns, Dir::Right)
}

// ------------------------------------------------------------------ herdr's JSON

/// What a new pane starts with (`layout.apply`'s pane node).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Leaf {
    pub cwd: Option<String>,
    /// plain environment (a preset's `env`): may also go on herdr's command
    /// line (`tab create --env`, `pane split --env`) when there's no socket
    pub env: Vec<(String, String)>,
    /// this pane's agent gets the OpenRouter key (`openrouter.rs`): only
    /// ever filled in by [`to_herdr_with`], into the one socket request —
    /// never in `env`, so never on a command line
    pub openrouter: bool,
}

/// The tree as `layout.apply`'s `root`, without a secret.
#[cfg(test)]
pub fn to_herdr(t: &Tree<Leaf>) -> Value {
    to_herdr_with(t, None)
}

/// The tree as `layout.apply`'s `root` (shells in every pane: no
/// `command`), with `secret` (name, value) in the environment of each pane
/// that asks for it ([`Leaf::openrouter`]) — pure, tested. The value lives
/// only in the JSON line sent over herdr's socket.
pub fn to_herdr_with(t: &Tree<Leaf>, secret: Option<(&str, &str)>) -> Value {
    match t {
        Tree::Pane(l) => {
            let mut m = Map::new();
            m.insert("type".into(), json!("pane"));
            if let Some(c) = &l.cwd {
                m.insert("cwd".into(), json!(c));
            }
            let mut env: Map<String, Value> =
                l.env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            if let (true, Some((k, v))) = (l.openrouter, secret) {
                env.insert(k.to_string(), json!(v));
            }
            if !env.is_empty() {
                m.insert("env".into(), Value::Object(env));
            }
            Value::Object(m)
        }
        Tree::Split {
            dir,
            ratio,
            first,
            second,
        } => json!({
            "type": "split",
            "direction": dir.name(),
            "ratio": round(*ratio),
            "first": to_herdr_with(first, secret),
            "second": to_herdr_with(second, secret),
        }),
    }
}

fn round(r: f32) -> f64 {
    (r as f64 * 1000.0).round() / 1000.0
}

/// A pane of a tree herdr sent (`layout.export` / `layout.apply`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Got {
    pub pane_id: Option<String>,
    pub cwd: Option<String>,
}

/// A tree, read defensively: a node with `first` and `second` is a split
/// (herdr's `direction` + `ratio`, or a preset's `right = 0.6` / `down =
/// 0.5`); anything else is a pane, turned into a leaf by `leaf`.
pub fn read<L>(v: &Value, leaf: &dyn Fn(&Value) -> Option<L>) -> Option<Tree<L>> {
    if let Some(o) = v.as_object() {
        if let (Some(a), Some(b)) = (o.get("first"), o.get("second")) {
            let num = |k: &str| o.get(k).and_then(Value::as_f64);
            let (dir, ratio) = match o.get("direction").and_then(Value::as_str) {
                Some(d) => (Dir::parse(d)?, num("ratio")),
                None if o.contains_key("right") => (Dir::Right, num("right")),
                None if o.contains_key("down") => (Dir::Down, num("down")),
                None => return None,
            };
            return Some(Tree::split(
                dir,
                clamp(ratio.unwrap_or(0.5)),
                read(a, leaf)?,
                read(b, leaf)?,
            ));
        }
    }
    leaf(v).map(Tree::Pane)
}

/// herdr's pane node → what we use of it.
fn got(v: &Value) -> Option<Got> {
    let o = v.as_object()?;
    let st = |k: &str| {
        o.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    Some(Got {
        pane_id: st("pane_id"),
        cwd: st("cwd"),
    })
}

/// The layout in a herdr answer, wherever it's nested: (tab id, tree).
pub fn layout_in(v: &Value) -> Option<(Option<String>, Tree<Got>)> {
    match v {
        Value::Object(m) => {
            if let Some(root) = m.get("root") {
                if let Some(t) = read(root, &got) {
                    let tab = m.get("tab_id").and_then(Value::as_str).map(String::from);
                    return Some((tab, t));
                }
            }
            m.values().find_map(layout_in)
        }
        Value::Array(a) => a.iter().find_map(layout_in),
        _ => None,
    }
}

// ------------------------------------------------------------------ presets

/// A preset's tree as TOML: `{ right = 0.6, first = "train", second = { down
/// = 0.5, first = "review", second = "eval" } }` — leaves name its panes.
pub fn to_toml(t: &Tree<String>) -> toml_edit::Value {
    match t {
        Tree::Pane(name) => name.as_str().into(),
        Tree::Split {
            dir,
            ratio,
            first,
            second,
        } => {
            let mut it = toml_edit::InlineTable::new();
            it.insert(dir.name(), round(*ratio).into());
            it.insert("first", to_toml(first));
            it.insert("second", to_toml(second));
            toml_edit::Value::InlineTable(it)
        }
    }
}

/// TOML (a table, an inline table, or a value) → JSON, for `read`.
pub fn toml_json(i: &toml_edit::Item) -> Value {
    use toml_edit::Item;
    match i {
        Item::Value(v) => value_json(v),
        Item::Table(t) => Value::Object(
            t.iter()
                .map(|(k, x)| (k.to_string(), toml_json(x)))
                .collect(),
        ),
        Item::ArrayOfTables(a) => Value::Array(
            a.iter()
                .map(|t| toml_json(&Item::Table(t.clone())))
                .collect(),
        ),
        Item::None => Value::Null,
    }
}

fn value_json(v: &toml_edit::Value) -> Value {
    use toml_edit::Value as T;
    match v {
        T::String(s) => json!(s.value()),
        T::Integer(n) => json!(n.value()),
        T::Float(f) => json!(f.value()),
        T::Boolean(b) => json!(b.value()),
        T::Datetime(d) => json!(d.value().to_string()),
        T::Array(a) => Value::Array(a.iter().map(value_json).collect()),
        T::InlineTable(t) => Value::Object(
            t.iter()
                .map(|(k, x)| (k.to_string(), value_json(x)))
                .collect(),
        ),
    }
}

/// A preset's leaf: a pane's name, its number (1-based), `{ pane = … }`, or
/// herdr's own pane node (its `label`).
pub fn name_leaf(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(o) => ["pane", "name", "label"]
            .iter()
            .find_map(|k| o.get(*k))
            .and_then(name_leaf),
        _ => None,
    }
}

/// `(train 60% | (review / eval))` — for plans and listings.
pub fn describe(t: &Tree<String>) -> String {
    match t {
        Tree::Pane(n) => n.clone(),
        Tree::Split {
            dir,
            ratio,
            first,
            second,
        } => {
            let pct = if (ratio - 0.5).abs() < 0.005 {
                String::new()
            } else {
                format!(" {:.0}%", ratio * 100.0)
            };
            let sep = if *dir == Dir::Right { "|" } else { "/" };
            format!("({}{pct} {sep} {})", describe(first), describe(second))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(t: &Tree<usize>) -> Vec<usize> {
        t.leaves().into_iter().copied().collect()
    }

    #[test]
    fn grids_and_lines() {
        // 4 → 2×2: two columns of two
        let g = grid(4).unwrap();
        assert_eq!(names(&g), vec![0, 1, 2, 3]);
        assert_eq!(
            g,
            Tree::split(
                Dir::Right,
                0.5,
                Tree::split(Dir::Down, 0.5, Tree::Pane(0), Tree::Pane(1)),
                Tree::split(Dir::Down, 0.5, Tree::Pane(2), Tree::Pane(3)),
            )
        );
        // 3 → two columns, the first a row taller (like the split grid)
        let g = grid(3).unwrap();
        assert_eq!(
            g,
            Tree::split(
                Dir::Right,
                0.5,
                Tree::split(Dir::Down, 0.5, Tree::Pane(0), Tree::Pane(1)),
                Tree::Pane(2),
            )
        );
        // 9 → 3×3, 1/3 then 1/2
        let g = grid(9).unwrap();
        assert_eq!(g.len(), 9);
        let Tree::Split { ratio, second, .. } = &g else {
            panic!()
        };
        assert!((ratio - 1.0 / 3.0).abs() < 1e-6);
        assert!(matches!(**second, Tree::Split { ratio, .. } if (ratio - 0.5).abs() < 1e-6));
        assert_eq!(grid(1), Some(Tree::Pane(0)));
        assert_eq!(grid(0), None);
        assert_eq!(line(3, Dir::Down).unwrap().len(), 3);
    }

    #[test]
    fn prunes_to_what_stays() {
        let g = grid(4).unwrap();
        let p = g.clone().prune(&mut |i| *i != 1).unwrap();
        // column one lost its second pane: the first takes the column
        assert_eq!(
            p,
            Tree::split(
                Dir::Right,
                0.5,
                Tree::Pane(0),
                Tree::split(Dir::Down, 0.5, Tree::Pane(2), Tree::Pane(3)),
            )
        );
        assert_eq!(g.prune(&mut |_| false), None);
    }

    #[test]
    fn herdr_round_trip() {
        let t = grid(3).unwrap().map(&mut |i| Leaf {
            cwd: Some(format!("/srv/{i}")),
            env: if *i == 0 {
                vec![("K".into(), "v".into())]
            } else {
                vec![]
            },
            openrouter: *i != 1,
        });
        let v = to_herdr(&t);
        assert_eq!(v["type"], "split");
        assert_eq!(v["direction"], "right");
        assert_eq!(v["first"]["first"]["env"]["K"], "v");
        assert!(v["second"].get("env").is_none());
        // without the secret, the flag adds nothing
        assert!(!v.to_string().contains("OPENROUTER"));
        // with it: only the panes that ask, beside their own env
        let s = to_herdr_with(&t, Some(("OPENROUTER_API_KEY", "sk-or-x")));
        assert_eq!(s["first"]["first"]["env"]["K"], "v");
        assert_eq!(s["first"]["first"]["env"]["OPENROUTER_API_KEY"], "sk-or-x");
        assert!(s["first"]["second"].get("env").is_none());
        assert_eq!(s["second"]["env"]["OPENROUTER_API_KEY"], "sk-or-x");
        // the leaves themselves never hold it (what the CLI fallback reads)
        assert!(t
            .leaves()
            .iter()
            .all(|l| l.env.iter().all(|(k, _)| k == "K")));
        // herdr's answer: the same shape with pane ids, nested anywhere
        let answer = json!({"id": "x", "result": {"type": "layout_apply", "layout": {
            "workspace_id": "w1", "tab_id": "w1:t9", "zoomed": false, "focused_pane_id": "w1:p7",
            "root": {"type": "split", "direction": "right", "ratio": 0.5,
                "first": {"type": "split", "direction": "down", "ratio": 0.5,
                    "first": {"type": "pane", "pane_id": "w1:p7", "cwd": "/srv/0"},
                    "second": {"type": "pane", "pane_id": "w1:p8"}},
                "second": {"type": "pane", "pane_id": "w1:p9", "label": "x", "command": ["sh"]}}}}});
        let (tab, tree) = layout_in(&answer).unwrap();
        assert_eq!(tab.as_deref(), Some("w1:t9"));
        let ids: Vec<String> = tree
            .leaves()
            .iter()
            .filter_map(|g| g.pane_id.clone())
            .collect();
        assert_eq!(ids, vec!["w1:p7", "w1:p8", "w1:p9"]);
        assert_eq!(tree.first().cwd.as_deref(), Some("/srv/0"));
        // nonsense is refused, not guessed
        assert!(layout_in(&json!({"result": {"type": "ok"}})).is_none());
        assert!(read(
            &json!({"type": "split", "direction": "sideways", "first": {}, "second": {}}),
            &got
        )
        .is_none());
    }

    #[test]
    fn toml_round_trip_and_describe() {
        let t = Tree::split(
            Dir::Right,
            0.6,
            Tree::Pane("train".to_string()),
            Tree::split(
                Dir::Down,
                0.5,
                Tree::Pane("review".into()),
                Tree::Pane("eval".into()),
            ),
        );
        let text = format!("tree = {}\n", to_toml(&t));
        let doc: toml_edit::DocumentMut = text.parse().unwrap();
        let back = read(&toml_json(&doc["tree"]), &name_leaf).unwrap();
        assert_eq!(back, t);
        assert_eq!(describe(&t), "(train 60% | (review / eval))");
        // the same tree written as tables, numbers for panes
        let doc: toml_edit::DocumentMut =
            "[tree]\nright = 0.6\nfirst = 1\n[tree.second]\ndown = 0.5\nfirst = { pane = \"review\" }\nsecond = 3\n"
                .parse()
                .unwrap();
        let t2 = read(&toml_json(&doc["tree"]), &name_leaf).unwrap();
        assert_eq!(
            t2.leaves().into_iter().cloned().collect::<Vec<_>>(),
            vec!["1", "review", "3"]
        );
    }
}
