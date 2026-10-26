//! Tests for the services registry, discovery and renderers. The scenario
//! tests run the real probe script through `sh -s` against simulated machines
//! (`tests/fixtures/services/<name>/`: tool output and recorded HTTP answers;
//! nothing is installed or started), so the script, the stdin protocol and
//! the classification are exercised together.

use super::*;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn all() -> Vec<&'static Service> {
    ALL.iter().collect()
}

fn ids(s: &Scan) -> Vec<&'static str> {
    s.found.iter().map(|f| f.service.id).collect()
}

fn get<'a>(s: &'a Scan, id: &str) -> &'a Found {
    s.found
        .iter()
        .find(|f| f.service.id == id)
        .unwrap_or_else(|| panic!("{id} not found in {:?}", ids(s)))
}

fn names(f: &Found) -> Vec<&str> {
    f.models.iter().map(|m| m.name.as_str()).collect()
}

const MIB: f64 = 1024.0 * 1024.0;

// ------------------------------------------------------------------ registry

#[test]
fn registry_is_consistent() {
    let ids: Vec<&str> = ALL.iter().map(|s| s.id).collect();
    assert_eq!(ids, IDS);
    for s in ALL {
        assert!(!s.ports.is_empty(), "{} has no default port", s.id);
        assert!(!s.procs.is_empty(), "{} has no process rule", s.id);
        assert!(!s.ident.is_empty(), "{} has nothing to ask", s.id);
        assert!(s.homepage.starts_with("https://"), "{}", s.id);
        assert_eq!(ui::width(s.glyph), 1, "{} glyph must be single-width", s.id);
        assert_eq!(find(s.id).map(|x| x.id), Some(s.id));
        assert_eq!(find(s.name).map(|x| x.id), Some(s.id));
        assert!(s.hex().starts_with('#') && s.hex().len() == 7);
        for p in s.paths() {
            assert!(
                p.starts_with('/') || p.starts_with("POST /"),
                "{}: {p}",
                s.id
            );
        }
    }
    let mut glyphs: Vec<&str> = ALL.iter().map(|s| s.glyph).collect();
    glyphs.sort();
    glyphs.dedup();
    assert!(
        glyphs.len() >= ALL.len() - 1,
        "glyphs should tell services apart"
    );
    assert_eq!(Kind::Gateway.as_str(), "gateway");
    // LiteLLM's /health calls every upstream model: never ask it
    assert!(!find("litellm").unwrap().paths().contains(&"/health"));
}

#[test]
fn script_is_filled_in_and_quote_safe() {
    let re = coarse_regex();
    assert!(!re.contains('\'') && !re.contains('\\') && !re.contains('"'));
    for t in [
        "ollama",
        "vllm",
        "llama-server",
        "text-generation-webui",
        "tabbyapi",
        "[.]gguf",
    ] {
        assert!(re.split('|').any(|x| x == t), "{t} missing from {re}");
    }
    // vague names never make it into the filter on their own
    assert!(!re
        .split('|')
        .any(|x| x == "server" || x == "chat" || x == "main[.]py"));
    let s = script();
    assert!(!s.contains("@RE@") && s.trim_start().starts_with('{') && s.trim_end().ends_with('}'));
    assert!(s.contains("--noproxy '*'") && s.contains("--max-time 2"));
}

// ------------------------------------------------------------------ process names

fn proc_of(comm: &str, args: &str) -> Proc {
    Proc {
        pid: 1,
        ppid: 0,
        comm: comm.into(),
        args: args.into(),
        cwd: String::new(),
    }
}

fn who(comm: &str, args: &str) -> Option<(&'static str, bool)> {
    recognise(&proc_of(comm, args), &all()).map(|(s, serves)| (s.id, serves))
}

