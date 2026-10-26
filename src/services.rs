//! Local LLM servers and AI gateways on every machine — Ollama, LM Studio,
//! llama.cpp, vLLM, TGI, LiteLLM, SGLang, … — found by one read-only `sh -s`
//! probe per machine and described through one interface that the CLI, the
//! dashboard and the Raycast JSON all share:
//!
//! * [`ALL`] — the registry, one [`Service`] per kind of server: ports,
//!   process [`Rule`]s, health / models paths, a reader for its answers
//!   ([`Service::read`]), OpenAI-compatibility, homepage, brand glyph + colour;
//! * [`Found`] — one running server (port, process, version, [`Health`],
//!   [`Model`]s, VRAM, load), and [`Scan`] — a machine's result;
//! * [`probe`] / [`scan_all`] — run the probe; [`Watcher`] — the background
//!   per-host watcher (only with `services=on`), cached on disk;
//! * [`Scan::to_json`] / [`Found::to_json`] — the JSON every consumer reads;
//! * [`panel`], [`draw_scan`] and [`summary`] — pure ratatui renderers.
//!
//! **Discovery never guesses from a port number alone.** A service is reported
//! when its process runs (program name and command line: `ollama serve`,
//! `python -m vllm.entrypoints…`, `server.py` inside text-generation-webui) or
//! when its HTTP answers identify it (Ollama's `/api/tags` shape, vLLM's
//! `owned_by: vllm`, TGI's `/info` router, …). One probe ([`PROBE`]):
//!
//! 1. the remote prints its listening TCP sockets with their owners (`ss
//!    -ltnpH`, `lsof -nP -iTCP -sTCP:LISTEN`, `netstat`, `/proc/net/tcp`), the
//!    processes that own them or look like LLM servers, and NVIDIA compute apps;
//! 2. here ([`discover`]) every listening port is classified: owned by a
//!    registry process → that service; owned by a runtime or container proxy
//!    (python, node, docker-proxy), or by an unknown owner on a registry default
//!    port → worth asking; owned by anything else (ControlCenter on :5000,
//!    cef_server on :30000, java, an ssh forward) → left alone;
//! 3. the HTTP requests go back over the same ssh session (stdin) and the remote
//!    answers them with curl, wget or nc (`127.0.0.1`, never via a proxy, 2 s
//!    each, ports in parallel); a second round reads the details (loaded models,
//!    VRAM, metrics) of whatever the first one identified.
//!
//! Every tool may be missing: without a listener tool the default ports are
//! asked; without an HTTP client, running servers are reported with health
//! unknown. Tests point the probe at fixture files instead of the real tools
//! (`ALEPH_SERVICES_FIXTURES`, see `tests/fixtures/services/`).

use crate::config::Config;
use crate::sshcfg;
use crate::ui::{self, c};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read as _, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ registry

/// What a service is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// runs models (Ollama, vLLM, llama.cpp, …)
    LlmServer,
    /// routes to other servers or APIs (LiteLLM, llama-swap)
    Gateway,
    /// a web UI in front of servers (Open WebUI, text-generation-webui)
    Ui,
}

impl Kind {
    /// Stable name for JSON output.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::LlmServer => "llm-server",
            Kind::Gateway => "gateway",
            Kind::Ui => "ui",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::LlmServer => "LLM server",
            Kind::Gateway => "AI gateway",
            Kind::Ui => "web UI",
        }
    }
}

/// How a service's process is recognised.
///
/// `exe` is compared (case-insensitively) with every name a process goes by:
/// its `comm` (Linux truncates it to 15 characters), `argv[0]`, and for
/// interpreters the script or `-m` module they run (`python3 /venv/bin/vllm`
/// → `vllm`, `python -m sglang.launch_server` → `sglang.launch_server`,
/// `uvicorn open_webui.main:app` → `open_webui.main`). A trailing `*` matches
/// a prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub exe: &'static str,
    /// its first argument must be this subcommand (`serve`); a value starting
    /// with `.` instead means "some argument ends with it" (`.gguf`)
    pub arg: Option<&'static str>,
    /// one of these must appear in its command line or working folder
    pub inside: &'static [&'static str],
    /// a process matching this is a running server by itself. Apps and CLIs
    /// that only sometimes serve (the LM Studio app, `ollama run`) only count
    /// when they own a listening port the HTTP probe confirms.
    pub serves: bool,
}

/// Recognises a port's owner; running alone doesn't mean it serves.
const fn owns(exe: &'static str) -> Rule {
    Rule {
        exe,
        arg: None,
        inside: &[],
        serves: false,
    }
}
const fn owns_in(exe: &'static str, inside: &'static [&'static str]) -> Rule {
    Rule {
        exe,
        arg: None,
        inside,
        serves: false,
    }
}
/// A server whenever it runs.
const fn serves(exe: &'static str) -> Rule {
    Rule {
        exe,
        arg: None,
        inside: &[],
        serves: true,
    }
}
/// A server when started with this subcommand (`ollama serve`).
const fn serves_with(exe: &'static str, arg: &'static str) -> Rule {
    Rule {
        exe,
        arg: Some(arg),
        inside: &[],
        serves: true,
    }
}
/// A server when its command line or folder mentions one of `inside`.
const fn serves_in(exe: &'static str, inside: &'static [&'static str]) -> Rule {
    Rule {
        exe,
        arg: None,
        inside,
        serves: true,
    }
}

/// One kind of local LLM server, gateway or UI.
#[derive(Debug, Clone, Copy)]
pub struct Service {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    /// default ports, most common first
    pub ports: &'static [u16],
    pub procs: &'static [Rule],
    /// GET path that answers 200 when it's up (503 while loading)
    pub health: Option<&'static str>,
    /// where it lists its models
    pub models: Option<&'static str>,
    /// paths whose answers tell it apart from other servers (asked first)
    pub ident: &'static [&'static str],
    /// read once it's identified: version, loaded models, metrics. `POST …`
    /// sends `{}`; `!lms` runs `lms ps --json` on the machine.
    pub extra: &'static [&'static str],
    /// it serves one model, named on its command line (`--model …`)
    pub one_model: bool,
    /// speaks the OpenAI API (`/v1/chat/completions`, `/v1/models`)
    pub openai: bool,
    pub homepage: &'static str,
    /// single-width brand glyph, from the square family (boxes, and arrows
    /// for gateways) so a server never looks like an agent (circles, stars,
    /// diamonds, hexagons, triangles)
    pub glyph: &'static str,
    pub rgb: (u8, u8, u8),
    /// recognise it from its answers, and read what they say
    pub read: fn(&Http) -> Read,
}

const fn s(id: &'static str, name: &'static str, kind: Kind) -> Service {
    Service {
        id,
        name,
        kind,
        ports: &[],
        procs: &[],
        health: None,
        models: None,
        ident: &[],
        extra: &[],
        one_model: false,
        openai: true,
        homepage: "",
        glyph: "▪",
        rgb: (180, 180, 180),
        read: read_none,
    }
}

/// The registry, in the order services are listed.
pub const ALL: &[Service] = &[
    Service {
        ports: &[11434],
        // `Ollama` (the macOS app) and `ollama runner` (one per loaded model,
        // on a random port) are recognised but don't count by themselves
        procs: &[serves_with("ollama", "serve"), owns("ollama")],
        health: Some("/api/version"),
        models: Some("/api/tags"),
        ident: &["/api/version", "/api/tags"],
        extra: &["/api/ps"],
        homepage: "https://ollama.com",
        glyph: "▣",
        rgb: (236, 236, 236),
        read: read_ollama,
        ..s("ollama", "Ollama", Kind::LlmServer)
    },
    Service {
        ports: &[1234],
        // the app (and its helpers) and the headless daemon; the server is optional
        procs: &[owns("lm studio*"), owns("lm-studio*"), owns("llmster")],
        models: Some("/api/v0/models"),
        ident: &["/api/v0/models", "/v1/models"],
        extra: &["!lms"],
        homepage: "https://lmstudio.ai",
        glyph: "▢",
        rgb: (125, 104, 255),
        read: read_lmstudio,
        ..s("lmstudio", "LM Studio", Kind::LlmServer)
    },
    Service {
        ports: &[8080],
        // old builds called it `server`: only with a .gguf model on its command line
        procs: &[
            serves("llama-server"),
            serves("llamafile"),
            serves("*.llamafile"),
            Rule {
                exe: "server",
                arg: Some(".gguf"),
                inside: &[],
                serves: true,
            },
            serves("llama_cpp.server*"),
        ],
        health: Some("/health"),
        models: Some("/v1/models"),
        ident: &["/props", "/v1/models"],
        extra: &["/health", "/metrics"],
        one_model: true,
        homepage: "https://github.com/ggml-org/llama.cpp",
        glyph: "▦",
        rgb: (255, 140, 50),
        read: read_llamacpp,
        ..s("llamacpp", "llama.cpp", Kind::LlmServer)
    },
    Service {
        ports: &[8000],
        // not `vllm bench serve` (a client): the subcommand must come first
        procs: &[
            serves_with("vllm", "serve"),
            serves("vllm.entrypoints*"),
            owns("vllm"),
        ],
        health: Some("/health"),
        models: Some("/v1/models"),
        ident: &["/v1/models"],
        extra: &["/version", "/health", "/metrics"],
        one_model: true,
        homepage: "https://docs.vllm.ai",
        glyph: "▥",
        rgb: (253, 183, 35),
        read: read_vllm,
        ..s("vllm", "vLLM", Kind::LlmServer)
    },
    Service {
        ports: &[8080, 3000],
        procs: &[
            serves("text-generation-launcher"),
            serves("text-generation-router"),
            serves("text-generation-server"),
        ],
        health: Some("/health"),
        models: Some("/info"),
        ident: &["/info"],
        extra: &["/health"],
        one_model: true,
        homepage: "https://huggingface.co/docs/text-generation-inference",
        glyph: "⊞",
        rgb: (255, 210, 30),
        read: read_tgi,
        ..s("tgi", "TGI", Kind::LlmServer)
    },
    Service {
        ports: &[4000],
        procs: &[serves("litellm"), serves("litellm.proxy*")],
        // never `/health`: LiteLLM answers it by calling every upstream model
        health: Some("/health/liveliness"),
        models: Some("/v1/models"),
        ident: &["/health/readiness", "/health/liveliness"],
        extra: &["/v1/models"],
        homepage: "https://docs.litellm.ai",
        glyph: "⇄",
        rgb: (90, 170, 255),
        read: read_litellm,
        ..s("litellm", "LiteLLM", Kind::Gateway)
    },
    Service {
        ports: &[8080],
        procs: &[serves("local-ai"), serves("localai")],
        health: Some("/readyz"),
        models: Some("/v1/models"),
        ident: &["/system"],
        extra: &["/version", "/v1/models", "/readyz"],
        homepage: "https://localai.io",
        glyph: "⊡",
        rgb: (0, 200, 140),
        read: read_localai,
        ..s("localai", "LocalAI", Kind::LlmServer)
    },
    Service {
        ports: &[1337],
        procs: &[owns("jan"), owns("jan helper*"), owns("cortex*")],
        models: Some("/v1/models"),
        ident: &["/v1/models"],
        homepage: "https://jan.ai",
        glyph: "◰",
        rgb: (80, 140, 255),
        read: read_jan,
        ..s("jan", "Jan", Kind::LlmServer)
    },
    Service {
        ports: &[5000, 7860],
        procs: &[
            serves_in("server.py", &["text-generation-webui", "oobabooga"]),
            serves_in("one_click.py", &["text-generation-webui", "oobabooga"]),
        ],
        models: Some("/v1/internal/model/list"),
        ident: &["/v1/internal/model/info", "/config"],
        extra: &["/v1/internal/model/list"],
        one_model: true,
        homepage: "https://github.com/oobabooga/text-generation-webui",
        glyph: "▤",
        rgb: (255, 120, 70),
        read: read_tgwebui,
        ..s("tgwebui", "text-generation-webui", Kind::Ui)
    },
    Service {
        ports: &[5001],
        procs: &[serves("koboldcpp*")],
        models: Some("/api/v1/model"),
        ident: &["/api/extra/version"],
        extra: &["/api/v1/model", "/api/extra/true_max_context_length"],
        one_model: true,
        homepage: "https://github.com/LostRuins/koboldcpp",
        glyph: "⊟",
        rgb: (150, 120, 255),
        read: read_koboldcpp,
        ..s("koboldcpp", "KoboldCpp", Kind::LlmServer)
    },
    Service {
        ports: &[3000, 8080],
        procs: &[serves_with("open-webui", "serve"), serves("open_webui*")],
        health: Some("/health"),
        ident: &["/api/config"],
        extra: &["/health", "/api/version"],
        openai: false,
        homepage: "https://openwebui.com",
        glyph: "□",
        rgb: (210, 210, 220),
        read: read_openwebui,
        ..s("openwebui", "Open WebUI", Kind::Ui)
    },
    Service {
        ports: &[30000],
        procs: &[
            serves("sglang.launch_server*"),
            serves_with("sglang", "serve"),
        ],
        health: Some("/health"),
        models: Some("/v1/models"),
        ident: &["/get_model_info", "/v1/models"],
        extra: &["/model_info", "/get_server_info", "/health", "/metrics"],
        one_model: true,
        homepage: "https://docs.sglang.ai",
        glyph: "⊠",
        rgb: (232, 72, 85),
        read: read_sglang,
        ..s("sglang", "SGLang", Kind::LlmServer)
    },
    Service {
        ports: &[5000],
        procs: &[
            serves_in("main.py", &["tabbyapi"]),
            serves_in("start.py", &["tabbyapi"]),
        ],
        health: Some("/health"),
        models: Some("/v1/models"),
        ident: &["/v1/model", "/health"],
        extra: &["/v1/models"],
        one_model: true,
        homepage: "https://github.com/theroyallab/tabbyAPI",
        glyph: "▧",
        rgb: (255, 170, 80),
        read: read_tabbyapi,
        ..s("tabbyapi", "TabbyAPI", Kind::LlmServer)
    },
    Service {
        ports: &[2242],
        procs: &[
            serves_with("aphrodite", "run"),
            serves("aphrodite.endpoints*"),
        ],
        health: Some("/health"),
        models: Some("/v1/models"),
        ident: &["/v1/models"],
        extra: &["/version", "/health", "/metrics"],
        one_model: true,
        homepage: "https://aphrodite.pygmalion.chat",
        glyph: "▨",
        rgb: (255, 120, 180),
        read: read_aphrodite,
        ..s("aphrodite", "Aphrodite", Kind::LlmServer)
    },
    Service {
        ports: &[8080],
        procs: &[
            serves("mlx_lm.server"),
            serves_with("mlx_lm", "server"),
            serves_with("mlx_lm.server", "server"),
        ],
        health: Some("/health"),
        models: Some("/v1/models"),
        ident: &["/v1/models"],
        extra: &["/health"],
        one_model: true,
        homepage: "https://github.com/ml-explore/mlx-lm",
        glyph: "▩",
        rgb: (190, 190, 200),
        read: read_mlxlm,
        ..s("mlxlm", "mlx-lm", Kind::LlmServer)
    },
    Service {
        ports: &[4891],
        procs: &[owns("gpt4all"), owns_in("chat", &["gpt4all"])],
        models: Some("/v1/models"),
        ident: &["/v1/models"],
        homepage: "https://www.nomic.ai/gpt4all",
        glyph: "◳",
        rgb: (90, 200, 120),
        read: read_gpt4all,
        ..s("gpt4all", "GPT4All", Kind::LlmServer)
    },
    Service {
        ports: &[9997],
        procs: &[serves("xinference-*")],
        models: Some("/v1/models"),
        ident: &["/v1/models"],
        homepage: "https://inference.readthedocs.io",
        glyph: "◱",
        rgb: (110, 150, 255),
        read: read_xinference,
        ..s("xinference", "Xinference", Kind::LlmServer)
    },
    Service {
        ports: &[8000],
        procs: &[serves("tritonserver")],
        health: Some("/v2/health/ready"),
        models: Some("POST /v2/repository/index"),
        ident: &["/v2"],
        extra: &["/v2/health/ready", "POST /v2/repository/index"],
        openai: false,
        homepage: "https://github.com/triton-inference-server/server",
        glyph: "◲",
        rgb: (118, 185, 0),
        read: read_triton,
        ..s("triton", "Triton", Kind::LlmServer)
    },
    Service {
        ports: &[8000],
        procs: &[serves("trtllm-serve")],
        health: Some("/health"),
        models: Some("/v1/models"),
        ident: &["/v1/models"],
        extra: &["/health", "/version"],
        one_model: true,
        homepage: "https://nvidia.github.io/TensorRT-LLM",
        glyph: "■",
        rgb: (118, 185, 0),
        read: read_trtllm,
        ..s("trtllm", "TensorRT-LLM", Kind::LlmServer)
    },
    Service {
        ports: &[8080],
        procs: &[serves("llama-swap")],
        models: Some("/v1/models"),
        ident: &["/v1/models", "/running"],
        homepage: "https://github.com/mostlygeek/llama-swap",
        glyph: "⇋",
        rgb: (255, 160, 90),
        read: read_llamaswap,
        ..s("llamaswap", "llama-swap", Kind::Gateway)
    },
];

