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
const MAX_BODY_EXCERPT_CHARS: usize = 8192;

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

    #[error("non-loopback target {host} requires explicit opt-in")]
    NonLoopbackDenied { host: String },

    #[error("failed to resolve server host {host}: {message}")]
    HostResolution { host: String, message: String },

    #[error("port check failed for {host}:{port}: {message}")]
    PortCheck {
        host: String,
        port: u16,
        message: String,
    },

    #[error("managed server exited while waiting for readiness: {evidence:?}")]
    ProcessExited { evidence: ProcessExitEvidence },

    #[error("server readiness was cancelled")]
    Cancelled,

    #[error("readiness timed out during {phase:?} after {elapsed:?}")]
    Timeout {
        phase: ProbePhase,
        elapsed: Duration,
        last_status: Option<u16>,
    },

    #[error("authentication failed during {phase:?} with HTTP {status_code}")]
    AuthenticationRejected { phase: ProbePhase, status_code: u16 },

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

pub fn check_port_available(
    endpoint: &ServerEndpoint,
) -> Result<PortAvailability, ServerReadinessError> {
    let addresses = resolve_endpoint(endpoint)?;
    for address in addresses {
        match TcpListener::bind(address) {
            Ok(listener) => drop(listener),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                return Ok(PortAvailability::InUse);
            }
            Err(error) => return Err(port_check_error(endpoint, error)),
        }
    }
    Ok(PortAvailability::Available)
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
    wait_for_ready(endpoint, policy, cancellation, || match process.state() {
        Ok(ManagedProcessState::Running(_)) => Ok(None),
        Ok(ManagedProcessState::Exited { evidence, .. }) => Ok(Some(evidence)),
        Err(error) => Err(error.to_string()),
    })
}

pub fn wait_for_ready_without_process(
    endpoint: &ServerEndpoint,
    policy: &ReadinessPolicy,
    cancellation: &AtomicBool,
) -> Result<ServerReadinessEvidence, ServerReadinessError> {
    wait_for_ready(endpoint, policy, cancellation, || Ok(None))
}

fn wait_for_ready<F>(
    endpoint: &ServerEndpoint,
    policy: &ReadinessPolicy,
    cancellation: &AtomicBool,
    mut process_exit: F,
) -> Result<ServerReadinessEvidence, ServerReadinessError>
where
    F: FnMut() -> Result<Option<ProcessExitEvidence>, String>,
{
    resolve_endpoint(endpoint)?;
    let started = Instant::now();
    let deadline = started + policy.timeout;
    let mut backoff = policy.initial_backoff.max(Duration::from_millis(1));
    let max_backoff = policy.max_backoff.max(backoff);
    let mut attempts = 0_u32;
    let mut last_phase = ProbePhase::Health;
    let mut last_status = None;

    loop {
        check_cancelled(cancellation)?;
        check_process_exit(&mut process_exit)?;
        ensure_before_deadline(started, deadline, last_phase, last_status)?;
        attempts = attempts.saturating_add(1);

        match probe_health(endpoint, policy) {
            Ok(Some(health)) => {
                last_phase = ProbePhase::Inference;
                match probe_inference(endpoint, &policy.inference, policy.request_timeout) {
                    Ok(inference) => {
                        return Ok(ServerReadinessEvidence {
                            endpoint: endpoint.authority(),
                            health,
                            inference,
                            attempts,
                            elapsed: started.elapsed(),
                            authenticated: endpoint.api_key.is_some(),
                        });
                    }
                    Err(AttemptError::Retry(status)) => last_status = status,
                    Err(AttemptError::Fatal(error)) => return Err(error),
                }
            }
            Ok(None) => last_status = None,
            Err(AttemptError::Retry(status)) => last_status = status,
            Err(AttemptError::Fatal(error)) => return Err(error),
        }

        check_cancelled(cancellation)?;
        check_process_exit(&mut process_exit)?;
        sleep_with_deadline(deadline, backoff);
        backoff = (backoff * 2).min(max_backoff);
    }
}

#[derive(Debug)]
enum AttemptError {
    Retry(Option<u16>),
    Fatal(ServerReadinessError),
}

fn probe_health(
    endpoint: &ServerEndpoint,
    policy: &ReadinessPolicy,
) -> Result<Option<HealthCapabilityEvidence>, AttemptError> {
    if policy.health_paths.is_empty() {
        return Ok(Some(HealthCapabilityEvidence::UnsupportedObserved(
            Vec::new(),
        )));
    }

    let mut unsupported = Vec::new();
    for path in &policy.health_paths {
        let evidence = match http_probe(endpoint, "GET", path, None, policy.request_timeout) {
            Ok(evidence) => evidence,
            Err(ServerReadinessError::Transport { .. }) => return Err(AttemptError::Retry(None)),
            Err(error) => return Err(AttemptError::Fatal(error)),
        };

        match evidence.status_code {
            200..=299 => return Ok(Some(HealthCapabilityEvidence::Healthy(evidence))),
            401 | 403 => {
                return Err(AttemptError::Fatal(
                    ServerReadinessError::AuthenticationRejected {
                        phase: ProbePhase::Health,
                        status_code: evidence.status_code,
                    },
                ));
            }
            404 | 405 => unsupported.push(evidence),
            500..=599 => return Err(AttemptError::Retry(Some(evidence.status_code))),
            _ => {
                return Err(AttemptError::Fatal(ServerReadinessError::HealthRejected {
                    path: evidence.path,
                    status_code: evidence.status_code,
                }));
            }
        }
    }

    Ok(Some(HealthCapabilityEvidence::UnsupportedObserved(
        unsupported,
    )))
}