#[test]
fn processes_are_recognised_by_what_they_run() {
    assert_eq!(
        who("ollama", "/usr/local/bin/ollama serve"),
        Some(("ollama", true))
    );
    assert_eq!(
        who("ollama", "ollama run llama3.2"),
        Some(("ollama", false))
    );
    assert_eq!(
        who(
            "/Applications/Ollama.app/Contents/MacOS/Ollama",
            "/Applications/Ollama.app/Contents/MacOS/Ollama"
        ),
        Some(("ollama", false))
    );
    // console script, module, wrappers
    assert_eq!(
        who("vllm", "/venv/bin/python3 /venv/bin/vllm serve m"),
        Some(("vllm", true))
    );
    assert_eq!(
        who(
            "python3",
            "python3 -m vllm.entrypoints.openai.api_server --model m"
        ),
        Some(("vllm", true))
    );
    assert_eq!(
        who(
            "python3",
            "python3 -u -m vllm.entrypoints.openai.api_server"
        ),
        Some(("vllm", true))
    );
    assert_eq!(
        who("vllm", "env CUDA_VISIBLE_DEVICES=0 vllm serve m"),
        Some(("vllm", true))
    );
    assert_eq!(who("vllm", "nohup vllm serve m"), Some(("vllm", true)));
    // a benchmark client isn't a server
    assert_eq!(
        who("vllm", "vllm bench serve --model m --port 8000"),
        Some(("vllm", false))
    );
    assert_eq!(
        who(
            "uvicorn",
            "/venv/bin/python /venv/bin/uvicorn open_webui.main:app --port 8080"
        ),
        Some(("openwebui", true))
    );
    assert_eq!(
        who(
            "uvicorn",
            "uvicorn --host 0.0.0.0 litellm.proxy.proxy_server:app"
        ),
        Some(("litellm", true))
    );
    assert_eq!(
        who("python3", "python3 -m sglang.launch_server --model-path q"),
        Some(("sglang", true))
    );
    // Linux keeps 15 characters of comm
    assert_eq!(
        who("text-generation", "text-generation-launcher --model-id m"),
        Some(("tgi", true))
    );
    assert_eq!(
        who(
            "xinference-loca",
            "/venv/bin/python /venv/bin/xinference-local"
        ),
        Some(("xinference", true))
    );
    assert_eq!(
        who(
            "koboldcpp-linux",
            "./koboldcpp-linux-x64-cuda1210 --model x.gguf"
        ),
        Some(("koboldcpp", true))
    );
    assert_eq!(
        who("python3", "python3 koboldcpp.py --model x.gguf"),
        Some(("koboldcpp", true))
    );
    // names with spaces (macOS apps)
    assert_eq!(
        who(
            "/Applications/LM Studio.app/Contents/MacOS/LM Studio",
            "/Applications/LM Studio.app/Contents/MacOS/LM Studio"
        ),
        Some(("lmstudio", false))
    );
    assert_eq!(
        who(
            "mlx_lm.server",
            "/venv/bin/python /venv/bin/mlx_lm.server --model m"
        ),
        Some(("mlxlm", true))
    );
    assert_eq!(
        who("python3.12", "python3.12 -m mlx_lm.server --model m"),
        Some(("mlxlm", true))
    );
    // old llama.cpp `server` only with a GGUF model
    assert_eq!(
        who("server", "./server -m models/x-Q4_K_M.gguf --port 8080"),
        Some(("llamacpp", true))
    );
    assert_eq!(
        who(
            "server",
            "/venv/.../tensorboard_data_server/bin/server --logdir=runs"
        ),
        None
    );
    // server.py / main.py only inside their projects
    let mut p = proc_of("python", "python server.py --api");
    assert!(recognise(&p, &all()).is_none());
    p.cwd = "/home/u/text-generation-webui".into();
    assert_eq!(recognise(&p, &all()).map(|x| x.0.id), Some("tgwebui"));
    let mut p = proc_of("python3", "python3 main.py");
    assert!(recognise(&p, &all()).is_none());
    p.cwd = "/home/u/tabbyAPI".into();
    assert_eq!(recognise(&p, &all()).map(|x| x.0.id), Some("tabbyapi"));
    // never from a mention in someone else's command line
    for (c, a) in [
        ("vim", "vim ollama-notes.md"),
        ("grep", "grep -r vllm serve ."),
        ("python3", "python3 train.py --model llama3 --use-vllm"),
        ("ssh", "ssh gpu ollama serve"),
        ("bash", "bash -c ollama serve"),
        ("tail", "tail -f /var/log/litellm.log"),
        ("docker", "docker run ollama/ollama"),
    ] {
        assert_eq!(who(c, a), None, "{a}");
    }
}

#[test]
fn port_owners_are_classified() {
    let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(owner_class(&n(&["python3"])), Owner::Runtime);
    assert_eq!(
        owner_class(&n(&["next-server (v15.1.0)", "node"])),
        Owner::Runtime
    );
    assert_eq!(owner_class(&n(&["docker-proxy"])), Owner::Runtime);
    assert_eq!(owner_class(&n(&["com.docker.backend"])), Owner::Runtime);
    assert_eq!(owner_class(&n(&["ssh"])), Owner::Tunnel);
    assert_eq!(owner_class(&n(&["controlcenter"])), Owner::Other);
    assert_eq!(owner_class(&n(&["cef_server"])), Owner::Other);
    assert_eq!(owner_class(&n(&["java"])), Owner::Other);
}

