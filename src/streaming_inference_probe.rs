use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use thiserror::Error;

use crate::{
    inference_telemetry::{
        InferenceRequestObservation, InferenceTelemetryParseError, InferenceTelemetrySnapshot,
        parse_llama_cpp_completion,
    },
    server_readiness::ServerEndpoint,
};

const MAX_STREAM_BYTES: usize = 512 * 1024;
const MAX_CONTROL_BYTES: usize = 256 * 1024;
const ROUTER_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq)]
pub struct StreamingInferenceProbeEvidence {
    pub status_code: u16,
    pub event_count: usize,
    pub ttft_ms: f64,
    pub request_latency_ms: f64,
    pub snapshot: InferenceTelemetrySnapshot,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StreamingInferenceProbeError {
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
    #[error("streaming inference {phase} failed: {message}")]
    Io {
        phase: &'static str,
        message: String,
    },
    #[error("streaming inference response exceeded {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("streaming inference returned an invalid HTTP status line")]
    InvalidStatusLine,
    #[error("streaming inference returned HTTP {status_code}")]
    HttpRejected { status_code: u16 },
    #[error("streaming inference response ended before HTTP headers completed")]
    MissingHeaders,
    #[error("llama.cpp router has no loaded model available for telemetry")]
    NoLoadedRouterModel,
    #[error(
        "llama.cpp router has {count} loaded models and no unique most-recent model; select/use one model before probing"
    )]
    AmbiguousRouterModels { count: usize },
    #[error("llama.cpp model {model} is busy; retry when an inference slot is free")]
    Busy { model: String },
    #[error("streaming inference response did not expose a generated token")]
    MissingFirstToken,
    #[error("streaming inference response did not expose a final timings event")]
    MissingTimings,
    #[error(transparent)]
    TelemetryParse(#[from] InferenceTelemetryParseError),
}

#[derive(Debug)]
struct BufferedHttpResponse {
    status_code: u16,
    body: Vec<u8>,
}

#[derive(Debug, Clone)]
struct RouterModelCandidate {
    id: String,
    last_used: Option<u64>,
}

pub fn check_endpoint_reachable(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<(), StreamingInferenceProbeError> {
    validate_api_key(endpoint)?;
    let stream = connect(endpoint, timeout)?;
    drop(stream);
    Ok(())
}

pub fn probe_llama_cpp_streaming(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<StreamingInferenceProbeEvidence, StreamingInferenceProbeError> {
    validate_api_key(endpoint)?;

    // Router mode is the stable public endpoint (normally :8080). Discover the loaded model
    // there instead of requiring users to know an ephemeral child-server port.
    let selected_model = discover_router_model(endpoint, timeout)?;
    if let Some(model) = selected_model.as_deref() {
        ensure_router_slot_available(endpoint, model, timeout)?;
    }

    let started = Instant::now();
    let mut stream = connect(endpoint, timeout)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| io_error("read-timeout setup", error))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| io_error("write-timeout setup", error))?;

    let request = build_completion_request(endpoint, selected_model.as_deref());
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|error| io_error("request write", error))?;

    let mut received = Vec::new();
    let mut total_bytes = 0_usize;
    let mut headers_done = false;
    let mut status_code = None;
    let mut body_pending = Vec::new();
    let mut first_token_elapsed = None;
    let mut final_event = None;
    let mut event_count = 0_usize;
    let mut buffer = [0_u8; 4096];

    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| io_error("response read", error))?;
        if read == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(read);
        if total_bytes > MAX_STREAM_BYTES {
            return Err(StreamingInferenceProbeError::ResponseTooLarge {
                limit: MAX_STREAM_BYTES,
            });
        }
        received.extend_from_slice(&buffer[..read]);

        if !headers_done {
            let Some(header_end) = find_bytes(&received, b"\r\n\r\n") else {
                continue;
            };
            let parsed_status = parse_status_code(&received[..header_end])?;
            if !(200..=299).contains(&parsed_status) {
                return Err(StreamingInferenceProbeError::HttpRejected {
                    status_code: parsed_status,
                });
            }
            status_code = Some(parsed_status);
            body_pending.extend_from_slice(&received[header_end + 4..]);
            received.clear();
            headers_done = true;
        } else {
            body_pending.extend_from_slice(&received);
            received.clear();
        }

        consume_sse_lines(
            &mut body_pending,
            started,
            &mut first_token_elapsed,
            &mut final_event,
            &mut event_count,
        );
        if final_event.is_some() {
            break;
        }
    }

    if !headers_done {
        return Err(StreamingInferenceProbeError::MissingHeaders);
    }
    if final_event.is_none() && body_pending.iter().any(|byte| !byte.is_ascii_whitespace()) {
        body_pending.push(b'\n');
        consume_sse_lines(
            &mut body_pending,
            started,
            &mut first_token_elapsed,
            &mut final_event,
            &mut event_count,
        );
    }

    let ttft_ms = first_token_elapsed
        .ok_or(StreamingInferenceProbeError::MissingFirstToken)?
        .as_secs_f64()
        * 1_000.0;
    let final_event = final_event.ok_or(StreamingInferenceProbeError::MissingTimings)?;
    let request_latency_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let observed_at_unix_ms = now_unix_ms();
    let snapshot = parse_llama_cpp_completion(
        &final_event,
        InferenceRequestObservation {
            request_id: format!("telemetry-probe-{observed_at_unix_ms}"),
            endpoint: endpoint.authority(),
            server_pid: None,
            requested_model: selected_model,
            request_latency_ms,
            ttft_ms: Some(ttft_ms),
            observed_at_unix_ms,
        },
    )?;

    Ok(StreamingInferenceProbeEvidence {
        status_code: status_code.expect("headers_done implies a parsed status"),
        event_count,
        ttft_ms,
        request_latency_ms,
        snapshot,
    })
}

