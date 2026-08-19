#![cfg(windows)]

use std::{
    env, fs,
    io::Write,
    net::{TcpListener, TcpStream, ToSocketAddrs},
    path::PathBuf,
    sync::{
        atomic::AtomicBool,
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use llamamanager::{
    llama::inspect_installation,
    passive_inference_telemetry::poll_passive_inference_telemetry,
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

fn hold_busy_completion(endpoint: ServerEndpoint) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
    let (stop_tx, stop_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let address = (endpoint.host.as_str(), endpoint.port)
            .to_socket_addrs()
            .expect("resolve real llama-server endpoint")
            .next()
            .expect("resolved endpoint must contain an address");
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))
            .expect("connect to real llama-server for busy-slot request");
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let body = r#"{"prompt":"Keep this slot occupied for passive monitoring.","n_predict":100000,"temperature":0,"ignore_eos":true,"stream":true}"#;
        let request = format!(
            "POST /completion HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: text/event-stream\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            endpoint.authority(),
            body.len(),
            body
        );
        stream.write_all(request.as_bytes()).unwrap();
        stream.flush().unwrap();

        // Deliberately do not consume the streaming body. The request occupies the single slot
        // while the test polls read-only monitoring endpoints from a separate connection.
        let _ = stop_rx.recv_timeout(Duration::from_secs(15));
        drop(stream);
    });
    (stop_tx, handle)
}

#[test]
#[ignore = "requires pinned real Windows llama.cpp binaries and published GGUF model"]
fn validates_passive_monitoring_while_single_inference_slot_is_busy() {
    let llama_root = PathBuf::from(required_env("LLAMAMANAGER_REAL_LLAMA_ROOT"));
    let model_path = PathBuf::from(required_env("LLAMAMANAGER_REAL_BENCH_MODEL"));
    let evidence_dir = PathBuf::from(required_env("LLAMAMANAGER_REAL_EVIDENCE_DIR"));
    fs::create_dir_all(&evidence_dir).unwrap();

    let installation =
        inspect_installation(&llama_root).expect("inspect pinned llama.cpp installation");
    let port = free_loopback_port();
    let endpoint = ServerEndpoint::loopback(port);
    require_port_available(&endpoint).expect("selected ephemeral port must still be available");

    let settings = ServerLaunchSettings {
        model: model_path.clone(),
        host: Some(endpoint.host.clone()),
        port: Some(port),
        ..ServerLaunchSettings::default()
    };
    let mut spec = build_server_launch_spec(&installation, &settings)
        .expect("pinned llama-server must support model/host/port launch options");
    spec.argv.push("--metrics".into());
    spec.argv.push("--parallel".into());
    spec.argv.push("1".into());
    let diagnostic_command = spec.diagnostic_command();

    let mut supervisor = ServerProcessSupervisor::new();
    let identity = supervisor
        .start_server(&spec)
        .expect("start pinned llama-server with metrics enabled")
        .clone();

    let policy = ReadinessPolicy {
        timeout: Duration::from_secs(90),
        request_timeout: Duration::from_secs(10),
        initial_backoff: Duration::from_millis(100),
        max_backoff: Duration::from_secs(1),
        ..ReadinessPolicy::default()
    };
    let cancellation = AtomicBool::new(false);
    {
        let process = supervisor
            .process_mut()
            .expect("managed server process exists");
        wait_for_server_ready(process, &endpoint, &policy, &cancellation)
            .expect("real llama-server must become ready before passive monitoring validation");
    }

    let (stop_tx, busy_thread) = hold_busy_completion(endpoint.clone());
    let deadline = Instant::now() + Duration::from_secs(10);
    let observed = loop {
        let snapshot = poll_passive_inference_telemetry(&endpoint, Duration::from_secs(2))
            .expect("passive polling must remain available while inference is active");
        let busy = snapshot.busy_slots.is_some_and(|value| value >= 1)
            || snapshot.requests_processing.is_some_and(|value| value >= 1.0);
        if busy {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "real single-slot request never became observable through passive telemetry"
        );
        thread::sleep(Duration::from_millis(50));
    };

    assert_eq!(observed.logical_endpoint, endpoint.authority());
    assert_eq!(observed.source_endpoint, endpoint.authority());
    assert_eq!(observed.total_slots, Some(1));
    assert!(observed.busy_slots.is_some_and(|value| value >= 1));
    assert!(observed.requests_processing.is_some_and(|value| value >= 1.0));
    assert!(observed.metrics_error.is_none());
    assert!(observed.slots_error.is_none());

    let _ = stop_tx.send(());
    busy_thread.join().expect("busy-slot client thread must exit");

    fs::write(
        evidence_dir.join("passive-inference-telemetry.json"),
        serde_json::to_vec_pretty(&json!({
            "llama_release_tag": env::var("LLAMAMANAGER_LLAMA_RELEASE_TAG").ok(),
            "runtime_root": llama_root,
            "model_path": model_path,
            "diagnostic_command": diagnostic_command,
            "pid": identity.pid,
            "logical_endpoint": observed.logical_endpoint,
            "source_endpoint": observed.source_endpoint,
            "model": observed.model,
            "prompt_tps": observed.prompt_tps,
            "decode_tps": observed.decode_tps,
            "prompt_tokens_total": observed.prompt_tokens_total,
            "predicted_tokens_total": observed.predicted_tokens_total,
            "requests_processing": observed.requests_processing,
            "requests_deferred": observed.requests_deferred,
            "total_slots": observed.total_slots,
            "busy_slots": observed.busy_slots,
            "current_decoded_tokens": observed.current_decoded_tokens,
            "context_capacity_tokens": observed.context_capacity_tokens,
            "mtp_explicit": observed.mtp_explicit,
            "speculative_draft_tokens_total": observed.speculative_draft_tokens_total,
            "speculative_accepted_tokens_total": observed.speculative_accepted_tokens_total,
            "mtp_acceptance_rate": observed.mtp_acceptance_rate,
            "observed_at_unix_ms": observed.observed_at_unix_ms,
            "github_sha": env::var("GITHUB_SHA").ok(),
            "runner_os": env::var("RUNNER_OS").ok()
        }))
        .unwrap(),
    )
    .unwrap();

    let exit = supervisor
        .process_mut()
        .expect("managed server process exists for shutdown")
        .force_kill()
        .expect("supervised server must stop without leaking its process tree");
    assert!(matches!(
        exit.kind,
        ProcessExitKind::ForceKilled | ProcessExitKind::Natural
    ));
}
