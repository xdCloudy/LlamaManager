use std::{
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use thiserror::Error;

use crate::server_process::{ManagedProcessState, ManagedServerProcess, ProcessExitEvidence};

const MAX_HTTP_RESPONSE_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEndpoint {
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub allow_non_loopback: bool,
}

impl ServerEndpoint {
    pub fn loopback(port: u16) -> Self {
        Self {
            host: "127.0.0.1".into(),
            port,
            api_key: None,
            allow_non_loopback: false,
        }
    }

    pub fn authority(&self) -> String {
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortAvailability {
    Available,
    InUse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbePhase {
    Health,
    Inference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpProbeEvidence {
    pub method: String,
    pub path: String,
    pub status_code: u16,
    pub body_excerpt: String,
    pub observed_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthCapabilityEvidence {
    Healthy(HttpProbeEvidence),
    UnsupportedObserved(Vec<HttpProbeEvidence>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferenceProbe {
    pub path: String,
    pub body: String,
    pub success_body_markers: Vec<String>,
}

impl InferenceProbe {
    pub fn llama_cpp_native_completion() -> Self {
        Self {
            path: "/completion".into(),
            body: r#"{"prompt":"Reply with OK","n_predict":1,"temperature":0}"#.into(),
            success_body_markers: vec!["\"content\"".into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessPolicy {
    pub timeout: Duration,
    pub request_timeout: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub health_paths: Vec<String>,
    pub inference: InferenceProbe,
}

impl Default for ReadinessPolicy {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            request_timeout: Duration::from_secs(2),
            initial_backoff: Duration::from_millis(50),
            max_backoff: Duration::from_millis(500),
            health_paths: vec!["/health".into(), "/v1/models".into()],
            inference: InferenceProbe::llama_cpp_native_completion(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerReadinessEvidence {
    pub endpoint: String,
    pub health: HealthCapabilityEvidence,
    pub inference: HttpProbeEvidence,
    pub attempts: u32,
    pub elapsed: Duration,
    pub authenticated: bool,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ServerReadinessError {
    #[error("server port {host}:{port} is already in use")]
    PortInUse { host: String, port: u16 },

    #[error("server port must be in 1..=65535")]
    InvalidPort,

    #[error("non-loopback bind/probe target {host} requires explicit opt-in")]
    NonLoopbackDenied { host: String },

    #[error("failed to resolve server host {host}: {message}")]
    HostResolution { host: String, message: String },

    #[error("port availability check failed for {host}:{port}: {message}")]
    PortCheck {
        host: String,
        port: u16,
        message: String,
    },

    #[error("managed server exited while waiting for readiness: {evidence:?}")]
    ProcessExited { evidence: ProcessExitEvidence },

    #[error("server readiness was cancelled")]
    Cancelled,

    #[error("server readiness timed out during {phase:?} after {elapsed:?}")]
    Timeout {
        phase: ProbePhase,
        elapsed: Duration,
        last_status: Option<u16>,
    },

    #[error("server authentication was rejected during {phase:?} with HTTP {status_code}")]
    AuthenticationRejected {
        phase: ProbePhase,
        status_code: u16,
    },

    #[error("health probe {path} was rejected with HTTP {status_code}")]
    HealthRejected { path: String, status_code: u16 },

    #[error("minimal inference probe was rejected: {reason}")]
    InferenceRejected {
        status_code: Option<u16>,
        reason: String,
    },

    #[error("HTTP probe transport failed for {path}: {message}")]
    Transport { path: String, message: String },
}

pub fn check_port_available(endpoint: &ServerEndpoint) -> Result<PortAvailability, ServerReadinessError> {
    let addresses = validate_and_resolve(endpoint)?;
    let mut saw_available = false;
    for address in addresses {
        match TcpListener::bind(address) {
            Ok(listener) => {
                saw_available = true;
                drop(listener);
            }
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                return Ok(PortAvailability::InUse);
            }
            Err(error) => {
                return Err(ServerReadinessError::PortCheck {
                    host: endpoint.host.clone(),
                    port: endpoint.port,
                    message: error.to_string(),
                });
            }
        }
    }
    if saw_available {
        Ok(PortAvailability::Available)
    } else {
        Err(ServerReadinessError::HostResolution {
            host: endpoint.host.clone(),
            message: "host resolved to no usable addresses".into(),
        })
    }
}

pub fn require_port_available(endpoint: &ServerEndpoint) -> Result<(), ServerReadinessError> {
    match check_port_available(endpoint)? {
        PortAvailability::Available => Ok(()),
        PortAvailability::InUse => Err(ServerReadinessError::PortInUse {
            host: endpoint.host.clone(),
            port: endpoint.port,
        }),
    }
}

pub fn wait_for_server_ready(
    process: &mut ManagedServerProcess,
    endpoint: &ServerEndpoint,
    policy: &ReadinessPolicy,
    cancellation: &AtomicBool,
) -> Result<ServerReadinessEvidence, ServerReadinessError> {
    wait_for_ready_with_state(endpoint, policy, cancellation, || match process.state() {
        Ok(ManagedProcessState::Running(_)) => Ok(None),
        Ok(ManagedProcessState::Exited { evidence, .. }) => Ok(Some(evidence)),
        Err(error) => Err(error.to_string()),
    })
}

fn wait_for_ready_with_state<F>(
    endpoint: &ServerEndpoint,
    policy: &ReadinessPolicy,
    cancellation: &AtomicBool,
    mut process_exit: F,
) -> Result<ServerReadinessEvidence, ServerReadinessError>
where
    F: FnMut() -> Result<Option<ProcessExitEvidence>, String>,
{
    validate_and_resolve(endpoint)?;
    let started = Instant::now();
    let deadline = started + policy.timeout;
    let mut backoff = policy.initial_backoff.max(Duration::from_millis(1));
    let max_backoff = policy.max_backoff.max(backoff);
    let mut attempts = 0_u32;
    let mut last_phase = ProbePhase::Health;
    let mut last_status = None;
    let mut unsupported_health = Vec::new();

    loop {
        if cancellation.load(Ordering::Acquire) {
            return Err(ServerReadinessError::Cancelled);
        }
        match process_exit().map_err(|message| ServerReadinessError::Transport {
            path: "<process-state>".into(),
            message,
        })? {
            Some(evidence) => return Err(ServerReadinessError::ProcessExited { evidence }),
            None => {}
        }
        if Instant::now() >= deadline {
            return Err(ServerReadinessError::Timeout {
                phase: last_phase,
                elapsed: started.elapsed(),
                last_status,
            });
        }

        attempts = attempts.saturating_add(1);
        unsupported_health.clear();
        let mut health_ready = false;
        let mut transient_health = false;
        let mut healthy_evidence = None;

        for path in &policy.health_paths {
            last_phase = ProbePhase::Health;
            match http_probe(endpoint, "GET", path, None, policy.request_timeout) {
                Ok(evidence) if is_success(evidence.status_code) => {
                    last_status = Some(evidence.status_code);
                    healthy_evidence = Some(evidence);
                    health_ready = true;
                    break;
                }
                Ok(evidence) if matches!(evidence.status_code, 401 | 403) => {
                    return Err(ServerReadinessError::AuthenticationRejected {
                        phase: ProbePhase::Health,
                        status_code: evidence.status_code,
                    });
                }
                Ok(evidence) if matches!(evidence.status_code, 404 | 405) => {
                    last_status = Some(evidence.status_code);
                    unsupported_health.push(evidence);
                }
                Ok(evidence) if evidence.status_code >= 500 => {
                    last_status = Some(evidence.status_code);
                    transient_health = true;
                }
                Ok(evidence) => {
                    return Err(ServerReadinessError::HealthRejected {
                        path: evidence.path,
                        status_code: evidence.status_code,
                    });
                }
                Err(ServerReadinessError::Transport { .. }) => {
                    transient_health = true;
                }
                Err(error) => return Err(error),
            }
        }

        let health = if let Some(evidence) = healthy_evidence {
            Some(HealthCapabilityEvidence::Healthy(evidence))
        } else if !policy.health_paths.is_empty()
            && unsupported_health.len() == policy.health_paths.len()
        {
            Some(HealthCapabilityEvidence::UnsupportedObserved(
                unsupported_health.clone(),
            ))
        } else if policy.health_paths.is_empty() {
            Some(HealthCapabilityEvidence::UnsupportedObserved(Vec::new()))
        } else {
            None
        };

        if health_ready || health.is_some() && !transient_health {
            last_phase = ProbePhase::Inference;
            match http_probe(
                endpoint,
                "POST",
                &policy.inference.path,
                Some(&policy.inference.body),
                policy.request_timeout,
            ) {
                Ok(evidence) if matches!(evidence.status_code, 401 | 403) => {
                    return Err(ServerReadinessError::AuthenticationRejected {
                        phase: ProbePhase::Inference,
                        status_code: evidence.status_code,
                    });
                }
                Ok(evidence) if is_success(evidence.status_code) => {
                    last_status = Some(evidence.status_code);
                    let markers_ok = policy
                        .inference
                        .success_body_markers
                        .iter()
                        .all(|marker| evidence.body_excerpt.contains(marker));
                    if !markers_ok {
                        return Err(ServerReadinessError::InferenceRejected {
                            status_code: Some(evidence.status_code),
                            reason: format!(
                                "successful HTTP response did not contain required evidence markers: {}",
                                policy.inference.success_body_markers.join(", ")
                            ),
                        });
                    }
                    return Ok(ServerReadinessEvidence {
                        endpoint: endpoint.authority(),
                        health: health.expect("health evidence established above"),
                        inference: evidence,
                        attempts,
                        elapsed: started.elapsed(),
                        authenticated: endpoint.api_key.is_some(),
                    });
                }
                Ok(evidence) if evidence.status_code >= 500 => {
                    last_status = Some(evidence.status_code);
                }
                Ok(evidence) => {
                    return Err(ServerReadinessError::InferenceRejected {
                        status_code: Some(evidence.status_code),
                        reason: format!("HTTP {} from {}", evidence.status_code, evidence.path),
                    });
                }
                Err(ServerReadinessError::Transport { .. }) => {}
                Err(error) => return Err(error),
            }
        }

        let now = Instant::now();
        if now >= deadline {
            return Err(ServerReadinessError::Timeout {
                phase: last_phase,
                elapsed: started.elapsed(),
                last_status,
            });
        }
        thread::sleep(backoff.min(deadline.saturating_duration_since(now)));
        backoff = (backoff * 2).min(max_backoff);
    }
}

fn validate_and_resolve(endpoint: &ServerEndpoint) -> Result<Vec<SocketAddr>, ServerReadinessError> {
    if endpoint.port == 0 {
        return Err(ServerReadinessError::InvalidPort);
    }
    let addresses: Vec<_> = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| ServerReadinessError::HostResolution {
            host: endpoint.host.clone(),
            message: error.to_string(),
        })?
        .collect();
    if addresses.is_empty() {
        return Err(ServerReadinessError::HostResolution {
            host: endpoint.host.clone(),
            message: "host resolved to no addresses".into(),
        });
    }
    if !endpoint.allow_non_loopback && addresses.iter().any(|address| !is_safe_loopback(address.ip())) {
        return Err(ServerReadinessError::NonLoopbackDenied {
            host: endpoint.host.clone(),
        });
    }
    Ok(addresses)
}

fn is_safe_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

fn http_probe(
    endpoint: &ServerEndpoint,
    method: &str,
    path: &str,
    body: Option<&str>,
    timeout: Duration,
) -> Result<HttpProbeEvidence, ServerReadinessError> {
    let addresses = validate_and_resolve(endpoint)?;
    let mut last_error = None;
    let mut stream = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(value) => {
                stream = Some(value);
                break;
            }
            Err(error) => last_error = Some(error),
        }
    }
    let mut stream = stream.ok_or_else(|| ServerReadinessError::Transport {
        path: path.into(),
        message: last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "no resolved address accepted the connection".into()),
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| transport_error(path, error))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| transport_error(path, error))?;

    let body = body.unwrap_or("");
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n",
        endpoint.authority()
    );
    if let Some(api_key) = &endpoint.api_key {
        request.push_str("Authorization: Bearer ");
        request.push_str(api_key);
        request.push_str("\r\n");
    }
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    request.push_str(body);

    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|error| transport_error(path, error))?;

    let mut bytes = Vec::new();
    stream
        .take(MAX_HTTP_RESPONSE_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| transport_error(path, error))?;
    let response = String::from_utf8_lossy(&bytes);
    let status_line = response
        .lines()
        .next()
        .ok_or_else(|| ServerReadinessError::Transport {
            path: path.into(),
            message: "empty HTTP response".into(),
        })?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| ServerReadinessError::Transport {
            path: path.into(),
            message: format!("invalid HTTP status line: {status_line}"),
        })?;
    let body_excerpt = response
        .split_once("\r\n\r\n")
        .map(|(_, value)| value)
        .unwrap_or("")
        .chars()
        .take(8192)
        .collect();

    Ok(HttpProbeEvidence {
        method: method.into(),
        path: path.into(),
        status_code,
        body_excerpt,
        observed_at_unix_ms: now_unix_ms(),
    })
}

fn transport_error(path: &str, error: io::Error) -> ServerReadinessError {
    ServerReadinessError::Transport {
        path: path.into(),
        message: error.to_string(),
    }
}

fn is_success(status_code: u16) -> bool {
    (200..300).contains(&status_code)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    #[test]
    fn port_collision_is_detected_before_launch() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let endpoint = ServerEndpoint::loopback(port);
        assert_eq!(check_port_available(&endpoint).unwrap(), PortAvailability::InUse);
        assert!(matches!(
            require_port_available(&endpoint),
            Err(ServerReadinessError::PortInUse { port: observed, .. }) if observed == port
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
    fn observed_health_then_minimal_inference_is_required_for_ready() {
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
        let policy = quick_policy();
        let cancellation = AtomicBool::new(false);
        let evidence = wait_for_ready_with_state(&endpoint, &policy, &cancellation, || Ok(None)).unwrap();

        assert!(matches!(evidence.health, HealthCapabilityEvidence::Healthy(_)));
        assert_eq!(evidence.inference.status_code, 200);
        assert!(evidence.inference.body_excerpt.contains("\"content\""));
    }

    #[test]
    fn unsupported_health_endpoint_is_observed_not_version_guessed() {
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
        let evidence = wait_for_ready_with_state(&endpoint, &policy, &cancellation, || Ok(None)).unwrap();

        match evidence.health {
            HealthCapabilityEvidence::UnsupportedObserved(items) => {
                assert_eq!(items.len(), policy.health_paths.len());
                assert!(items.iter().all(|item| item.status_code == 404));
            }
            other => panic!("expected observed unsupported health evidence, got {other:?}"),
        }
    }

    #[test]
    fn bearer_auth_is_supported_and_rejection_is_distinct() {
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

        let unauthenticated = ServerEndpoint::loopback(server.port());
        let cancellation = AtomicBool::new(false);
        assert!(matches!(
            wait_for_ready_with_state(&unauthenticated, &quick_policy(), &cancellation, || Ok(None)),
            Err(ServerReadinessError::AuthenticationRejected {
                phase: ProbePhase::Health,
                status_code: 401
            })
        ));

        let mut authenticated = ServerEndpoint::loopback(server.port());
        authenticated.api_key = Some("correct-key".into());
        let ready = wait_for_ready_with_state(&authenticated, &quick_policy(), &cancellation, || Ok(None)).unwrap();
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
        let error = wait_for_ready_with_state(
            &ServerEndpoint::loopback(server.port()),
            &quick_policy(),
            &cancellation,
            || Ok(None),
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
    fn cancellation_and_process_exit_are_distinct() {
        let cancellation = AtomicBool::new(true);
        let endpoint = ServerEndpoint::loopback(9);
        assert_eq!(
            wait_for_ready_with_state(&endpoint, &quick_policy(), &cancellation, || Ok(None))
                .unwrap_err(),
            ServerReadinessError::Cancelled
        );

        let cancellation = AtomicBool::new(false);
        let exit = ProcessExitEvidence {
            code: Some(7),
            kind: crate::server_process::ProcessExitKind::Natural,
            observed_at_unix_ms: 1,
        };
        assert_eq!(
            wait_for_ready_with_state(&endpoint, &quick_policy(), &cancellation, || {
                Ok(Some(exit.clone()))
            })
            .unwrap_err(),
            ServerReadinessError::ProcessExited { evidence: exit }
        );
    }

    #[test]
    fn loading_server_times_out_with_bounded_backoff() {
        let server = FakeHttpServer::spawn(|_| response(503, r#"{"status":"loading"}"#));
        let endpoint = ServerEndpoint::loopback(server.port());
        let mut policy = quick_policy();
        policy.timeout = Duration::from_millis(120);
        policy.request_timeout = Duration::from_millis(40);
        let cancellation = AtomicBool::new(false);
        let started = Instant::now();
        let error = wait_for_ready_with_state(&endpoint, &policy, &cancellation, || Ok(None)).unwrap_err();
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
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline {
                    match worker_listener.accept() {
                        Ok((mut stream, _)) => {
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
                            let request = String::from_utf8_lossy(&request).into_owned();
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
}