fn discover_router_model(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<Option<String>, StreamingInferenceProbeError> {
    let timeout = timeout.min(ROUTER_DISCOVERY_TIMEOUT);
    let request = build_get_request(endpoint, "/models");
    let response = buffered_request(endpoint, &request, timeout, "router model discovery")?;

    if matches!(response.status_code, 404 | 405) {
        return Ok(None);
    }
    if !(200..=299).contains(&response.status_code) {
        return Err(StreamingInferenceProbeError::HttpRejected {
            status_code: response.status_code,
        });
    }

    let Ok(payload) = serde_json::from_slice::<Value>(&response.body) else {
        return Ok(None);
    };
    let Some(models) = payload.get("data").and_then(Value::as_array) else {
        return Ok(None);
    };

    let mut loaded = Vec::new();
    let mut router_shape_seen = false;
    for model in models {
        let Some(id) = model.get("id").and_then(Value::as_str) else {
            continue;
        };
        let status = model
            .pointer("/status/value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !status.is_empty() {
            router_shape_seen = true;
        }
        if matches!(status, "loaded" | "sleeping") {
            loaded.push(RouterModelCandidate {
                id: id.to_owned(),
                last_used: model
                    .pointer("/status/last_used")
                    .and_then(Value::as_u64)
                    .or_else(|| model.get("last_used").and_then(Value::as_u64))
                    .or_else(|| model.get("last_used_ms").and_then(Value::as_u64)),
            });
        }
    }

    if !router_shape_seen {
        return Ok(None);
    }
    select_loaded_router_model(loaded).map(Some)
}

fn select_loaded_router_model(
    mut loaded: Vec<RouterModelCandidate>,
) -> Result<String, StreamingInferenceProbeError> {
    match loaded.len() {
        0 => Err(StreamingInferenceProbeError::NoLoadedRouterModel),
        1 => Ok(loaded.remove(0).id),
        count => {
            let newest = loaded
                .iter()
                .filter_map(|model| {
                    model
                        .last_used
                        .map(|last_used| (last_used, model.id.as_str()))
                })
                .max_by_key(|(last_used, _)| *last_used);
            let Some((newest_timestamp, newest_id)) = newest else {
                return Err(StreamingInferenceProbeError::AmbiguousRouterModels { count });
            };
            let unique_newest = loaded.iter().filter(|model| {
                model
                    .last_used
                    .is_some_and(|last_used| last_used == newest_timestamp)
            });
            if unique_newest.count() != 1 {
                return Err(StreamingInferenceProbeError::AmbiguousRouterModels { count });
            }
            Ok(newest_id.to_owned())
        }
    }
}

fn ensure_router_slot_available(
    endpoint: &ServerEndpoint,
    model: &str,
    timeout: Duration,
) -> Result<(), StreamingInferenceProbeError> {
    let timeout = timeout.min(ROUTER_DISCOVERY_TIMEOUT);
    let model_query = percent_encode_query(model);
    let path = format!("/slots?model={model_query}&fail_on_no_slot=1");
    let request = build_get_request(endpoint, &path);
    let response = buffered_request(endpoint, &request, timeout, "slot availability")?;

    match response.status_code {
        200..=299 => Ok(()),
        403..=405 => Ok(()),
        503 => Err(StreamingInferenceProbeError::Busy {
            model: model.to_owned(),
        }),
        status_code => Err(StreamingInferenceProbeError::HttpRejected { status_code }),
    }
}

fn buffered_request(
    endpoint: &ServerEndpoint,
    request: &str,
    timeout: Duration,
    phase: &'static str,
) -> Result<BufferedHttpResponse, StreamingInferenceProbeError> {
    let mut stream = connect(endpoint, timeout)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| io_error("control read-timeout setup", error))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| io_error("control write-timeout setup", error))?;
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|error| io_error(phase, error))?;

    let mut received = Vec::new();
    let mut expected_total = None;
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| io_error(phase, error))?;
        if read == 0 {
            break;
        }
        received.extend_from_slice(&buffer[..read]);
        if received.len() > MAX_CONTROL_BYTES {
            return Err(StreamingInferenceProbeError::ResponseTooLarge {
                limit: MAX_CONTROL_BYTES,
            });
        }

        if expected_total.is_none()
            && let Some(header_end) = find_bytes(&received, b"\r\n\r\n")
            && let Some(content_length) = parse_content_length(&received[..header_end])
        {
            let total = header_end
                .checked_add(4)
                .and_then(|value| value.checked_add(content_length))
                .ok_or(StreamingInferenceProbeError::ResponseTooLarge {
                    limit: MAX_CONTROL_BYTES,
                })?;
            if total > MAX_CONTROL_BYTES {
                return Err(StreamingInferenceProbeError::ResponseTooLarge {
                    limit: MAX_CONTROL_BYTES,
                });
            }
            expected_total = Some(total);
        }

        if expected_total.is_some_and(|total| received.len() >= total) {
            break;
        }
    }

    let header_end =
        find_bytes(&received, b"\r\n\r\n").ok_or(StreamingInferenceProbeError::MissingHeaders)?;
    let headers = &received[..header_end];
    if let Some(expected) = expected_total
        && received.len() < expected
    {
        return Err(StreamingInferenceProbeError::Io {
            phase,
            message: format!(
                "response ended before declared Content-Length completed: received {} of {expected} bytes",
                received.len()
            ),
        });
    }

    let mut body = received[header_end + 4..].to_vec();
    if let Some(content_length) = parse_content_length(headers) {
        body.truncate(content_length);
    }

    Ok(BufferedHttpResponse {
        status_code: parse_status_code(headers)?,
        body,
    })
}