fn probe_inference(
    endpoint: &ServerEndpoint,
    probe: &InferenceProbe,
    timeout: Duration,
) -> Result<HttpProbeEvidence, AttemptError> {
    let evidence = match http_probe(endpoint, "POST", &probe.path, Some(&probe.body), timeout) {
        Ok(evidence) => evidence,
        Err(ServerReadinessError::Transport { .. }) => return Err(AttemptError::Retry(None)),
        Err(error) => return Err(AttemptError::Fatal(error)),
    };

    match evidence.status_code {
        200..=299 => {
            let markers_ok = probe
                .success_body_markers
                .iter()
                .all(|marker| evidence.body_excerpt.contains(marker));
            if markers_ok {
                Ok(evidence)
            } else {
                Err(AttemptError::Fatal(
                    ServerReadinessError::InferenceRejected {
                        status_code: Some(evidence.status_code),
                        reason: "successful HTTP response lacked required evidence markers".into(),
                    },
                ))
            }
        }
        401 | 403 => Err(AttemptError::Fatal(
            ServerReadinessError::AuthenticationRejected {
                phase: ProbePhase::Inference,
                status_code: evidence.status_code,
            },
        )),
        500..=599 => Err(AttemptError::Retry(Some(evidence.status_code))),
        _ => Err(AttemptError::Fatal(
            ServerReadinessError::InferenceRejected {
                status_code: Some(evidence.status_code),
                reason: format!("HTTP {} from {}", evidence.status_code, evidence.path),
            },
        )),
    }
}

fn check_cancelled(cancellation: &AtomicBool) -> Result<(), ServerReadinessError> {
    if cancellation.load(Ordering::Acquire) {
        Err(ServerReadinessError::Cancelled)
    } else {
        Ok(())
    }
}

fn check_process_exit<F>(process_exit: &mut F) -> Result<(), ServerReadinessError>
where
    F: FnMut() -> Result<Option<ProcessExitEvidence>, String>,
{
    match process_exit() {
        Ok(Some(evidence)) => Err(ServerReadinessError::ProcessExited { evidence }),
        Ok(None) => Ok(()),
        Err(message) => Err(ServerReadinessError::Transport {
            path: "<process-state>".into(),
            message,
        }),
    }
}

fn ensure_before_deadline(
    started: Instant,
    deadline: Instant,
    phase: ProbePhase,
    last_status: Option<u16>,
) -> Result<(), ServerReadinessError> {
    if Instant::now() < deadline {
        Ok(())
    } else {
        Err(ServerReadinessError::Timeout {
            phase,
            elapsed: started.elapsed(),
            last_status,
        })
    }
}

fn sleep_with_deadline(deadline: Instant, backoff: Duration) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() {
        thread::sleep(backoff.min(remaining));
    }
}

fn resolve_endpoint(endpoint: &ServerEndpoint) -> Result<Vec<SocketAddr>, ServerReadinessError> {
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

    let non_loopback = addresses.iter().any(|address| !is_loopback(address.ip()));
    if non_loopback && !endpoint.allow_non_loopback {
        return Err(ServerReadinessError::NonLoopbackDenied {
            host: endpoint.host.clone(),
        });
    }
    Ok(addresses)
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

fn port_check_error(endpoint: &ServerEndpoint, error: io::Error) -> ServerReadinessError {
    ServerReadinessError::PortCheck {
        host: endpoint.host.clone(),
        port: endpoint.port,
        message: error.to_string(),
    }
}

fn http_probe(
    endpoint: &ServerEndpoint,
    method: &str,
    path: &str,
    body: Option<&str>,
    timeout: Duration,
) -> Result<HttpProbeEvidence, ServerReadinessError> {
    let mut stream = connect(endpoint, timeout, path)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| transport_error(path, error))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| transport_error(path, error))?;

    let body = body.unwrap_or("");
    let request = build_http_request(endpoint, method, path, body);
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|error| transport_error(path, error))?;

    let mut bytes = Vec::new();
    stream
        .take(MAX_HTTP_RESPONSE_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| transport_error(path, error))?;
    parse_http_response(method, path, &bytes)
}

fn connect(
    endpoint: &ServerEndpoint,
    timeout: Duration,
    path: &str,
) -> Result<TcpStream, ServerReadinessError> {
    let mut last_error = None;
    for address in resolve_endpoint(endpoint)? {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }

    Err(ServerReadinessError::Transport {
        path: path.into(),
        message: last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "no resolved address accepted the connection".into()),
    })
}

fn build_http_request(endpoint: &ServerEndpoint, method: &str, path: &str, body: &str) -> String {
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
        endpoint.authority()
    );
    request.push_str("Accept: application/json\r\n");
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
    request
}

fn parse_http_response(
    method: &str,
    path: &str,
    bytes: &[u8],
) -> Result<HttpProbeEvidence, ServerReadinessError> {
    let response = String::from_utf8_lossy(bytes);
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
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, value)| value)
        .unwrap_or("");
    let body_excerpt = body.chars().take(MAX_BODY_EXCERPT_CHARS).collect();

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

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
