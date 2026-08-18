#![cfg(windows)]

use std::{
    env, fs,
    net::TcpListener,
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::Duration,
};

use llamamanager::{
    llama::inspect_installation,
    server_command::{ServerLaunchSettings, build_server_launch_spec},
    server_process::{ProcessExitKind, ServerProcessSupervisor},
    server_readiness::{
        ReadinessPolicy, ServerEndpoint, require_port_available, wait_for_server_ready,
    },
};
use serde_json::json;

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required environment variable {name} is missing"))
}

fn free_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral loopback port");
    listener.local_addr().expect("read local address").port()
}

#[test]
#[ignore = "requires pinned real Windows llama.cpp binaries and published GGUF model"]
fn validates_real_llama_server_readiness_and_inference() {
    let llama_root = PathBuf::from(required_env("LLAMAMANAGER_REAL_LLAMA_ROOT"));
    let model_path = PathBuf::from(required_env("LLAMAMANAGER_REAL_BENCH_MODEL"));
    let evidence_dir = PathBuf::from(required_env("LLAMAMANAGER_REAL_EVIDENCE_DIR"));
    fs::create_dir_all(&evidence_dir).unwrap();

    let installation = inspect_installation(&llama_root).expect("inspect pinned llama.cpp installation");
    let port = free_loopback_port();
    let endpoint = ServerEndpoint::loopback(port);
    require_port_available(&endpoint).expect("selected ephemeral port must still be available");

    let settings = ServerLaunchSettings {
        model: model_path.clone(),
        host: Some(endpoint.host.clone()),
        port: Some(port),
        ..ServerLaunchSettings::default()
    };
    let spec = build_server_launch_spec(&installation, &settings)
        .expect("pinned llama-server must support model/host/port launch options");
    let diagnostic_command = spec.diagnostic_command();

    let mut supervisor = ServerProcessSupervisor::new();
    let identity = supervisor
        .start_server(&spec)
        .expect("start pinned llama-server under Windows Job Object supervision")
        .clone();

    let mut policy = ReadinessPolicy::default();
    policy.timeout = Duration::from_secs(90);
    policy.request_timeout = Duration::from_secs(10);
    policy.initial_backoff = Duration::from_millis(100);
    policy.max_backoff = Duration::from_secs(1);
    let cancellation = AtomicBool::new(false);

    let readiness = {
        let process = supervisor.process_mut().expect("managed server process exists");
        wait_for_server_ready(process, &endpoint, &policy, &cancellation)
            .expect("real llama-server must become ready and answer minimal inference")
    };

    assert!((200..=299).contains(&readiness.inference.status_code));
    assert!(readiness.inference.body_excerpt.contains("\"content\""));

    let exit = supervisor
        .process_mut()
        .expect("managed server process exists for shutdown")
        .force_kill()
        .expect("supervised server must stop without leaking its process tree");
    assert!(matches!(
        exit.kind,
        ProcessExitKind::ForceKilled | ProcessExitKind::Natural
    ));

    let evidence = json!({
        "llama_release_tag": env::var("LLAMAMANAGER_LLAMA_RELEASE_TAG").ok(),
        "runtime_root": llama_root,
        "model_path": model_path,
        "diagnostic_command": diagnostic_command,
        "pid": identity.pid,
        "endpoint": readiness.endpoint,
        "readiness_attempts": readiness.attempts,
        "readiness_elapsed_ms": readiness.elapsed.as_millis(),
        "health": format!("{:?}", readiness.health),
        "inference_path": readiness.inference.path,
        "inference_status": readiness.inference.status_code,
        "inference_body_excerpt": readiness.inference.body_excerpt,
        "authenticated": readiness.authenticated,
        "shutdown_kind": format!("{:?}", exit.kind),
        "shutdown_code": exit.code,
        "github_sha": env::var("GITHUB_SHA").ok(),
        "runner_os": env::var("RUNNER_OS").ok()
    });
    fs::write(
        evidence_dir.join("server-readiness-inference.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
}
