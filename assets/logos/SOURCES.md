# Logo sources

Logos identify the operating system, GPU, agent or LLM server a session runs on or
with. The marks are trademarks of their owners, used here only to name the software
or hardware they stand for (nominative use), never to suggest endorsement.

* **Simple Icons** files are fetched at the exact package version in the URL. Their SVG
  data is CC0 1.0 unless the licence column names another one (Simple Icons records
  those per icon); bitmaps and PNGs made from such a file carry its licence.
* **Monograms** are drawn here, by `tools/logogen` (its own stroke font, no font files):
  the letters cut out of a rounded square for agents and of a circle for servers, in
  the colour the app already uses for them. MIT, like the rest of this repo.
* **Colours** are Simple Icons' `hex` unless noted. Black, white and grey marks are drawn
  in the theme's text colour; a colour that wouldn't show on the background is made
  lighter or darker (same hue). The `-dark` / `-light` PNGs are those variants.

This file is written by `tools/logogen` from `tools/logogen/src/sources.rs`; add a row
there for every new logo.

| id | kind | identifies | source | licence | colour | note |
|---|---|---|---|---|---|---|
| `apple` | os | macOS | [Simple Icons 13.21.0 `apple`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/apple.svg) | CC0-1.0 | `#000000` |  |
| `ubuntu` | os | Ubuntu | [Simple Icons 13.21.0 `ubuntu`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/ubuntu.svg) | CC0-1.0 | `#E95420` |  |
| `debian` | os | Debian | [Simple Icons 13.21.0 `debian`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/debian.svg) | CC-BY-SA-3.0 | `#A81D33` |  |
| `fedora` | os | Fedora | [Simple Icons 13.21.0 `fedora`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/fedora.svg) | custom: https://docs.fedoraproject.org/en-US/project/brand/ | `#51A2DA` |  |
| `redhat` | os | Red Hat Enterprise Linux | [Simple Icons 13.21.0 `redhat`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/redhat.svg) | CC0-1.0 | `#EE0000` |  |
| `centos` | os | CentOS | [Simple Icons 13.21.0 `centos`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/centos.svg) | CC0-1.0 | `#262577` |  |
| `rockylinux` | os | Rocky Linux | [Simple Icons 13.21.0 `rockylinux`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/rockylinux.svg) | CC-BY-SA-4.0 | `#10B981` |  |
| `almalinux` | os | AlmaLinux | [Simple Icons 13.21.0 `almalinux`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/almalinux.svg) | CC0-1.0 | `#000000` |  |
| `archlinux` | os | Arch Linux | [Simple Icons 13.21.0 `archlinux`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/archlinux.svg) | CC0-1.0 | `#1793D1` |  |
| `alpinelinux` | os | Alpine Linux | [Simple Icons 13.21.0 `alpinelinux`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/alpinelinux.svg) | CC0-1.0 | `#0D597F` |  |
| `opensuse` | os | openSUSE | [Simple Icons 13.21.0 `opensuse`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/opensuse.svg) | CC0-1.0 | `#73BA25` |  |
| `suse` | os | SUSE Linux Enterprise | [Simple Icons 13.21.0 `suse`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/suse.svg) | CC0-1.0 | `#0C322C` |  |
| `linuxmint` | os | Linux Mint | [Simple Icons 13.21.0 `linuxmint`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/linuxmint.svg) | CC0-1.0 | `#86BE43` |  |
| `manjaro` | os | Manjaro | [Simple Icons 13.21.0 `manjaro`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/manjaro.svg) | CC0-1.0 | `#35BF5C` |  |
| `endeavouros` | os | EndeavourOS | [Simple Icons 13.21.0 `endeavouros`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/endeavouros.svg) | CC0-1.0 | `#7F7FFF` |  |
| `nixos` | os | NixOS | [Simple Icons 13.21.0 `nixos`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/nixos.svg) | CC0-1.0 | `#5277C3` |  |
| `gentoo` | os | Gentoo | [Simple Icons 13.21.0 `gentoo`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/gentoo.svg) | CC-BY-SA-2.5 | `#54487A` |  |
| `kalilinux` | os | Kali Linux | [Simple Icons 13.21.0 `kalilinux`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/kalilinux.svg) | CC0-1.0 | `#557C94` |  |
| `popos` | os | Pop!_OS | [Simple Icons 13.21.0 `popos`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/popos.svg) | CC0-1.0 | `#48B9C7` |  |
| `elementary` | os | elementary OS | [Simple Icons 13.21.0 `elementary`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/elementary.svg) | CC0-1.0 | `#64BAFF` |  |
| `raspberrypi` | os | Raspberry Pi OS | [Simple Icons 13.21.0 `raspberrypi`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/raspberrypi.svg) | CC0-1.0 | `#A22846` |  |
| `voidlinux` | os | Void Linux | [Simple Icons 13.21.0 `voidlinux`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/voidlinux.svg) | CC0-1.0 | `#478061` |  |
| `freebsd` | os | FreeBSD | [Simple Icons 13.21.0 `freebsd`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/freebsd.svg) | CC0-1.0 | `#AB2B28` |  |
| `openbsd` | os | OpenBSD | [Simple Icons 13.21.0 `openbsd`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/openbsd.svg) | CC0-1.0 | `#F2CA30` |  |
| `windows` | os | Windows (and WSL) | drawn here | MIT | `#0078D4` | four squares after the Windows 11 logo; Microsoft's marks aren't in Simple Icons |
| `oraclelinux` | os | Oracle Linux | [Simple Icons 13.21.0 `oracle`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/oracle.svg) | CC0-1.0 | `#F80000` | Oracle's mark |
| `amazonlinux` | os | Amazon Linux | [Simple Icons 13.21.0 `amazonwebservices`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/amazonwebservices.svg) | CC0-1.0 | `#FF9900` | the AWS mark in AWS orange (Simple Icons lists #232F3E, the wordmark's ink) |
| `linux` | os | Linux (any other distribution) | [Simple Icons 13.21.0 `linux`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/linux.svg) | CC0-1.0 | `#FCC624` |  |
| `nvidia` | gpu | NVIDIA GPUs | [Simple Icons 13.21.0 `nvidia`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/nvidia.svg) | CC0-1.0 | `#76B900` |  |
| `amd` | gpu | AMD GPUs | [Simple Icons 13.21.0 `amd`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/amd.svg) | CC0-1.0 | `#ED1C24` |  |
| `intel` | gpu | Intel GPUs | [Simple Icons 13.21.0 `intel`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/intel.svg) | CC0-1.0 | `#0071C5` |  |
| `apple` | gpu | Apple GPUs | [Simple Icons 13.21.0 `apple`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/apple.svg) | CC0-1.0 | `#000000` |  |
| `claude` | agent | Claude Code | [Simple Icons 13.21.0 `claude`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/claude.svg) | CC0-1.0 | `#D97757` | the Claude mark |
| `codex` | agent | Codex | [Simple Icons 13.21.0 `openai`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/openai.svg) | CC0-1.0 | `#412991` | OpenAI's mark |
| `opencode` | agent | OpenCode | [Simple Icons 16.35.0 `opencode`](https://cdn.jsdelivr.net/npm/simple-icons@16.35.0/icons/opencode.svg) | CC0-1.0 | `#000000` |  |
| `omp` | agent | OMP (oh-my-pi) | monogram “OM”, drawn here | MIT | `#C878C8` |  |
| `pi` | agent | Pi | monogram “π”, drawn here | MIT | `#78C878` |  |
| `cursor` | agent | Cursor CLI | [Simple Icons 16.35.0 `cursor`](https://cdn.jsdelivr.net/npm/simple-icons@16.35.0/icons/cursor.svg) | CC0-1.0 | `#000000` |  |
| `devin` | agent | Devin CLI | monogram “D”, drawn here | MIT | `#50C8A0` |  |
| `gemini` | agent | Gemini CLI | [Simple Icons 13.21.0 `googlegemini`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/googlegemini.svg) | CC0-1.0 | `#8E75B2` | Google Gemini's mark |
| `agy` | agent | Antigravity CLI | monogram “∀”, drawn here | MIT | `#4285F4` | ∀: an upside-down A, for anti-gravity |
| `cline` | agent | Cline CLI | [Simple Icons 16.35.0 `cline`](https://cdn.jsdelivr.net/npm/simple-icons@16.35.0/icons/cline.svg) | CC0-1.0 | `#18181B` |  |
| `mastracode` | agent | Mastra Code | monogram “M”, drawn here | MIT | `#FF7850` |  |
| `copilot` | agent | GitHub Copilot CLI | [Simple Icons 13.21.0 `githubcopilot`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/githubcopilot.svg) | MIT | `#000000` |  |
| `kimi` | agent | Kimi Code | [Simple Icons 16.35.0 `kimi`](https://cdn.jsdelivr.net/npm/simple-icons@16.35.0/icons/kimi.svg) | CC0-1.0 | `#000000` | Kimi's mark (Moonshot AI) |
| `kiro` | agent | Kiro CLI | monogram “KR”, drawn here | MIT | `#8C64FF` |  |
| `droid` | agent | Factory Droid | monogram “F”, drawn here | MIT | `#FF9632` | F for Factory |
| `amp` | agent | Amp | monogram “A”, drawn here | MIT | `#FF5C5C` |  |
| `grok` | agent | Grok CLI | monogram “X”, drawn here | MIT | `#E6E6E6` | X for xAI; xAI's and Grok's marks aren't in Simple Icons |
| `hermes` | agent | Hermes Agent | monogram “H”, drawn here | MIT | `#B4B4FF` |  |
| `kilo` | agent | Kilo Code CLI | monogram “K”, drawn here | MIT | `#F0C83C` |  |
| `qodercli` | agent | Qoder CLI | monogram “Q”, drawn here | MIT | `#00C8B4` |  |
| `qwen` | agent | Qwen Code | [Simple Icons 16.35.0 `qwen`](https://cdn.jsdelivr.net/npm/simple-icons@16.35.0/icons/qwen.svg) | CC0-1.0 | `#6950EF` | Qwen's mark (Alibaba Cloud) |
| `letta` | agent | Letta Code | monogram “L”, drawn here | MIT | `#FFB400` |  |
| `maki` | agent | Maki | monogram “MK”, drawn here | MIT | `#FF6496` |  |
| `muse` | agent | Muse Code | monogram “MU”, drawn here | MIT | `#0082FA` |  |
| `crush` | agent | Crush | monogram “C”, drawn here | MIT | `#FF50B4` |  |
| `commandcode` | agent | Command Code | monogram “⌘”, drawn here | MIT | `#FFD23C` |  |
| `prime` | agent | Prime Agent | monogram “P”, drawn here | MIT | `#FFFFFF` |  |
| `ollama` | server | Ollama | [Simple Icons 13.21.0 `ollama`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/ollama.svg) | CC0-1.0 | `#000000` |  |
| `lmstudio` | server | LM Studio | [Simple Icons 16.35.0 `lmstudio`](https://cdn.jsdelivr.net/npm/simple-icons@16.35.0/icons/lmstudio.svg) | CC0-1.0 | `#000000` |  |
| `llamacpp` | server | llama.cpp | monogram “LC”, drawn here | MIT | `#FF8C32` |  |
| `vllm` | server | vLLM | [Simple Icons 16.35.0 `vllm`](https://cdn.jsdelivr.net/npm/simple-icons@16.35.0/icons/vllm.svg) | CC0-1.0 | `#30A2FF` |  |
| `tgi` | server | Text Generation Inference | [Simple Icons 13.21.0 `huggingface`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/huggingface.svg) | CC0-1.0 | `#FFD21E` | Hugging Face's mark |
| `litellm` | server | LiteLLM | monogram “LL”, drawn here | MIT | `#5AAAFF` |  |
| `localai` | server | LocalAI | monogram “LA”, drawn here | MIT | `#00C88C` |  |
| `jan` | server | Jan | monogram “J”, drawn here | MIT | `#508CFF` |  |
| `tgwebui` | server | text-generation-webui | monogram “TG”, drawn here | MIT | `#FF7846` |  |
| `koboldcpp` | server | KoboldCpp | monogram “K”, drawn here | MIT | `#9678FF` |  |
| `openwebui` | server | Open WebUI | monogram “OW”, drawn here | MIT | `#D2D2DC` |  |
| `sglang` | server | SGLang | monogram “SG”, drawn here | MIT | `#E84855` |  |
| `tabbyapi` | server | TabbyAPI | monogram “T”, drawn here | MIT | `#FFAA50` |  |
| `aphrodite` | server | Aphrodite Engine | monogram “A”, drawn here | MIT | `#FF78B4` |  |
| `mlxlm` | server | mlx-lm (MLX) | [Simple Icons 13.21.0 `apple`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/apple.svg) | CC0-1.0 | `#000000` | Apple's mark: MLX is Apple's |
| `gpt4all` | server | GPT4All | monogram “G”, drawn here | MIT | `#5AC878` |  |
| `xinference` | server | Xinference | monogram “X”, drawn here | MIT | `#6E96FF` |  |
| `triton` | server | Triton Inference Server | [Simple Icons 13.21.0 `nvidia`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/nvidia.svg) | CC0-1.0 | `#76B900` | NVIDIA's mark |
| `trtllm` | server | TensorRT-LLM | [Simple Icons 13.21.0 `nvidia`](https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/nvidia.svg) | CC0-1.0 | `#76B900` | NVIDIA's mark |
| `llamaswap` | server | llama-swap | monogram “LS”, drawn here | MIT | `#FFA05A` |  |

79 logos: 28 systems, 27 agents, 4 GPU vendors, 20 servers and gateways.