fn validate_api_key(endpoint: &ServerEndpoint) -> Result<(), StreamingInferenceProbeError> {
    if endpoint
        .api_key
        .as_deref()
        .is_some_and(|key| key.contains('\r') || key.contains('\n'))
    {
        Err(StreamingInferenceProbeError::InvalidApiKey)
    } else {
        Ok(())
    }
}

fn resolve_endpoint(
    endpoint: &ServerEndpoint,
) -> Result<Vec<SocketAddr>, StreamingInferenceProbeError> {
    if endpoint.port == 0 {
        return Err(StreamingInferenceProbeError::InvalidPort);
    }
    let addresses: Vec<_> = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| StreamingInferenceProbeError::HostResolution {
            host: endpoint.host.clone(),
            message: error.to_string(),
        })?
        .collect();
    if addresses.is_empty() {
        return Err(StreamingInferenceProbeError::HostResolution {
            host: endpoint.host.clone(),
            message: "host resolved to no addresses".into(),
        });
    }
    if addresses.iter().any(|address| !address.ip().is_loopback()) && !endpoint.allow_non_loopback {
        return Err(StreamingInferenceProbeError::NonLoopbackDenied {
            host: endpoint.host.clone(),
        });
    }
    Ok(addresses)
}

fn connect(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<TcpStream, StreamingInferenceProbeError> {
    let mut last_error = None;
    for address in resolve_endpoint(endpoint)? {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(StreamingInferenceProbeError::Connect {
        endpoint: endpoint.authority(),
        message: last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "no resolved address accepted the connection".into()),
    })
}

fn build_get_request(endpoint: &ServerEndpoint, path: &str) -> String {
    let mut request = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n",
        endpoint.authority()
    );
    append_auth_header(&mut request, endpoint);
    request.push_str("\r\n");
    request
}