#[test]
fn ports_and_models_from_command_lines() {
    let w = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
    assert_eq!(arg_port(&w("serve m --port 8001")), Some(8001));
    assert_eq!(arg_port(&w("--port=30001")), Some(30001));
    assert_eq!(
        arg_port(&w("--http-port 8010 --grpc-port 8011")),
        Some(8010)
    );
    assert_eq!(arg_port(&w("--listen 127.0.0.1:9292")), Some(9292));
    assert_eq!(arg_port(&w("--address :8081")), Some(8081));
    assert_eq!(arg_port(&w("--host 0.0.0.0")), None);
    let s = |id| find(id).unwrap();
    assert_eq!(
        arg_model(s("vllm"), &w("serve Qwen/Qwen2.5-7B --port 8000")).as_deref(),
        Some("Qwen/Qwen2.5-7B")
    );
    assert_eq!(
        arg_model(s("llamacpp"), &w("-m /models/x-Q4_K_M.gguf -c 4096")).as_deref(),
        Some("x-Q4_K_M")
    );
    assert_eq!(
        arg_model(s("sglang"), &w("--model-path=meta/llama")).as_deref(),
        Some("meta/llama")
    );
    assert_eq!(
        arg_model(s("tgi"), &w("--model-id bigscience/bloom")).as_deref(),
        Some("bigscience/bloom")
    );
    assert_eq!(
        arg_model(s("trtllm"), &w("/engines/llama-8b --port 8002")).as_deref(),
        Some("llama-8b")
    );
    assert_eq!(arg_model(s("llamacpp"), &w("--port 8080")), None);
}

#[test]
fn quantisation_from_names() {
    assert_eq!(
        quant_of("Llama-3.1-8B-Instruct-Q4_K_M.gguf").as_deref(),
        Some("Q4_K_M")
    );
    assert_eq!(
        quant_of("qwen2.5-7b-instruct-q8_0").as_deref(),
        Some("Q8_0")
    );
    assert_eq!(quant_of("x-IQ3_XXS.gguf").as_deref(), Some("IQ3_XXS"));
    assert_eq!(
        quant_of("mlx-community/Qwen2.5-7B-Instruct-4bit").as_deref(),
        Some("4bit")
    );
    assert_eq!(
        quant_of("Qwen/Qwen2.5-32B-Instruct-AWQ").as_deref(),
        Some("AWQ")
    );
    assert_eq!(quant_of("repo/x-GGUF:Q8_0").as_deref(), Some("Q8_0"));
    assert_eq!(quant_of("llama-3.1-8b-fp8").as_deref(), Some("FP8"));
    assert_eq!(quant_of("Qwen2.5-Coder"), None);
    assert_eq!(quant_of("meta-llama/Llama-3.1-8B-Instruct"), None);
}

// ------------------------------------------------------------------ the remote's report

#[test]
fn listener_tools_are_parsed() {
    let ss = "K|listeners=ss\nL|LISTEN 0 4096 127.0.0.1:11434 0.0.0.0:*\nL|LISTEN 0 128 [::1]:8080 [::]:* users:((\"python\",pid=1567,fd=3),(\"python\",pid=1061,fd=3))\nL|LISTEN 0 4096 127.0.0.53%lo:53 0.0.0.0:*\nL|State Recv-Q Send-Q Local Address:Port Peer Address:Port Process";
    let r = Remote::parse(ss);
    assert_eq!(r.listeners.len(), 3);
    assert_eq!((r.listeners[0].port, r.listeners[0].pid), (11434, None));
    assert_eq!(
        (
            r.listeners[1].port,
            r.listeners[1].bind.as_str(),
            r.listeners[1].pid,
            r.listeners[1].name.as_deref()
        ),
        (8080, "::1", Some(1061), Some("python"))
    );
    assert_eq!(r.listeners[2].bind, "127.0.0.53");
    let lsof = "K|listeners=lsof\nL|p801\nL|cLM Studio\nL|f45\nL|n127.0.0.1:1234\nL|f46\nL|n[::1]:1235\nL|p9\nL|cx\nL|n*:7000";
    let r = Remote::parse(lsof);
    assert_eq!(r.listeners.len(), 3);
    assert_eq!(r.listeners[1].name.as_deref(), Some("LM Studio"));
    assert_eq!((r.listeners[2].port, r.listeners[2].pid), (7000, Some(9)));
    let ns = "K|listeners=netstat\nL|tcp 0 0 0.0.0.0:5001 0.0.0.0:* LISTEN 2202/koboldcpp-linu\nL|tcp6 0 0 :::8888 :::* LISTEN -\nL|tcp4 0 0 127.0.0.1.11434 *.* LISTEN";
    let r = Remote::parse(ns);
    assert_eq!(r.listeners.len(), 3);
    assert_eq!(r.listeners[0].pid, Some(2202));
    assert_eq!((r.listeners[1].port, r.listeners[1].pid), (8888, None));
    assert_eq!(
        (r.listeners[2].port, r.listeners[2].bind.as_str()),
        (11434, "127.0.0.1")
    );
    let pr = "K|listeners=proc\nL|0: 0100007F:2CAA 00000000:0000 0A 0\nL|2: 00000000000000000000000001000000:1F90 00000000000000000000000000000000:0000 0A 0";
    let r = Remote::parse(pr);
    assert_eq!(r.listeners[0].port, 11434);
    assert_eq!(r.listeners[0].bind, "127.0.0.1");
    assert_eq!(
        (r.listeners[1].port, r.listeners[1].bind.as_str()),
        (8080, "::1")
    );
    assert_eq!(connect_addr("0.0.0.0"), "127.0.0.1");
    assert_eq!(connect_addr("*"), "127.0.0.1");
    assert_eq!(connect_addr("::"), "127.0.0.1");
    assert_eq!(connect_addr("::1"), "[::1]");
    assert_eq!(connect_addr("10.0.0.49"), "10.0.0.49");
}

