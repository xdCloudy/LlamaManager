use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use thiserror::Error;

use crate::{
    inference_telemetry::{
        InferenceRequestObservation, InferenceTelemetryParseError, InferenceTelemetrySnapshot,
        parse_llama_cpp_completion,
    },
    server_readiness::ServerEndpoint,
};

const MAX_STREAM_BYTES: usize = 512 * 1024;
const TELEMETRY_PROBE_BODY: &str =
    r#"{"prompt":"Telemetry probe: reply OK","n_predict":4,"temperature":0,"stream":true}"#;

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
    #[error("streaming inference response did not expose a generated token")]
    MissingFirstToken,
    #[error("streaming inference response did not expose a final timings event")]
    MissingTimings,
    #[error(transparent)]
    TelemetryParse(#[from] InferenceTelemetryParseError),
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
    let started = Instant::now();
    let mut stream = connect(endpoint, timeout)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| io_error("read-timeout setup", error))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| io_error("write-timeout setup", error))?;

    let request = build_request(endpoint);
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|error| io_error("request write", error))?;

    let mut received = Vec::new();
    let mut total_bytes = 0_usize;
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
            let headers = String::from_utf8_lossy(&received[..header_end]);
            let status_line = headers
                .lines()
                .next()
                .ok_or(StreamingInferenceProbeError::InvalidStatusLine)?;
            let parsed_status = status_line
                .split_whitespace()
                .nth(1)
                .and_then(|value| value.parse::<u16>().ok())
                .ok_or(StreamingInferenceProbeError::InvalidStatusLine)?;
            if !(200..=299).contains(&parsed_status) {
                return Err(StreamingInferenceProbeError::HttpRejected {
                    status_code: parsed_status,
                });
            }
            status_code = Some(parsed_status);
            body_pending.push_str(&String::from_utf8_lossy(&received[header_end + 4..]));
            received.clear();
            headers_done = true;
        } else {
            body_pending.push_str(&String::from_utf8_lossy(&received));
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
    if final_event.is_none() && !body_pending.trim().is_empty() {
        body_pending.push('\n');
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
            requested_model: None,
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

fn build_request(endpoint: &ServerEndpoint) -> String {
    let mut request = format!(
        "POST /completion HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: text/event-stream\r\nContent-Type: application/json\r\n",
        endpoint.authority()
    );
    if let Some(api_key) = endpoint.api_key.as_deref() {
        request.push_str("Authorization: Bearer ");
        request.push_str(api_key);
        request.push_str("\r\n");
    }
    request.push_str(&format!(
        "Content-Length: {}\r\n\r\n{}",
        TELEMETRY_PROBE_BODY.len(),
        TELEMETRY_PROBE_BODY
    ));
    request
}

fn consume_sse_lines(
    pending: &mut String,
    started: Instant,
    first_token_elapsed: &mut Option<Duration>,
    final_event: &mut Option<String>,
    event_count: &mut usize,
) {
    while let Some(newline) = pending.find('\n') {
        let line = pending[..newline].trim().to_owned();
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

    fn spawn_sse_server(status: u16) -> (ServerEndpoint, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            tx.send(request).unwrap();

            if status != 200 {
                let response = format!(
                    "HTTP/1.1 {status} Rejected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                stream.write_all(response.as_bytes()).unwrap();
                return;
            }

            let body = concat!(
                "data: {\"content\":\"O\"}\n\n",
                "data: {\"content\":\"\",\"model\":\"fake.gguf\",\"timings\":{\"prompt_per_second\":123.0,\"predicted_per_second\":45.0,\"prompt_n\":3,\"predicted_n\":2,\"cache_n\":0},\"generation_settings\":{\"speculative.types\":\"none\"}}\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
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
}