/// Ids, for the per-service opt-outs (`service.<id>=off`); kept in sync by a test.
pub const IDS: &[&str] = &[
    "ollama",
    "lmstudio",
    "llamacpp",
    "vllm",
    "tgi",
    "litellm",
    "localai",
    "jan",
    "tgwebui",
    "koboldcpp",
    "openwebui",
    "sglang",
    "tabbyapi",
    "aphrodite",
    "mlxlm",
    "gpt4all",
    "xinference",
    "triton",
    "trtllm",
    "llamaswap",
];

/// Paths asked on a port nobody has claimed (a python process, a container
/// proxy): enough for every OpenAI-compatible server and Ollama to say who it is.
const GENERIC: &[&str] = &["/v1/models", "/api/tags"];

/// A service by id (or its name, case-insensitively).
pub fn find(q: &str) -> Option<&'static Service> {
    let q = q.trim().to_lowercase();
    ALL.iter()
        .find(|s| s.id == q)
        .or_else(|| ALL.iter().find(|s| s.name.to_lowercase() == q))
}

impl Service {
    /// Every path worth asking once it's identified (the `!lms` command aside).
    pub fn paths(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Vec::new();
        for p in self
            .ident
            .iter()
            .copied()
            .chain(self.health)
            .chain(self.models)
            .chain(self.extra.iter().copied())
        {
            if !p.starts_with('!') && !v.contains(&p) {
                v.push(p);
            }
        }
        v
    }
    /// Its brand colour (very light ones turn to the text colour on light themes).
    pub fn color(&self) -> Color {
        let (r, g, b) = self.rgb;
        let lum = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
        if c::pal().light && lum > 200.0 {
            c::TEXT()
        } else {
            Color::Rgb(r, g, b)
        }
    }
    pub fn hex(&self) -> String {
        let (r, g, b) = self.rgb;
        format!("#{r:02x}{g:02x}{b:02x}")
    }
    /// Does a process (seen as `prog`) belong to this service? `Some(true)`
    /// when it's a server by itself.
    fn recognise(&self, prog: &Prog, args_cwd: &str) -> Option<bool> {
        let mut hit = None;
        for r in self.procs {
            if r.matches(prog, args_cwd) {
                if r.serves {
                    return Some(true);
                }
                hit = Some(false);
            }
        }
        hit
    }
}

impl Rule {
    fn matches(&self, prog: &Prog, args_cwd: &str) -> bool {
        if !prog.names.iter().any(|n| name_match(self.exe, n)) {
            return false;
        }
        if let Some(a) = self.arg {
            let ok = if a.starts_with('.') {
                prog.rest.iter().any(|w| w.to_lowercase().ends_with(a))
            } else {
                prog.rest
                    .iter()
                    .find(|w| !w.starts_with('-'))
                    .is_some_and(|w| w.eq_ignore_ascii_case(a))
            };
            if !ok {
                return false;
            }
        }
        self.inside.is_empty() || self.inside.iter().any(|i| args_cwd.contains(i))
    }
}

/// `pat` (lower case, `*` = prefix or suffix) against a process name. Linux
/// keeps only 15 characters of a process's `comm`.
fn name_match(pat: &str, name: &str) -> bool {
    let n = name.trim_end_matches(".exe");
    if n.is_empty() {
        return false;
    }
    if let Some(pre) = pat.strip_suffix('*') {
        n.starts_with(pre) || (n.len() == 15 && pre.starts_with(n))
    } else if let Some(suf) = pat.strip_prefix('*') {
        n.ends_with(suf)
    } else {
        n == pat || (n.len() == 15 && pat.starts_with(n))
    }
}

/// The regex (awk ERE, lower case, no quotes or backslashes) that picks the
/// processes worth sending: anything any rule could match.
fn coarse_regex() -> String {
    const VAGUE: &[&str] = &["server", "chat", "server.py", "main.py", "start.py"];
    let mut terms: Vec<String> = Vec::new();
    for s in ALL {
        for r in s.procs {
            let mut add: Vec<&str> = Vec::new();
            if !r.inside.is_empty() {
                add.extend(r.inside);
            } else if VAGUE.contains(&r.exe) {
                add.extend(r.arg);
            } else {
                add.push(r.exe.trim_matches('*'));
            }
            for t in add {
                let t = t.to_lowercase().replace('.', "[.]");
                if !t.is_empty() && !terms.contains(&t) {
                    terms.push(t);
                }
            }
        }
    }
    terms.join("|")
}

/// Per-service opt-outs: `service.<id>=off`.
pub fn enabled_services(cfg: &Config) -> Vec<&'static Service> {
    ALL.iter()
        .filter(|s| {
            !matches!(
                cfg.get(&format!("service.{}", s.id))
                    .unwrap_or_default()
                    .to_lowercase()
                    .as_str(),
                "off" | "false" | "0" | "no"
            )
        })
        .collect()
}

/// Is the live view on (`services=on`)?
pub fn enabled(cfg: &Config) -> bool {
    matches!(
        cfg.get_or("services").to_lowercase().as_str(),
        "on" | "true" | "1" | "yes"
    )
}

/// Seconds between scans of a machine (`services_interval`, at least 5).
pub fn interval(cfg: &Config) -> Duration {
    Duration::from_secs(cfg.num("services_interval").max(5))
}

/// Check a `service.<id>` setting (for `aleph settings set`).
pub fn check_setting(id: &str, v: &str) -> Result<(), String> {
    if find(id).map(|s| s.id) != Some(id) {
        let near = IDS
            .iter()
            .filter_map(|x| crate::fuzzy::score(id, x).map(|(sc, _)| (sc, *x)))
            .max_by_key(|x| x.0)
            .map(|x| format!(" — did you mean '{}'?", x.1))
            .unwrap_or_default();
        return Err(format!(
            "unknown service '{id}'{near} (aleph services --help lists them: {})",
            IDS.join(", ")
        ));
    }
    if ["on", "off", "true", "false", "1", "0", "yes", "no"].contains(&v) {
        Ok(())
    } else {
        Err(format!("service.{id} is on or off"))
    }
}

// ------------------------------------------------------------------ what a probe finds

/// How a server is doing.
#[derive(Debug, Clone, PartialEq)]
pub enum Health {
    /// it answers
    Up,
    /// it answers 503 (loading a model, warming up)
    Loading,
    /// it answers with a server error
    Error(String),
    /// its port is open but nothing answers
    Down,
    /// the process runs but its port isn't listening (yet)
    NotListening,
    /// couldn't check (no HTTP client, or its port isn't known)
    Unknown,
}

impl Health {
    /// Stable name for JSON output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Health::Up => "up",
            Health::Loading => "loading",
            Health::Error(_) => "error",
            Health::Down => "down",
            Health::NotListening => "not_listening",
            Health::Unknown => "unknown",
        }
    }
    pub fn label(&self) -> String {
        match self {
            Health::Up => "up".into(),
            Health::Loading => "loading".into(),
            Health::Error(e) => e.clone(),
            Health::Down => "no answer".into(),
            Health::NotListening => "not listening".into(),
            Health::Unknown => "unknown".into(),
        }
    }
    fn parse(s: &str, detail: &str) -> Health {
        match s {
            "up" => Health::Up,
            "loading" => Health::Loading,
            "error" => Health::Error(detail.to_string()),
            "down" => Health::Down,
            "not_listening" => Health::NotListening,
            _ => Health::Unknown,
        }
    }
    pub fn is_up(&self) -> bool {
        matches!(self, Health::Up | Health::Loading)
    }
    /// (glyph, colour) — the shapes alone carry the meaning under NO_COLOR.
    pub fn dot(&self, t0: Instant) -> (&'static str, Color) {
        match self {
            Health::Up => ("●", c::GREEN()),
            Health::Loading => (ui::spinner(t0), c::ACCENT()),
            Health::Error(_) => ("✕", c::RED()),
            Health::Down => ("○", c::RED()),
            Health::NotListening => ("○", c::YELLOW()),
            Health::Unknown => ("?", c::MUTED()),
        }
    }
}

/// One model a server has (on disk, configured or loaded).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub name: String,
    /// bytes of weights on disk
    pub size: Option<f64>,
    /// parameter count as the server says it (`8.0B`)
    pub params: Option<String>,
    /// `Q4_K_M`, `4bit`, `FP8`, …
    pub quant: Option<String>,
    /// `gguf`, `mlx`, `safetensors`, …
    pub format: Option<String>,
    /// `llm`, `vlm`, `embeddings`, `lora`, `route`, …
    pub kind: Option<String>,
    /// context length (loaded, else the maximum)
    pub ctx: Option<u64>,
    /// in memory now? `None` when the server doesn't say
    pub loaded: Option<bool>,
    /// bytes of GPU memory it takes
    pub vram: Option<f64>,
    /// bytes of memory it takes in all (Ollama: weights + KV cache)
    pub mem: Option<f64>,
}

impl Model {
    fn named(name: impl Into<String>) -> Model {
        let name = name.into();
        Model {
            quant: quant_of(&name),
            name,
            ..Default::default()
        }
    }
    /// `Q4_K_M · 8.0B · 4.9G`
    pub fn facts(&self) -> String {
        let mut v: Vec<String> = Vec::new();
        if let Some(s) = self.size {
            v.push(ui::bytes(s));
        }
        if let Some(q) = &self.quant {
            v.push(q.clone());
        }
        if let Some(p) = &self.params {
            v.push(p.clone());
        }
        if let Some(c) = self.ctx {
            v.push(format!("{} ctx", ctx_text(c)));
        }
        v.join(" ")
    }
    /// Share of its memory on the GPU (Ollama's `100% GPU`).
    pub fn gpu_share(&self) -> Option<f64> {
        match (self.vram, self.mem) {
            (Some(v), Some(m)) if m > 0.0 => Some((v / m).clamp(0.0, 1.0)),
            _ => None,
        }
    }
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name, "size": self.size, "params": self.params, "quant": self.quant,
            "format": self.format, "kind": self.kind, "ctx": self.ctx, "loaded": self.loaded,
            "vram": self.vram, "mem": self.mem, "gpu_share": self.gpu_share(),
        })
    }
    pub fn from_json(v: &Value) -> Option<Model> {
        Some(Model {
            name: v.get("name")?.as_str()?.to_string(),
            size: v.get("size").and_then(Value::as_f64),
            params: str_at(v, "params"),
            quant: str_at(v, "quant"),
            format: str_at(v, "format"),
            kind: str_at(v, "kind"),
            ctx: v.get("ctx").and_then(Value::as_u64),
            loaded: v.get("loaded").and_then(Value::as_bool),
            vram: v.get("vram").and_then(Value::as_f64),
            mem: v.get("mem").and_then(Value::as_f64),
        })
    }
}

/// Requests in flight, from the server's metrics (vLLM, SGLang, Aphrodite, TGI,
/// llama.cpp with `--metrics`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Load {
    pub running: Option<f64>,
    pub waiting: Option<f64>,
    /// KV-cache use, 0..=1
    pub kv: Option<f64>,
}

/// One running server on a machine.
#[derive(Debug, Clone)]
pub struct Found {
    pub service: &'static Service,
    /// the port it answers on (None: running, port unknown)
    pub port: Option<u16>,
    /// every port its process listens on (incl. `port`)
    pub ports: Vec<u16>,
    /// where the probe reached it (`127.0.0.1`, `[::1]`, …)
    pub addr: String,
    /// what it's bound to (`0.0.0.0` = reachable from the network)
    pub bind: String,
    pub pid: Option<u32>,
    pub process: String,
    pub version: Option<String>,
    pub health: Health,
    /// its models list wants an API key
    pub auth: bool,
    pub models: Vec<Model>,
    /// bytes of GPU memory its processes take (NVIDIA compute apps)
    pub vram: Option<f64>,
    pub load: Load,
    /// found by its process / by its HTTP answers
    pub by_process: bool,
    pub by_http: bool,
}

