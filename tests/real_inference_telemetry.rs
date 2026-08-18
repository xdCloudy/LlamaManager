#![cfg(windows)]

use std::{
    env, fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use llamamanager::{
    hardware_telemetry::TelemetryState,
    inference_telemetry::{InferenceRequestObservation, parse_llama_cpp_completion},
    llama::inspect_installation,
    server_command::{ServerLaunchSettings, build_server_launch_spec},
    server_process::{ProcessExitKind, ServerProcessSupervisor},
    server_readiness::{
        ReadinessPolicy, ServerEndpoint, require_port_available, wait_for_server_ready,
    },
};
use serde_json::{Value, json};

const MAX_STREAM_BYTES: usize = 512 * 1024;

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required environment variable {name} is missing"))
}

fn free_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral loopback port");
    listener.local_addr().expect("read local address").port()
}

#[derive(Debug)]
struct StreamEvidence {
    status_code: u16,
    ttft_ms: f64,
    request_latency_ms: f64,
    event_count: usize,
    final_event: String,
}

fn stream_completion(
    endpoint: &ServerEndpoint,
    request_body: &str,
    timeout: Duration,
) -> StreamEvidence {
    let started = Instant::now();
    let address = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .expect("resolve loopback llama-server endpoint")
        .next()
        .expect("resolved endpoint must contain an address");
    let mut stream = TcpStream::connect_timeout(&address, timeout)
        .expect("connect to ready llama-server for streaming telemetry probe");
    stream.set_read_timeout(Some(timeout)).unwrap();
    stream.set_write_timeout(Some(timeout)).unwrap();

    let request = format!(
        "POST /completion HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: text/event-stream\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        endpoint.authority(),
        request_body.len(),
        request_body
    );
    stream.write_all(request.as_bytes()).unwrap();
    stream.flush().unwrap();

    let mut received = Vec::new();
    let mut headers_done = false;
    let mut status_code = None;
    let mut body_pending = String::new();
    let mut first_token_elapsed = None;
    let mut final_event = None;
    let mut event_count = 0_usize;
    let mut buffer = [0_u8; 4096];

    loop {
        let read = stream
            .read(&mut buffer)
            .expect("read streaming completion response");
        if read == 0 {
            break;
        }
        received.extend_from_slice(&buffer[..read]);
        assert!(
            received.len() <= MAX_STREAM_BYTES,
            "streaming completion exceeded {MAX_STREAM_BYTES} bytes"
        );

        if !headers_done {
            let Some(header_end) = find_bytes(&received, b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&received[..header_end]);
            let status_line = headers.lines().next().expect("HTTP status line");
            status_code = status_line
                .split_whitespace()
                .nth(1)
                .and_then(|value| value.parse::<u16>().ok());
            assert!(
                status_code.is_some_and(|status| (200..=299).contains(&status)),
                "streaming completion returned non-success status: {status_line}"
            );
            body_pending.push_str(&String::from_utf8_lossy(&received[header_end + 4..]));
            received.clear();
            headers_done = true;
        } else {
            body_pending.push_str(&String::from_utf8_lossy(&received));
            received.clear();
        }

        while let Some(newline) = body_pending.find('\n') {
            let line = body_pending[..newline].trim().to_owned();
            body_pending.drain(..=newline);
            let Some(data_position) = line.find("data: ") else {
                continue;
            };
            let payload = line[data_position + 6..].trim();
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            let Ok(event) = serde_json::from_str::<Value>(payload) else {
                continue;
            };
            event_count += 1;

            let has_token = event
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| !content.is_empty())
                || event
                    .get("tokens")
                    .and_then(Value::as_array)
                    .is_some_and(|tokens| !tokens.is_empty());
            if has_token && first_token_elapsed.is_none() {
                first_token_elapsed = Some(started.elapsed());
            }

            if event.get("timings").is_some() {
                final_event = Some(payload.to_owned());
                break;
            }
        }

        if final_event.is_some() {
            break;
        }
    }

    StreamEvidence {
        status_code: status_code.expect("streaming response headers must be observed"),
        ttft_ms: first_token_elapsed
            .expect("streaming response must expose a first generated token")
            .as_secs_f64()
            * 1_000.0,
        request_latency_ms: started.elapsed().as_secs_f64() * 1_000.0,
        event_count,
        final_event: final_event.expect("streaming response must expose a final timings event"),
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[test]
#[ignore = "requires pinned real Windows llama.cpp binaries and published GGUF model"]
fn validates_real_streaming_inference_telemetry() {
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
    let spec = build_server_launch_spec(&installation, &settings)
        .expect("pinned llama-server must support model/host/port launch options");
    let diagnostic_command = spec.diagnostic_command();

    let mut supervisor = ServerProcessSupervisor::new();
    let identity = supervisor
        .start_server(&spec)
        .expect("start pinned llama-server under Windows Job Object supervision")
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
            .expect("real llama-server must become ready before telemetry request");
    }

    let request_body =
        r#"{"prompt":"Count: one two three","n_predict":4,"temperature":0,"stream":true}"#;
    let stream = stream_completion(&endpoint, request_body, Duration::from_secs(20));
    assert!((200..=299).contains(&stream.status_code));
    assert!(stream.event_count >= 2);
    assert!(stream.ttft_ms >= 0.0);
    assert!(stream.request_latency_ms >= stream.ttft_ms);

    let snapshot = parse_llama_cpp_completion(
        &stream.final_event,
        InferenceRequestObservation {
            request_id: "real-streaming-1".to_owned(),
            endpoint: endpoint.authority(),
            server_pid: Some(identity.pid),
            requested_model: Some(model_path.display().to_string()),
            request_latency_ms: stream.request_latency_ms,
            ttft_ms: Some(stream.ttft_ms),
            observed_at_unix_ms: now_unix_ms(),
        },
    )
    .expect("final streaming llama.cpp event must parse as inference telemetry");

    assert_eq!(snapshot.identity.request_id, "real-streaming-1");
    assert_eq!(snapshot.identity.server_pid, Some(identity.pid));
    assert!(
        snapshot
            .prompt_tps
            .live_value()
            .is_some_and(|value| *value > 0.0)
    );
    assert!(
        snapshot
            .decode_tps
            .live_value()
            .is_some_and(|value| *value >= 0.0)
    );
    assert!(
        snapshot
            .ttft_ms
            .live_value()
            .is_some_and(|value| *value >= 0.0)
    );
    assert!(
        snapshot
            .request_latency_ms
            .live_value()
            .is_some_and(|value| *value >= stream.ttft_ms)
    );
    assert!(
        snapshot
            .context_tokens
            .live_value()
            .is_some_and(|value| *value > 0)
    );

    assert!(matches!(
        &snapshot.mtp_generated_tokens.state,
        TelemetryState::Unavailable { .. }
    ));
    assert!(matches!(
        &snapshot.mtp_accepted_tokens.state,
        TelemetryState::Unavailable { .. }
    ));
    assert!(matches!(
        &snapshot.mtp_acceptance_rate.state,
        TelemetryState::Unavailable { .. }
    ));

    fs::write(
        evidence_dir.join("inference-telemetry.json"),
        serde_json::to_vec_pretty(&json!({
            "llama_release_tag": env::var("LLAMAMANAGER_LLAMA_RELEASE_TAG").ok(),
            "runtime_root": llama_root,
            "model_path": model_path,
            "diagnostic_command": diagnostic_command,
            "pid": identity.pid,
            "endpoint": endpoint.authority(),
            "stream_status": stream.status_code,
            "stream_event_count": stream.event_count,
            "ttft_ms": stream.ttft_ms,
            "request_latency_ms": stream.request_latency_ms,
            "final_event": serde_json::from_str::<Value>(&stream.final_event).unwrap(),
            "telemetry": snapshot,
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