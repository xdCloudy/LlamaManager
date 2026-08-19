use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

use crate::server_readiness::ServerEndpoint;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct PassiveInferenceTelemetrySnapshot {
    pub logical_endpoint: String,
    pub source_endpoint: String,
    pub model: Option<String>,
    pub observed_at_unix_ms: u64,
    pub prompt_tps: Option<f64>,
    pub decode_tps: Option<f64>,
    pub prompt_tokens_total: Option<u64>,
    pub predicted_tokens_total: Option<u64>,
    pub requests_processing: Option<f64>,
    pub requests_deferred: Option<f64>,
    pub total_slots: Option<u64>,
    pub busy_slots: Option<u64>,
    pub current_decoded_tokens: Option<u64>,
    pub context_capacity_tokens: Option<u64>,
    pub mtp_explicit: bool,
    pub speculative_draft_tokens_total: Option<u64>,
    pub speculative_accepted_tokens_total: Option<u64>,
    pub mtp_acceptance_rate: Option<f64>,
    pub metrics_error: Option<String>,
    pub slots_error: Option<String>,
}

#[derive(Debug, Clone)]
struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

#[derive(Debug, Clone)]
struct RouterCandidate {
    id: String,
    last_used: Option<u64>,
    child_port: Option<u16>,
    mtp_explicit: bool,
}

#[derive(Debug, Clone)]
struct PollTarget {
    endpoint: ServerEndpoint,
    model: Option<String>,
    router_query: bool,
    mtp_explicit: bool,
}

#[derive(Debug, Default)]
struct SlotSnapshot {
    total_slots: Option<u64>,
    busy_slots: Option<u64>,
    current_decoded_tokens: Option<u64>,
    context_capacity_tokens: Option<u64>,
    mtp_explicit: bool,
}

