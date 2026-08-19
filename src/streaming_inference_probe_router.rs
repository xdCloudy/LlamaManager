use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::Duration,
};

use serde_json::Value;

use crate::{server_readiness::ServerEndpoint, streaming_inference_probe_legacy};

pub use crate::streaming_inference_probe_legacy::{
    StreamingInferenceProbeError, StreamingInferenceProbeEvidence, check_endpoint_reachable,
};

const FALLBACK_CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONTROL_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouterChildCandidate {
    id: String,
    last_used: Option<u64>,
    child_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildSlotState {
    Available,
    Busy,
    Unsupported,
}

/// Run the request-bound llama.cpp telemetry probe.
///
/// The stable user-facing endpoint remains the configured router (normally :8080). Some
/// llama.cpp router builds can time out while proxying `/slots` to a busy child even though the
/// child itself is healthy. When that exact failure is observed on a loopback router, discover
/// the selected child's ephemeral port from `/models` and retry locally without asking the user
/// to know or configure that port.
pub fn probe_llama_cpp_streaming(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<StreamingInferenceProbeEvidence, StreamingInferenceProbeError> {
    let initial = streaming_inference_probe_legacy::probe_llama_cpp_streaming(endpoint, timeout);
    let original_error = match initial {
        Ok(evidence) => return Ok(evidence),
        Err(error) => error,
    };

    if !is_router_slot_transport_failure(&original_error) {
        return Err(original_error);
    }

    let fallback = discover_loopback_router_child(endpoint, timeout.min(FALLBACK_CONTROL_TIMEOUT));
    let Some((model, child_endpoint)) = fallback.ok().flatten() else {
        return Err(original_error);
    };

    match check_child_slot(&child_endpoint, timeout.min(FALLBACK_CONTROL_TIMEOUT)) {
        Ok(ChildSlotState::Busy) => {
            return Err(StreamingInferenceProbeError::Busy { model });
        }
        Ok(ChildSlotState::Available | ChildSlotState::Unsupported) | Err(_) => {}
    }

    let mut evidence =
        streaming_inference_probe_legacy::probe_llama_cpp_streaming(&child_endpoint, timeout)?;
    if evidence.snapshot.identity.requested_model.is_none() {
        evidence.snapshot.identity.requested_model = Some(model);
    }
    Ok(evidence)
}

fn is_router_slot_transport_failure(error: &StreamingInferenceProbeError) -> bool {
    matches!(
        error,
        StreamingInferenceProbeError::Io { phase, .. } if *phase == "slot availability"
    )
}

fn discover_loopback_router_child(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<Option<(String, ServerEndpoint)>, String> {
    if !endpoint_is_loopback(endpoint)? {
        return Ok(None);
    }

    let response = control_get(endpoint, "/models", timeout)?;
    if !(200..=299).contains(&response.0) {
        return Ok(None);
    }
    let payload: Value = serde_json::from_slice(&response.1).map_err(|error| error.to_string())?;
    let Some(models) = payload.get("data").and_then(Value::as_array) else {
        return Ok(None);
    };

    let mut candidates = Vec::new();
    for model in models {
        let Some(id) = model.get("id").and_then(Value::as_str) else {
            continue;
        };
        let status = model
            .pointer("/status/value")
            .and_then(Value::as_str)
            .unwrap_or_default();
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
        });
    }

    let Some(candidate) = select_router_child(candidates) else {
        return Ok(None);
    };
    let Some(port) = candidate.child_port.filter(|port| *port != endpoint.port) else {
        return Ok(None);
    };

    Ok(Some((
        candidate.id,
        ServerEndpoint {
            host: "127.0.0.1".to_owned(),
            port,
            api_key: endpoint.api_key.clone(),
            allow_non_loopback: false,
        },
    )))
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
            if newest.next().is_some() {
                None
            } else {
                Some(selected)
            }
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
            if let Some(port) = args.get(index + 1).and_then(parse_port_value) {
                return Some(port);
            }
            index += 2;
            continue;
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

fn check_child_slot(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<ChildSlotState, String> {
    let response = control_get(endpoint, "/slots?fail_on_no_slot=1", timeout)?;
    match response.0 {
        200..=299 => Ok(ChildSlotState::Available),
        403..=405 => Ok(ChildSlotState::Unsupported),
        503 => Ok(ChildSlotState::Busy),
        status => Err(format!(
            "autodiscovered child /slots returned HTTP {status}"
        )),
    }
}

fn endpoint_is_loopback(endpoint: &ServerEndpoint) -> Result<bool, String> {
    if endpoint.port == 0 {
        return Ok(false);
    }
    let addresses = resolve(endpoint)?;
    Ok(!addresses.is_empty() && addresses.iter().all(|address| address.ip().is_loopback()))
}

fn resolve(endpoint: &ServerEndpoint) -> Result<Vec<SocketAddr>, String> {
    (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map(|addresses| addresses.collect())
        .map_err(|error| error.to_string())
}

fn connect(endpoint: &ServerEndpoint, timeout: Duration) -> Result<TcpStream, String> {
    let mut last_error = None;
    for address in resolve(endpoint)? {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error
        .map(|error| error.to_string())
        .unwrap_or_else(|| "endpoint resolved to no connectable address".to_owned()))
}

fn control_get(
    endpoint: &ServerEndpoint,
    path: &str,
    timeout: Duration,
) -> Result<(u16, Vec<u8>), String> {
    let mut stream = connect(endpoint, timeout)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| error.to_string())?;

    let mut request = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n",
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
        .map_err(|error| error.to_string())?;

    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut expected_total = None;
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_CONTROL_BYTES {
            return Err(format!(
                "control response exceeded {MAX_CONTROL_BYTES} bytes"
            ));
        }
        if expected_total.is_none()
            && let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n")
            && let Some(content_length) = parse_content_length(&bytes[..header_end])
        {
            expected_total = header_end
                .checked_add(4)
                .and_then(|value| value.checked_add(content_length));
        }
        if expected_total.is_some_and(|expected| bytes.len() >= expected) {
            break;
        }
    }

    let header_end = find_bytes(&bytes, b"\r\n\r\n")
        .ok_or_else(|| "control response ended before HTTP headers completed".to_owned())?;
    let status = parse_status(&bytes[..header_end])?;
    let mut body = bytes[header_end + 4..].to_vec();
    if let Some(content_length) = parse_content_length(&bytes[..header_end]) {
        body.truncate(content_length);
    }
    Ok((status, body))
}

fn parse_status(headers: &[u8]) -> Result<u16, String> {
    let headers = String::from_utf8_lossy(headers);
    headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "control response returned an invalid HTTP status line".to_owned())
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let headers = String::from_utf8_lossy(headers);
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
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

    fn write_json(stream: &mut TcpStream, status: u16, body: &str) {
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    fn write_sse(stream: &mut TcpStream, model: &str) {
        let body = format!(
            "data: {{\"content\":\"O\"}}\n\ndata: {{\"content\":\"\",\"model\":\"{model}\",\"timings\":{{\"prompt_per_second\":120.0,\"predicted_per_second\":40.0,\"prompt_n\":3,\"predicted_n\":2,\"cache_n\":0}},\"generation_settings\":{{\"speculative.types\":\"none\"}}}}\n\n"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    #[test]
    fn child_port_is_read_from_router_args_without_user_configuration() {
        let model: Value = serde_json::from_str(
            r#"{"status":{"args":["llama-server","--host","127.0.0.1","--port","53146"]}}"#,
        )
        .unwrap();
        assert_eq!(child_port_from_model(&model), Some(53146));

        let model: Value = serde_json::from_str(r#"{"port":53147,"status":{"args":[]}}"#).unwrap();
        assert_eq!(child_port_from_model(&model), Some(53147));
    }

    #[test]
    fn unique_most_recent_model_is_selected_for_fallback() {
        let selected = select_router_child(vec![
            RouterChildCandidate {
                id: "old".to_owned(),
                last_used: Some(10),
                child_port: Some(5001),
            },
            RouterChildCandidate {
                id: "new".to_owned(),
                last_used: Some(20),
                child_port: Some(5002),
            },
        ])
        .unwrap();
        assert_eq!(selected.id, "new");
        assert_eq!(selected.child_port, Some(5002));
    }

    #[test]
    fn router_slot_timeout_falls_back_to_autodiscovered_child_port() {
        let child_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let child_port = child_listener.local_addr().unwrap().port();
        let router_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let router_port = router_listener.local_addr().unwrap().port();
        let router_requests = Arc::new(Mutex::new(Vec::new()));
        let child_requests = Arc::new(Mutex::new(Vec::new()));

        let router_requests_worker = Arc::clone(&router_requests);
        let router_thread = thread::spawn(move || {
            for index in 0..3 {
                let (mut stream, _) = router_listener.accept().unwrap();
                let request = read_request(&mut stream);
                router_requests_worker.lock().unwrap().push(request);
                if index == 1 {
                    thread::sleep(Duration::from_millis(150));
                    continue;
                }
                let models = format!(
                    "{{\"object\":\"list\",\"data\":[{{\"id\":\"Qwen-MTP\",\"status\":{{\"value\":\"loaded\",\"last_used\":99,\"args\":[\"llama-server\",\"--host\",\"127.0.0.1\",\"--port\",\"{child_port}\"]}}}}]}}"
                );
                write_json(&mut stream, 200, &models);
            }
        });

        let child_requests_worker = Arc::clone(&child_requests);
        let child_thread = thread::spawn(move || {
            for index in 0..3 {
                let (mut stream, _) = child_listener.accept().unwrap();
                let request = read_request(&mut stream);
                child_requests_worker.lock().unwrap().push(request);
                match index {
                    0 => write_json(&mut stream, 200, r#"[{"id":0,"is_processing":false}]"#),
                    1 => write_json(&mut stream, 404, "{}"),
                    _ => write_sse(&mut stream, "Qwen-MTP"),
                }
            }
        });

        let endpoint = ServerEndpoint::loopback(router_port);
        let evidence = probe_llama_cpp_streaming(&endpoint, Duration::from_millis(100)).unwrap();
        router_thread.join().unwrap();
        child_thread.join().unwrap();

        let router_requests = router_requests.lock().unwrap();
        assert!(router_requests[0].starts_with("GET /models HTTP/1.1"));
        assert!(
            router_requests[1].starts_with("GET /slots?model=Qwen-MTP&fail_on_no_slot=1 HTTP/1.1")
        );
        assert!(router_requests[2].starts_with("GET /models HTTP/1.1"));

        let child_requests = child_requests.lock().unwrap();
        assert!(child_requests[0].starts_with("GET /slots?fail_on_no_slot=1 HTTP/1.1"));
        assert!(child_requests[1].starts_with("GET /models HTTP/1.1"));
        assert!(child_requests[2].starts_with("POST /completion HTTP/1.1"));
        assert_eq!(
            evidence.snapshot.identity.requested_model.as_deref(),
            Some("Qwen-MTP")
        );
        assert_eq!(
            evidence.snapshot.identity.reported_model.as_deref(),
            Some("Qwen-MTP")
        );
    }
}