fn build_completion_request(endpoint: &ServerEndpoint, model: Option<&str>) -> String {
    let mut body = json!({
        "prompt": "Telemetry probe: reply OK",
        "n_predict": 4,
        "temperature": 0,
        "stream": true
    });
    if let Some(model) = model {
        body["model"] = Value::String(model.to_owned());
    }
    let body = serde_json::to_string(&body).expect("telemetry probe body is serializable");

    let mut request = format!(
        "POST /completion HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: text/event-stream\r\nContent-Type: application/json\r\n",
        endpoint.authority()
    );
    append_auth_header(&mut request, endpoint);
    request.push_str(&format!("Content-Length: {}\r\n\r\n{}", body.len(), body));
    request
}

fn append_auth_header(request: &mut String, endpoint: &ServerEndpoint) {
    if let Some(api_key) = endpoint.api_key.as_deref() {
        request.push_str("Authorization: Bearer ");
        request.push_str(api_key);
        request.push_str("\r\n");
    }
}

fn parse_status_code(headers: &[u8]) -> Result<u16, StreamingInferenceProbeError> {
    let headers = String::from_utf8_lossy(headers);
    let status_line = headers
        .lines()
        .next()
        .ok_or(StreamingInferenceProbeError::InvalidStatusLine)?;
    status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or(StreamingInferenceProbeError::InvalidStatusLine)
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

fn percent_encode_query(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

fn consume_sse_lines(
    pending: &mut Vec<u8>,
    started: Instant,
    first_token_elapsed: &mut Option<Duration>,
    final_event: &mut Option<String>,
    event_count: &mut usize,
) {
    while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
        let line = String::from_utf8_lossy(&pending[..newline])
            .trim()
            .to_owned();
        pending.drain(..=newline);
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
        *event_count = event_count.saturating_add(1);

        let has_token = event
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| !content.is_empty())
            || event
                .get("tokens")
                .and_then(Value::as_array)
                .is_some_and(|tokens| !tokens.is_empty());
        if has_token && first_token_elapsed.is_none() {
            *first_token_elapsed = Some(started.elapsed());
        }
        if event.get("timings").is_some() {
            *final_event = Some(payload.to_owned());
            break;
        }
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn io_error(phase: &'static str, error: std::io::Error) -> StreamingInferenceProbeError {
    StreamingInferenceProbeError::Io {
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
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::mpsc,
        thread,
    };

    use crate::hardware_telemetry::TelemetryState;

    use super::*;

    fn read_http_request(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 2048];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= header_end + 4 + content_length {
                break;
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn write_json_response(stream: &mut TcpStream, status: u16, body: &str) {
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    fn write_sse_response(stream: &mut TcpStream, status: u16, model: &str) {
        if status != 200 {
            write_json_response(stream, status, "{}");
            return;
        }
        let body = format!(
            "data: {{\"content\":\"O\"}}\n\ndata: {{\"content\":\"\",\"model\":\"{model}\",\"timings\":{{\"prompt_per_second\":123.0,\"predicted_per_second\":45.0,\"prompt_n\":3,\"predicted_n\":2,\"cache_n\":0}},\"generation_settings\":{{\"speculative.types\":\"none\"}}}}\n\n"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    fn spawn_sse_server(status: u16) -> (ServerEndpoint, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut discovery, _) = listener.accept().unwrap();
            let discovery_request = read_http_request(&mut discovery);
            assert!(discovery_request.starts_with("GET /models HTTP/1.1"));
            write_json_response(&mut discovery, 404, "{}");

            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            let _ = tx.send(request);
            write_sse_response(&mut stream, status, "fake.gguf");
        });
        (ServerEndpoint::loopback(port), rx)
    }

    fn spawn_fragmented_unicode_sse_server() -> ServerEndpoint {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut discovery, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut discovery);
            write_json_response(&mut discovery, 404, "{}");

            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            let body = concat!(
                "data: {\"content\":\"O\"}\n\n",
                "data: {\"content\":\"\",\"model\":\"mødel.gguf\",\"timings\":{\"prompt_per_second\":123.0,\"predicted_per_second\":45.0,\"prompt_n\":3,\"predicted_n\":2,\"cache_n\":0},\"generation_settings\":{\"speculative.types\":\"none\"}}\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let bytes = response.as_bytes();
            let utf8_start = find_bytes(bytes, "ø".as_bytes()).unwrap();
            let split = utf8_start + 1;
            stream.write_all(&bytes[..split]).unwrap();
            stream.flush().unwrap();
            thread::sleep(Duration::from_millis(50));
            stream.write_all(&bytes[split..]).unwrap();
            stream.flush().unwrap();
        });
        ServerEndpoint::loopback(port)
    }

    fn spawn_router_server(
        slots_status: u16,
    ) -> (ServerEndpoint, mpsc::Receiver<(String, String)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut models_stream, _) = listener.accept().unwrap();
            let models_request = read_http_request(&mut models_stream);
            let models = r#"{"object":"list","data":[{"id":"Agents-A1","status":{"value":"unloaded","last_used":0}},{"id":"Qwen3.8-27B","status":{"value":"loaded","last_used":99}},{"id":"KAT-Coder","status":{"value":"unloaded","last_used":0}}]}"#;
            write_json_response(&mut models_stream, 200, models);

            let (mut slots_stream, _) = listener.accept().unwrap();
            let slots_request = read_http_request(&mut slots_stream);
            if slots_status == 200 {
                write_json_response(
                    &mut slots_stream,
                    200,
                    r#"[{"id":0,"is_processing":false}]"#,
                );
            } else {
                write_json_response(&mut slots_stream, slots_status, "{}");
            }

            if slots_status != 200 {
                let _ = tx.send((models_request, slots_request));
                return;
            }

            let (mut completion_stream, _) = listener.accept().unwrap();
            let completion_request = read_http_request(&mut completion_stream);
            let _ = tx.send((slots_request, completion_request));
            write_sse_response(&mut completion_stream, 200, "Qwen3.8-27B");
        });
        (ServerEndpoint::loopback(port), rx)
    }

    #[test]
    fn streaming_probe_returns_request_bound_metrics_and_auth_header() {
        let (mut endpoint, request_rx) = spawn_sse_server(200);
        endpoint.api_key = Some("secret-token".into());
        let evidence = probe_llama_cpp_streaming(&endpoint, Duration::from_secs(2)).unwrap();

        let request = request_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request.starts_with("POST /completion HTTP/1.1"));
        assert!(request.contains("Authorization: Bearer secret-token\r\n"));
        assert!(request.contains("\"stream\":true"));
        assert_eq!(evidence.status_code, 200);
        assert_eq!(evidence.event_count, 2);
        assert!(evidence.ttft_ms >= 0.0);
        assert!(evidence.request_latency_ms >= evidence.ttft_ms);
        assert_eq!(evidence.snapshot.identity.endpoint, endpoint.authority());
        assert_eq!(evidence.snapshot.prompt_tps.live_value(), Some(&123.0));
        assert_eq!(evidence.snapshot.decode_tps.live_value(), Some(&45.0));
        assert!(matches!(
            evidence.snapshot.mtp_generated_tokens.state,
            TelemetryState::Unavailable { .. }
        ));
    }

    #[test]
    fn router_endpoint_autodiscovers_loaded_model_and_routes_probe() {
        let (mut endpoint, request_rx) = spawn_router_server(200);
        endpoint.api_key = Some("secret-token".into());
        let evidence = probe_llama_cpp_streaming(&endpoint, Duration::from_secs(2)).unwrap();

        let (slots_request, completion_request) =
            request_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            slots_request.starts_with("GET /slots?model=Qwen3.8-27B&fail_on_no_slot=1 HTTP/1.1")
        );
        assert!(slots_request.contains("Authorization: Bearer secret-token\r\n"));
        assert!(completion_request.starts_with("POST /completion HTTP/1.1"));
        assert!(completion_request.contains("\"model\":\"Qwen3.8-27B\""));
        assert_eq!(
            evidence.snapshot.identity.requested_model.as_deref(),
            Some("Qwen3.8-27B")
        );
        assert_eq!(
            evidence.snapshot.identity.reported_model.as_deref(),
            Some("Qwen3.8-27B")
        );
    }

    #[test]
    fn router_busy_slot_is_reported_before_a_long_queued_probe() {
        let (endpoint, request_rx) = spawn_router_server(503);
        assert_eq!(
            probe_llama_cpp_streaming(&endpoint, Duration::from_secs(2)).unwrap_err(),
            StreamingInferenceProbeError::Busy {
                model: "Qwen3.8-27B".to_owned()
            }
        );
        let (models_request, slots_request) =
            request_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(models_request.starts_with("GET /models HTTP/1.1"));
        assert!(slots_request.contains("fail_on_no_slot=1"));
    }

    #[test]
    fn router_multiple_loaded_models_selects_unique_most_recent() {
        assert_eq!(
            select_loaded_router_model(vec![
                RouterModelCandidate {
                    id: "older".into(),
                    last_used: Some(10),
                },
                RouterModelCandidate {
                    id: "newer".into(),
                    last_used: Some(20),
                },
            ])
            .unwrap(),
            "newer"
        );
    }

    #[test]
    fn router_multiple_loaded_models_without_recency_is_truthfully_ambiguous() {
        assert_eq!(
            select_loaded_router_model(vec![
                RouterModelCandidate {
                    id: "a".into(),
                    last_used: None,
                },
                RouterModelCandidate {
                    id: "b".into(),
                    last_used: None,
                },
            ])
            .unwrap_err(),
            StreamingInferenceProbeError::AmbiguousRouterModels { count: 2 }
        );
    }

    #[test]
    fn streaming_probe_preserves_utf8_split_across_tcp_reads() {
        let endpoint = spawn_fragmented_unicode_sse_server();
        let evidence = probe_llama_cpp_streaming(&endpoint, Duration::from_secs(2)).unwrap();
        assert_eq!(
            evidence.snapshot.identity.reported_model.as_deref(),
            Some("mødel.gguf")
        );
    }

    #[test]
    fn streaming_probe_preserves_http_failure_instead_of_parsing_fake_metrics() {
        let (endpoint, _) = spawn_sse_server(401);
        assert_eq!(
            probe_llama_cpp_streaming(&endpoint, Duration::from_secs(2)).unwrap_err(),
            StreamingInferenceProbeError::HttpRejected { status_code: 401 }
        );
    }

    #[test]
    fn api_key_header_injection_is_rejected_before_network_io() {
        let mut endpoint = ServerEndpoint::loopback(1);
        endpoint.api_key = Some("secret\r\nInjected: yes".into());
        assert_eq!(
            probe_llama_cpp_streaming(&endpoint, Duration::from_millis(10)).unwrap_err(),
            StreamingInferenceProbeError::InvalidApiKey
        );
    }

    #[test]
    fn router_model_names_are_percent_encoded_for_slot_query() {
        assert_eq!(
            percent_encode_query("Qwen/3.8:27B A"),
            "Qwen%2F3.8%3A27B%20A"
        );
    }

    #[test]
    fn content_length_parser_is_case_insensitive() {
        assert_eq!(
            parse_content_length(b"HTTP/1.1 200 OK\r\ncontent-length: 42\r\n"),
            Some(42)
        );
    }
}