#[test]
fn report_lines_are_parsed() {
    let t = "K|os=Linux\nK|http=curl\nK|lms=1\nC| 4242  2000 vllm\nA| 4242 /venv/bin/vllm serve m\nW|4242 /srv\nG|4300 38912\nT|4300 4242\nC|  801     1 /Applications/LM Studio.app/Contents/MacOS/LM Studio\nnoise";
    let r = Remote::parse(t);
    assert_eq!(
        (r.os.as_str(), r.http_tool.as_str(), r.lms),
        ("Linux", "curl", true)
    );
    let p = &r.procs[&4242];
    assert_eq!(
        (p.ppid, p.comm.as_str(), p.args.as_str(), p.cwd.as_str()),
        (2000, "vllm", "/venv/bin/vllm serve m", "/srv")
    );
    assert_eq!(
        r.procs[&801].comm,
        "/Applications/LM Studio.app/Contents/MacOS/LM Studio"
    );
    assert_eq!(r.gpu, vec![(4300, 38912.0 * MIB)]);
    assert!(r.descends(4300, 4242) && !r.descends(4242, 4300));
}

#[test]
fn metrics_are_summed() {
    let t = "# HELP x\nvllm:num_requests_running{model_name=\"a\"} 2.0\nvllm:num_requests_running{model_name=\"b\"} 1\nvllm:num_requests_running_total 9\nvllm:gpu_cache_usage_perc 0.5";
    assert_eq!(metric(t, "vllm:num_requests_running"), Some(3.0));
    assert_eq!(metric(t, "vllm:gpu_cache_usage_perc"), Some(0.5));
    assert_eq!(metric(t, "vllm:num_requests_waiting"), None);
}

// ------------------------------------------------------------------ readers

#[test]
fn health_follows_the_answers() {
    let s = find("llamacpp").unwrap();
    assert_eq!(
        health_of(s, &Http::with(1, &[("/health", 200, "")])),
        Health::Up
    );
    assert_eq!(
        health_of(
            s,
            &Http::with(1, &[("/health", 503, ""), ("/props", 503, "")])
        ),
        Health::Loading
    );
    assert_eq!(
        health_of(s, &Http::with(1, &[("/health", 0, ""), ("/props", 0, "")])),
        Health::Down
    );
    // a 404 still means it's alive
    assert_eq!(
        health_of(s, &Http::with(1, &[("/health", 404, "")])),
        Health::Up
    );
    assert_eq!(
        health_of(s, &Http::with(1, &[("/props", 500, "")])),
        Health::Error("HTTP 500".into())
    );
    assert_eq!(health_of(s, &Http::default()), Health::Unknown);
}

#[test]
fn lms_ps_marks_loaded_models() {
    let lms = r#"[{"type":"llm","modelKey":"qwen2.5-7b-instruct","format":"gguf","displayName":"Qwen2.5 7B Instruct","path":"lmstudio-community/Qwen2.5-7B-Instruct-GGUF/Qwen2.5-7B-Instruct-Q4_K_M.gguf","sizeBytes":4683073952,"paramsString":"7B","identifier":"qwen2.5-7b-instruct","contextLength":4096}]"#;
    let v1 = r#"{"data":[{"id":"qwen2.5-7b-instruct","object":"model","owned_by":"organization_owner"},{"id":"phi-4","object":"model","owned_by":"organization_owner"}],"object":"list"}"#;
    let r = read_lmstudio(&Http::with(
        1234,
        &[
            ("/api/v0/models", 404, ""),
            ("/v1/models", 200, v1),
            ("!lms", 200, lms),
        ],
    ));
    assert!(r.ident);
    let q = r
        .models
        .iter()
        .find(|m| m.name == "qwen2.5-7b-instruct")
        .unwrap();
    assert_eq!(
        (q.loaded, q.size, q.ctx),
        (Some(true), Some(4683073952.0), Some(4096))
    );
    assert_eq!(
        r.models.iter().find(|m| m.name == "phi-4").unwrap().loaded,
        None
    );
}