impl Found {
    fn new(service: &'static Service) -> Found {
        Found {
            service,
            port: None,
            ports: vec![],
            addr: String::new(),
            bind: String::new(),
            pid: None,
            process: String::new(),
            version: None,
            health: Health::Unknown,
            auth: false,
            models: vec![],
            vram: None,
            load: Load::default(),
            by_process: false,
            by_http: false,
        }
    }
    /// `http://127.0.0.1:11434`
    pub fn url(&self) -> Option<String> {
        let a = if self.addr.is_empty() {
            "127.0.0.1"
        } else {
            &self.addr
        };
        self.port.map(|p| format!("http://{a}:{p}"))
    }
    /// The OpenAI-compatible base URL, where it has one.
    pub fn openai_base(&self) -> Option<String> {
        self.service
            .openai
            .then(|| self.url().map(|u| format!("{u}/v1")))
            .flatten()
    }
    pub fn loaded(&self) -> impl Iterator<Item = &Model> {
        self.models.iter().filter(|m| m.loaded == Some(true))
    }
    /// Does the server say which models are loaded?
    pub fn knows_loaded(&self) -> bool {
        self.models.iter().any(|m| m.loaded.is_some())
    }
    /// The count shown in summaries: loaded models where the server says,
    /// else every model.
    pub fn shown_count(&self) -> usize {
        if self.knows_loaded() {
            self.loaded().count()
        } else {
            self.models.len()
        }
    }
    /// GPU memory: its processes' (nvidia-smi), else what its models report.
    pub fn vram_total(&self) -> Option<f64> {
        self.vram.or_else(|| {
            let v: f64 = self.models.iter().filter_map(|m| m.vram).sum();
            (v > 0.0).then_some(v)
        })
    }
    /// `3 models · 1 loaded`, `1 model`, `routes 4`, `needs a key`.
    pub fn models_text(&self) -> String {
        let n = self.models.len();
        if n == 0 {
            return if self.auth {
                "needs a key".into()
            } else {
                String::new()
            };
        }
        let noun = match self.service.kind {
            Kind::Gateway => {
                if n == 1 {
                    "route"
                } else {
                    "routes"
                }
            }
            _ => {
                if n == 1 {
                    "model"
                } else {
                    "models"
                }
            }
        };
        let loaded = self.loaded().count();
        if self.knows_loaded() && loaded != n {
            format!("{n} {noun} · {loaded} loaded")
        } else {
            format!("{n} {noun}")
        }
    }
    /// `2 running · 1 queued · kv 34%` (servers with metrics).
    pub fn load_text(&self) -> String {
        let mut v = Vec::new();
        if let Some(r) = self.load.running {
            v.push(format!("{r:.0} running"));
        }
        if let Some(q) = self.load.waiting.filter(|q| *q > 0.0) {
            v.push(format!("{q:.0} queued"));
        }
        if let Some(k) = self.load.kv {
            v.push(format!("kv {:.0}%", k * 100.0));
        }
        v.join(" · ")
    }
    /// Models, loaded first.
    pub fn models_sorted(&self) -> Vec<&Model> {
        let mut v: Vec<&Model> = self.models.iter().collect();
        v.sort_by_key(|m| match m.loaded {
            Some(true) => 0,
            None => 1,
            Some(false) => 2,
        });
        v
    }
    pub fn to_json(&self) -> Value {
        let s = self.service;
        let mut by = Vec::new();
        if self.by_process {
            by.push("process");
        }
        if self.by_http {
            by.push("http");
        }
        let detail = match &self.health {
            Health::Error(e) => e.clone(),
            _ => String::new(),
        };
        json!({
            "id": s.id, "name": s.name, "kind": s.kind.as_str(), "openai": s.openai,
            "homepage": s.homepage, "glyph": s.glyph, "color": s.hex(),
            "port": self.port, "ports": self.ports, "addr": self.addr, "bind": self.bind,
            "url": self.url(), "openai_base": self.openai_base(),
            "pid": self.pid, "process": self.process, "version": self.version,
            "health": self.health.as_str(), "health_label": self.health.label(), "health_detail": detail,
            "auth_required": self.auth, "found_by": by,
            "model_count": self.models.len(), "loaded_count": self.loaded().count(),
            "models": self.models.iter().map(Model::to_json).collect::<Vec<_>>(),
            "vram": self.vram_total(),
            "requests_running": self.load.running, "requests_waiting": self.load.waiting,
            "kv_cache": self.load.kv,
        })
    }
    pub fn from_json(v: &Value) -> Option<Found> {
        let s = find(v.get("id")?.as_str()?)?;
        let by: Vec<&str> = v
            .get("found_by")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        Some(Found {
            service: s,
            port: v.get("port").and_then(Value::as_u64).map(|p| p as u16),
            ports: v
                .get("ports")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.as_u64().map(|p| p as u16))
                        .collect()
                })
                .unwrap_or_default(),
            addr: str_at(v, "addr").unwrap_or_default(),
            bind: str_at(v, "bind").unwrap_or_default(),
            pid: v.get("pid").and_then(Value::as_u64).map(|p| p as u32),
            process: str_at(v, "process").unwrap_or_default(),
            version: str_at(v, "version"),
            health: Health::parse(
                v.get("health").and_then(Value::as_str).unwrap_or(""),
                v.get("health_detail").and_then(Value::as_str).unwrap_or(""),
            ),
            auth: v
                .get("auth_required")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            models: v
                .get("models")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Model::from_json).collect())
                .unwrap_or_default(),
            vram: v.get("vram").and_then(Value::as_f64),
            load: Load {
                running: v.get("requests_running").and_then(Value::as_f64),
                waiting: v.get("requests_waiting").and_then(Value::as_f64),
                kv: v.get("kv_cache").and_then(Value::as_f64),
            },
            by_process: by.contains(&"process"),
            by_http: by.contains(&"http"),
        })
    }
}

/// A machine's services, as of its last scan.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub alias: String,
    pub found: Vec<Found>,
    /// epoch seconds of the last finished scan (0 = never)
    pub at: u64,
    pub took_ms: u64,
    /// why the last scan failed (`found` is then the one before)
    pub err: Option<String>,
    /// from the cache, not refreshed yet (or the last refresh failed)
    pub stale: bool,
    /// a scan is running
    pub busy: bool,
    /// the probe ran out of time part-way (what it had is shown)
    pub partial: bool,
    /// which tools the machine had: `ss`/`lsof`/`netstat`/`proc`/`none`,
    /// `curl`/`wget`/`nc`/`none`
    pub listeners: String,
    pub http: String,
}

impl Scan {
    pub fn new(alias: &str) -> Scan {
        Scan {
            alias: alias.to_string(),
            ..Default::default()
        }
    }
    pub fn reachable(&self) -> bool {
        self.at > 0 && self.err.is_none()
    }
    /// The JSON for one machine (`color` is the machine's colour).
    pub fn to_json(&self, color: &str) -> Value {
        json!({
            "alias": self.alias, "color": color, "reachable": self.reachable(),
            "error": self.err, "at": self.at, "took_ms": self.took_ms, "stale": self.stale,
            "partial": self.partial, "listeners": self.listeners, "http": self.http,
            "services": self.found.iter().map(Found::to_json).collect::<Vec<_>>(),
        })
    }
    fn from_json(v: &Value) -> Option<Scan> {
        Some(Scan {
            alias: v.get("alias")?.as_str()?.to_string(),
            found: v
                .get("services")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Found::from_json).collect())
                .unwrap_or_default(),
            at: v.get("at").and_then(Value::as_u64).unwrap_or(0),
            took_ms: v.get("took_ms").and_then(Value::as_u64).unwrap_or(0),
            listeners: str_at(v, "listeners").unwrap_or_default(),
            http: str_at(v, "http").unwrap_or_default(),
            partial: v.get("partial").and_then(Value::as_bool).unwrap_or(false),
            ..Default::default()
        })
    }
}

// ------------------------------------------------------------------ HTTP answers

/// One answer: status code (0 = no answer) and body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resp {
    pub code: u16,
    pub body: String,
}

/// Everything one port answered, by request (`/v1/models`, `POST /v2/…`, `!lms`).
#[derive(Debug, Clone, Default)]
pub struct Http {
    pub port: u16,
    answers: BTreeMap<String, Resp>,
}

fn is2xx(c: u16) -> bool {
    (200..300).contains(&c)
}

impl Http {
    pub fn with(port: u16, answers: &[(&str, u16, &str)]) -> Http {
        let mut h = Http {
            port,
            ..Default::default()
        };
        for (p, c, b) in answers {
            h.put(p, *c, b);
        }
        h
    }
    pub fn put(&mut self, path: &str, code: u16, body: &str) {
        self.answers.insert(
            path.to_string(),
            Resp {
                code,
                body: body.to_string(),
            },
        );
    }
    pub fn asked(&self, path: &str) -> bool {
        self.answers.contains_key(path)
    }
    pub fn code(&self, path: &str) -> Option<u16> {
        self.answers.get(path).map(|r| r.code)
    }
    pub fn ok(&self, path: &str) -> bool {
        self.code(path).is_some_and(is2xx)
    }
    pub fn denied(&self, path: &str) -> bool {
        matches!(self.code(path), Some(401 | 403))
    }
    /// The body of a 2xx answer.
    pub fn text(&self, path: &str) -> Option<&str> {
        self.answers
            .get(path)
            .filter(|r| is2xx(r.code))
            .map(|r| r.body.as_str())
    }
    /// A 2xx answer that is JSON.
    pub fn json(&self, path: &str) -> Option<Value> {
        let t = self.text(path)?.trim();
        if !(t.starts_with('{') || t.starts_with('[')) {
            return None;
        }
        serde_json::from_str(t).ok()
    }
    fn codes(&self) -> Vec<u16> {
        self.answers
            .iter()
            .filter(|(k, _)| !k.starts_with('!'))
            .map(|(_, r)| r.code)
            .collect()
    }
}

// ------------------------------------------------------------------ readers

/// What a service's answers say. `ident` = only this service answers like that.
#[derive(Debug, Clone, Default)]
pub struct Read {
    pub ident: bool,
    pub version: Option<String>,
    pub models: Vec<Model>,
    pub health: Option<Health>,
    pub auth: bool,
    pub load: Load,
}

fn read_none(_: &Http) -> Read {
    Read::default()
}

fn str_at(v: &Value, k: &str) -> Option<String> {
    match v.get(k)? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}
fn num_at(v: &Value, k: &str) -> Option<f64> {
    match v.get(k)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}
fn list_at<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
/// `data` of an OpenAI-style list (`{"object":"list","data":[…]}`).
fn data(v: &Value) -> &[Value] {
    list_at(v, "data")
}
/// Is any item of `path`'s OpenAI list `owned_by` one of `who`?
fn owned_by(h: &Http, path: &str, who: &[&str]) -> bool {
    h.json(path).is_some_and(|v| {
        data(&v).iter().any(|m| {
            m.get("owned_by")
                .and_then(Value::as_str)
                .is_some_and(|o| who.iter().any(|w| o.eq_ignore_ascii_case(w)))
        })
    })
}
/// `32k`, `128k`, `4096` for a context length.
pub fn ctx_text(c: u64) -> String {
    if c >= 1024 && c.is_multiple_of(1024) {
        format!("{}k", c / 1024)
    } else if c >= 1000 {
        format!("{}k", (c as f64 / 1000.0).round())
    } else {
        c.to_string()
    }
}

/// `8.0B`, `350M` from a parameter count.
fn params_of(n: f64) -> String {
    if n >= 1e9 {
        format!("{:.1}B", n / 1e9)
    } else {
        format!("{:.0}M", n / 1e6)
    }
}
/// The last path component, without `.gguf` (model paths → names).
fn short_name(id: &str) -> String {
    let t = id.trim_end_matches('/');
    if t.starts_with('/') || t.starts_with("./") || t.starts_with('~') || t.ends_with(".gguf") {
        let b = t.rsplit('/').next().unwrap_or(t);
        b.trim_end_matches(".gguf").to_string()
    } else {
        t.to_string()
    }
}
/// The quantisation a model's name carries (`Q4_K_M`, `IQ3_XXS`, `4bit`, `FP8`, `AWQ`).
pub fn quant_of(name: &str) -> Option<String> {
    const WORDS: &[&str] = &[
        "F16", "BF16", "FP16", "F32", "FP32", "FP8", "FP4", "NVFP4", "MXFP4", "INT8", "INT4",
        "AWQ", "GPTQ", "EXL2", "EXL3", "NF4", "4BIT", "8BIT", "3BIT", "6BIT", "2BIT", "W4A16",
        "W8A8",
    ];
    let up = name.to_uppercase();
    let mut found = None;
    for t in up.split(['-', '.', '/', ' ', ':', '+']) {
        let ggml = {
            let core = t.strip_prefix('I').unwrap_or(t);
            core.len() >= 2
                && core.starts_with('Q')
                && core.as_bytes()[1].is_ascii_digit()
                && core
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        };
        if ggml {
            found = Some(t.to_string());
        } else if WORDS.contains(&t) {
            found = Some(if t.ends_with("BIT") {
                t.to_lowercase()
            } else {
                t.to_string()
            });
        }
    }
    found
}

/// Sum a Prometheus gauge over its label sets.
fn metric(text: &str, name: &str) -> Option<f64> {
    let mut sum = None;
    for l in text.lines() {
        let l = l.trim();
        let Some(rest) = l.strip_prefix(name) else {
            continue;
        };
        let rest = match rest.chars().next() {
            Some('{') => rest.split_once('}').map(|x| x.1).unwrap_or(""),
            Some(' ') => rest,
            _ => continue,
        };
        if let Some(v) = rest
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<f64>().ok())
        {
            if v.is_finite() {
                sum = Some(sum.unwrap_or(0.0) + v);
            }
        }
    }
    sum
}

/// Requests running / waiting and KV-cache use from `/metrics`.
fn load_of(h: &Http, running: &[&str], waiting: &[&str], kv: &[&str]) -> Load {
    let Some(t) = h.text("/metrics") else {
        return Load::default();
    };
    let first = |names: &[&str]| names.iter().find_map(|n| metric(t, n));
    Load {
        running: first(running),
        waiting: first(waiting),
        kv: first(kv).map(|k| if k > 1.0 { k / 100.0 } else { k }),
    }
}

/// Models of an OpenAI `/v1/models` list.
fn openai_models(h: &Http, path: &str, loaded: Option<bool>) -> Vec<Model> {
    let Some(v) = h.json(path) else {
        return vec![];
    };
    data(&v)
        .iter()
        .filter_map(|m| {
            let id = str_at(m, "id")?;
            let mut x = Model::named(short_name(&id));
            x.loaded = loaded;
            x.ctx = num_at(m, "max_model_len")
                .or_else(|| num_at(m, "context_length"))
                .or_else(|| num_at(m, "max_context_length"))
                .map(|c| c as u64);
            if m.get("parent").is_some_and(|p| !p.is_null()) {
                x.kind = Some("lora".into());
            }
            Some(x)
        })
        .collect()
}

/// The shared shape of vLLM-family servers: `/v1/models` owned by `who`,
/// `/version`, `/metrics`.
fn openai_server(h: &Http, who: &[&str], prefix: &str) -> Read {
    let run = format!("{prefix}num_requests_running");
    let wait = format!("{prefix}num_requests_waiting");
    let kv1 = format!("{prefix}gpu_cache_usage_perc");
    let kv2 = format!("{prefix}kv_cache_usage_perc");
    Read {
        ident: owned_by(h, "/v1/models", who),
        version: h.json("/version").and_then(|v| str_at(&v, "version")),
        models: openai_models(h, "/v1/models", Some(true)),
        auth: h.denied("/v1/models"),
        load: load_of(h, &[&run], &[&wait], &[&kv1, &kv2]),
        ..Default::default()
    }
}

fn read_ollama(h: &Http) -> Read {
    let mut r = Read {
        version: h.json("/api/version").and_then(|v| str_at(&v, "version")),
        ..Default::default()
    };
    let model = |m: &Value| -> Option<Model> {
        let name = str_at(m, "name").or_else(|| str_at(m, "model"))?;
        let d = m.get("details").cloned().unwrap_or(Value::Null);
        let fam = list_at(&d, "families");
        let embed = fam
            .iter()
            .chain(std::iter::once(d.get("family").unwrap_or(&Value::Null)))
            .filter_map(Value::as_str)
            .any(|f| f.contains("bert"));
        Some(Model {
            params: str_at(&d, "parameter_size"),
            quant: str_at(&d, "quantization_level").or_else(|| quant_of(&name)),
            format: str_at(&d, "format"),
            kind: embed.then(|| "embeddings".to_string()),
            name,
            ..Default::default()
        })
    };
    if let Some(t) = h.json("/api/tags") {
        if let Some(list) = t.get("models").and_then(Value::as_array) {
            // Ollama's shape: every model has a name and a digest or details
            let shaped = list.iter().all(|m| {
                m.get("name").is_some() && (m.get("digest").is_some() || m.get("details").is_some())
            });
            r.ident = shaped && (!list.is_empty() || r.version.is_some());
            for m in list {
                if let Some(mut x) = model(m) {
                    x.size = num_at(m, "size");
                    x.loaded = Some(false);
                    r.models.push(x);
                }
            }
        }
    }
    if let Some(ps) = h.json("/api/ps") {
        for m in list_at(&ps, "models") {
            let Some(mut x) = model(m) else { continue };
            let (vram, mem) = (num_at(m, "size_vram"), num_at(m, "size"));
            let ctx = num_at(m, "context_length").map(|c| c as u64);
            match r.models.iter_mut().find(|y| y.name == x.name) {
                Some(y) => {
                    y.loaded = Some(true);
                    y.vram = vram;
                    y.mem = mem;
                    y.ctx = ctx.or(y.ctx);
                }
                None => {
                    x.loaded = Some(true);
                    x.vram = vram;
                    x.mem = mem;
                    x.ctx = ctx;
                    r.models.push(x);
                }
            }
        }
    }
    r
}

