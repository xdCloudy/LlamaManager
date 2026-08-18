use std::{
    io::{self, Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

use llamamanager::server_readiness::{
    HealthCapabilityEvidence, PortAvailability, ProbePhase, ReadinessPolicy, ServerEndpoint,
    ServerReadinessError, check_port_available, require_port_available,
    wait_for_ready_without_process,
};

#[test]
fn port_collision_is_detected_before_launch() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let endpoint = ServerEndpoint::loopback(port);

    assert_eq!(
        check_port_available(&endpoint).unwrap(),
        PortAvailability::InUse
    );
    assert!(matches!(
        require_port_available(&endpoint),
        Err(ServerReadinessError::PortInUse {
            port: observed,
            ..
        }) if observed == port
    ));
}

#[test]
fn non_loopback_requires_explicit_opt_in() {
    let endpoint = ServerEndpoint {
        host: "0.0.0.0".into(),
        port: 8080,
        api_key: None,
        allow_non_loopback: false,
    };

    assert!(matches!(
        check_port_available(&endpoint),
        Err(ServerReadinessError::NonLoopbackDenied { .. })
    ));
}

#[test]
fn health_and_minimal_inference_are_both_required() {
    let server = FakeHttpServer::spawn(|request| {
        if request.starts_with("GET /health ") {
            response(200, r#"{"status":"ok"}"#)
        } else if request.starts_with("POST /completion ") {
            response(200, r#"{"content":"OK"}"#)
        } else {
            response(404, "{}")
        }
    });
    let endpoint = ServerEndpoint::loopback(server.port());
    let cancellation = AtomicBool::new(false);

    let evidence = wait_for_ready_without_process(
        &endpoint,
        &quick_policy(),
        &cancellation,
    )
    .unwrap();

    assert!(matches!(
        evidence.health,
        HealthCapabilityEvidence::Healthy(_)
    ));
    assert_eq!(evidence.inference.status_code, 200);
    assert!(evidence.inference.body_excerpt.contains("\"content\""));
}

#[test]
fn unsupported_health_is_observed_instead_of_version_guessed() {
    let server = FakeHttpServer::spawn(|request| {
        if request.starts_with("POST /completion ") {
            response(200, r#"{"content":"OK"}"#)
        } else {
            response(404, "{}")
        }
    });
    let endpoint = ServerEndpoint::loopback(server.port());
    let policy = quick_policy();
    let cancellation = AtomicBool::new(false);

    let evidence = wait_for_ready_without_process(
        &endpoint,
        &policy,
        &cancellation,
    )
    .unwrap();

    match evidence.health {
        HealthCapabilityEvidence::UnsupportedObserved(items) => {
            assert_eq!(items.len(), policy.health_paths.len());
            assert!(items.iter().all(|item| item.status_code == 404));
        }
        other => panic!("expected unsupported health evidence, got {other:?}"),
    }
}

#[test]
fn bearer_auth_success_and_rejection_are_distinct() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);
    let server = FakeHttpServer::spawn(move |request| {
        captured.lock().unwrap().push(request.clone());
        if !request.contains("Authorization: Bearer correct-key\r\n") {
            return response(401, "{}");
        }
        if request.starts_with("GET /health ") {
            response(200, r#"{"status":"ok"}"#)
        } else {
            response(200, r#"{"content":"OK"}"#)
        }
    });
    let cancellation = AtomicBool::new(false);

    let unauthenticated = ServerEndpoint::loopback(server.port());
    let error = wait_for_ready_without_process(
        &unauthenticated,
        &quick_policy(),
        &cancellation,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ServerReadinessError::AuthenticationRejected {
            phase: ProbePhase::Health,
            status_code: 401
        }
    ));

    let mut authenticated = ServerEndpoint::loopback(server.port());
    authenticated.api_key = Some("correct-key".into());
    let ready = wait_for_ready_without_process(
        &authenticated,
        &quick_policy(),
        &cancellation,
    )
    .unwrap();

    assert!(ready.authenticated);
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.contains("Authorization: Bearer correct-key\r\n"))
    );
}

#[test]
fn inference_rejection_is_distinct_from_health_failure() {
    let server = FakeHttpServer::spawn(|request| {
        if request.starts_with("GET /health ") {
            response(200, r#"{"status":"ok"}"#)
        } else {
            response(400, r#"{"error":"bad inference"}"#)
        }
    });
    let cancellation = AtomicBool::new(false);

    let error = wait_for_ready_without_process(
        &ServerEndpoint::loopback(server.port()),
        &quick_policy(),
        &cancellation,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ServerReadinessError::InferenceRejected {
            status_code: Some(400),
            ..
        }
    ));
}

#[test]
fn cancellation_is_distinct_from_transport_failure() {
    let cancellation = AtomicBool::new(true);
    let endpoint = ServerEndpoint::loopback(9);

    assert_eq!(
        wait_for_ready_without_process(
            &endpoint,
            &quick_policy(),
            &cancellation,
        )
        .unwrap_err(),
        ServerReadinessError::Cancelled
    );
}

#[test]
fn loading_server_times_out_with_bounded_backoff() {
    let server = FakeHttpServer::spawn(|_| {
        response(503, r#"{"status":"loading"}"#)
    });
    let endpoint = ServerEndpoint::loopback(server.port());
    let mut policy = quick_policy();
    policy.timeout = Duration::from_millis(120);
    policy.request_timeout = Duration::from_millis(40);
    let cancellation = AtomicBool::new(false);
    let started = Instant::now();

    let error = wait_for_ready_without_process(
        &endpoint,
        &policy,
        &cancellation,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ServerReadinessError::Timeout {
            phase: ProbePhase::Health,
            last_status: Some(503),
            ..
        }
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
}

fn quick_policy() -> ReadinessPolicy {
    ReadinessPolicy {
        timeout: Duration::from_secs(2),
        request_timeout: Duration::from_millis(250),
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(40),
        ..ReadinessPolicy::default()
    }
}

fn response(status: u16, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        503 => "Service Unavailable",
        _ => "Response",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

struct FakeHttpServer {
    listener: TcpListener,
    _worker: thread::JoinHandle<()>,
}

impl FakeHttpServer {
    fn spawn<F>(handler: F) -> Self
    where
        F: Fn(String) -> String + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let worker_listener = listener.try_clone().unwrap();
        let handler = Arc::new(handler);
        let worker = thread::spawn(move || {
            serve_requests(worker_listener, handler);
        });
        Self {
            listener,
            _worker: worker,
        }
    }

    fn port(&self) -> u16 {
        self.listener.local_addr().unwrap().port()
    }
}

fn serve_requests<F>(listener: TcpListener, handler: Arc<F>)
where
    F: Fn(String) -> String + Send + Sync + 'static,
{
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let request = read_request(&mut stream);
                let response = handler(request);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(_) => break,
        }
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}
