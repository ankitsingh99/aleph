//! Every logo Aleph ships: what it identifies, where its SVG comes from, the
//! SVG's licence and the brand colour. `logogen` writes
//! assets/logos/SOURCES.md from this table, draws the monograms, fetches the
//! Simple Icons files (`logogen fetch`) and renders them all.
//!
//! Simple Icons first (exact versions pinned: 13.21.0, and 16.35.0 only for
//! marks 13 doesn't have); a monogram drawn here when no mark with a clear
//! licence exists. The colour is Simple Icons' own `hex` unless the note says
//! otherwise; monograms take the colour the app already uses for that agent
//! or server (`icons::brand`, `services::ALL`).

/// Where a logo's SVG comes from.
pub enum From {
    /// Simple Icons: (exact package version, slug)
    Si(&'static str, &'static str),
    /// a monogram tile drawn here: its letters
    Mono(&'static str),
    /// drawn here from simple shapes: the SVG's body (inside a 24×24 viewBox)
    Drawn(&'static str),
}

pub struct Src {
    /// os | agent | gpu | server — also the folder under assets/logos
    pub kind: &'static str,
    /// the app's id: agents::ALL / services::ALL ids, OS and GPU names
    pub id: &'static str,
    /// what it identifies
    pub what: &'static str,
    pub from: From,
    /// brand colour, `#RRGGBB`
    pub color: &'static str,
    /// the SVG's licence when Simple Icons lists one other than CC0
    pub license: &'static str,
    pub note: &'static str,
}

pub const SI13: &str = "13.21.0";
pub const SI16: &str = "16.35.0";

const fn si(
    kind: &'static str,
    id: &'static str,
    what: &'static str,
    ver: &'static str,
    slug: &'static str,
    color: &'static str,
) -> Src {
    Src {
        kind,
        id,
        what,
        from: From::Si(ver, slug),
        color,
        license: "",
        note: "",
    }
}

const fn mono(
    kind: &'static str,
    id: &'static str,
    what: &'static str,
    text: &'static str,
    color: &'static str,
) -> Src {
    Src {
        kind,
        id,
        what,
        from: From::Mono(text),
        color,
        license: "",
        note: "",
    }
}

/// Windows 11's four panes, drawn as four squares.
const WINDOWS: &str =
    r##"<path fill="#0078D4" d="M2 2h9v9H2zM13 2h9v9h-9zM2 13h9v9H2zM13 13h9v9h-9z"/>"##;

pub const ALL: &[Src] = &[
    // ------------------------------------------------------------ systems
    si("os", "apple", "macOS", SI13, "apple", "#000000"),
    si("os", "ubuntu", "Ubuntu", SI13, "ubuntu", "#E95420"),
    Src {
        license: "CC-BY-SA-3.0",
        ..si("os", "debian", "Debian", SI13, "debian", "#A81D33")
    },
    Src {
        license: "custom: https://docs.fedoraproject.org/en-US/project/brand/",
        ..si("os", "fedora", "Fedora", SI13, "fedora", "#51A2DA")
    },
    si(
        "os",
        "redhat",
        "Red Hat Enterprise Linux",
        SI13,
        "redhat",
        "#EE0000",
    ),
    si("os", "centos", "CentOS", SI13, "centos", "#262577"),
    Src {
        license: "CC-BY-SA-4.0",
        ..si(
            "os",
            "rockylinux",
            "Rocky Linux",
            SI13,
            "rockylinux",
            "#10B981",
        )
    },
    si("os", "almalinux", "AlmaLinux", SI13, "almalinux", "#000000"),
    si(
        "os",
        "archlinux",
        "Arch Linux",
        SI13,
        "archlinux",
        "#1793D1",
    ),
    si(
        "os",
        "alpinelinux",
        "Alpine Linux",
        SI13,
        "alpinelinux",
        "#0D597F",
    ),
    si("os", "opensuse", "openSUSE", SI13, "opensuse", "#73BA25"),
    si(
        "os",
        "suse",
        "SUSE Linux Enterprise",
        SI13,
        "suse",
        "#0C322C",
    ),
    si(
        "os",
        "linuxmint",
        "Linux Mint",
        SI13,
        "linuxmint",
        "#86BE43",
    ),
    si("os", "manjaro", "Manjaro", SI13, "manjaro", "#35BF5C"),
    si(
        "os",
        "endeavouros",
        "EndeavourOS",
        SI13,
        "endeavouros",
        "#7F7FFF",
    ),
    si("os", "nixos", "NixOS", SI13, "nixos", "#5277C3"),
    Src {
        license: "CC-BY-SA-2.5",
        ..si("os", "gentoo", "Gentoo", SI13, "gentoo", "#54487A")
    },
    si(
        "os",
        "kalilinux",
        "Kali Linux",
        SI13,
        "kalilinux",
        "#557C94",
    ),
    si("os", "popos", "Pop!_OS", SI13, "popos", "#48B9C7"),
    si(
        "os",
        "elementary",
        "elementary OS",
        SI13,
        "elementary",
        "#64BAFF",
    ),
    si(
        "os",
        "raspberrypi",
        "Raspberry Pi OS",
        SI13,
        "raspberrypi",
        "#A22846",
    ),
    si(
        "os",
        "voidlinux",
        "Void Linux",
        SI13,
        "voidlinux",
        "#478061",
    ),
    si("os", "freebsd", "FreeBSD", SI13, "freebsd", "#AB2B28"),
    si("os", "openbsd", "OpenBSD", SI13, "openbsd", "#F2CA30"),
    Src {
        kind: "os",
        id: "windows",
        what: "Windows (and WSL)",
        from: From::Drawn(WINDOWS),
        color: "#0078D4",
        license: "",
        note: "four squares after the Windows 11 logo; Microsoft's marks aren't in Simple Icons",
    },
    Src {
        note: "Oracle's mark",
        ..si(
            "os",
            "oraclelinux",
            "Oracle Linux",
            SI13,
            "oracle",
            "#F80000",
        )
    },
    Src {
        note: "the AWS mark in AWS orange (Simple Icons lists #232F3E, the wordmark's ink)",
        ..si(
            "os",
            "amazonlinux",
            "Amazon Linux",
            SI13,
            "amazonwebservices",
            "#FF9900",
        )
    },
    si(
        "os",
        "linux",
        "Linux (any other distribution)",
        SI13,
        "linux",
        "#FCC624",
    ),
    // ------------------------------------------------------------ GPUs
    si("gpu", "nvidia", "NVIDIA GPUs", SI13, "nvidia", "#76B900"),
    si("gpu", "amd", "AMD GPUs", SI13, "amd", "#ED1C24"),
    si("gpu", "intel", "Intel GPUs", SI13, "intel", "#0071C5"),
    si("gpu", "apple", "Apple GPUs", SI13, "apple", "#000000"),
    // ------------------------------------------------------------ agents
    Src {
        note: "the Claude mark",
        ..si("agent", "claude", "Claude Code", SI13, "claude", "#D97757")
    },
    Src {
        note: "OpenAI's mark",
        ..si("agent", "codex", "Codex", SI13, "openai", "#412991")
    },
    si("agent", "opencode", "OpenCode", SI16, "opencode", "#000000"),
    mono("agent", "omp", "OMP (oh-my-pi)", "OM", "#C878C8"),
    mono("agent", "pi", "Pi", "π", "#78C878"),
    si("agent", "cursor", "Cursor CLI", SI16, "cursor", "#000000"),
    mono("agent", "devin", "Devin CLI", "D", "#50C8A0"),
    Src {
        note: "Google Gemini's mark",
        ..si(
            "agent",
            "gemini",
            "Gemini CLI",
            SI13,
            "googlegemini",
            "#8E75B2",
        )
    },
    Src {
        note: "∀: an upside-down A, for anti-gravity",
        ..mono("agent", "agy", "Antigravity CLI", "∀", "#4285F4")
    },
    si("agent", "cline", "Cline CLI", SI16, "cline", "#18181B"),
    mono("agent", "mastracode", "Mastra Code", "M", "#FF7850"),
    Src {
        license: "MIT",
        ..si(
            "agent",
            "copilot",
            "GitHub Copilot CLI",
            SI13,
            "githubcopilot",
            "#000000",
        )
    },
    Src {
        note: "Kimi's mark (Moonshot AI)",
        ..si("agent", "kimi", "Kimi Code", SI16, "kimi", "#000000")
    },
    mono("agent", "kiro", "Kiro CLI", "KR", "#8C64FF"),
    Src {
        note: "F for Factory",
        ..mono("agent", "droid", "Factory Droid", "F", "#FF9632")
    },
    mono("agent", "amp", "Amp", "A", "#FF5C5C"),
    Src {
        note: "X for xAI; xAI's and Grok's marks aren't in Simple Icons",
        ..mono("agent", "grok", "Grok CLI", "X", "#E6E6E6")
    },
    mono("agent", "hermes", "Hermes Agent", "H", "#B4B4FF"),
    mono("agent", "kilo", "Kilo Code CLI", "K", "#F0C83C"),
    mono("agent", "qodercli", "Qoder CLI", "Q", "#00C8B4"),
    Src {
        note: "Qwen's mark (Alibaba Cloud)",
        ..si("agent", "qwen", "Qwen Code", SI16, "qwen", "#6950EF")
    },
    mono("agent", "letta", "Letta Code", "L", "#FFB400"),
    mono("agent", "maki", "Maki", "MK", "#FF6496"),
    mono("agent", "muse", "Muse Code", "MU", "#0082FA"),
    mono("agent", "crush", "Crush", "C", "#FF50B4"),
    mono("agent", "commandcode", "Command Code", "⌘", "#FFD23C"),
    mono("agent", "prime", "Prime Agent", "P", "#FFFFFF"),
    // ------------------------------------------------------------ LLM servers and gateways
    si("server", "ollama", "Ollama", SI13, "ollama", "#000000"),
    si(
        "server",
        "lmstudio",
        "LM Studio",
        SI16,
        "lmstudio",
        "#000000",
    ),
    mono("server", "llamacpp", "llama.cpp", "LC", "#FF8C32"),
    si("server", "vllm", "vLLM", SI16, "vllm", "#30A2FF"),
    Src {
        note: "Hugging Face's mark",
        ..si(
            "server",
            "tgi",
            "Text Generation Inference",
            SI13,
            "huggingface",
            "#FFD21E",
        )
    },
    mono("server", "litellm", "LiteLLM", "LL", "#5AAAFF"),
    mono("server", "localai", "LocalAI", "LA", "#00C88C"),
    mono("server", "jan", "Jan", "J", "#508CFF"),
    mono(
        "server",
        "tgwebui",
        "text-generation-webui",
        "TG",
        "#FF7846",
    ),
    mono("server", "koboldcpp", "KoboldCpp", "K", "#9678FF"),
    mono("server", "openwebui", "Open WebUI", "OW", "#D2D2DC"),
    mono("server", "sglang", "SGLang", "SG", "#E84855"),
    mono("server", "tabbyapi", "TabbyAPI", "T", "#FFAA50"),
    mono("server", "aphrodite", "Aphrodite Engine", "A", "#FF78B4"),
    Src {
        note: "Apple's mark: MLX is Apple's",
        ..si("server", "mlxlm", "mlx-lm (MLX)", SI13, "apple", "#000000")
    },
    mono("server", "gpt4all", "GPT4All", "G", "#5AC878"),
    mono("server", "xinference", "Xinference", "X", "#6E96FF"),
    Src {
        note: "NVIDIA's mark",
        ..si(
            "server",
            "triton",
            "Triton Inference Server",
            SI13,
            "nvidia",
            "#76B900",
        )
    },
    Src {
        note: "NVIDIA's mark",
        ..si(
            "server",
            "trtllm",
            "TensorRT-LLM",
            SI13,
            "nvidia",
            "#76B900",
        )
    },
    mono("server", "llamaswap", "llama-swap", "LS", "#FFA05A"),
];