fn read_lmstudio(h: &Http) -> Read {
    let mut r = Read::default();
    if let Some(v) = h.json("/api/v0/models") {
        let items = data(&v);
        r.ident = items.iter().any(|m| {
            m.get("state").is_some()
                && (m.get("compatibility_type").is_some()
                    || m.get("quantization").is_some()
                    || m.get("publisher").is_some())
        });
        for m in items {
            let Some(id) = str_at(m, "id") else { continue };
            let mut x = Model::named(id);
            x.kind = str_at(m, "type");
            x.format = str_at(m, "compatibility_type");
            x.quant = str_at(m, "quantization").or(x.quant);
            x.ctx = num_at(m, "loaded_context_length")
                .or_else(|| num_at(m, "max_context_length"))
                .map(|c| c as u64);
            x.loaded = str_at(m, "state").map(|s| s == "loaded");
            r.models.push(x);
        }
    } else {
        r.ident = owned_by(h, "/v1/models", &["organization_owner"]);
        r.models = openai_models(h, "/v1/models", None);
    }
    // `lms ps --json`: the loaded models (older LM Studio has no /api/v0)
    if let Some(v) = h.json("!lms") {
        let items: &[Value] = match &v {
            Value::Array(a) => a,
            o => list_at(o, "models"),
        };
        for m in items {
            let Some(name) = str_at(m, "identifier")
                .or_else(|| str_at(m, "modelKey"))
                .or_else(|| str_at(m, "path"))
            else {
                continue;
            };
            let size = num_at(m, "sizeBytes");
            let ctx = num_at(m, "contextLength").map(|c| c as u64);
            match r.models.iter_mut().find(|y| y.name == name) {
                Some(y) => {
                    y.loaded = Some(true);
                    y.size = size.or(y.size);
                    y.ctx = ctx.or(y.ctx);
                }
                None => {
                    let mut x = Model::named(name);
                    x.loaded = Some(true);
                    x.size = size;
                    x.ctx = ctx;
                    x.params = str_at(m, "paramsString");
                    x.format = str_at(m, "format");
                    r.models.push(x);
                }
            }
        }
    }
    r
}

fn read_llamacpp(h: &Http) -> Read {
    let mut r = Read::default();
    let mut ctx = None;
    let mut path = None;
    if let Some(p) = h.json("/props") {
        r.ident = p.get("default_generation_settings").is_some() || p.get("total_slots").is_some();
        r.version = str_at(&p, "build_info");
        ctx = p
            .get("default_generation_settings")
            .and_then(|d| num_at(d, "n_ctx"))
            .or_else(|| num_at(&p, "n_ctx"))
            .map(|c| c as u64);
        path = str_at(&p, "model_path");
    }
    if let Some(v) = h.json("/v1/models") {
        r.ident |= data(&v)
            .iter()
            .any(|m| m.get("owned_by").and_then(Value::as_str) == Some("llamacpp"));
        for m in data(&v) {
            let Some(id) = str_at(m, "id") else { continue };
            let mut x = Model::named(short_name(&id));
            let meta = m.get("meta").cloned().unwrap_or(Value::Null);
            x.size = num_at(&meta, "size");
            x.params = num_at(&meta, "n_params").map(params_of);
            x.ctx = ctx.or_else(|| num_at(&meta, "n_ctx_train").map(|c| c as u64));
            x.format = Some("gguf".into());
            // router mode lists every model it can load, with a status
            x.loaded = match m.get("status") {
                Some(Value::String(s)) => Some(s == "loaded"),
                Some(o @ Value::Object(_)) => str_at(o, "value").map(|s| s == "loaded"),
                _ => Some(true),
            };
            r.models.push(x);
        }
    }
    if r.models.is_empty() {
        if let Some(p) = path {
            let mut x = Model::named(short_name(&p));
            x.loaded = Some(true);
            x.ctx = ctx;
            x.format = Some("gguf".into());
            r.models.push(x);
        }
    }
    r.load = load_of(
        h,
        &["llamacpp:requests_processing"],
        &["llamacpp:requests_deferred"],
        &["llamacpp:kv_cache_usage_ratio"],
    );
    r
}

fn read_vllm(h: &Http) -> Read {
    openai_server(h, &["vllm"], "vllm:")
}

fn read_aphrodite(h: &Http) -> Read {
    openai_server(h, &["pygmalionai", "aphrodite"], "aphrodite:")
}

fn read_trtllm(h: &Http) -> Read {
    openai_server(h, &["tensorrt_llm", "tensorrt-llm", "trtllm"], "trtllm:")
}

fn read_tgi(h: &Http) -> Read {
    let mut r = Read::default();
    if let Some(i) = h.json("/info") {
        let router = str_at(&i, "router").is_some_and(|x| x.contains("text-generation-router"));
        let shaped = i.get("model_dtype").is_some()
            && (i.get("max_total_tokens").is_some() || i.get("max_input_tokens").is_some());
        r.ident = i.get("model_id").is_some() && (router || shaped);
        r.version = str_at(&i, "version");
        if let Some(id) = str_at(&i, "model_id") {
            let mut x = Model::named(short_name(&id));
            x.loaded = Some(true);
            x.ctx = num_at(&i, "max_total_tokens").map(|c| c as u64);
            x.quant = x.quant.or_else(|| {
                str_at(&i, "model_dtype").map(|d| d.trim_start_matches("torch.").to_string())
            });
            r.models.push(x);
        }
    }
    r.load = load_of(h, &["tgi_batch_current_size"], &["tgi_queue_size"], &[]);
    r
}

fn read_litellm(h: &Http) -> Read {
    let mut r = Read::default();
    if let Some(v) = h.json("/health/readiness") {
        r.version = str_at(&v, "litellm_version");
        r.ident = r.version.is_some();
    }
    if h.text("/health/liveliness")
        .is_some_and(|t| t.contains("I'm alive"))
    {
        r.ident = true;
    }
    r.models = openai_models(h, "/v1/models", None);
    for m in &mut r.models {
        m.kind = Some("route".into());
        m.quant = None;
    }
    r.auth = h.denied("/v1/models");
    r
}

fn read_localai(h: &Http) -> Read {
    let mut r = Read::default();
    let mut loaded: Vec<String> = Vec::new();
    if let Some(v) = h.json("/system") {
        r.ident = v.get("backends").is_some_and(Value::is_array);
        loaded = list_at(&v, "loaded_models")
            .iter()
            .filter_map(|m| str_at(m, "id"))
            .collect();
    }
    r.version = h.json("/version").and_then(|v| str_at(&v, "version"));
    r.models = openai_models(h, "/v1/models", None);
    if r.ident {
        for m in &mut r.models {
            m.loaded = Some(loaded.contains(&m.name));
        }
    }
    r.auth = h.denied("/v1/models");
    r
}

fn read_jan(h: &Http) -> Read {
    let mut r = Read::default();
    let Some(v) = h.json("/v1/models") else {
        return r;
    };
    // Jan's (cortex) model.json shape: engine + settings / metadata
    let local = |e: &str| {
        ["llama", "cortex", "nitro", "onnx", "tensorrt", "mlx"]
            .iter()
            .any(|k| e.contains(k))
    };
    for m in data(&v) {
        let engine = str_at(m, "engine").unwrap_or_default();
        if !engine.is_empty() && (m.get("settings").is_some() || m.get("metadata").is_some()) {
            r.ident = true;
        }
        if !engine.is_empty() && !local(&engine) {
            continue; // a remote API Jan knows about, not a local model
        }
        let Some(id) = str_at(m, "id") else { continue };
        let mut x = Model::named(id);
        let meta = m.get("metadata").cloned().unwrap_or(Value::Null);
        x.size = num_at(&meta, "size");
        x.format = str_at(m, "format");
        x.ctx = m
            .get("settings")
            .and_then(|s| num_at(s, "ctx_len"))
            .map(|c| c as u64);
        r.models.push(x);
    }
    r
}

fn read_tgwebui(h: &Http) -> Read {
    let mut r = Read::default();
    let mut current = None;
    if let Some(v) = h.json("/v1/internal/model/info") {
        r.ident = v.get("model_name").is_some() && v.get("lora_names").is_some();
        current = str_at(&v, "model_name").filter(|n| n != "None");
    }
    if h.text("/config")
        .is_some_and(|t| t.contains("Text generation web UI"))
    {
        r.ident = true;
    }
    if let Some(v) = h.json("/v1/internal/model/list") {
        for n in list_at(&v, "model_names").iter().filter_map(Value::as_str) {
            if n == "None" {
                continue;
            }
            let mut x = Model::named(n);
            x.loaded = Some(current.as_deref() == Some(n));
            r.models.push(x);
        }
    }
    if let Some(cur) = current {
        if !r.models.iter().any(|m| m.name == cur) {
            let mut x = Model::named(cur);
            x.loaded = Some(true);
            r.models.push(x);
        }
    }
    r
}

fn read_koboldcpp(h: &Http) -> Read {
    let mut r = Read::default();
    if let Some(v) = h.json("/api/extra/version") {
        r.ident = str_at(&v, "result").is_some_and(|x| x.eq_ignore_ascii_case("koboldcpp"));
        r.version = str_at(&v, "version");
    }
    if let Some(v) = h.json("/api/v1/model") {
        if let Some(m) = str_at(&v, "result") {
            let name = m.strip_prefix("koboldcpp/").unwrap_or(&m).to_string();
            if !name.is_empty() && name != "inactive" {
                let mut x = Model::named(name);
                x.loaded = Some(true);
                x.format = Some("gguf".into());
                x.ctx = h
                    .json("/api/extra/true_max_context_length")
                    .and_then(|v| num_at(&v, "value"))
                    .map(|c| c as u64);
                r.models.push(x);
            }
        }
    }
    r
}

fn read_openwebui(h: &Http) -> Read {
    let mut r = Read::default();
    if let Some(v) = h.json("/api/config") {
        r.ident = str_at(&v, "name").is_some_and(|n| n.contains("Open WebUI"));
        r.version = str_at(&v, "version");
    }
    if r.version.is_none() {
        r.version = h.json("/api/version").and_then(|v| str_at(&v, "version"));
    }
    // its model list needs a signed-in user
    r.auth = r.ident;
    r
}

fn read_sglang(h: &Http) -> Read {
    let mut r = Read::default();
    let info = h.json("/get_model_info").or_else(|| h.json("/model_info"));
    let server = h
        .json("/get_server_info")
        .or_else(|| h.json("/server_info"));
    if let Some(i) = &info {
        r.ident = i.get("model_path").is_some()
            && (i.get("is_generation").is_some() || i.get("tokenizer_path").is_some());
    }
    r.ident |= owned_by(h, "/v1/models", &["sglang"]);
    r.version = server.as_ref().and_then(|s| str_at(s, "version"));
    let path = info.as_ref().and_then(|i| str_at(i, "model_path"));
    r.models = openai_models(h, "/v1/models", Some(true));
    if r.models.is_empty() {
        if let Some(p) = path {
            let mut x = Model::named(short_name(&p));
            x.loaded = Some(true);
            r.models.push(x);
        }
    }
    if let Some(ctx) = server
        .as_ref()
        .and_then(|s| num_at(s, "context_length"))
        .map(|c| c as u64)
    {
        for m in &mut r.models {
            m.ctx = m.ctx.or(Some(ctx));
        }
    }
    r.load = load_of(
        h,
        &["sglang:num_running_reqs"],
        &["sglang:num_queue_reqs"],
        &["sglang:token_usage"],
    );
    r
}

fn read_tabbyapi(h: &Http) -> Read {
    let mut r = Read::default();
    let mut current = None;
    if let Some(v) = h.json("/v1/model") {
        if str_at(&v, "owned_by").is_some_and(|o| o.eq_ignore_ascii_case("tabbyapi")) {
            r.ident = true;
        }
        if let Some(id) = str_at(&v, "id") {
            let mut x = Model::named(id.clone());
            x.loaded = Some(true);
            x.format = Some("exl2".into());
            x.ctx = v
                .get("parameters")
                .and_then(|p| num_at(p, "max_seq_len"))
                .map(|c| c as u64);
            r.models.push(x);
            current = Some(id);
        }
    }
    r.ident |= owned_by(h, "/v1/models", &["tabbyapi"]);
    if let Some(v) = h.json("/health") {
        // TabbyAPI's health check: {"status": "healthy", "issues": []}
        r.ident |= v.get("issues").is_some_and(Value::is_array) && v.get("status").is_some();
    }
    for mut x in openai_models(h, "/v1/models", Some(false)) {
        if Some(&x.name) != current.as_ref() {
            x.format = Some("exl2".into());
            r.models.push(x);
        }
    }
    r.auth = h.denied("/v1/model") || h.denied("/v1/models");
    r
}

fn read_mlxlm(h: &Http) -> Read {
    // nothing in its answers is unique: it's recognised by its process
    let mut models = openai_models(h, "/v1/models", None);
    for m in &mut models {
        m.format = Some("mlx".into());
    }
    Read {
        models,
        ..Default::default()
    }
}

fn read_gpt4all(h: &Http) -> Read {
    Read {
        ident: owned_by(h, "/v1/models", &["humanity"]),
        models: openai_models(h, "/v1/models", None),
        ..Default::default()
    }
}

fn read_xinference(h: &Http) -> Read {
    let mut r = Read {
        ident: owned_by(h, "/v1/models", &["xinference"]),
        ..Default::default()
    };
    if let Some(v) = h.json("/v1/models") {
        for m in data(&v) {
            let Some(id) = str_at(m, "id") else { continue };
            let mut x = Model::named(id);
            x.loaded = Some(true); // Xinference lists the models it's running
            x.kind = str_at(m, "model_type").map(|k| k.to_lowercase());
            x.params = num_at(m, "model_size_in_billions").map(|b| format!("{b}B"));
            x.quant = str_at(m, "quantization")
                .filter(|q| q != "none")
                .or(x.quant);
            x.format = str_at(m, "model_format");
            x.ctx = num_at(m, "context_length").map(|c| c as u64);
            r.models.push(x);
        }
    }
    r
}

fn read_triton(h: &Http) -> Read {
    let mut r = Read::default();
    if let Some(v) = h.json("/v2") {
        r.ident = str_at(&v, "name").is_some_and(|n| n == "triton");
        r.version = str_at(&v, "version");
    }
    if let Some(Value::Array(a)) = h.json("POST /v2/repository/index") {
        for m in &a {
            let Some(n) = str_at(m, "name") else { continue };
            let mut x = Model::named(n);
            x.quant = None;
            x.loaded = Some(str_at(m, "state").is_some_and(|s| s == "READY"));
            r.models.push(x);
        }
    }
    r
}

fn read_llamaswap(h: &Http) -> Read {
    let mut r = Read {
        ident: owned_by(h, "/v1/models", &["llama-swap"]),
        models: openai_models(h, "/v1/models", Some(false)),
        ..Default::default()
    };
    if let Some(v) = h.json("/running") {
        if let Some(run) = v.get("running").and_then(Value::as_array) {
            r.ident |= run
                .iter()
                .all(|m| m.get("model").is_some() && m.get("state").is_some());
            for m in run {
                let Some(n) = str_at(m, "model") else {
                    continue;
                };
                match r.models.iter_mut().find(|x| x.name == n) {
                    Some(x) => x.loaded = Some(true),
                    None => {
                        let mut x = Model::named(n);
                        x.loaded = Some(true);
                        r.models.push(x);
                    }
                }
            }
        }
    }
    r
}

/// Health from the answers: any answer = up (a 404 still means it's alive),
/// 503 = loading, other 5xx = error, none = down.
fn health_of(s: &Service, h: &Http) -> Health {
    let codes = h.codes();
    if codes.is_empty() {
        return Health::Unknown;
    }
    let hc = s.health.and_then(|p| h.code(p));
    if hc == Some(503) {
        return Health::Loading;
    }
    if codes.iter().all(|c| *c == 0) {
        return Health::Down;
    }
    if hc.is_some_and(is2xx) || codes.iter().any(|c| is2xx(*c)) {
        return Health::Up;
    }
    if codes.contains(&503) {
        return Health::Loading;
    }
    match codes.iter().copied().find(|c| *c >= 500) {
        Some(c) => Health::Error(format!("HTTP {c}")),
        None => Health::Up,
    }
}