#[test]
fn llamacpp_router_mode_and_props() {
    let models = r#"{"object":"list","data":[{"id":"/m/a-Q4_K_M.gguf","object":"model","owned_by":"llamacpp","status":{"value":"loaded"},"meta":{"size":4912898304,"n_params":8030261248}},{"id":"b","object":"model","owned_by":"llamacpp","status":{"value":"unloaded"}}]}"#;
    let props = r#"{"default_generation_settings":{"n_ctx":8192},"total_slots":1,"model_path":"/m/a-Q4_K_M.gguf","build_info":"b6123-1d72c841"}"#;
    let r = read_llamacpp(&Http::with(
        8080,
        &[("/v1/models", 200, models), ("/props", 200, props)],
    ));
    assert!(r.ident);
    assert_eq!(r.version.as_deref(), Some("b6123-1d72c841"));
    let a = &r.models[0];
    assert_eq!(
        (
            a.name.as_str(),
            a.loaded,
            a.ctx,
            a.params.as_deref(),
            a.quant.as_deref()
        ),
        (
            "a-Q4_K_M",
            Some(true),
            Some(8192),
            Some("8.0B"),
            Some("Q4_K_M")
        )
    );
    assert_eq!(r.models[1].loaded, Some(false));
}

#[test]
fn generic_openai_lists_identify_nobody() {
    // a plain OpenAI list says nothing about who serves it
    let plain = r#"{"object":"list","data":[{"id":"m","object":"model","owned_by":"user"}]}"#;
    let h = Http::with(9000, &[("/v1/models", 200, plain), ("/api/tags", 404, "")]);
    for s in ALL {
        assert!(!(s.read)(&h).ident, "{} claimed a generic /v1/models", s.id);
    }
    // nor does an HTML page or garbage
    let h = Http::with(
        8080,
        &[
            ("/v1/models", 200, "<html>hi</html>"),
            ("/props", 200, "{nope"),
            ("/info", 200, "[]"),
        ],
    );
    for s in ALL {
        assert!(!(s.read)(&h).ident, "{} claimed an HTML page", s.id);
    }
}

// ------------------------------------------------------------------ simulated machines

#[test]
fn gpu_box_finds_its_servers_and_nothing_else() {
    let s = simulate("gpu-box", &all());
    assert_eq!(s.err, None);
    assert_eq!((s.listeners.as_str(), s.http.as_str()), ("ss", "curl"));
    assert_eq!(
        ids(&s),
        ["ollama", "vllm", "tgi", "sglang", "triton", "litellm"]
    );
    // the traps: cef_server on SGLang's port, http.server, an unknown :5000,
    // tensorboard, its `server`, an ssh forward on LM Studio's port, Ollama's runner
    for f in &s.found {
        for p in &f.ports {
            assert!(
                ![30000, 8001, 5000, 6006, 36467, 1234, 39999, 22].contains(p),
                "{} on :{p}",
                f.service.id
            );
        }
    }
    // Ollama as another user: found by its answers, tied to `ollama serve`
    let o = get(&s, "ollama");
    assert_eq!(
        (o.port, o.pid, o.version.as_deref()),
        (Some(11434), Some(1200), Some("0.5.7"))
    );
    assert!(o.by_http && o.by_process && o.health == Health::Up);
    assert_eq!(
        names(o),
        ["llama3.1:8b", "nomic-embed-text:latest", "qwen2.5-coder:7b"]
    );
    let l = &o.models[0];
    assert_eq!(
        (l.loaded, l.quant.as_deref(), l.params.as_deref()),
        (Some(true), Some("Q4_K_M"), Some("8.0B"))
    );
    assert_eq!(
        (l.vram, l.ctx, l.gpu_share()),
        (Some(6654289920.0), Some(8192), Some(1.0))
    );
    assert_eq!(o.models[1].kind.as_deref(), Some("embeddings"));
    assert_eq!(o.loaded().count(), 1);
    assert_eq!(o.vram, Some(6346.0 * MIB)); // its runner, per nvidia-smi
                                            // vLLM: its API port plus an internal one, one entry
    let v = get(&s, "vllm");
    assert_eq!(
        (v.port, v.ports.clone(), v.pid),
        (Some(8000), vec![8000, 43117], Some(4242))
    );
    assert_eq!(v.version.as_deref(), Some("0.6.3.post1"));
    assert_eq!(names(v), ["meta-llama/Llama-3.1-8B-Instruct"]);
    assert_eq!(v.models[0].ctx, Some(32768));
    assert_eq!(
        v.load,
        Load {
            running: Some(2.0),
            waiting: Some(1.0),
            kv: Some(0.341)
        }
    );
    assert_eq!(v.vram, Some(38912.0 * MIB)); // EngineCore, a child
    assert_eq!(v.models[0].vram, Some(38912.0 * MIB));
    assert_eq!(v.bind, "0.0.0.0");
    assert_eq!(v.openai_base().as_deref(), Some("http://127.0.0.1:8000/v1"));
    // SGLang on a non-default port, with its scheduler's VRAM
    let g = get(&s, "sglang");
    assert_eq!(
        (g.port, g.version.as_deref()),
        (Some(30001), Some("0.4.1.post5"))
    );
    assert_eq!(names(g), ["Qwen/Qwen2.5-7B-Instruct"]);
    assert_eq!(
        (g.models[0].ctx, g.vram),
        (Some(32768), Some(20480.0 * MIB))
    );
    // TGI in Docker: found by its /info; its in-container processes add nothing
    let t = get(&s, "tgi");
    assert_eq!(
        (t.port, t.process.as_str(), t.version.as_deref()),
        (Some(8080), "docker-proxy", Some("2.4.0"))
    );
    assert!(t.by_http);
    assert_eq!(names(t), ["meta-llama/Llama-3.1-8B-Instruct"]);
    assert_eq!(
        (t.models[0].ctx, t.models[0].quant.as_deref()),
        (Some(4096), Some("float16"))
    );
    // Triton on its own ports (POST /v2/repository/index)
    let tr = get(&s, "triton");
    assert_eq!(
        (tr.port, tr.ports.len(), tr.version.as_deref()),
        (Some(8010), 3, Some("2.41.0"))
    );
    assert_eq!((tr.models.len(), tr.loaded().count()), (5, 4));
    // LiteLLM: up, but its routes want a key
    let li = get(&s, "litellm");
    assert_eq!(
        (li.version.as_deref(), li.auth, li.health.clone()),
        (Some("1.55.8"), true, Health::Up)
    );
    assert_eq!(li.models_text(), "needs a key");
}

