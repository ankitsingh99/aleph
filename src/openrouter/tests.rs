use super::*;

const KEY: &str = include_str!("../../tests/fixtures/openrouter/key.json");
const KEY_OLD: &str = include_str!("../../tests/fixtures/openrouter/key_old.json");
const CREDITS: &str = include_str!("../../tests/fixtures/openrouter/credits.json");
const ERR_401: &str = include_str!("../../tests/fixtures/openrouter/error_401.json");
const MODELS: &str = include_str!("../../tests/fixtures/openrouter/models.json");

/// What a real key looks like (made up).
const FAKE: &str = "sk-or-v1-0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab";

fn v(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

#[test]
fn spend_is_read_by_field_name() {
    let mut s = Spend::default();
    parse_key(&v(KEY), &mut s);
    parse_credits(&v(CREDITS), &mut s);
    assert_eq!(s.usage, Some(40.1032));
    assert_eq!(s.today, Some(0.4211));
    assert_eq!(s.week, Some(3.5));
    // a number sent as a string still counts
    assert_eq!(s.month, Some(12.75));
    assert_eq!((s.limit, s.limit_left), (Some(50.0), Some(37.25)));
    assert_eq!(s.limit_reset.as_deref(), Some("monthly"));
    assert_eq!(s.free_tier, Some(false));
    // OpenRouter's default label is a piece of the key: never kept
    assert_eq!(s.label, None);
    assert_eq!((s.credits, s.credits_used), (Some(100.5), Some(40.1032)));
    assert!((s.left().unwrap() - 60.3968).abs() < 1e-9);
    // the key's limit is the headroom when it has one
    assert_eq!(s.headroom(), Some((37.25, 50.0)));
    // an older answer: no daily numbers, no limit, a label of your own
    let mut o = Spend::default();
    parse_key(&v(KEY_OLD), &mut o);
    assert_eq!((o.usage, o.today, o.limit), (Some(2.5), None, None));
    assert_eq!(o.label.as_deref(), Some("laptop"));
    assert!(o.any() && o.left().is_none() && o.headroom().is_none());
    // nothing that looks like spend: nothing read
    let mut n = Spend::default();
    parse_key(&json!({"data": {"hello": 1}}), &mut n);
    parse_credits(&json!([1, 2]), &mut n);
    assert!(!n.any());
}

#[test]
fn spend_keeps_and_reads_back() {
    let mut s = Spend {
        at: 1_000,
        ..Default::default()
    };
    parse_key(&v(KEY), &mut s);
    parse_credits(&v(CREDITS), &mut s);
    let j = s.to_json(1_000 + STALE + 1);
    assert_eq!(j["stale"], true);
    assert_eq!(j["spent"]["today"], 0.4211);
    assert_eq!(j["credits"]["left"], 60.3968);
    let back = Spend::from_json(&j).unwrap();
    assert_eq!(back, s);
    assert!(!back.stale(1_000 + 60));
}

#[test]
fn models_with_prices_per_million() {
    let m = parse_models(&v(MODELS));
    let ids: Vec<&str> = m.iter().map(|x| x.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "qwen/qwen3-coder",
            "anthropic/claude-sonnet-4.5",
            "qwen/qwen3-coder:free",
            "openrouter/auto",
            "z-ai/glm-4.6"
        ]
    );
    assert_eq!(m[0].context, 262_144);
    assert_eq!((m[0].input, m[0].output), (Some(0.22), Some(0.95)));
    assert_eq!(m[0].facts(), "262k ctx · $0.22/$0.95 per M");
    assert_eq!(m[1].facts(), "1M ctx · $3.00/$15 per M");
    assert_eq!(m[2].facts(), "262k ctx · free");
    // the router's price is variable: not said
    assert_eq!((m[3].input, m[3].facts().as_str()), (None, "2M ctx"));
    // numbers as numbers, the context from its top provider
    assert_eq!((m[4].context, m[4].input), (202_752, Some(0.6)));
    // kept and read back the same
    let kept = models_json(5, &m);
    assert_eq!(parse_models(&kept), m);
}

#[test]
fn models_are_fuzzy_found() {
    let m = parse_models(&v(MODELS));
    assert_eq!(search(&m, "").len(), m.len());
    let s = search(&m, "sonnet");
    assert_eq!(s[0].id, "anthropic/claude-sonnet-4.5");
    let s = search(&m, "qwen coder free");
    assert!(s.iter().any(|x| x.id == "qwen/qwen3-coder:free"));
    assert!(search(&m, "zzzz-none").is_empty());
}

#[test]
fn the_key_goes_on_curls_stdin_never_its_arguments() {
    let cfg = curl_config("https://openrouter.ai/api/v1/key", Some(FAKE)).unwrap();
    assert!(cfg.contains(&format!("header = \"Authorization: Bearer {FAKE}\"")));
    assert!(cfg.starts_with("url = \"https://openrouter.ai/api/v1/key\"\n"));
    // the command line itself: nothing of the key
    let args = curl_args();
    assert!(args
        .iter()
        .all(|a| !a.contains("sk-or") && !a.contains("Bearer")));
    assert!(args.windows(2).any(|w| w[0] == "-K" && w[1] == "-"));
    assert!(args.windows(2).any(|w| w[0] == "-m"));
    // the public model list: no header at all
    let pubc = curl_config("https://openrouter.ai/api/v1/models", None).unwrap();
    assert!(!pubc.contains("Authorization"));
    // a key that would break out of curl's quoting is refused
    for bad in ["a\"b", "x\ny", "back\\slash", "has space", ""] {
        assert!(curl_config("https://x/y", Some(bad)).is_none(), "{bad:?}");
    }
}