// ------------------------------------------------------------------ processes

/// A listening TCP socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    pub port: u16,
    /// the bound address as the tool printed it (`0.0.0.0`, `::1`, `*`)
    pub bind: String,
    pub pid: Option<u32>,
    pub name: Option<String>,
}

/// A process the probe sent (it owns a socket or looks like an LLM server).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub args: String,
    pub cwd: String,
}

/// The names a process goes by, and its arguments after the program.
#[derive(Debug, Clone, Default)]
struct Prog {
    names: Vec<String>,
    rest: Vec<String>,
}

fn base(s: &str) -> &str {
    let t = s.trim().trim_end_matches('/');
    t.rsplit('/').next().unwrap_or(t)
}
fn interpreter(n: &str) -> bool {
    n.starts_with("python") || n.starts_with("pypy") || matches!(n, "node" | "bun" | "deno")
}
/// Wrappers that exec what follows (`nohup vllm serve …`).
fn wrapper(n: &str) -> bool {
    matches!(
        n,
        "env" | "nohup" | "setsid" | "exec" | "nice" | "stdbuf" | "numactl" | "chrt" | "time"
    )
}
fn asgi(n: &str) -> bool {
    matches!(
        n,
        "uvicorn" | "gunicorn" | "hypercorn" | "granian" | "fastapi" | "daphne"
    )
}

