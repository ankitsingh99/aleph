//! JSON on its way out, masked by what each field is: ids and states pass,
//! versions get whimsical ones, accounts and secrets go, every other string
//! is scrubbed (`scan`). Maps keyed by a machine get the machine's fake as
//! the key. The shape never changes, so every client reads it as before.

use super::{dict, fake, scan};
use crate::showcase::Kind;
use serde_json::{Map, Value};

/// Fields that are never private (ids keep clients' state working: dismiss,
/// read marks), and numbers' names.
fn passes(k: &str) -> bool {
    k == "id"
        || k == "ids"
        || k.ends_with("_id")
        || matches!(
            k,
            "os" | "os_name"
                | "kernel"
                | "cpu_model"
                | "status"
                | "kind"
                | "agent"
                | "model"
                | "models"
                | "gen"
                | "t"
                | "ts"
                | "seq"
                | "color"
                | "glyph"
                | "hue"
                | "mark"
                | "scope"
                | "code"
                | "pid"
                | "type"
                | "state"
                | "source"
                | "plan"
                | "effort"
                | "method"
                | "vendor"
                | "platform"
                | "unit"
                | "health"
                // your own words (showcase.watermark): never masked
                | "watermark"
        )
}

fn versionish(k: &str) -> bool {
    k == "version" || k.ends_with("_version") || k == "aleph" || k == "herdr"
}

fn secret(k: &str) -> bool {
    matches!(
        k,
        "token" | "secret" | "password" | "endpoint" | "topic" | "code" | "pairing"
    ) || crate::configcmd::is_secret(k)
}

fn walk(d: &dict::Dict, key: &str, v: Value) -> Value {
    match v {
        Value::String(s) if passes(key) && key != "code" => Value::String(s),
        Value::String(s) if key == "account" => {
            // an account's hash: another of the same length
            let x = fake::seed(&d.salt, Kind::User, &s, 0);
            Value::String(
                (0..s.len())
                    .map(|i| {
                        char::from_digit(((x >> ((i * 4) % 60)) & 15) as u32, 16).unwrap_or('0')
                    })
                    .collect(),
            )
        }
        Value::String(s)
            if versionish(key) && s.chars().next().is_some_and(|c| c.is_ascii_digit()) =>
        {
            Value::String(fake::version(&s))
        }
        Value::String(s) if secret(key) && !s.is_empty() => Value::String("••••".into()),
        Value::String(s) => Value::String(scan::scrub_with(d, &s)),
        Value::Array(a) => Value::Array(a.into_iter().map(|x| walk(d, key, x)).collect()),
        Value::Object(o) => {
            let mut m = Map::with_capacity(o.len());
            for (k, x) in o {
                let x = walk(d, &k, x);
                // a map keyed by a machine (or a session): its fake as the key
                let k2 = match d.fake_of(&k) {
                    Some(f) if !passes(&k) => f.to_string(),
                    _ => k,
                };
                m.insert(k2, x);
            }
            Value::Object(m)
        }
        // safety: a $ amount scaled or hidden (`showcase.money`)
        Value::Number(n) if super::safety::money_key(key) => super::safety::money_json(&n),
        other => other,
    }
}

pub fn mask(v: Value) -> Value {
    let d = dict::get();
    walk(&d, "", v)
}
