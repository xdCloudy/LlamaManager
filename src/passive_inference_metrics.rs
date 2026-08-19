use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use thiserror::Error;

use crate::server_readiness::ServerEndpoint;

const MAX_CONTROL_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct PassiveInferenceMetricsSnapshot {
    pub model: Option<String>,
    pub source_endpoint: String,
    pub speculative_type: Option<String>,
    pub observed_at_unix_ms: u64,
    pub prompt_tps: Option<f64>,
    pub decode_tps: Option<f64>,
    pub prompt_tokens_total: Option<f64>,
    pub cached_prompt_tokens_total: Option<f64>,
    pub decode_tokens_total: Option<f64>,
    pub requests_processing: Option<f64>,
    pub requests_deferred: Option<f64>,
    pub busy_slots_per_decode: Option<f64>,
    pub speculative_draft_tokens_total: Option<f64>,
    pub speculative_accepted_tokens_total: Option<f64>,
    pub speculative_drafts_total: Option<f64>,
    pub speculative_acceptance_rate: Option<f64>,
}

impl PassiveInferenceMetricsSnapshot {
    pub fn is_mtp(&self) -> bool {
        self.speculative_type
            .as_deref()
            .is_some_and(|mode| mode.to_ascii_lowercase().contains("mtp"))
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PassiveInferenceMetricsError {
    #[error("server port must be in 1..=65535")]
    InvalidPort,
    #[error("API key cannot contain CR/LF characters")]
    InvalidApiKey,
    #[error("failed to resolve server host {host}: {message}")]
    HostResolution { host: String, message: String },
    #[error("non-loopback target {host} requires explicit opt-in")]
    NonLoopbackDenied { host: String },
    #[error("could not connect to {endpoint}: {message}")]
    Connect { endpoint: String, message: String },
    #[error("passive telemetry {phase} failed: {message}")]
    Io {
        phase: &'static str,
        message: String,
    },
    #[error("passive telemetry response exceeded {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("passive telemetry response ended before HTTP headers completed")]
    MissingHeaders,
    #[error("passive telemetry returned an invalid HTTP status line")]
    InvalidStatusLine,
    #[error("llama.cpp /metrics returned HTTP {status_code}")]
    MetricsHttpRejected { status_code: u16 },
    #[error("llama.cpp /metrics is unavailable on this runtime (HTTP {status_code})")]
    MetricsUnsupported { status_code: u16 },
    #[error("llama.cpp /metrics response was not valid UTF-8")]
    InvalidUtf8,
    #[error("llama.cpp /metrics response contained no recognized metrics")]
    NoRecognizedMetrics,
}

#[derive(Debug, Clone)]
struct HttpResponse {
    status_code: u16,
    body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouterChildCandidate {
    id: String,
    last_used: Option<u64>,
    child_port: Option<u16>,
    speculative_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PassiveMetricsTarget {
    endpoint: ServerEndpoint,
    model: Option<String>,
    speculative_type: Option<String>,
}

pub fn poll_passive_inference_metrics(
    configured_endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<PassiveInferenceMetricsSnapshot, PassiveInferenceMetricsError> {
    validate_endpoint(configured_endpoint)?;
    let target = resolve_passive_metrics_target(configured_endpoint, timeout)?;
    let response = get(&target.endpoint, "/metrics", timeout, "metrics read")?;

    match response.status_code {
        200..=299 => {}
        404 | 405 | 501 => {
            return Err(PassiveInferenceMetricsError::MetricsUnsupported {
                status_code: response.status_code,
            });
        }
        status_code => {
            return Err(PassiveInferenceMetricsError::MetricsHttpRejected { status_code });
        }
    }

    let body = std::str::from_utf8(&response.body)
        .map_err(|_| PassiveInferenceMetricsError::InvalidUtf8)?;
    parse_prometheus_metrics(
        body,
        target.model,
        target.endpoint.authority(),
        target.speculative_type,
        now_unix_ms(),
    )
}

fn resolve_passive_metrics_target(
    configured_endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<PassiveMetricsTarget, PassiveInferenceMetricsError> {
    if !endpoint_is_loopback(configured_endpoint)? {
        return Ok(direct_target(configured_endpoint));
    }

    let response = match get(
        configured_endpoint,
        "/models",
        timeout,
        "router model discovery",
    ) {
        Ok(response) => response,
        Err(_) => return Ok(direct_target(configured_endpoint)),
    };
    if !(200..=299).contains(&response.status_code) {
        return Ok(direct_target(configured_endpoint));
    }

    let Ok(payload) = serde_json::from_slice::<Value>(&response.body) else {
        return Ok(direct_target(configured_endpoint));
    };
    let Some(models) = payload.get("data").and_then(Value::as_array) else {
        return Ok(direct_target(configured_endpoint));
    };

    let mut router_shape_seen = false;
    let mut candidates = Vec::new();
    for model in models {
        let Some(id) = model.get("id").and_then(Value::as_str) else {
            continue;
        };
        let status = model
            .pointer("/status/value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        router_shape_seen |= !status.is_empty();
        if !matches!(status, "loaded" | "sleeping") {
            continue;
        }
        candidates.push(RouterChildCandidate {
            id: id.to_owned(),
            last_used: model
                .pointer("/status/last_used")
                .and_then(Value::as_u64)
                .or_else(|| model.get("last_used").and_then(Value::as_u64))
                .or_else(|| model.get("last_used_ms").and_then(Value::as_u64)),
            child_port: child_port_from_model(model),
            speculative_type: speculative_type_from_model(model),
        });
    }

    if !router_shape_seen {
        return Ok(direct_target(configured_endpoint));
    }
    let Some(candidate) = select_router_child(candidates) else {
        return Ok(direct_target(configured_endpoint));
    };

    let Some(port) = candidate
        .child_port
        .filter(|port| *port != configured_endpoint.port)
    else {
        return Ok(PassiveMetricsTarget {
            endpoint: configured_endpoint.clone(),
            model: Some(candidate.id),
            speculative_type: candidate.speculative_type,
        });
    };

    Ok(PassiveMetricsTarget {
        endpoint: ServerEndpoint {
            host: "127.0.0.1".to_owned(),
            port,
            api_key: configured_endpoint.api_key.clone(),
            allow_non_loopback: false,
        },
        model: Some(candidate.id),
        speculative_type: candidate.speculative_type,
    })
}

fn direct_target(endpoint: &ServerEndpoint) -> PassiveMetricsTarget {
    PassiveMetricsTarget {
        endpoint: endpoint.clone(),
        model: None,
        speculative_type: None,
    }
}

fn select_router_child(mut candidates: Vec<RouterChildCandidate>) -> Option<RouterChildCandidate> {
    match candidates.len() {
        0 => None,
        1 => candidates.pop(),
        _ => {
            let newest_timestamp = candidates
                .iter()
                .filter_map(|candidate| candidate.last_used)
                .max()?;
            let mut newest = candidates
                .into_iter()
                .filter(|candidate| candidate.last_used == Some(newest_timestamp));
            let selected = newest.next()?;
            newest.next().is_none().then_some(selected)
        }
    }
}

fn child_port_from_model(model: &Value) -> Option<u16> {
    model
        .get("port")
        .and_then(parse_port_value)
        .or_else(|| model.pointer("/status/port").and_then(parse_port_value))
        .or_else(|| {
            model
                .pointer("/status/args")
                .and_then(Value::as_array)
                .and_then(|args| child_port_from_args(args))
        })
}

fn speculative_type_from_model(model: &Value) -> Option<String> {
    model
        .pointer("/status/args")
        .and_then(Value::as_array)
        .and_then(|args| speculative_type_from_args(args))
}

fn parse_port_value(value: &Value) -> Option<u16> {
    value
        .as_u64()
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .or_else(|| {
            value
                .as_str()
                .and_then(|port| port.parse::<u16>().ok())
                .filter(|port| *port != 0)
        })
}

fn child_port_from_args(args: &[Value]) -> Option<u16> {
    let mut index = 0;
    while index < args.len() {
        let Some(argument) = args[index].as_str() else {
            index += 1;
            continue;
        };
        if matches!(argument, "--port" | "-p") {
            return args.get(index + 1).and_then(parse_port_value);
        }
        if let Some(port) = argument
            .strip_prefix("--port=")
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port != 0)
        {
            return Some(port);
        }
        index += 1;
    }
    None
}

fn speculative_type_from_args(args: &[Value]) -> Option<String> {
    let mut index = 0;
    while index < args.len() {
        let Some(argument) = args[index].as_str() else {
            index += 1;
            continue;
        };
        if argument == "--spec-type" {
            return args
                .get(index + 1)
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }
        if let Some(value) = argument.strip_prefix("--spec-type=")
            && !value.is_empty()
        {
            return Some(value.to_owned());
        }
        index += 1;
    }
    None
}

fn parse_prometheus_metrics(
    body: &str,
    model: Option<String>,
    source_endpoint: String,
    speculative_type: Option<String>,
    observed_at_unix_ms: u64,
) -> Result<PassiveInferenceMetricsSnapshot, PassiveInferenceMetricsError> {
    let mut metrics = BTreeMap::<&str, f64>::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.starts_with("llamacpp:") {
            continue;
        }
        let Some((name, value)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if name.contains('{') {
            continue;
        }
        let Ok(value) = value.trim().parse::<f64>() else {
            continue;
        };
        if value.is_finite() && value >= 0.0 {
            metrics.insert(name, value);
        }
    }

    let recognized = [
        "llamacpp:prompt_tokens_seconds",
        "llamacpp:predicted_tokens_seconds",
        "llamacpp:prompt_tokens_total",
        "llamacpp:prompt_tokens_cached_total",
        "llamacpp:tokens_predicted_total",
        "llamacpp:requests_processing",
        "llamacpp:requests_deferred",
        "llamacpp:n_busy_slots_per_decode",
        "llamacpp:spec_decode_num_draft_tokens_total",
        "llamacpp:spec_decode_num_accepted_tokens_total",
        "llamacpp:spec_decode_num_drafts_total",
    ];
    if !recognized.iter().any(|name| metrics.contains_key(name)) {
        return Err(PassiveInferenceMetricsError::NoRecognizedMetrics);
    }

    let draft = metric(&metrics, "llamacpp:spec_decode_num_draft_tokens_total");
    let accepted = metric(&metrics, "llamacpp:spec_decode_num_accepted_tokens_total");
    let speculative_acceptance_rate = match (draft, accepted) {
        (Some(draft), Some(accepted)) if draft > 0.0 => Some((accepted / draft).clamp(0.0, 1.0)),
        _ => None,
    };

    Ok(PassiveInferenceMetricsSnapshot {
        model,
        source_endpoint,
        speculative_type,
        observed_at_unix_ms,
        prompt_tps: metric(&metrics, "llamacpp:prompt_tokens_seconds"),
        decode_tps: metric(&metrics, "llamacpp:predicted_tokens_seconds"),
        prompt_tokens_total: metric(&metrics, "llamacpp:prompt_tokens_total"),
        cached_prompt_tokens_total: metric(&metrics, "llamacpp:prompt_tokens_cached_total"),
        decode_tokens_total: metric(&metrics, "llamacpp:tokens_predicted_total"),
        requests_processing: metric(&metrics, "llamacpp:requests_processing"),
        requests_deferred: metric(&metrics, "llamacpp:requests_deferred"),
        busy_slots_per_decode: metric(&metrics, "llamacpp:n_busy_slots_per_decode"),
        speculative_draft_tokens_total: draft,
        speculative_accepted_tokens_total: accepted,
        speculative_drafts_total: metric(&metrics, "llamacpp:spec_decode_num_drafts_total"),
        speculative_acceptance_rate,
    })
}

fn metric(metrics: &BTreeMap<&str, f64>, name: &str) -> Option<f64> {
    metrics.get(name).copied()
}

fn validate_endpoint(endpoint: &ServerEndpoint) -> Result<(), PassiveInferenceMetricsError> {
    if endpoint.port == 0 {
        return Err(PassiveInferenceMetricsError::InvalidPort);
    }
    if endpoint
        .api_key
        .as_deref()
        .is_some_and(|key| key.contains('\r') || key.contains('\n'))
    {
        return Err(PassiveInferenceMetricsError::InvalidApiKey);
    }
    let addresses = resolve(endpoint)?;
    if addresses.iter().any(|address| !address.ip().is_loopback()) && !endpoint.allow_non_loopback {
        return Err(PassiveInferenceMetricsError::NonLoopbackDenied {
            host: endpoint.host.clone(),
        });
    }
    Ok(())
}

fn endpoint_is_loopback(endpoint: &ServerEndpoint) -> Result<bool, PassiveInferenceMetricsError> {
    let addresses = resolve(endpoint)?;
    Ok(!addresses.is_empty() && addresses.iter().all(|address| address.ip().is_loopback()))
}

fn resolve(endpoint: &ServerEndpoint) -> Result<Vec<SocketAddr>, PassiveInferenceMetricsError> {
    let addresses: Vec<_> = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| PassiveInferenceMetricsError::HostResolution {
            host: endpoint.host.clone(),
            message: error.to_string(),
        })?
        .collect();
    if addresses.is_empty() {
        return Err(PassiveInferenceMetricsError::HostResolution {
            host: endpoint.host.clone(),
            message: "host resolved to no addresses".to_owned(),
        });
    }
    Ok(addresses)
}

fn connect(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<TcpStream, PassiveInferenceMetricsError> {
    let mut last_error = None;
    for address in resolve(endpoint)? {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(PassiveInferenceMetricsError::Connect {
        endpoint: endpoint.authority(),
        message: last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "no resolved address accepted the connection".to_owned()),
    })
}

fn get(
    endpoint: &ServerEndpoint,
    path: &str,
    timeout: Duration,
    phase: &'static str,
) -> Result<HttpResponse, PassiveInferenceMetricsError> {
    let mut stream = connect(endpoint, timeout)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| io_error("read-timeout setup", error))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| io_error("write-timeout setup", error))?;

    let mut request = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: */*\r\n",
        endpoint.authority()
    );
    if let Some(api_key) = endpoint.api_key.as_deref() {
        request.push_str("Authorization: Bearer ");
        request.push_str(api_key);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|error| io_error(phase, error))?;

    let mut bytes = Vec::new();
    let mut expected_total = None;
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| io_error(phase, error))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_CONTROL_BYTES {
            return Err(PassiveInferenceMetricsError::ResponseTooLarge {
                limit: MAX_CONTROL_BYTES,
            });
        }
        if expected_total.is_none()
            && let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n")
            && let Some(content_length) = parse_content_length(&bytes[..header_end])
        {
            let total = header_end
                .checked_add(4)
                .and_then(|value| value.checked_add(content_length))
                .ok_or(PassiveInferenceMetricsError::ResponseTooLarge {
                    limit: MAX_CONTROL_BYTES,
                })?;
            if total > MAX_CONTROL_BYTES {
                return Err(PassiveInferenceMetricsError::ResponseTooLarge {
                    limit: MAX_CONTROL_BYTES,
                });
            }
            expected_total = Some(total);
        }
        if expected_total.is_some_and(|total| bytes.len() >= total) {
            break;
        }
    }

    let header_end =
        find_bytes(&bytes, b"\r\n\r\n").ok_or(PassiveInferenceMetricsError::MissingHeaders)?;
    let headers = &bytes[..header_end];
    let status_code = parse_status(headers)?;
    let mut body = bytes[header_end + 4..].to_vec();
    if let Some(content_length) = parse_content_length(headers) {
        body.truncate(content_length);
    }
    Ok(HttpResponse { status_code, body })
}

fn parse_status(headers: &[u8]) -> Result<u16, PassiveInferenceMetricsError> {
    let headers = String::from_utf8_lossy(headers);
    headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or(PassiveInferenceMetricsError::InvalidStatusLine)
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let headers = String::from_utf8_lossy(headers);
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            value.trim().parse::<usize>().ok()
        } else {
            None
        }
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn io_error(phase: &'static str, error: std::io::Error) -> PassiveInferenceMetricsError {
    PassiveInferenceMetricsError::Io {
        phase,
        message: error.to_string(),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use std::{
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    use super::*;

    fn read_request(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 2048];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if find_bytes(&bytes, b"\r\n\r\n").is_some() {
                break;
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    #[test]
    fn parses_runtime_metrics_without_zero_filling_missing_fields() {
        let body = concat!(
            "# TYPE llamacpp:prompt_tokens_seconds gauge\n",
            "llamacpp:prompt_tokens_seconds 41.78\n",
            "llamacpp:predicted_tokens_seconds 4.54\n",
            "llamacpp:prompt_tokens_total 29907\n",
            "llamacpp:tokens_predicted_total 94\n",
            "llamacpp:requests_processing 1\n",
            "llamacpp:spec_decode_num_draft_tokens_total 79\n",
            "llamacpp:spec_decode_num_accepted_tokens_total 75\n",
        );
        let snapshot = parse_prometheus_metrics(
            body,
            Some("Qwen3.8-27B".to_owned()),
            "127.0.0.1:65421".to_owned(),
            Some("draft-mtp".to_owned()),
            1234,
        )
        .unwrap();

        assert_eq!(snapshot.prompt_tps, Some(41.78));
        assert_eq!(snapshot.decode_tps, Some(4.54));
        assert_eq!(snapshot.requests_processing, Some(1.0));
        assert_eq!(snapshot.cached_prompt_tokens_total, None);
        assert!(snapshot.is_mtp());
        assert!(
            snapshot
                .speculative_acceptance_rate
                .is_some_and(|ratio| (ratio - 75.0 / 79.0).abs() < 1e-9)
        );
    }

    #[test]
    fn router_args_expose_child_port_and_explicit_mtp_mode() {
        let model: Value = serde_json::from_str(
            r#"{"status":{"args":["llama-server","--port","65421","--spec-type","draft-mtp"]}}"#,
        )
        .unwrap();
        assert_eq!(child_port_from_model(&model), Some(65421));
        assert_eq!(
            speculative_type_from_model(&model).as_deref(),
            Some("draft-mtp")
        );
    }

    #[test]
    fn passive_poll_uses_router_child_metrics_while_never_touching_completion() {
        let child = TcpListener::bind("127.0.0.1:0").unwrap();
        let child_port = child.local_addr().unwrap().port();
        let router = TcpListener::bind("127.0.0.1:0").unwrap();
        let router_port = router.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));

        let router_requests = Arc::clone(&requests);
        let router_thread = thread::spawn(move || {
            let (mut stream, _) = router.accept().unwrap();
            let request = read_request(&mut stream);
            router_requests.lock().unwrap().push(request);
            let body = format!(
                "{{\"data\":[{{\"id\":\"Qwen3.8-27B\",\"status\":{{\"value\":\"loaded\",\"last_used\":99,\"args\":[\"llama-server\",\"--port\",\"{child_port}\",\"--spec-type\",\"draft-mtp\"]}}}}]}}"
            );
            write_response(&mut stream, 200, "application/json", &body);
        });

        let child_requests = Arc::clone(&requests);
        let child_thread = thread::spawn(move || {
            let (mut stream, _) = child.accept().unwrap();
            let request = read_request(&mut stream);
            child_requests.lock().unwrap().push(request);
            write_response(
                &mut stream,
                200,
                "text/plain",
                concat!(
                    "llamacpp:requests_processing 1\n",
                    "llamacpp:predicted_tokens_seconds 3.39\n",
                    "llamacpp:spec_decode_num_draft_tokens_total 145\n",
                    "llamacpp:spec_decode_num_accepted_tokens_total 114\n",
                ),
            );
        });

        let snapshot = poll_passive_inference_metrics(
            &ServerEndpoint::loopback(router_port),
            Duration::from_secs(1),
        )
        .unwrap();
        router_thread.join().unwrap();
        child_thread.join().unwrap();

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /models HTTP/1.1"));
        assert!(requests[1].starts_with("GET /metrics HTTP/1.1"));
        assert!(
            requests
                .iter()
                .all(|request| !request.contains("/completion"))
        );
        assert_eq!(snapshot.model.as_deref(), Some("Qwen3.8-27B"));
        assert_eq!(snapshot.requests_processing, Some(1.0));
        assert!(snapshot.is_mtp());
    }
}