impl Prog {
    fn of(p: &Proc) -> Prog {
        let words: Vec<&str> = p.args.split_whitespace().collect();
        let mut names = Vec::new();
        let comm = base(&p.comm).to_lowercase();
        if !comm.is_empty() {
            names.push(comm);
        }
        let at = argv_names(&words, &mut names, 0);
        names.dedup();
        Prog {
            names,
            rest: words[at.min(words.len())..]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

/// Push the names `w` (an argv) runs as; the index where its own arguments start.
fn argv_names(w: &[&str], names: &mut Vec<String>, depth: usize) -> usize {
    let Some(a0) = w.first() else { return 0 };
    let n0 = base(a0).to_lowercase();
    names.push(n0.clone());
    if depth > 3 {
        return 1;
    }
    if wrapper(&n0) {
        let mut i = 1;
        while i < w.len()
            && (w[i].starts_with('-') || w[i].contains('=') || w[i].parse::<f64>().is_ok())
        {
            i += 1;
        }
        return i + argv_names(&w[i..], names, depth + 1);
    }
    if asgi(&n0) {
        return 1 + asgi_app(&w[1..], names);
    }
    if !interpreter(&n0) {
        return 1;
    }
    let mut i = 1;
    while i < w.len() {
        let a = w[i];
        if a == "-m" {
            if let Some(m) = w.get(i + 1) {
                names.push(m.to_lowercase());
            }
            return i + 2;
        }
        if let Some(m) = a
            .strip_prefix("-m")
            .filter(|m| !m.is_empty() && !a.starts_with("--"))
        {
            names.push(m.to_lowercase());
            return i + 1;
        }
        if a == "-c" {
            return w.len();
        }
        if matches!(a, "-X" | "-W" | "-Q") {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        // a script (a console-script wrapper like /venv/bin/vllm is one too)
        let sn = base(a).to_lowercase();
        names.push(sn.clone());
        if asgi(&sn) {
            return i + 1 + asgi_app(&w[i + 1..], names);
        }
        return i + 1;
    }
    i
}

/// `module:app` an ASGI server runs → its module name; how many words that took.
fn asgi_app(w: &[&str], names: &mut Vec<String>) -> usize {
    let mut i = 0;
    while i < w.len() {
        let a = w[i];
        if a.starts_with('-') {
            // flags that take a value
            if !a.contains('=')
                && matches!(
                    a,
                    "--host"
                        | "--port"
                        | "--workers"
                        | "-w"
                        | "-b"
                        | "--bind"
                        | "--app-dir"
                        | "--log-level"
                        | "-k"
                        | "--worker-class"
                        | "--root-path"
                )
            {
                i += 1;
            }
            i += 1;
            continue;
        }
        if a.eq_ignore_ascii_case("run") || a.eq_ignore_ascii_case("serve") {
            i += 1;
            continue;
        }
        let m = a.split(':').next().unwrap_or(a).to_lowercase();
        names.push(m);
        return i + 1;
    }
    i
}

/// Which registry service a process is, and whether it serves by itself.
fn recognise(p: &Proc, on: &[&'static Service]) -> Option<(&'static Service, bool)> {
    let prog = Prog::of(p);
    let hay = format!("{} {}", p.args, p.cwd).to_lowercase();
    let mut owner = None;
    for s in on {
        match s.recognise(&prog, &hay) {
            Some(true) => return Some((s, true)),
            Some(false) if owner.is_none() => owner = Some((*s, false)),
            _ => {}
        }
    }
    owner
}

/// What owns a port that no registry process claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// an interpreter, ASGI server or container proxy: could be anything, ask it
    Runtime,
    /// an ssh or other tunnel: the server is elsewhere
    Tunnel,
    /// a program we know isn't one of ours (ControlCenter, java, redis, …)
    Other,
}

fn owner_class(names: &[String]) -> Owner {
    const TUNNELS: &[&str] = &[
        "ssh",
        "sshd",
        "autossh",
        "cloudflared",
        "ngrok",
        "tailscaled",
        "frpc",
        "chisel",
        "socat",
        "kubectl",
        "mosh-server",
        "code-tunnel",
    ];
    const RUNTIMES: &[&str] = &[
        "uv",
        "docker-proxy",
        "rootlessport",
        "rootlesskit",
        "conmon",
        "slirp4netns",
        "gvproxy",
        "podman",
        "limactl",
        "vpnkit-bridge",
    ];
    if names.iter().any(|n| TUNNELS.contains(&n.as_str())) {
        return Owner::Tunnel;
    }
    let runtime = |n: &str| {
        interpreter(n)
            || asgi(n)
            || RUNTIMES.contains(&n)
            || n.starts_with("pasta")
            || n.starts_with("com.docker")
            || n.starts_with("containerd-shim")
            || n.starts_with("orbstack")
    };
    if names.iter().any(|n| runtime(n)) {
        Owner::Runtime
    } else {
        Owner::Other
    }
}

/// `--port 8001` / `--port=8001` / `--http-port` / `--listen host:port`.
fn arg_port(rest: &[String]) -> Option<u16> {
    const KEYS: &[&str] = &[
        "--port",
        "--http-port",
        "--api-port",
        "--listen-port",
        "--server-port",
    ];
    const HOSTPORT: &[&str] = &["--listen", "--address", "--bind", "--host"];
    let port_of = |v: &str| -> Option<u16> {
        v.rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .filter(|p| *p > 0)
    };
    for (i, w) in rest.iter().enumerate() {
        let (k, v) = match w.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (w.as_str(), rest.get(i + 1).cloned()),
        };
        if KEYS.contains(&k) {
            if let Some(p) = v.and_then(|v| v.parse().ok()).filter(|p| *p > 0) {
                return Some(p);
            }
        } else if HOSTPORT.contains(&k) {
            if let Some(p) = v.filter(|v| v.contains(':')).and_then(|v| port_of(&v)) {
                return Some(p);
            }
        }
    }
    None
}

/// The model a one-model server was started with (`--model …`, `vllm serve <model>`).
fn arg_model(s: &Service, rest: &[String]) -> Option<String> {
    const KEYS: &[&str] = &[
        "--model",
        "--model-path",
        "--model_path",
        "--model-id",
        "--model-name",
        "--served-model-name",
        "-m",
        "-hf",
        "--hf-repo",
    ];
    for (i, w) in rest.iter().enumerate() {
        if let Some((k, v)) = w.split_once('=') {
            if KEYS.contains(&k) && !v.is_empty() {
                return Some(short_name(v));
            }
        } else if KEYS.contains(&w.as_str()) {
            if let Some(v) = rest.get(i + 1).filter(|v| !v.starts_with('-')) {
                return Some(short_name(v));
            }
        }
    }
    // the first positional: `vllm serve <model>`, `trtllm-serve <engine>`,
    // `koboldcpp model.gguf`
    let pos = rest.iter().position(|w| !w.starts_with('-'))?;
    let first = &rest[pos];
    let next = if matches!(first.as_str(), "serve" | "run") {
        rest.get(pos + 1).filter(|w| !w.starts_with('-'))
    } else {
        Some(first)
    };
    next.filter(|_| {
        matches!(
            s.id,
            "vllm" | "aphrodite" | "trtllm" | "koboldcpp" | "llamacpp"
        )
    })
    .filter(|w| s.id != "koboldcpp" && s.id != "llamacpp" || w.ends_with(".gguf"))
    .map(|w| short_name(w))
}

// ------------------------------------------------------------------ the remote's report

/// What the probe's first step printed.
#[derive(Debug, Clone, Default)]
pub struct Remote {
    pub os: String,
    /// `ss`, `lsof`, `netstat`, `proc` or `none`
    pub listeners_tool: String,
    /// `curl`, `wget`, `nc`, `none` (or `fixture` in tests)
    pub http_tool: String,
    /// `lms` (LM Studio's CLI) is installed
    pub lms: bool,
    pub listeners: Vec<Listener>,
    pub procs: BTreeMap<u32, Proc>,
    /// NVIDIA compute apps: pid, bytes
    pub gpu: Vec<(u32, f64)>,
    /// pid → parent, for adding up a server's GPU processes
    pub tree: HashMap<u32, u32>,
}

/// The text after the first `n` whitespace-separated fields.
fn after_fields(s: &str, n: usize) -> &str {
    let mut t = s.trim_start();
    for _ in 0..n {
        let i = t.find(char::is_whitespace).unwrap_or(t.len());
        t = t[i..].trim_start();
    }
    t.trim_end()
}

impl Remote {
    pub fn parse(text: &str) -> Remote {
        let mut r = Remote::default();
        let mut raw: Vec<&str> = Vec::new();
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            let Some((tag, rest)) = line.split_once('|') else {
                continue;
            };
            let mut w = rest.split_whitespace();
            let pid = || -> Option<u32> { rest.split_whitespace().next()?.parse().ok() };
            match tag {
                "K" => {
                    if let Some((k, v)) = rest.split_once('=') {
                        match k {
                            "os" => r.os = v.to_string(),
                            "listeners" => r.listeners_tool = v.to_string(),
                            "http" => r.http_tool = v.to_string(),
                            "lms" => r.lms = v == "1",
                            _ => {}
                        }
                    }
                }
                "L" => raw.push(rest),
                "C" => {
                    let (Some(p), Some(pp)) = (
                        w.next().and_then(|x| x.parse::<u32>().ok()),
                        w.next().and_then(|x| x.parse::<u32>().ok()),
                    ) else {
                        continue;
                    };
                    let e = r.procs.entry(p).or_default();
                    e.pid = p;
                    e.ppid = pp;
                    e.comm = after_fields(rest, 2).to_string();
                }
                "A" => {
                    let Some(p) = pid() else { continue };
                    let e = r.procs.entry(p).or_default();
                    e.pid = p;
                    e.args = after_fields(rest, 1).to_string();
                }
                "W" => {
                    let Some(p) = pid() else { continue };
                    if let Some(e) = r.procs.get_mut(&p) {
                        e.cwd = after_fields(rest, 1).to_string();
                    }
                }
                "G" => {
                    let (Some(p), Some(mib)) = (
                        w.next().and_then(|x| x.parse::<u32>().ok()),
                        w.next().and_then(|x| x.parse::<f64>().ok()),
                    ) else {
                        continue;
                    };
                    r.gpu.push((p, mib * 1024.0 * 1024.0));
                }
                "T" => {
                    if let (Some(p), Some(pp)) = (
                        w.next().and_then(|x| x.parse::<u32>().ok()),
                        w.next().and_then(|x| x.parse::<u32>().ok()),
                    ) {
                        r.tree.insert(p, pp);
                    }
                }
                _ => {}
            }
        }
        r.listeners = parse_listeners(&r.listeners_tool, &raw);
        for p in r.procs.values() {
            r.tree.entry(p.pid).or_insert(p.ppid);
        }
        r
    }

    fn parent(&self, pid: u32) -> Option<u32> {
        self.tree
            .get(&pid)
            .copied()
            .filter(|p| *p != pid && *p != 0)
    }
    /// Is `anc` an ancestor of (or the same as) `pid`?
    fn descends(&self, pid: u32, anc: u32) -> bool {
        let mut p = pid;
        for _ in 0..64 {
            if p == anc {
                return true;
            }
            match self.parent(p) {
                Some(x) => p = x,
                None => return false,
            }
        }
        false
    }
    /// Does the listener tool show every socket (not just this user's)?
    fn complete(&self) -> bool {
        matches!(self.listeners_tool.as_str(), "ss" | "netstat" | "proc")
    }
}

/// `addr:port` (also `[v6]:port`, `127.0.0.53%lo:53`, BSD `*.5000`) → (addr, port).
fn split_addr(s: &str) -> Option<(String, u16)> {
    let s = s.trim();
    let (a, p) = if s.contains(':') {
        s.rsplit_once(':')?
    } else {
        s.rsplit_once('.')?
    };
    let port = p.parse::<u16>().ok().filter(|p| *p > 0)?;
    let a = a.trim_start_matches('[').trim_end_matches(']');
    let a = a.split('%').next().unwrap_or(a);
    Some((a.to_string(), port))
}

fn parse_listeners(tool: &str, raw: &[&str]) -> Vec<Listener> {
    let mut v: Vec<Listener> = Vec::new();
    match tool {
        "ss" => {
            for l in raw {
                let w: Vec<&str> = l.split_whitespace().collect();
                if w.first() == Some(&"State") || w.len() < 4 {
                    continue;
                }
                // `LISTEN 0 4096 127.0.0.1:11434 0.0.0.0:* users:(("ollama",pid=1,fd=3))`
                let Some((bind, port)) = split_addr(w[3]) else {
                    continue;
                };
                let mut owners: Vec<(u32, String)> = Vec::new();
                if let Some(i) = l.find("users:(") {
                    for part in l[i..].split("((").flat_map(|x| x.split("),(")) {
                        let name = part.split('"').nth(1).map(str::to_string);
                        let pid = part
                            .split("pid=")
                            .nth(1)
                            .and_then(|x| x.split(|c: char| !c.is_ascii_digit()).next())
                            .and_then(|x| x.parse::<u32>().ok());
                        if let (Some(n), Some(p)) = (name, pid) {
                            owners.push((p, n));
                        }
                    }
                }
                owners.sort();
                let o = owners.into_iter().next();
                v.push(Listener {
                    port,
                    bind,
                    pid: o.as_ref().map(|x| x.0),
                    name: o.map(|x| x.1),
                });
            }
        }
        "lsof" => {
            let (mut pid, mut name) = (None, None);
            for l in raw {
                if let Some(p) = l.strip_prefix('p') {
                    pid = p.trim().parse::<u32>().ok();
                    name = None;
                } else if let Some(c) = l.strip_prefix('c') {
                    name = Some(c.to_string());
                } else if let Some(n) = l.strip_prefix('n') {
                    if n.contains("->") {
                        continue;
                    }
                    if let Some((bind, port)) = split_addr(n) {
                        v.push(Listener {
                            port,
                            bind,
                            pid,
                            name: name.clone(),
                        });
                    }
                }
            }
        }
        "netstat" => {
            for l in raw {
                let w: Vec<&str> = l.split_whitespace().collect();
                if w.len() < 4 || !w[0].starts_with("tcp") || !l.contains("LISTEN") {
                    continue;
                }
                let Some((bind, port)) = split_addr(w[3]) else {
                    continue;
                };
                let owner = w
                    .last()
                    .and_then(|x| x.split_once('/'))
                    .and_then(|(p, n)| p.parse::<u32>().ok().map(|p| (p, n.to_string())));
                v.push(Listener {
                    port,
                    bind,
                    pid: owner.as_ref().map(|x| x.0),
                    name: owner.map(|x| x.1),
                });
            }
        }
        "proc" => {
            // `0: 0100007F:2CAA 00000000:0000 0A …` (hex, little-endian words)
            for l in raw {
                let w: Vec<&str> = l.split_whitespace().collect();
                if w.len() < 4 || w[3] != "0A" {
                    continue;
                }
                let Some((a, p)) = w[1].split_once(':') else {
                    continue;
                };
                let Ok(port) = u16::from_str_radix(p, 16) else {
                    continue;
                };
                v.push(Listener {
                    port,
                    bind: hex_addr(a),
                    pid: None,
                    name: None,
                });
            }
        }
        _ => {}
    }
    v.dedup();
    v
}

/// `/proc/net/tcp` addresses: `0100007F` → `127.0.0.1`; 32 hex digits → IPv6.
fn hex_addr(h: &str) -> String {
    let word = |s: &str| u32::from_str_radix(s, 16).ok().map(u32::swap_bytes);
    if h.len() == 8 {
        return word(h)
            .map(|x| std::net::Ipv4Addr::from(x).to_string())
            .unwrap_or_default();
    }
    if h.len() == 32 {
        let mut b = [0u8; 16];
        for i in 0..4 {
            if let Some(x) = word(&h[i * 8..i * 8 + 8]) {
                b[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
            }
        }
        return std::net::Ipv6Addr::from(b).to_string();
    }
    String::new()
}

/// Where to reach a socket bound to `bind`.
fn connect_addr(bind: &str) -> String {
    match bind {
        "" | "*" | "0.0.0.0" | "::" | "127.0.0.1" | "::ffff:0.0.0.0" | "localhost" => {
            "127.0.0.1".into()
        }
        "::1" => "[::1]".into(),
        x if x.contains(':') => format!("[{x}]"),
        x => x.to_string(),
    }
}

// ------------------------------------------------------------------ discovery

/// One request for the remote: an HTTP `GET`/`POST`, or a command (`!lms`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Req {
    pub port: u16,
    pub addr: String,
    /// as the registry writes it: `/v1/models`, `POST /v2/repository/index`, `!lms`
    pub spec: String,
}

impl Req {
    /// (method, path) as the remote script takes them.
    pub fn method_path(&self) -> (&str, &str) {
        if let Some(p) = self.spec.strip_prefix("POST ") {
            ("POST", p)
        } else if let Some(c) = self.spec.strip_prefix('!') {
            ("CMD", c)
        } else {
            ("GET", &self.spec)
        }
    }
}

/// A listening port worth a look.
struct Cand {
    port: u16,
    addr: String,
    bind: String,
    pid: Option<u32>,
    process: String,
    /// its owner is this service's process (`true`: one that serves by itself)
    known: Option<(&'static Service, bool)>,
    /// its owner's own socket (not just a port named on a command line)
    owned: bool,
    /// services whose default port this is
    guess: Vec<&'static Service>,
    http: Http,
    id: Option<&'static Service>,
}

/// Classify every listening port, ask the right questions (`fetch` sends
/// them to the machine and returns the answers in order) and say what runs
/// there. Pure apart from `fetch`, so tests drive it with fixtures.
pub fn discover(
    rm: &Remote,
    on: &[&'static Service],
    fetch: &mut dyn FnMut(&[Req]) -> Vec<Resp>,
) -> Vec<Found> {
    let progs: BTreeMap<u32, Prog> = rm.procs.iter().map(|(p, x)| (*p, Prog::of(x))).collect();
    let matched: BTreeMap<u32, (&'static Service, bool)> = rm
        .procs
        .iter()
        .filter_map(|(p, x)| recognise(x, on).map(|m| (*p, m)))
        .collect();
    let defaults = |port: u16| -> Vec<&'static Service> {
        on.iter()
            .copied()
            .filter(|s| s.ports.contains(&port))
            .collect()
    };
    let has_http = !matches!(rm.http_tool.as_str(), "none" | "");

    // 1. the ports worth asking
    let mut cands: Vec<Cand> = Vec::new();
    let mut ports: BTreeMap<u16, Vec<&Listener>> = BTreeMap::new();
    for l in &rm.listeners {
        ports.entry(l.port).or_default().push(l);
    }
    let mut runtimes = 0;
    for (port, ls) in &ports {
        let pid = ls.iter().find_map(|l| l.pid);
        let l = ls
            .iter()
            .find(|l| connect_addr(&l.bind) == "127.0.0.1")
            .unwrap_or(&ls[0]);
        let name = ls
            .iter()
            .find_map(|l| l.name.clone())
            .or_else(|| {
                pid.and_then(|p| rm.procs.get(&p))
                    .map(|p| base(&p.comm).to_string())
            })
            .unwrap_or_default();
        let guess = defaults(*port);
        let mut c = Cand {
            port: *port,
            addr: connect_addr(&l.bind),
            bind: l.bind.clone(),
            pid,
            process: name.clone(),
            known: None,
            owned: pid.is_some(),
            guess: vec![],
            http: Http {
                port: *port,
                ..Default::default()
            },
            id: None,
        };
        match pid {
            Some(p) => {
                if let Some(m) = matched.get(&p) {
                    c.known = Some(*m);
                } else {
                    // what ps calls it, and what the socket tool does (a
                    // process can retitle itself: node → `next-server (v16)`)
                    let mut names = progs.get(&p).map(|x| x.names.clone()).unwrap_or_default();
                    names.extend(
                        ls.iter()
                            .filter_map(|l| l.name.as_deref())
                            .map(str::to_lowercase),
                    );
                    match owner_class(&names) {
                        Owner::Runtime => {
                            if guess.is_empty() {
                                runtimes += 1;
                                if runtimes > 16 {
                                    continue;
                                }
                            }
                            c.guess = guess;
                        }
                        Owner::Tunnel | Owner::Other => continue,
                    }
                }
            }
            None if guess.is_empty() => continue,
            None => c.guess = guess,
        }
        cands.push(c);
    }
    // nothing lists sockets here: ask the default ports
    if matches!(rm.listeners_tool.as_str(), "none" | "") {
        for s in on {
            for p in s.ports {
                if !cands.iter().any(|c| c.port == *p) {
                    cands.push(Cand {
                        port: *p,
                        addr: "127.0.0.1".into(),
                        bind: String::new(),
                        pid: None,
                        process: String::new(),
                        known: None,
                        owned: false,
                        guess: defaults(*p),
                        http: Http {
                            port: *p,
                            ..Default::default()
                        },
                        id: None,
                    });
                }
            }
        }
    }
    // a server's port named on its command line, when its socket shows no
    // owner (another user's process): ask it that service's questions
    for (pid, m) in &matched {
        let Some(p) = arg_port(&progs[pid].rest) else {
            continue;
        };
        if let Some(c) = cands.iter_mut().find(|c| c.port == p && c.pid.is_none()) {
            c.known = Some((m.0, false));
            c.pid = Some(*pid);
            c.process = base(&rm.procs[pid].comm).to_string();
        }
    }

    // 2. first round: who are you?
    let ask = |c: &Cand| -> Vec<&'static str> {
        match (c.known, c.guess.is_empty()) {
            (Some((s, _)), _) => s.paths(),
            (None, false) => {
                let mut v: Vec<&'static str> = Vec::new();
                for s in &c.guess {
                    for p in s.ident {
                        if !v.contains(p) {
                            v.push(p);
                        }
                    }
                }
                v
            }
            (None, true) => GENERIC.to_vec(),
        }
    };
    if has_http {
        let plan: Vec<(usize, Vec<&'static str>)> =
            cands.iter().enumerate().map(|(i, c)| (i, ask(c))).collect();
        round(&mut cands, &plan, fetch);
    }
    for c in &mut cands {
        c.id = match c.known {
            Some((s, serves)) => {
                let r = (s.read)(&c.http);
                let alive = c.http.codes().iter().any(|x| *x != 0);
                let default = s.ports.contains(&c.port);
                let accept =
                    r.ident || (c.owned && serves) || (c.owned && default && (alive || !has_http));
                accept.then_some(s)
            }
            None => c
                .guess
                .iter()
                .chain(on.iter())
                .copied()
                .find(|s| (s.read)(&c.http).ident),
        };
    }

    // 3. second round: the details of whatever answered
    if has_http {
        let plan: Vec<(usize, Vec<&'static str>)> = cands
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let s = c.id?;
                let mut v: Vec<&'static str> =
                    s.paths().into_iter().filter(|p| !c.http.asked(p)).collect();
                if s.id == "lmstudio" && rm.lms && !c.http.ok("/api/v0/models") {
                    v.push("!lms");
                }
                (!v.is_empty()).then_some((i, v))
            })
            .collect();
        round(&mut cands, &plan, fetch);
    }

    // 4. what runs where
    let mut out: Vec<Found> = Vec::new();
    for c in &cands {
        let Some(s) = c.id else { continue };
        let r = (s.read)(&c.http);
        let mut f = Found::new(s);
        f.port = Some(c.port);
        f.ports = vec![c.port];
        f.addr = c.addr.clone();
        f.bind = c.bind.clone();
        f.pid = c.pid;
        f.process = c.process.clone();
        f.version = r.version;
        f.models = r.models;
        f.load = r.load;
        f.auth = r.auth || s.models.is_some_and(|p| c.http.denied(p));
        f.health = if !has_http {
            Health::Unknown
        } else {
            r.health.unwrap_or_else(|| health_of(s, &c.http))
        };
        f.by_http = r.ident;
        f.by_process = c.known.is_some();
        out.push(f);
    }
    // a server found by its answers, whose process we saw running: tie them
    for f in out.iter_mut().filter(|f| f.pid.is_none()) {
        let port = f.port;
        if let Some((pid, _)) = matched.iter().find(|(pid, (s, serves))| {
            s.id == f.service.id
                && *serves
                && arg_port(&progs[*pid].rest)
                    .map_or(port.is_some_and(|p| s.ports.contains(&p)), |p| {
                        Some(p) == port
                    })
        }) {
            f.pid = Some(*pid);
            f.process = base(&rm.procs[pid].comm).to_string();
            f.by_process = true;
        }
    }
    // servers running without a port we could reach (loading, another user's,
    // in a container, no listener tool): one entry per service
    for (pid, (s, serves)) in &matched {
        if !serves || out.iter().any(|f| f.service.id == s.id) {
            continue;
        }
        if matched
            .iter()
            .any(|(q, (t, _))| q != pid && t.id == s.id && rm.descends(*pid, *q))
        {
            continue; // a child of another of its processes
        }
        let p = &rm.procs[pid];
        let mut f = Found::new(s);
        f.port = arg_port(&progs[pid].rest);
        f.ports = f.port.into_iter().collect();
        f.addr = "127.0.0.1".into();
        f.pid = Some(*pid);
        f.process = base(&p.comm).to_string();
        f.by_process = true;
        f.health = match f.port {
            Some(port) if rm.complete() && !rm.listeners.iter().any(|l| l.port == port) => {
                Health::NotListening
            }
            _ => Health::Unknown,
        };
        out.push(f);
    }
    // one entry per server process: its other ports join it
    let mut merged: Vec<Found> = Vec::new();
    for f in out {
        match merged
            .iter_mut()
            .find(|g| g.service.id == f.service.id && g.pid.is_some() && g.pid == f.pid)
        {
            Some(g) => {
                for p in f.ports {
                    if !g.ports.contains(&p) {
                        g.ports.push(p);
                    }
                }
                if !g.by_http && f.by_http {
                    let ports = std::mem::take(&mut g.ports);
                    *g = Found { ports, ..f };
                }
            }
            None => merged.push(f),
        }
    }
    for f in &mut merged {
        if let Some(p) = f.port {
            f.ports.retain(|x| *x != p);
            f.ports.insert(0, p);
        }
        // the model it was started with, when its answers don't say
        if f.models.is_empty() && f.service.one_model {
            if let Some(m) = f
                .pid
                .and_then(|p| progs.get(&p))
                .and_then(|p| arg_model(f.service, &p.rest))
            {
                f.models.push(Model::named(m));
            }
        }
        // GPU memory of the server and its children
        if let Some(pid) = f.pid {
            let v: f64 = rm
                .gpu
                .iter()
                .filter(|(g, _)| rm.descends(*g, pid))
                .map(|x| x.1)
                .sum();
            if v > 0.0 {
                f.vram = Some(v);
                let mut loaded = f.models.iter_mut().filter(|m| m.loaded != Some(false));
                if let (Some(m), None) = (loaded.next(), loaded.next()) {
                    m.vram = m.vram.or(Some(v));
                }
            }
        }
    }
    let rank = |f: &Found| {
        (
            f.service.kind as u8,
            ALL.iter().position(|s| s.id == f.service.id).unwrap_or(99),
            f.port.unwrap_or(u16::MAX),
        )
    };
    merged.sort_by_key(rank);
    merged
}

/// Send one round of requests and file the answers under their ports.
fn round(
    cands: &mut [Cand],
    plan: &[(usize, Vec<&'static str>)],
    fetch: &mut dyn FnMut(&[Req]) -> Vec<Resp>,
) {
    let mut reqs: Vec<(usize, Req)> = Vec::new();
    for (i, paths) in plan {
        for p in paths {
            reqs.push((
                *i,
                Req {
                    port: cands[*i].port,
                    addr: cands[*i].addr.clone(),
                    spec: p.to_string(),
                },
            ));
        }
    }
    if reqs.is_empty() {
        return;
    }
    let only: Vec<Req> = reqs.iter().map(|x| x.1.clone()).collect();
    let answers = fetch(&only);
    for (k, (i, r)) in reqs.iter().enumerate() {
        let a = answers.get(k).cloned().unwrap_or_default();
        cands[*i].http.put(&r.spec, a.code, &a.body);
    }
}

// ------------------------------------------------------------------ the remote probe

/// The probe (POSIX sh + awk, read-only, sent on stdin). It's one `{ … }`
/// block, so the shell has read all of it before it runs anything — later
/// stdin is then only the requests (`id⇥port⇥address⇥method⇥path` lines, `go`
/// to run them, `end` to stop).
///
/// | tag | fields |
/// |---|---|
/// | `K` | `os=`, `listeners=ss|lsof|netstat|proc|none`, `http=curl|wget|nc|none`, `lms=1` |
/// | `L` | a raw line of the listener tool |
/// | `C` | pid, ppid, comm (processes that own a socket or look like LLM servers) |
/// | `A` | pid, command line (≤ 1500 chars) |
/// | `W` | pid, working folder (interpreters running a `*.py`) |
/// | `G` | pid, MiB (NVIDIA compute apps) — then `T` pid, ppid for every process |
/// | `@@1` | end of the report |
/// | `R` | `id|code` of an answer; its body follows as `|`-prefixed lines |
/// | `@@done` | end of a batch |
pub const PROBE: &str = r#"{
# aleph: LLM servers and AI gateways on this machine. POSIX sh + awk,
# read-only, nothing installed, every tool optional.
export LC_ALL=C
PATH="$PATH:/usr/sbin:/sbin:/usr/local/bin:/opt/homebrew/bin"
unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY
no_proxy='*'; NO_PROXY='*'; export no_proxy NO_PROXY
TAB=$(printf '\t')
T=$(mktemp -d 2>/dev/null) || { T=/tmp/aleph-svc.$$; mkdir -m 700 "$T" || exit 1; }
trap 'rm -rf "$T"' EXIT
trap 'exit 1' HUP INT TERM
FX=${ALEPH_SERVICES_FIXTURES:-}
[ -n "$FX" ] && [ ! -d "$FX" ] && FX=
RE='@RE@'
has() { command -v "$1" >/dev/null 2>&1; }
# limit S cmd…: run cmd for at most S seconds (its stdin comes through fd 3)
limit() {
  _s=$1; shift
  "$@" 0<&3 3<&- & _p=$!
  ( sleep "$_s"; kill "$_p" 2>/dev/null ) >/dev/null 2>&1 3<&- & _w=$!
  wait "$_p"; _r=$?
  kill "$_w" 2>/dev/null
  return $_r
} 3<&0

report() {
  echo "K|os=$(uname -s 2>/dev/null)"
  # ---- (a) listening TCP sockets, with their owners where the tool can tell
  L=none; : > "$T/l"
  if [ -n "$FX" ]; then
    L=$(cat "$FX/tool" 2>/dev/null); [ -n "$L" ] || L=none
    cat "$FX/listeners" > "$T/l" 2>/dev/null
  else
    if has ss; then { ss -ltnpH 2>/dev/null || ss -ltnp 2>/dev/null; } > "$T/l"; [ -s "$T/l" ] && L=ss; fi
    if [ $L = none ] && has lsof; then lsof -nP -iTCP -sTCP:LISTEN +c 0 -Fpcn > "$T/l" 2>/dev/null; [ -s "$T/l" ] && L=lsof; fi
    if [ $L = none ] && has netstat; then { netstat -ltnp 2>/dev/null || netstat -ltn 2>/dev/null || netstat -an 2>/dev/null; } | grep LISTEN > "$T/l"; [ -s "$T/l" ] && L=netstat; fi
    if [ $L = none ] && [ -r /proc/net/tcp ]; then cat /proc/net/tcp /proc/net/tcp6 2>/dev/null | awk '$4 == "0A"' > "$T/l"; L=proc; fi
  fi
  echo "K|listeners=$L"
  awk '{ print "L|" $0 }' "$T/l"
  case $L in
    ss) grep -o 'pid=[0-9]*' "$T/l" | cut -d= -f2 ;;
    lsof) sed -n 's/^p//p' "$T/l" ;;
    netstat) awk '{ print $NF }' "$T/l" | sed -n 's#^\([0-9][0-9]*\)/.*#\1#p' ;;
  esac | sort -u > "$T/lp"
  # ---- (b) processes: socket owners, and anything that looks like an LLM server
  # (ps runs before awk, so the filter never matches itself)
  if [ -n "$FX" ]; then
    cat "$FX/ps" > "$T/pc" 2>/dev/null; cat "$FX/args" > "$T/pa" 2>/dev/null
  else
    ps -A -o pid=,ppid=,comm= > "$T/pc" 2>/dev/null || ps -o pid,ppid,comm > "$T/pc" 2>/dev/null
    ps -A -ww -o pid=,args= > "$T/pa" 2>/dev/null || ps -A -o pid=,args= > "$T/pa" 2>/dev/null || ps -o pid,args > "$T/pa" 2>/dev/null
  fi
  LP=" $(tr '\n' ' ' < "$T/lp") "
  awk -v re="$RE" -v lp="$LP" '$1 ~ /^[0-9]+$/ && (index(lp, " " $1 " ") || tolower($0) ~ re) { print $1 }' "$T/pa" | head -n 300 > "$T/sel"
  S=" $(tr '\n' ' ' < "$T/sel") "
  awk -v s="$S" '$1 ~ /^[0-9]+$/ && index(s, " " $1 " ") { print "C|" $0 }' "$T/pc"
  awk -v s="$S" '$1 ~ /^[0-9]+$/ && index(s, " " $1 " ") { print "A|" substr($0, 1, 1500) }' "$T/pa"
  # the working folder of a script an interpreter runs (server.py, main.py, …)
  awk -v s="$S" '$1 ~ /^[0-9]+$/ && index(s, " " $1 " ") && $0 ~ /[A-Za-z_]+[.]py/ { print $1 }' "$T/pa" | head -n 20 > "$T/cw"
  while read -r p; do
    d=
    if [ -n "$FX" ]; then d=$(awk -v p="$p" '$1 == p { sub(/^[ \t]*[0-9]+[ \t]+/, ""); print; exit }' "$FX/cwd" 2>/dev/null)
    elif [ -d "/proc/$p" ]; then d=$(readlink "/proc/$p/cwd" 2>/dev/null)
    elif has lsof; then d=$(lsof -a -p "$p" -d cwd -Fn 2>/dev/null </dev/null | sed -n 's/^n//p' | head -n 1)
    fi
    [ -n "$d" ] && echo "W|$p $d"
  done < "$T/cw"
  # ---- NVIDIA compute apps (VRAM per process), with the tree to add them up
  G=
  if [ -n "$FX" ]; then G=$(cat "$FX/gpu" 2>/dev/null)
  else
    NV=$(command -v nvidia-smi 2>/dev/null)
    for f in /usr/lib/wsl/lib/nvidia-smi /usr/local/nvidia/bin/nvidia-smi; do [ -z "$NV" ] && [ -x "$f" ] && NV=$f; done
    [ -n "$NV" ] && G=$(limit 8 "$NV" --query-compute-apps=pid,used_memory --format=csv,noheader,nounits 2>/dev/null)
  fi
  if [ -n "$G" ]; then
    printf '%s\n' "$G" | awk -F', *' '$1 ~ /^[0-9]+$/ && $2 ~ /^[0-9]+$/ { print "G|" $1 " " $2 }'
    awk '$1 ~ /^[0-9]+$/ { print "T|" $1 " " $2 }' "$T/pc"
  fi
  # ---- (c) the HTTP client, and LM Studio's CLI
  if [ -n "$FX" ]; then HTTP=$(cat "$FX/http_tool" 2>/dev/null); [ -n "$HTTP" ] || HTTP=curl
  elif has curl; then HTTP=curl
  elif has wget; then HTTP=wget
  elif has nc; then HTTP=nc
  else HTTP=none; fi
  echo "K|http=$HTTP"
  LMS=
  if [ -n "$FX" ]; then [ -f "$FX/lms" ] && LMS=fixture
  else
    LMS=$(command -v lms 2>/dev/null)
    for f in "$HOME/.lmstudio/bin/lms" "$HOME/.cache/lm-studio/bin/lms"; do [ -z "$LMS" ] && [ -x "$f" ] && LMS=$f; done
  fi
  [ -n "$LMS" ] && echo "K|lms=1"
  echo "@@1"
}

# fetch id address port method path → $T/id.c (status, 000 = no answer), $T/id.b (body)
fetch() {
  b="$T/$1.b"; : > "$b"; c=000
  if [ "$4" = CMD ]; then
    if [ "$5" = lms ]; then
      if [ "$LMS" = fixture ]; then cat "$FX/lms" > "$b"; c=200
      elif [ -n "$LMS" ]; then limit 8 "$LMS" ps --json > "$b" 2>/dev/null </dev/null && c=200; fi
    fi
  else
    u="http://$2:$3$5"
    [ -n "$FX" ] && H=fixture || H=$HTTP
    case $H in
    fixture)
      f="$FX/http/$3$(printf '%s' "$5" | tr '/' '_')"
      if [ -f "$f" ]; then c=$(sed -n 1p "$f"); sed 1d "$f" > "$b"; fi ;;
    curl)
      if [ "$4" = POST ]; then
        c=$(curl -s -o "$b" -w '%{http_code}' --noproxy '*' --connect-timeout 1 --max-time 2 -X POST -H 'Content-Type: application/json' -d '{}' "$u" 2>/dev/null </dev/null)
      else
        c=$(curl -s -o "$b" -w '%{http_code}' --noproxy '*' --connect-timeout 1 --max-time 2 "$u" 2>/dev/null </dev/null)
      fi ;;
    wget)
      if [ "$4" = POST ]; then
        wget -q -S -T 2 -t 1 -O "$b" --header='Content-Type: application/json' --post-data='{}' "$u" 2> "$T/$1.h" </dev/null
      else
        wget -q -S -T 2 -t 1 -O "$b" "$u" 2> "$T/$1.h" </dev/null
      fi
      k=$?; c=$(awk '$1 ~ /^HTTP\// { c = $2 } END { print c }' "$T/$1.h")
      [ -z "$c" ] && [ $k = 0 ] && c=200 ;;
    nc)
      h=$2; case $h in \[*) h=${h#?}; h=${h%?} ;; esac
      if [ "$4" = POST ]; then
        printf 'POST %s HTTP/1.0\r\nHost: %s:%s\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}' "$5" "$2" "$3"
      else
        printf 'GET %s HTTP/1.0\r\nHost: %s:%s\r\nConnection: close\r\n\r\n' "$5" "$2" "$3"
      fi > "$T/$1.q"
      limit 3 nc -w 2 "$h" "$3" < "$T/$1.q" > "$T/$1.r" 2>/dev/null
      c=$(sed -n '1s/^HTTP\/[0-9.]* \([0-9][0-9]*\).*/\1/p' "$T/$1.r")
      awk 'f { print } /^\r?$/ { f = 1 }' "$T/$1.r" > "$b" ;;
    esac
  fi
  case $c in [0-9][0-9][0-9]) ;; *) c=000 ;; esac
  echo "$c" > "$T/$1.c"
}