#[test]
fn answers_and_refusals_in_words() {
    assert_eq!(split_status("{\"a\":1}\n200"), ("{\"a\":1}", 200));
    assert_eq!(split_status("401"), ("", 401));
    let r = refusal(401, ERR_401);
    assert!(
        r.contains("refused the key") && r.contains("aleph secret set openrouter"),
        "{r}"
    );
    let r = refusal(403, "{\"error\":{\"message\":\"Only management keys\"}}");
    assert!(r.contains("403") && r.contains("management keys"), "{r}");
    // an error that echoes a key back is masked
    let r = refusal(
        500,
        &format!("{{\"error\":{{\"message\":\"bad key {FAKE}\"}}}}"),
    );
    assert!(!r.contains(FAKE), "{r}");
}

#[test]
fn which_agents_get_the_key() {
    for a in ["opencode", "kilo", "pi", "omp", "crush", "hermes", "maki"] {
        assert!(wants_in(DEFAULT_AGENTS, a), "{a}");
        assert!(
            crate::agents::find(a).is_some(),
            "{a} isn't in the registry"
        );
    }
    for a in ["claude", "codex", "cline", "qwen", "shell", ""] {
        assert!(!wants_in(DEFAULT_AGENTS, a), "{a}");
    }
    assert!(wants_in("all", "claude") && !wants_in("all", "shell"));
    assert!(!wants_in("all,-claude", "claude") && wants_in("all,-claude", "codex"));
    assert!(!wants_in("off", "opencode") && !wants_in("opencode,off", "opencode"));
    // aliases name the agent
    assert!(wants_in("open-code", "opencode"));
}

#[test]
fn models_go_to_each_agent_its_own_way() {
    assert_eq!(
        model_args("opencode", "qwen/qwen3-coder").unwrap(),
        vec!["--model", "openrouter/qwen/qwen3-coder"]
    );
    // already written with the provider: not doubled
    assert_eq!(
        model_args("kilo", "openrouter/z-ai/glm-4.6").unwrap(),
        vec!["--model", "openrouter/z-ai/glm-4.6"]
    );
    assert_eq!(
        model_args("pi", "anthropic/claude-sonnet-4.5").unwrap(),
        vec![
            "--provider",
            "openrouter",
            "--model",
            "anthropic/claude-sonnet-4.5"
        ]
    );
    assert_eq!(
        model_args("omp", "qwen/qwen3-coder").unwrap(),
        vec!["--model", "openrouter/qwen/qwen3-coder"]
    );
    // the key, but no model at launch; or neither
    for a in ["crush", "hermes", "claude", "codex"] {
        assert!(model_args(a, "qwen/qwen3-coder").is_none(), "{a}");
        assert!(!takes_model(a), "{a}");
    }
    assert!(model_args("opencode", "two words").is_none());
    assert!(takes_model("opencode") && model_agents().contains(&"OpenCode".to_string()));
    // every agent that takes a model gets the key by default
    for (a, _) in MODEL_ARGS {
        assert!(wants_in(DEFAULT_AGENTS, a), "{a}");
    }
}

#[test]
fn no_key_no_session_env() {
    let cfg = Config::in_memory(&["openrouter.key=off"]);
    assert!(!configured(&cfg));
    assert!(for_agent(&cfg, "opencode").is_none());
    let cfg = Config::in_memory(&["openrouter.agents=off"]);
    assert!(for_agent(&cfg, "opencode").is_none());
    // a plain key in the settings (not the keystore) works, and is masked from then on
    let cfg = Config::in_memory(&[&format!("openrouter.key={FAKE}")]);
    forget();
    assert_eq!(for_agent(&cfg, "opencode").as_deref(), Some(FAKE));
    assert!(for_agent(&cfg, "claude").is_none());
    assert!(!crate::log::redact(&format!("x {FAKE} y")).contains(FAKE));
    forget();
}

#[test]
fn free_or_paid_only_as_opencodes_own_config() {
    let v: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/openrouter/models.json")).unwrap();
    let all = parse_models(&v);
    assert!(all
        .iter()
        .any(|m| m.id == "qwen/qwen3-coder:free" && is_free(m)));
    assert!(!is_free(
        all.iter().find(|m| m.id == "qwen/qwen3-coder").unwrap()
    ));
    let free: Value = serde_json::from_str(&opencode_config_for("free", &all).unwrap()).unwrap();
    assert_eq!(
        free["provider"]["openrouter"]["whitelist"],
        json!(["qwen/qwen3-coder:free"])
    );
    let paid: Value = serde_json::from_str(&opencode_config_for("paid", &all).unwrap()).unwrap();
    assert_eq!(
        paid["provider"]["openrouter"]["blacklist"],
        json!(["qwen/qwen3-coder:free"])
    );
    assert!(opencode_config_for("all", &all).is_none());
    assert!(
        opencode_config_for("free", &[]).is_none(),
        "no list: nothing to name"
    );
}