#[test]
fn mac_finds_apps_and_skips_airplay() {
    let s = simulate("mac", &all());
    assert_eq!(s.err, None);
    assert_eq!(ids(&s), ["ollama", "lmstudio", "jan", "mlxlm", "gpt4all"]);
    for f in &s.found {
        assert!(
            !f.ports
                .iter()
                .any(|p| [5000, 7000, 3000, 6379, 41343].contains(p)),
            "{}",
            f.service.id
        );
    }
    let lm = get(&s, "lmstudio");
    assert_eq!(
        (lm.port, lm.pid, lm.process.as_str()),
        (Some(1234), Some(801), "LM Studio")
    );
    assert_eq!(lm.ports, vec![1234]); // not its internal port
    assert_eq!(lm.models.len(), 3);
    assert_eq!(lm.loaded().count(), 2);
    let q = &lm.models_sorted()[0];
    assert_eq!(
        (q.format.as_deref(), q.quant.as_deref(), q.ctx),
        (Some("gguf"), Some("Q4_K_M"), Some(8192))
    );
    let o = get(&s, "ollama");
    assert_eq!(
        (o.pid, o.version.as_deref(), o.loaded().count()),
        (Some(802), Some("0.6.2"), 1)
    );
    let jan = get(&s, "jan");
    assert_eq!(names(jan), ["llama3.2-3b-instruct"]); // not the remote GPT-4o
    assert_eq!(
        (jan.models[0].size, jan.models[0].ctx),
        (Some(3420000000.0), Some(8192))
    );
    let mlx = get(&s, "mlxlm");
    assert!(mlx.by_process && !mlx.by_http);
    assert_eq!(mlx.models.len(), 2);
    assert_eq!(mlx.models[0].quant.as_deref(), Some("4bit"));
    assert_eq!(names(get(&s, "gpt4all")), ["Llama 3 8B Instruct"]);
}