# run the queued requests (one job per port, ports in parallel) and print the answers
batch() {
  for port in $(cut -f2 "$T/q" | sort -u); do
    awk -F '\t' -v p="$port" '$2 == p' "$T/q" > "$T/q.$port"
    ( while IFS="$TAB" read -r id pt ad me pa; do fetch "$id" "$ad" "$pt" "$me" "$pa"; done < "$T/q.$port" ) &
  done
  wait
  while IFS="$TAB" read -r id pt ad me pa; do
    printf 'R|%s|%s\n' "$id" "$(cat "$T/$id.c" 2>/dev/null || echo 000)"
    case $pa in
      */metrics) grep -v -e '^#' -e '_bucket' "$T/$id.b" 2>/dev/null | head -n 300 ;;
      *) dd if="$T/$id.b" bs=4096 count=64 2>/dev/null ;;
    esac | awk '{ print "|" $0 }'
  done < "$T/q"
  echo "@@done"
}

report </dev/null
: > "$T/q"
while IFS= read -r line; do
  case $line in
    go) batch; : > "$T/q" ;;
    end) break ;;
    *) printf '%s\n' "$line" >> "$T/q" ;;
  esac
done
}
"#;

/// The probe with the process filter filled in.
pub fn script() -> String {
    PROBE.replace("@RE@", &coarse_regex())
}

/// How long one machine's probe may take, all rounds included.
pub const TIMEOUT: Duration = Duration::from_secs(25);

/// Scan `alias` once (`local` = this computer): ssh, the probe, two rounds of
/// questions. `slot` holds the running ssh so a [`Watcher`] can kill it.
pub fn probe(alias: &str, on: &[&'static Service]) -> Scan {
    let slot = Arc::new(Mutex::new(None));
    run(alias, sshcfg::shell(alias, "sh -s"), on, TIMEOUT, &slot)
}

/// Scan every host in parallel.
pub fn scan_all(hosts: &[String], on: &[&'static Service]) -> Vec<Scan> {
    let hs: Vec<_> = hosts
        .iter()
        .map(|h| {
            let (h, on) = (h.clone(), on.to_vec());
            thread::spawn(move || probe(&h, &on))
        })
        .collect();
    hosts
        .iter()
        .zip(hs)
        .map(|(h, t)| {
            t.join().unwrap_or_else(|_| Scan {
                err: Some("probe crashed".into()),
                ..Scan::new(h)
            })
        })
        .collect()
}

/// Probe a simulated machine (`tests/fixtures/services/<name>/`: tool output
/// and recorded HTTP answers) with the real script, here.
#[cfg(test)]
pub fn simulate(name: &str, on: &[&'static Service]) -> Scan {
    let mut c = Command::new("sh");
    c.arg("-s").env(
        "ALEPH_SERVICES_FIXTURES",
        format!(
            "{}/tests/fixtures/services/{name}",
            env!("CARGO_MANIFEST_DIR")
        ),
    );
    let slot = Arc::new(Mutex::new(None));
    run(name, c, on, Duration::from_secs(20), &slot)
}

/// Run the probe through `cmd` (`sh -s` here or over ssh).
fn run(
    alias: &str,
    mut cmd: Command,
    on: &[&'static Service],
    deadline: Duration,
    slot: &Arc<Mutex<Option<Child>>>,
) -> Scan {
    // showcase's demo world: its machines' servers
    if let Some(s) = crate::showcase::services(alias, on) {
        return s;
    }
    let t0 = Instant::now();
    let end = t0 + deadline;
    let mut scan = Scan::new(alias);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            scan.err = Some(format!("ssh: {e}"));
            return scan;
        }
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        scan.err = Some("no pipes to the probe".into());
        let _ = child.kill();
        return scan;
    };
    let errbuf = Arc::new(Mutex::new(String::new()));
    if let Some(mut e) = child.stderr.take() {
        let eb = errbuf.clone();
        thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            let mut g = eb.lock().unwrap();
            g.push_str(&s[..s.len().min(4000)]);
        });
    }
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut r = BufReader::new(stdout);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match r.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let l = String::from_utf8_lossy(&buf);
                    if tx
                        .send(l.trim_end_matches(['\n', '\r']).to_string())
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    *slot.lock().unwrap() = Some(child);
    let mut timed_out = false;
    let result = talk(alias, &mut stdin, &rx, end, on, &errbuf, &mut timed_out);
    drop(stdin);
    // reap: the probe exits on `end`; give it a moment, then make sure
    if let Some(mut c) = slot.lock().unwrap().take() {
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until && matches!(c.try_wait(), Ok(None)) {
            thread::sleep(Duration::from_millis(20));
        }
        let _ = c.kill();
        let _ = c.wait();
    }
    scan.took_ms = t0.elapsed().as_millis() as u64;
    match result {
        Ok((rm, found)) => {
            scan.found = found;
            scan.listeners = rm.listeners_tool;
            scan.http = rm.http_tool;
            scan.at = crate::cache::now();
            scan.partial = timed_out;
        }
        Err(e) => scan.err = Some(e),
    }
    crate::proc::dlog(&format!(
        "{:>6}ms services {alias} → {} found{}{}",
        scan.took_ms,
        scan.found.len(),
        if scan.partial { " (partial)" } else { "" },
        scan.err
            .as_ref()
            .map(|e| format!(" · {e}"))
            .unwrap_or_default()
    ));
    scan
}