pub fn poll_passive_inference_telemetry(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<PassiveInferenceTelemetrySnapshot, String> {
    validate_endpoint(endpoint)?;
    let target = discover_poll_target(endpoint, timeout)?;
    let model_query = target
        .model
        .as_deref()
        .map(percent_encode_query)
        .unwrap_or_default();
    let metrics_path = if target.router_query {
        format!("/metrics?model={model_query}")
    } else {
        "/metrics".to_owned()
    };
    let slots_path = if target.router_query {
        format!("/slots?model={model_query}")
    } else {
        "/slots".to_owned()
    };

    let metrics_result = get(&target.endpoint, &metrics_path, timeout)
        .and_then(|response| require_success(response, "metrics"));
    let slots_result = get(&target.endpoint, &slots_path, timeout)
        .and_then(|response| require_success(response, "slots"));

    if metrics_result.is_err() && slots_result.is_err() {
        return Err(format!(
            "passive llama.cpp monitoring failed: metrics: {}; slots: {}",
            metrics_result.as_ref().unwrap_err(),
            slots_result.as_ref().unwrap_err()
        ));
    }

    let metrics_error = metrics_result.as_ref().err().cloned();
    let slots_error = slots_result.as_ref().err().cloned();
    let metrics = metrics_result
        .as_ref()
        .ok()
        .map(|body| parse_prometheus_metrics(body))
        .unwrap_or_default();
    let slots = slots_result
        .as_ref()
        .ok()
        .and_then(|body| parse_slots(body).ok())
        .unwrap_or_default();

    let speculative_draft_tokens_total = metric_u64(
        &metrics,
        "llamacpp:spec_decode_num_draft_tokens_total",
    );
    let speculative_accepted_tokens_total = metric_u64(
        &metrics,
        "llamacpp:spec_decode_num_accepted_tokens_total",
    );
    let mtp_explicit = target.mtp_explicit || slots.mtp_explicit;
    let mtp_acceptance_rate = if mtp_explicit {
        speculative_draft_tokens_total.and_then(|drafted| {
            (drafted > 0).then(|| {
                speculative_accepted_tokens_total.unwrap_or(0) as f64 / drafted as f64
            })
        })
    } else {
        None
    };

    Ok(PassiveInferenceTelemetrySnapshot {
        logical_endpoint: endpoint.authority(),
        source_endpoint: target.endpoint.authority(),
        model: target.model,
        observed_at_unix_ms: now_unix_ms(),
        prompt_tps: metric_f64(&metrics, "llamacpp:prompt_tokens_seconds"),
        decode_tps: metric_f64(&metrics, "llamacpp:predicted_tokens_seconds"),
        prompt_tokens_total: metric_u64(&metrics, "llamacpp:prompt_tokens_total"),
        predicted_tokens_total: metric_u64(&metrics, "llamacpp:tokens_predicted_total"),
        requests_processing: metric_f64(&metrics, "llamacpp:requests_processing"),
        requests_deferred: metric_f64(&metrics, "llamacpp:requests_deferred"),
        total_slots: slots.total_slots,
        busy_slots: slots.busy_slots,
        current_decoded_tokens: slots.current_decoded_tokens,
        context_capacity_tokens: slots.context_capacity_tokens,
        mtp_explicit,
        speculative_draft_tokens_total,
        speculative_accepted_tokens_total,
        mtp_acceptance_rate,
        metrics_error,
        slots_error,
    })
}

fn discover_poll_target(endpoint: &ServerEndpoint, timeout: Duration) -> Result<PollTarget, String> {
    let response = get(endpoint, "/models", timeout)?;
    if matches!(response.status, 404 | 405) {
        return Ok(direct_target(endpoint));
    }
    if !(200..=299).contains(&response.status) {
        return Ok(direct_target(endpoint));
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&response.body) else {
        return Ok(direct_target(endpoint));
    };
    let Some(models) = payload.get("data").and_then(Value::as_array) else {
        return Ok(direct_target(endpoint));
    };

    let mut router_shape = false;
    let mut candidates = Vec::new();
    for model in models {
        let Some(id) = model.get("id").and_then(Value::as_str) else {
            continue;
        };
        let status = model
            .pointer("/status/value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !status.is_empty() {
            router_shape = true;
        }
        if !matches!(status, "loaded" | "sleeping") {
            continue;
        }
        candidates.push(RouterCandidate {
            id: id.to_owned(),
            last_used: model
                .pointer("/status/last_used")
                .and_then(Value::as_u64)
                .or_else(|| model.get("last_used").and_then(Value::as_u64))
                .or_else(|| model.get("last_used_ms").and_then(Value::as_u64)),
            child_port: child_port_from_model(model),
            mtp_explicit: model_explicit_mtp(model),
        });
    }

    if !router_shape {
        return Ok(direct_target(endpoint));
    }
    let selected = select_router_candidate(candidates)
        .ok_or_else(|| "router has no uniquely selectable loaded model for passive polling".to_owned())?;

    if endpoint_is_loopback(endpoint)?
        && let Some(port) = selected.child_port.filter(|port| *port != endpoint.port)
    {
        return Ok(PollTarget {
            endpoint: ServerEndpoint {
                host: "127.0.0.1".to_owned(),
                port,
                api_key: endpoint.api_key.clone(),
                allow_non_loopback: false,
            },
            model: Some(selected.id),
            router_query: false,
            mtp_explicit: selected.mtp_explicit,
        });
    }

    Ok(PollTarget {
        endpoint: endpoint.clone(),
        model: Some(selected.id),
        router_query: true,
        mtp_explicit: selected.mtp_explicit,
    })
}

fn direct_target(endpoint: &ServerEndpoint) -> PollTarget {
    PollTarget {
        endpoint: endpoint.clone(),
        model: None,
        router_query: false,
        mtp_explicit: false,
    }
}

fn select_router_candidate(mut candidates: Vec<RouterCandidate>) -> Option<RouterCandidate> {
    match candidates.len() {
        0 => None,
        1 => candidates.pop(),
        _ => {
            let newest = candidates.iter().filter_map(|item| item.last_used).max()?;
            let mut matching = candidates
                .into_iter()
                .filter(|item| item.last_used == Some(newest));
            let selected = matching.next()?;
            matching.next().is_none().then_some(selected)
        }
    }
}

fn model_explicit_mtp(model: &Value) -> bool {
    model
        .pointer("/status/args")
        .and_then(Value::as_array)
        .is_some_and(|args| args_explicit_mtp(args))
}

fn args_explicit_mtp(args: &[Value]) -> bool {
    let mut index = 0;
    while index < args.len() {
        let Some(arg) = args[index].as_str() else {
            index += 1;
            continue;
        };
        if arg == "--spec-type" {
            if args
                .get(index + 1)
                .and_then(Value::as_str)
                .is_some_and(|value| value.to_ascii_lowercase().contains("mtp"))
            {
                return true;
            }
            index += 2;
            continue;
        }
        if arg
            .strip_prefix("--spec-type=")
            .is_some_and(|value| value.to_ascii_lowercase().contains("mtp"))
        {
            return true;
        }
        index += 1;
    }
    false
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

fn child_port_from_args(args: &[Value]) -> Option<u16> {
    let mut index = 0;
    while index < args.len() {
        let Some(arg) = args[index].as_str() else {
            index += 1;
            continue;
        };
        if matches!(arg, "--port" | "-p") {
            return args.get(index + 1).and_then(parse_port_value);
        }
        if let Some(port) = arg
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

fn parse_prometheus_metrics(body: &[u8]) -> std::collections::HashMap<String, f64> {
    let text = String::from_utf8_lossy(body);
    let mut metrics = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else {
            continue;
        };
        if !name.starts_with("llamacpp:") || name.contains('{') {
            continue;
        }
        let Some(value) = parts.next().and_then(|value| value.parse::<f64>().ok()) else {
            continue;
        };
        if value.is_finite() && value >= 0.0 {
            metrics.insert(name.to_owned(), value);
        }
    }
    metrics
}

fn metric_f64(metrics: &std::collections::HashMap<String, f64>, name: &str) -> Option<f64> {
    metrics.get(name).copied()
}

fn metric_u64(metrics: &std::collections::HashMap<String, f64>, name: &str) -> Option<u64> {
    metric_f64(metrics, name)
        .filter(|value| *value <= u64::MAX as f64)
        .map(|value| value.round() as u64)
}

fn parse_slots(body: &[u8]) -> Result<SlotSnapshot, String> {
    let payload: Value = serde_json::from_slice(body).map_err(|error| error.to_string())?;
    let slots = payload
        .as_array()
        .ok_or_else(|| "slots response root is not an array".to_owned())?;
    if slots.is_empty() {
        return Ok(SlotSnapshot {
            total_slots: Some(0),
            busy_slots: Some(0),
            ..SlotSnapshot::default()
        });
    }

    let busy: Vec<_> = slots
        .iter()
        .filter(|slot| {
            slot.get("is_processing")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .collect();
    let representative = busy.first().copied().or_else(|| slots.first());
    let current_decoded_tokens = if busy.is_empty() {
        representative.and_then(slot_decoded_tokens)
    } else {
        Some(
            busy.iter()
                .filter_map(|slot| slot_decoded_tokens(slot))
                .sum(),
        )
    };
    let mtp_explicit = slots.iter().any(slot_explicit_mtp);

    Ok(SlotSnapshot {
        total_slots: Some(slots.len() as u64),
        busy_slots: Some(busy.len() as u64),
        current_decoded_tokens,
        context_capacity_tokens: representative
            .and_then(|slot| slot.get("n_ctx"))
            .and_then(Value::as_u64),
        mtp_explicit,
    })
}

fn slot_decoded_tokens(slot: &Value) -> Option<u64> {
    slot.pointer("/next_token/n_decoded")
        .and_then(Value::as_u64)
        .or_else(|| slot.get("n_decoded").and_then(Value::as_u64))
}

fn slot_explicit_mtp(slot: &Value) -> bool {
    let Some(value) = slot.pointer("/params/speculative.types") else {
        return false;
    };
    if let Some(mode) = value.as_str() {
        return mode.to_ascii_lowercase().contains("mtp");
    }
    value.as_array().is_some_and(|modes| {
        modes.iter().any(|mode| {
            mode.as_str()
                .is_some_and(|mode| mode.to_ascii_lowercase().contains("mtp"))
        })
    })
}

fn require_success(response: HttpResponse, endpoint: &str) -> Result<Vec<u8>, String> {
    if (200..=299).contains(&response.status) {
        Ok(response.body)
    } else {
        Err(format!("{endpoint} returned HTTP {}", response.status))
    }
}

fn get(endpoint: &ServerEndpoint, path: &str, timeout: Duration) -> Result<HttpResponse, String> {
    let mut stream = connect(endpoint, timeout)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| error.to_string())?;

    let mut request = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nAccept: */*\r\nConnection: close\r\n",
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
        let read = stream.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(format!("response exceeded {MAX_RESPONSE_BYTES} bytes"));
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
        .ok_or_else(|| "response ended before HTTP headers completed".to_owned())?;
    let status = parse_status(&bytes[..header_end])?;
    let mut body = bytes[header_end + 4..].to_vec();
    if let Some(content_length) = parse_content_length(&bytes[..header_end]) {
        body.truncate(content_length);
    }
    Ok(HttpResponse { status, body })
}

fn validate_endpoint(endpoint: &ServerEndpoint) -> Result<(), String> {
    if endpoint.port == 0 {
        return Err("server port must be in 1..=65535".to_owned());
    }
    if endpoint
        .api_key
        .as_deref()
        .is_some_and(|key| key.contains('\r') || key.contains('\n'))
    {
        return Err("API key cannot contain CR/LF characters".to_owned());
    }
    let addresses = resolve(endpoint)?;
    if addresses.iter().any(|address| !address.ip().is_loopback()) && !endpoint.allow_non_loopback {
        return Err(format!(
            "non-loopback target {} requires explicit opt-in",
            endpoint.host
        ));
    }
    Ok(())
}

fn endpoint_is_loopback(endpoint: &ServerEndpoint) -> Result<bool, String> {
    let addresses = resolve(endpoint)?;
    Ok(!addresses.is_empty() && addresses.iter().all(|address| address.ip().is_loopback()))
}

fn resolve(endpoint: &ServerEndpoint) -> Result<Vec<SocketAddr>, String> {
    let addresses: Vec<_> = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| error.to_string())?
        .collect();
    if addresses.is_empty() {
        return Err("endpoint resolved to no addresses".to_owned());
    }
    Ok(addresses)
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

fn parse_status(headers: &[u8]) -> Result<u16, String> {
    String::from_utf8_lossy(headers)
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "response returned an invalid HTTP status line".to_owned())
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    String::from_utf8_lossy(headers).lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
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
    fn prometheus_parser_extracts_busy_safe_rates_and_mtp_counters() {
        let metrics = parse_prometheus_metrics(
            b"# TYPE llamacpp:prompt_tokens_seconds gauge\nllamacpp:prompt_tokens_seconds 41.78\nllamacpp:predicted_tokens_seconds 4.54\nllamacpp:requests_processing 1\nllamacpp:spec_decode_num_draft_tokens_total 79\nllamacpp:spec_decode_num_accepted_tokens_total 75\n",
        );
        assert_eq!(metric_f64(&metrics, "llamacpp:prompt_tokens_seconds"), Some(41.78));
        assert_eq!(metric_f64(&metrics, "llamacpp:requests_processing"), Some(1.0));
        assert_eq!(
            metric_u64(&metrics, "llamacpp:spec_decode_num_accepted_tokens_total"),
            Some(75)
        );
    }

    #[test]
    fn slots_parser_observes_processing_without_reserving_a_slot() {
        let slots = parse_slots(
            br#"[{"id":0,"n_ctx":98304,"is_processing":true,"params":{"speculative.types":"draft-mtp"},"next_token":{"n_decoded":171,"n_remain":-1}}]"#,
        )
        .unwrap();
        assert_eq!(slots.total_slots, Some(1));
        assert_eq!(slots.busy_slots, Some(1));
        assert_eq!(slots.current_decoded_tokens, Some(171));
        assert_eq!(slots.context_capacity_tokens, Some(98304));
        assert!(slots.mtp_explicit);
    }

    #[test]
    fn child_port_and_mtp_are_read_from_router_metadata() {
        let model: Value = serde_json::from_str(
            r#"{"status":{"args":["llama-server","--port","65421","--spec-type","draft-mtp"]}}"#,
        )
        .unwrap();
        assert_eq!(child_port_from_model(&model), Some(65421));
        assert!(model_explicit_mtp(&model));
    }

    #[test]
    fn passive_poll_succeeds_while_the_only_slot_is_busy_without_completion_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_requests = Arc::clone(&requests);
        thread::spawn(move || {
            for index in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                server_requests.lock().unwrap().push(request.clone());
                match index {
                    0 => write_response(&mut stream, 404, "application/json", "{}"),
                    1 => write_response(
                        &mut stream,
                        200,
                        "text/plain",
                        "llamacpp:prompt_tokens_seconds 41.78\nllamacpp:predicted_tokens_seconds 4.54\nllamacpp:requests_processing 1\nllamacpp:requests_deferred 0\nllamacpp:prompt_tokens_total 29907\nllamacpp:tokens_predicted_total 94\nllamacpp:spec_decode_num_draft_tokens_total 79\nllamacpp:spec_decode_num_accepted_tokens_total 75\n",
                    ),
                    _ => write_response(
                        &mut stream,
                        200,
                        "application/json",
                        r#"[{"id":0,"n_ctx":98304,"is_processing":true,"params":{"speculative.types":"draft-mtp"},"next_token":{"n_decoded":94,"n_remain":-1}}]"#,
                    ),
                }
            }
        });

        let snapshot = poll_passive_inference_telemetry(
            &ServerEndpoint::loopback(port),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(snapshot.busy_slots, Some(1));
        assert_eq!(snapshot.prompt_tps, Some(41.78));
        assert_eq!(snapshot.decode_tps, Some(4.54));
        assert!(snapshot.mtp_explicit);
        assert_eq!(snapshot.mtp_acceptance_rate, Some(75.0 / 79.0));

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|request| request.starts_with("GET ")));
        assert!(requests.iter().all(|request| !request.contains("/completion")));
    }
}