#[test]
fn workstation_finds_every_kind() {
    let s = simulate("ws", &all());
    assert_eq!(s.err, None);
    assert_eq!(
        ids(&s),
        [
            "llamacpp",
            "localai",
            "koboldcpp",
            "tabbyapi",
            "aphrodite",
            "xinference",
            "trtllm",
            "llamaswap",
            "tgwebui",
            "openwebui"
        ]
    );
    // no ollama (only a client runs), no vllm (a benchmark), no jupyter/java
    for f in &s.found {
        assert!(
            !f.ports
                .iter()
                .any(|p| [8888, 8090, 22, 3306, 8000].contains(p)),
            "{}",
            f.service.id
        );
    }
    // llama.cpp still loading: its model comes from its command line
    let l = get(&s, "llamacpp");
    assert_eq!(l.health, Health::Loading);
    assert_eq!(names(l), ["Qwen2.5-Coder-32B-Instruct-Q4_K_M"]);
    assert_eq!(l.models[0].quant.as_deref(), Some("Q4_K_M"));
    let k = get(&s, "koboldcpp");
    assert_eq!(
        (k.version.as_deref(), k.models[0].ctx),
        (Some("1.80.3"), Some(8192))
    );
    assert_eq!(names(k), ["Mistral-Nemo-12B-Instruct-Q5_K_M"]);
    // text-generation-webui: API and UI ports, one entry, from its working folder
    let w = get(&s, "tgwebui");
    assert_eq!((w.port, w.ports.clone()), (Some(5000), vec![5000, 7860]));
    assert_eq!(w.models.len(), 3);
    assert_eq!(
        w.loaded().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["Llama-3-8B-Instruct-exl2"]
    );
    // Open WebUI in a container (no owner) on its default port
    let ow = get(&s, "openwebui");
    assert_eq!(
        (ow.port, ow.pid, ow.version.as_deref(), ow.auth),
        (Some(3000), None, Some("0.5.20"), true)
    );
    let la = get(&s, "localai");
    assert_eq!(
        (la.port, la.version.as_deref()),
        (Some(8081), Some("v2.24.2"))
    );
    assert_eq!(
        la.loaded().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["phi-3.5-mini-instruct"]
    );
    let tb = get(&s, "tabbyapi");
    assert_eq!(tb.port, Some(5005));
    assert_eq!(tb.models.len(), 2);
    assert_eq!(tb.models[0].ctx, Some(16384));
    assert_eq!(tb.loaded().count(), 1);
    let ap = get(&s, "aphrodite");
    assert_eq!(
        (ap.version.as_deref(), ap.load.kv),
        (Some("0.6.2"), Some(0.02))
    );
    let x = get(&s, "xinference");
    assert_eq!(x.models.len(), 2);
    assert_eq!(
        (x.models[0].params.as_deref(), x.models[0].quant.as_deref()),
        (Some("7B"), None)
    );
    assert_eq!(x.models[1].kind.as_deref(), Some("embedding"));
    assert_eq!(names(get(&s, "trtllm")), ["llama-3.1-8b-fp8"]);
    let sw = get(&s, "llamaswap");
    assert_eq!(sw.service.kind, Kind::Gateway);
    assert_eq!(
        sw.loaded().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["qwen-coder-32b"]
    );
    assert_eq!(sw.models_text(), "2 routes · 1 loaded");
}

#[test]
fn without_tools_processes_still_say_what_runs() {
    let s = simulate("bare", &all());
    assert_eq!(s.err, None);
    assert_eq!((s.listeners.as_str(), s.http.as_str()), ("none", "none"));
    assert_eq!(ids(&s), ["ollama", "llamacpp", "vllm"]);
    for f in &s.found {
        assert_eq!(f.health, Health::Unknown, "{}", f.service.id);
        assert!(f.by_process && !f.by_http);
    }
    let v = get(&s, "vllm");
    assert_eq!((v.port, v.pid), (Some(8000), Some(100)));
    assert_eq!(names(v), ["Qwen/Qwen2.5-32B-Instruct-AWQ"]);
    assert_eq!(v.models[0].quant.as_deref(), Some("AWQ"));
    assert_eq!(get(&s, "ollama").port, None);
    assert_eq!(get(&s, "llamacpp").models[0].quant.as_deref(), Some("Q8_0"));
}

#[test]
fn busybox_with_proc_net_tcp() {
    let s = simulate("busybox", &all());
    assert_eq!(s.err, None);
    assert_eq!(ids(&s), ["ollama"]);
    let o = get(&s, "ollama");
    assert_eq!(
        (o.port, o.pid, o.bind.as_str()),
        (Some(11434), Some(412), "127.0.0.1")
    );
    assert_eq!((o.models.len(), o.loaded().count()), (1, 0));
    assert_eq!(o.models_text(), "1 model · 0 loaded");
}

#[test]
fn opted_out_services_are_never_asked() {
    let cfg = Config::in_memory(&["service.ollama=off", "service.vllm=false"]);
    let on = enabled_services(&cfg);
    assert!(!on.iter().any(|s| s.id == "ollama" || s.id == "vllm"));
    let s = simulate("gpu-box", &on);
    assert_eq!(ids(&s), ["tgi", "sglang", "triton", "litellm"]);
    assert!(check_setting("ollama", "off").is_ok());
    assert!(check_setting("olama", "off")
        .unwrap_err()
        .contains("did you mean 'ollama'"));
    assert!(check_setting("ollama", "maybe").is_err());
    assert!(crate::config::check("service.vllm", "off").is_ok());
    assert!(crate::config::check("service.nope", "off").is_err());
    assert!(
        crate::config::check("services", "on").is_ok()
            && crate::config::check("services", "maybe").is_err()
    );
    assert!(!enabled(&Config::in_memory(&[])));
    assert!(enabled(&Config::in_memory(&["services=on"])));
    assert_eq!(interval(&Config::in_memory(&[])), Duration::from_secs(30));
    assert_eq!(
        interval(&Config::in_memory(&["services_interval=1"])),
        Duration::from_secs(5)
    );
}