/// The conversation: send the probe, read its report, then answer
/// [`discover`]'s rounds of questions over the same stdin/stdout.
fn talk(
    alias: &str,
    stdin: &mut std::process::ChildStdin,
    rx: &mpsc::Receiver<String>,
    end: Instant,
    on: &[&'static Service],
    errbuf: &Arc<Mutex<String>>,
    timed_out: &mut bool,
) -> Result<(Remote, Vec<Found>), String> {
    let left = || end.saturating_duration_since(Instant::now());
    stdin
        .write_all(script().as_bytes())
        .and_then(|_| stdin.flush())
        .map_err(|e| format!("couldn't send the probe: {e}"))?;
    let mut first = String::new();
    loop {
        match rx.recv_timeout(left()) {
            Ok(l) if l == "@@1" => break,
            Ok(l) => {
                first.push_str(&l);
                first.push('\n');
            }
            Err(mpsc::RecvTimeoutError::Timeout) => return Err("timed out".into()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // let the stderr reader catch ssh's last words
                thread::sleep(Duration::from_millis(100));
                return Err(errbuf
                    .lock()
                    .unwrap()
                    .lines()
                    .map(str::trim)
                    .rfind(|l| !l.is_empty() && !l.starts_with("Warning: Permanently added"))
                    .map(str::to_string)
                    .unwrap_or_else(|| "the probe ended early".into()));
            }
        }
    }
    let rm = Remote::parse(&first);
    let mut id = 0usize;
    let found = discover(&rm, on, &mut |reqs: &[Req]| {
        let mut out = vec![Resp::default(); reqs.len()];
        if *timed_out {
            return out;
        }
        let mut msg = String::new();
        for (k, r) in reqs.iter().enumerate() {
            let (m, p) = r.method_path();
            msg.push_str(&format!("{}\t{}\t{}\t{m}\t{p}\n", id + k, r.port, r.addr));
        }
        msg.push_str("go\n");
        if crate::proc::debug_path().is_some() {
            let mut by: BTreeMap<u16, Vec<&str>> = BTreeMap::new();
            for r in reqs {
                by.entry(r.port).or_default().push(&r.spec);
            }
            let asks: Vec<String> = by
                .iter()
                .map(|(p, v)| format!(":{p} {}", v.join(" ")))
                .collect();
            crate::proc::dlog(&format!("services {alias} asks {}", asks.join(" · ")));
        }
        if stdin
            .write_all(msg.as_bytes())
            .and_then(|_| stdin.flush())
            .is_err()
        {
            return out;
        }
        let mut cur: Option<usize> = None;
        loop {
            match rx.recv_timeout(left()) {
                Ok(l) if l == "@@done" => break,
                Ok(l) => {
                    if let Some(b) = l.strip_prefix('|') {
                        if let Some(r) = cur.and_then(|k| out.get_mut(k)) {
                            if !r.body.is_empty() {
                                r.body.push('\n');
                            }
                            r.body.push_str(b);
                        }
                    } else if let Some(x) = l.strip_prefix("R|") {
                        let (i, code) = x.split_once('|').unwrap_or((x, "0"));
                        cur = i.parse::<usize>().ok().and_then(|i| i.checked_sub(id));
                        if let Some(r) = cur.and_then(|k| out.get_mut(k)) {
                            r.code = code.trim().parse().unwrap_or(0);
                        }
                    }
                }
                Err(_) => {
                    *timed_out = true;
                    break;
                }
            }
        }
        id += reqs.len();
        out
    });
    let _ = stdin.write_all(b"end\n");
    Ok((rm, found))
}

// ------------------------------------------------------------------ cache + watcher

fn cache_file(alias: &str) -> std::path::PathBuf {
    let safe: String = alias
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.@".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    crate::config::world_cache()
        .join("services")
        .join(format!("{safe}.json"))
}

/// Remember a successful scan (so the next view shows it at once).
pub fn save_cache(s: &Scan) {
    if s.reachable() {
        let _ = crate::config::write_atomic(
            &cache_file(&s.alias),
            s.to_json("").to_string().as_bytes(),
        );
    }
}

/// The last scan of `alias` we saw, marked stale.
pub fn load_cache(alias: &str) -> Option<Scan> {
    let t = std::fs::read_to_string(cache_file(alias)).ok()?;
    let mut s = Scan::from_json(&serde_json::from_str(&t).ok()?)?;
    s.stale = true;
    Some(s)
}

/// Rescans one machine every `services_interval` seconds in the background,
/// starting from its cached result. Dropping it stops the scans (and kills a
/// probe in flight).
pub struct Watcher {
    pub alias: String,
    scan: Arc<Mutex<Scan>>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Watcher {
    pub fn start(alias: &str, every: Duration, on: Vec<&'static Service>) -> Watcher {
        let first = load_cache(alias).unwrap_or_else(|| Scan::new(alias));
        let scan = Arc::new(Mutex::new(first));
        let stop = Arc::new(AtomicBool::new(false));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let (a, sc, sp, ch) = (alias.to_string(), scan.clone(), stop.clone(), child.clone());
        thread::spawn(move || loop {
            if sp.load(Ordering::Relaxed) {
                return;
            }
            sc.lock().unwrap().busy = true;
            let s = run(&a, sshcfg::shell(&a, "sh -s"), &on, TIMEOUT, &ch);
            if sp.load(Ordering::Relaxed) {
                return;
            }
            {
                let mut g = sc.lock().unwrap();
                if s.reachable() {
                    save_cache(&s);
                    *g = s;
                } else {
                    g.err = s.err;
                    g.stale = g.at > 0;
                    g.took_ms = s.took_ms;
                }
                g.busy = false;
            }
            let until = Instant::now() + every;
            while Instant::now() < until {
                if sp.load(Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_millis(200));
            }
        });
        Watcher {
            alias: alias.to_string(),
            scan,
            stop,
            child,
        }
    }
    pub fn snapshot(&self) -> Scan {
        self.scan.lock().unwrap().clone()
    }
    pub fn found(&self) -> Vec<Found> {
        self.scan.lock().unwrap().found.clone()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// One watcher per host when `services=on`; none otherwise.
pub fn watch(cfg: &Config, hosts: &[String]) -> Vec<Watcher> {
    if !enabled(cfg) {
        return vec![];
    }
    let on = enabled_services(cfg);
    let every = interval(cfg);
    hosts
        .iter()
        .map(|h| Watcher::start(h, every, on.clone()))
        .collect()
}

// ------------------------------------------------------------------ drawing

/// A service's glyph in its brand colour.
pub fn badge(s: &Service) -> Span<'static> {
    if c::mono() {
        Span::styled(s.glyph, ui::bold())
    } else {
        Span::styled(
            s.glyph,
            Style::new().fg(s.color()).add_modifier(Modifier::BOLD),
        )
    }
}

/// `▣ ollama 3 · ▢ lmstudio 1` — for machine header rows (loaded models where
/// the server says, else models). Down servers are dimmed.
pub fn summary(found: &[Found]) -> Vec<Span<'static>> {
    let mut groups: Vec<(&'static Service, usize, bool)> = Vec::new();
    for f in found {
        match groups.iter_mut().find(|g| g.0.id == f.service.id) {
            Some(g) => {
                g.1 += f.shown_count();
                g.2 |= f.health.is_up();
            }
            None => groups.push((f.service, f.shown_count(), f.health.is_up())),
        }
    }
    let mut v = Vec::new();
    for (i, (s, n, up)) in groups.into_iter().enumerate() {
        if i > 0 {
            v.push(Span::styled(" · ", ui::faint()));
        }
        v.push(if up {
            badge(s)
        } else {
            Span::styled(s.glyph, ui::faint())
        });
        v.push(Span::styled(
            format!(" {}", s.id),
            if up { ui::muted() } else { ui::faint() },
        ));
        v.push(Span::styled(
            format!(" {n}"),
            if up { ui::bold() } else { ui::faint() },
        ));
    }
    v
}

fn spans_width(v: &[Span]) -> usize {
    v.iter().map(|s| ui::width(&s.content)).sum()
}

/// A server's header row: glyph, name, version … health, port, models, VRAM.
fn head_line(f: &Found, w: usize, t0: Instant) -> Line<'static> {
    let s = f.service;
    let (dot, col) = f.health.dot(t0);
    // right-hand pieces, by how long they stay when room runs out (lower = longer)
    let mut right: Vec<(u8, Vec<Span<'static>>)> = vec![(
        0,
        vec![
            Span::styled(format!("{dot} "), ui::fg(col)),
            Span::styled(
                f.health.label(),
                ui::fg(if f.health == Health::Up {
                    c::MUTED()
                } else {
                    col
                }),
            ),
        ],
    )];
    if let Some(p) = f.port {
        let mut v = vec![Span::styled(format!("  :{p}"), ui::muted())];
        if f.ports.len() > 1 {
            v.push(Span::styled(format!("+{}", f.ports.len() - 1), ui::faint()));
        }
        right.push((1, v));
    }
    let mt = f.models_text();
    if !mt.is_empty() {
        right.push((3, vec![Span::styled(format!("  {mt}"), ui::faint())]));
    }
    let lt = f.load_text();
    if !lt.is_empty() {
        right.push((4, vec![Span::styled(format!("  {lt}"), ui::fg(c::TEAL()))]));
    }
    if let Some(v) = f.vram_total() {
        right.push((
            2,
            vec![Span::styled(
                format!("  {}", ui::bytes(v)),
                ui::fg(c::MAUVE()),
            )],
        ));
    }
    let mut left: Vec<Span<'static>> = vec![
        badge(s),
        Span::raw(" "),
        Span::styled(s.name.to_string(), ui::bold()),
    ];
    let version = f
        .version
        .as_ref()
        .map(|v| Span::styled(format!(" {}", ui::trunc(v, 14)), ui::faint()));
    let rw = |r: &[(u8, Vec<Span<'static>>)]| r.iter().map(|x| spans_width(&x.1)).sum::<usize>();
    let mut lw = spans_width(&left) + version.as_ref().map_or(0, |v| ui::width(&v.content));
    let mut keep_version = true;
    // drop the load, the models count, the version, VRAM, then the port until it fits
    for step in [4u8, 3, 99, 2, 1] {
        if lw + rw(&right) < w {
            break;
        }
        if step == 99 {
            keep_version = false;
            lw = spans_width(&left);
        } else {
            right.retain(|x| x.0 != step);
        }
    }
    if keep_version {
        left.extend(version);
    }
    let rs: Vec<Span<'static>> = right.into_iter().flat_map(|x| x.1).collect();
    let gap = w
        .saturating_sub(spans_width(&left) + spans_width(&rs))
        .max(1);
    left.push(Span::raw(" ".repeat(gap)));
    left.extend(rs);
    Line::from(left)
}

/// A model row: loaded dot, name, size/quant/params, VRAM (+ GPU share).
fn model_line(m: &Model, w: usize, nw: usize) -> Line<'static> {
    let (dot, col) = match m.loaded {
        Some(true) => ("●", c::GREEN()),
        Some(false) => ("○", c::FAINT()),
        None => ("·", c::MUTED()),
    };
    let mut right = String::new();
    if let Some(v) = m.vram {
        right = ui::bytes(v);
        if let Some(g) = m.gpu_share().filter(|g| *g < 0.995) {
            right.push_str(&format!(" {:.0}% GPU", g * 100.0));
        }
    }
    let facts = m.facts();
    let rw = if right.is_empty() {
        0
    } else {
        ui::width(&right) + 1
    };
    // the name column, unless that leaves no room for anything else
    let nw = nw.min(w.saturating_sub(4 + rw)).max(8);
    let name = ui::pad(&m.name, nw);
    let mut v = vec![
        Span::raw("  "),
        Span::styled(format!("{dot} "), ui::fg(col)),
        Span::styled(
            name.clone(),
            if m.loaded == Some(true) {
                Style::new()
            } else {
                ui::muted()
            },
        ),
    ];
    let mut used = 4 + ui::width(&name);
    let room = w.saturating_sub(used + rw + 1);
    if !facts.is_empty() && room >= 4 {
        let f = ui::trunc(&facts, room);
        used += 1 + ui::width(&f);
        v.push(Span::styled(format!(" {f}"), ui::faint()));
    }
    if !right.is_empty() && used + 1 + ui::width(&right) <= w {
        v.push(Span::raw(" ".repeat(w - used - ui::width(&right))));
        v.push(Span::styled(right, ui::fg(c::MAUVE())));
    }
    Line::from(v)
}

/// The panel's rows for `found` in a `w`×`h` area: every server's header
/// first, then models (loaded first) as room allows, `+N more` where cut.
pub fn panel_lines(found: &[Found], w: usize, h: usize, t0: Instant) -> Vec<Line<'static>> {
    if h == 0 {
        return vec![];
    }
    if found.is_empty() {
        return vec![Line::from(Span::styled(
            "no LLM servers or gateways found",
            ui::muted(),
        ))];
    }
    let shown = found.len().min(h);
    let mut budget = h - shown;
    let mut lines = Vec::new();
    for (i, f) in found.iter().take(shown).enumerate() {
        lines.push(head_line(f, w, t0));
        let models = f.models_sorted();
        let left = shown - i;
        let share = budget.div_ceil(left).min(budget);
        let take = if models.len() > share {
            share.saturating_sub(1)
        } else {
            models.len()
        };
        let nw = models
            .iter()
            .take(take)
            .map(|m| ui::width(&m.name))
            .max()
            .unwrap_or(0)
            .min(w / 2);
        for m in models.iter().take(take) {
            lines.push(model_line(m, w, nw));
        }
        let mut used = take;
        if models.len() > take && share > 0 {
            lines.push(Line::from(Span::styled(
                format!("    +{} more", models.len() - take),
                ui::faint(),
            )));
            used += 1;
        }
        budget -= used.min(budget);
    }
    if shown < found.len() {
        if let Some(l) = lines.last_mut() {
            *l = Line::from(Span::styled(
                format!("+{} more servers", found.len() - shown + 1),
                ui::faint(),
            ));
        }
    }
    lines.truncate(h);
    lines
}

/// The services panel for one machine (rounded, like the machine panel).
pub fn panel(f: &mut Frame, r: Rect, host: &str, found: &[Found], t0: Instant) {
    let block = panel_block(host, found, None);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let lines = panel_lines(found, inner.width as usize, inner.height as usize, t0);
    f.render_widget(Paragraph::new(lines), inner);
}

fn panel_block<'a>(
    host: &str,
    found: &[Found],
    note: Option<Span<'static>>,
) -> ratatui::widgets::Block<'a> {
    let title = Line::from(vec![
        Span::styled("services ", ui::bold()),
        Span::styled(format!("· {host} "), ui::muted()),
    ]);
    let mut b = ui::panel(title, false);
    let up = found.iter().filter(|f| f.health.is_up()).count();
    let loaded: usize = found.iter().map(|f| f.loaded().count()).sum();
    let mut right = Vec::new();
    if !found.is_empty() {
        right.push(Span::styled(
            format!(
                " {up}/{} up{} ",
                found.len(),
                if loaded > 0 {
                    format!(" · {loaded} loaded")
                } else {
                    String::new()
                }
            ),
            ui::faint(),
        ));
    }
    if !right.is_empty() {
        b = b.title_top(Line::from(right).right_aligned());
    }
    if let Some(n) = note {
        b = b.title_bottom(Line::from(vec![Span::raw(" "), n, Span::raw(" ")]));
    }
    b
}

/// The panel for a [`Scan`]: a spinner before the first answer, the error when
/// there's nothing to show, "cached 3m ago" while stale.
pub fn draw_scan(f: &mut Frame, r: Rect, scan: &Scan, t0: Instant) {
    let now = crate::cache::now();
    let note = if scan.busy && scan.at == 0 {
        None
    } else if let Some(e) = &scan.err {
        Some(Span::styled(
            format!("✕ {}", ui::trunc(e, 40)),
            ui::fg(c::RED()),
        ))
    } else if scan.stale && scan.at > 0 {
        Some(Span::styled(
            format!("cached {}", ui::ago(now.saturating_sub(scan.at))),
            ui::faint(),
        ))
    } else {
        None
    };
    let block = panel_block(&scan.alias, &scan.found, note);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let (w, h) = (inner.width as usize, inner.height as usize);
    let lines = if scan.at == 0 && scan.found.is_empty() {
        vec![match &scan.err {
            Some(e) => Line::from(vec![
                Span::styled("✕ ", ui::fg(c::RED())),
                Span::styled(ui::trunc(e, w.saturating_sub(2)), ui::muted()),
            ]),
            None => Line::from(vec![
                Span::styled(format!("{} ", ui::spinner(t0)), ui::fg(c::ACCENT())),
                Span::styled("looking for LLM servers…", ui::muted()),
            ]),
        }]
    } else {
        panel_lines(&scan.found, w, h, t0)
    };
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests;