#[test]
fn json_round_trips_through_the_cache_shape() {
    let s = simulate("gpu-box", &all());
    let v = s.to_json("#89b4fa");
    assert_eq!(v["reachable"], true);
    assert_eq!(v["services"][0]["id"], "ollama");
    assert_eq!(v["services"][0]["url"], "http://127.0.0.1:11434");
    assert_eq!(v["services"][0]["loaded_count"], 1);
    assert_eq!(v["services"][1]["found_by"], json!(["process", "http"]));
    let back = Scan::from_json(&v).unwrap();
    assert_eq!(ids(&back), ids(&s));
    let (a, b) = (get(&s, "vllm"), get(&back, "vllm"));
    assert_eq!(
        (a.port, &a.ports, a.pid, &a.version),
        (b.port, &b.ports, b.pid, &b.version)
    );
    assert_eq!(a.models, b.models);
    assert_eq!(a.load, b.load);
    assert!(get(&back, "litellm").auth);
    let e = Found {
        health: Health::Error("HTTP 500".into()),
        ..Found::new(find("tgi").unwrap())
    };
    assert_eq!(
        Found::from_json(&e.to_json()).unwrap().health,
        Health::Error("HTTP 500".into())
    );
}

#[test]
fn a_real_local_probe_runs_cleanly() {
    // whatever this machine runs: the script must finish with a report
    let mut c = Command::new("sh");
    c.arg("-s").env_remove("ALEPH_SERVICES_FIXTURES");
    let slot = Arc::new(Mutex::new(None));
    let s = run("local", c, &all(), Duration::from_secs(20), &slot);
    assert_eq!(s.err, None);
    assert!(!s.listeners.is_empty() && !s.http.is_empty());
    assert!(s.at > 0 && !s.partial);
}

#[test]
fn a_dead_machine_is_an_error_not_a_hang() {
    let mut c = Command::new("sh");
    c.arg("-c")
        .arg("echo 'ssh: connect to host x port 22: Connection refused' >&2; exit 255");
    let slot = Arc::new(Mutex::new(None));
    let t = Instant::now();
    let s = run("x", c, &all(), Duration::from_secs(5), &slot);
    assert!(t.elapsed() < Duration::from_secs(3));
    assert_eq!(
        s.err.as_deref(),
        Some("ssh: connect to host x port 22: Connection refused")
    );
    assert!(!s.reachable());
}

// ------------------------------------------------------------------ drawing

fn render(found: &[Found], w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| panel(f, f.area(), "b31", found, Instant::now()))
        .unwrap();
    let buf = t.backend().buffer().clone();
    let mut s = String::new();
    for y in 0..h {
        for x in 0..w {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

#[test]
fn panel_shows_servers_models_and_vram() {
    let s = simulate("gpu-box", &all());
    let out = render(&s.found, 80, 24);
    for want in [
        "services · b31",
        "6/6 up",
        "▣ Ollama 0.5.7",
        ":11434",
        "● llama3.1:8b",
        "6.2G",
        "○ qwen2.5-coder:7b",
        "▥ vLLM",
        ":8000+1",
        "meta-llama/Llama-3.1-8B-Instruct",
        "38.0G",
        "LiteLLM",
        "needs a key",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
    // tight: every server keeps its header row, models give way
    let out = render(&s.found, 50, 8);
    for n in ["Ollama", "vLLM", "TGI", "SGLang", "Triton", "LiteLLM"] {
        assert!(out.contains(n), "{n} lost in\n{out}");
    }
    // tiny sizes never panic
    for (w, h) in [(1, 1), (8, 3), (20, 2), (30, 4)] {
        render(&s.found, w, h);
    }
    assert!(render(&[], 40, 4).contains("no LLM servers"));
}

#[test]
fn draw_scan_states() {
    let mut t = Terminal::new(TestBackend::new(60, 6)).unwrap();
    let text = |t: &Terminal<TestBackend>| {
        let b = t.backend().buffer();
        (0..6)
            .map(|y| {
                (0..60)
                    .map(|x| b[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut s = Scan::new("gpu2");
    s.busy = true;
    t.draw(|f| draw_scan(f, f.area(), &s, Instant::now()))
        .unwrap();
    assert!(text(&t).contains("looking for LLM servers"));
    s.busy = false;
    s.err = Some("Connection refused".into());
    t.draw(|f| draw_scan(f, f.area(), &s, Instant::now()))
        .unwrap();
    assert!(text(&t).contains("✕ Connection refused"));
    let mut s = simulate("busybox", &all());
    s.stale = true;
    s.at = crate::cache::now() - 300;
    t.draw(|f| draw_scan(f, f.area(), &s, Instant::now()))
        .unwrap();
    let out = text(&t);
    assert!(
        out.contains("cached 5m ago") && out.contains("Ollama"),
        "{out}"
    );
}

#[test]
fn summary_for_header_rows() {
    let s = simulate("mac", &all());
    let text: String = summary(&s.found)
        .iter()
        .map(|x| x.content.to_string())
        .collect();
    assert_eq!(
        text,
        "▣ ollama 1 · ▢ lmstudio 2 · ◰ jan 1 · ▩ mlxlm 2 · ◳ gpt4all 1"
    );
    assert!(summary(&[]).is_empty());
}
