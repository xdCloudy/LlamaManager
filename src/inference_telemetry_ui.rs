use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use dioxus::prelude::*;

use crate::{
    hardware_telemetry::TelemetryState,
    inference_telemetry::{InferenceMetric, InferenceTelemetrySnapshot},
    server_readiness::ServerEndpoint,
    streaming_inference_probe::{
        StreamingInferenceProbeError, check_endpoint_reachable, probe_llama_cpp_streaming,
    },
};

const REACHABILITY_CADENCE: Duration = Duration::from_secs(2);
const REACHABILITY_TIMEOUT: Duration = Duration::from_millis(250);
const INFERENCE_PROBE_TIMEOUT: Duration = Duration::from_secs(20);

const INFERENCE_UI_CSS: &str = r#"
.tm-inference-body{padding:12px}.tm-probe-fields{display:grid;grid-template-columns:minmax(0,1fr) 110px minmax(180px,.8fr);gap:8px}.tm-probe-field{min-width:0}.tm-probe-field label{display:block;margin-bottom:5px;color:#927da0;font-size:7px;letter-spacing:.07em;text-transform:uppercase}.tm-probe-input{width:100%;min-height:32px;padding:6px 8px;border:1px solid rgba(0,255,255,.30);border-radius:0;background:#030008;color:#f6eaff;font:inherit;font-size:9px}.tm-probe-input:focus-visible,.tm-probe-button:focus-visible{outline:2px solid #ff00ff;outline-offset:2px}.tm-probe-actions{display:flex;align-items:center;flex-wrap:wrap;gap:8px;margin-top:9px}.tm-probe-button{min-height:32px;padding:0 10px;border:1px solid #00dbe7;border-radius:0;background:transparent;color:#00f5ff;font:inherit;font-size:8px;font-weight:900;letter-spacing:.07em;text-transform:uppercase;cursor:pointer}.tm-probe-button:hover:not(:disabled),.tm-probe-button.primary{background:#00ffff;color:#050009}.tm-probe-button.magenta{border-color:#ff00d4;color:#ff55e7}.tm-probe-button:disabled{opacity:.34;cursor:not-allowed}.tm-continuity{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-top:10px;padding:8px 9px;border-left:2px solid #00ffff;background:rgba(0,255,255,.035);font-size:8px;line-height:1.5}.tm-continuity.stale{border-left-color:#ffd36b}.tm-continuity.error{border-left-color:#ff3d7f}.tm-probe-notice{margin-top:9px;padding:8px 9px;border:1px solid rgba(117,255,226,.42);color:#baffed;background:rgba(0,20,18,.45);font-size:8px;line-height:1.5;overflow-wrap:anywhere}.tm-probe-notice.error{border-color:rgba(255,61,127,.48);color:#ff91b5;background:rgba(40,0,18,.48)}.tm-inference-identity{margin-top:10px;padding:8px 9px;border:1px solid rgba(255,0,212,.24);background:rgba(20,0,28,.34);color:#a995b8;font-size:8px;line-height:1.55;overflow-wrap:anywhere;word-break:break-word}.tm-probe-help{margin-top:8px;color:#887595;font-size:7px;line-height:1.5}.tm-inference .tm-metrics{padding:10px 0 0;grid-template-columns:repeat(3,minmax(0,1fr))}@media(max-width:980px){.tm-probe-fields{grid-template-columns:1fr 110px}.tm-probe-field.key{grid-column:1/-1}.tm-inference .tm-metrics{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:650px){.tm-probe-fields{grid-template-columns:1fr}.tm-probe-field.key{grid-column:auto}.tm-inference .tm-metrics{grid-template-columns:1fr}}
"#;

#[derive(Debug, Clone, Default)]
struct InferenceUiState {
    snapshot: Option<InferenceTelemetrySnapshot>,
    endpoint: Option<ServerEndpoint>,
    reachable: Option<bool>,
    stale: bool,
    running: bool,
    notice: Option<(bool, String)>,
    event_count: Option<usize>,
    generation: u64,
}

type InferenceStateSignal = Signal<InferenceUiState, SyncStorage>;

#[derive(Clone)]
struct InferenceMonitorWorker {
    _inner: Arc<InferenceMonitorWorkerInner>,
}

struct InferenceMonitorWorkerInner {
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl InferenceMonitorWorker {
    fn spawn(mut state: InferenceStateSignal) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                let (endpoint, generation) = {
                    let current = state.read();
                    (current.endpoint.clone(), current.generation)
                };

                if let Some(endpoint) = endpoint {
                    let reachable =
                        check_endpoint_reachable(&endpoint, REACHABILITY_TIMEOUT).is_ok();
                    let mut current = state.write();
                    if current.generation == generation
                        && current.endpoint.as_ref() == Some(&endpoint)
                    {
                        apply_reachability(&mut current, reachable);
                    }
                }

                thread::park_timeout(REACHABILITY_CADENCE);
            }
        });

        Self {
            _inner: Arc::new(InferenceMonitorWorkerInner {
                stop,
                handle: Mutex::new(Some(handle)),
            }),
        }
    }
}

impl Drop for InferenceMonitorWorkerInner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(handle) = self.handle.get_mut()
            && let Some(handle) = handle.take()
        {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

fn apply_reachability(state: &mut InferenceUiState, reachable: bool) {
    state.reachable = Some(reachable);
    if !reachable && state.snapshot.is_some() {
        state.stale = true;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MetricPresentation {
    value: String,
    state_label: &'static str,
    state_class: &'static str,
    detail: String,
}

fn present_state<T>(
    state: &TelemetryState<T>,
    format_live: impl FnOnce(&T) -> String,
) -> MetricPresentation {
    match state {
        TelemetryState::Live { value } => MetricPresentation {
            value: format_live(value),
            state_label: "LIVE",
            state_class: "",
            detail: "Current request evidence".to_owned(),
        },
        TelemetryState::Unavailable { reason } => MetricPresentation {
            value: "Unavailable".to_owned(),
            state_label: "UNAVAILABLE",
            state_class: "unavailable",
            detail: reason.clone(),
        },
        TelemetryState::Error { message } => MetricPresentation {
            value: "Error".to_owned(),
            state_label: "ERROR",
            state_class: "error",
            detail: message.clone(),
        },
        TelemetryState::Stale {
            last_value,
            last_observed_at_unix_ms,
            reason,
        } => MetricPresentation {
            value: last_value
                .as_ref()
                .map(format_live)
                .unwrap_or_else(|| "No prior value".to_owned()),
            state_label: "STALE",
            state_class: "stale",
            detail: match last_observed_at_unix_ms {
                Some(timestamp) => format!("Last observed {timestamp} ms UNIX — {reason}"),
                None => reason.clone(),
            },
        },
    }
}

fn present_metric<T>(
    metric: &InferenceMetric<T>,
    continuity_stale: bool,
    format_live: impl FnOnce(&T) -> String,
) -> MetricPresentation {
    let was_live = matches!(&metric.state, TelemetryState::Live { .. });
    let mut presentation = present_state(&metric.state, format_live);
    if continuity_stale && was_live {
        presentation.state_label = "STALE";
        presentation.state_class = "stale";
        presentation.detail = format!(
            "Last request evidence observed {} ms UNIX; endpoint continuity was interrupted. Run a new streaming probe to re-establish live request evidence.",
            metric.observed_at_unix_ms
        );
    }
    presentation
}

fn metric_source<T>(metric: &InferenceMetric<T>) -> String {
    format!("{} · {}", metric.source.provider, metric.source.field)
}

fn metric_card(label: &'static str, metric: MetricPresentation, source: String) -> Element {
    rsx! {
        div { class: "tm-metric",
            div { class: "tm-metric-label", "{label}" }
            div { class: "tm-metric-value", "{metric.value}" }
            div { class: "tm-metric-meta", "{metric.detail}" }
            div { class: "tm-metric-meta", "SOURCE: {source}" }
            span { class: "tm-state {metric.state_class}", "{metric.state_label}" }
        }
    }
}

fn build_endpoint(
    host: &str,
    port: &str,
    api_key: &str,
    allow_non_loopback: bool,
) -> Result<ServerEndpoint, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("Host cannot be empty.".to_owned());
    }
    let port = port
        .trim()
        .parse::<u16>()
        .map_err(|_| "Port must be an integer in 1..=65535.".to_owned())?;
    if port == 0 {
        return Err("Port must be in 1..=65535.".to_owned());
    }
    let api_key = api_key.trim();
    if api_key.contains('\r') || api_key.contains('\n') {
        return Err("API key cannot contain CR/LF characters.".to_owned());
    }

    Ok(ServerEndpoint {
        host: host.to_owned(),
        port,
        api_key: (!api_key.is_empty()).then(|| api_key.to_owned()),
        allow_non_loopback,
    })
}

fn endpoint_without_secret(endpoint: &ServerEndpoint) -> ServerEndpoint {
    ServerEndpoint {
        host: endpoint.host.clone(),
        port: endpoint.port,
        api_key: None,
        allow_non_loopback: endpoint.allow_non_loopback,
    }
}

fn failed_probe_reachability(error: &StreamingInferenceProbeError) -> Option<bool> {
    match error {
        StreamingInferenceProbeError::Connect { .. }
        | StreamingInferenceProbeError::HostResolution { .. } => Some(false),
        StreamingInferenceProbeError::InvalidPort
        | StreamingInferenceProbeError::InvalidApiKey
        | StreamingInferenceProbeError::NonLoopbackDenied { .. } => None,
        StreamingInferenceProbeError::Io { .. }
        | StreamingInferenceProbeError::ResponseTooLarge { .. }
        | StreamingInferenceProbeError::InvalidStatusLine
        | StreamingInferenceProbeError::HttpRejected { .. }
        | StreamingInferenceProbeError::MissingHeaders
        | StreamingInferenceProbeError::NoLoadedRouterModel
        | StreamingInferenceProbeError::AmbiguousRouterModels { .. }
        | StreamingInferenceProbeError::Busy { .. }
        | StreamingInferenceProbeError::MissingFirstToken
        | StreamingInferenceProbeError::MissingTimings
        | StreamingInferenceProbeError::TelemetryParse(_) => Some(true),
    }
}

fn run_probe(mut state: InferenceStateSignal, endpoint: ServerEndpoint) {
    let monitored_endpoint = endpoint_without_secret(&endpoint);
    {
        let mut current = state.write();
        if current.running {
            return;
        }
        if current.endpoint.as_ref() != Some(&monitored_endpoint) && current.snapshot.is_some() {
            current.stale = true;
        }
        current.running = true;
        current.generation = current.generation.saturating_add(1);
        current.endpoint = Some(monitored_endpoint.clone());
        current.reachable = None;
        current.notice = Some((
            true,
            format!(
                "Streaming request probe in progress against {}.",
                monitored_endpoint.authority()
            ),
        ));
    }

    let result = probe_llama_cpp_streaming(&endpoint, INFERENCE_PROBE_TIMEOUT);
    let mut current = state.write();
    if current.endpoint.as_ref() != Some(&monitored_endpoint) {
        current.running = false;
        return;
    }
    current.generation = current.generation.saturating_add(1);
    current.running = false;

    match result {
        Ok(evidence) => {
            current.reachable = Some(true);
            current.stale = false;
            current.event_count = Some(evidence.event_count);
            current.notice = Some((
                true,
                format!(
                    "Request-bound inference evidence captured: HTTP {} · {} SSE events · TTFT {:.2} ms · latency {:.2} ms.",
                    evidence.status_code,
                    evidence.event_count,
                    evidence.ttft_ms,
                    evidence.request_latency_ms
                ),
            ));
            current.snapshot = Some(evidence.snapshot);
        }
        Err(error) => {
            current.reachable = failed_probe_reachability(&error);
            if current.snapshot.is_some() {
                current.stale = true;
            }
            current.notice = Some((false, format!("Streaming inference probe failed: {error}")));
        }
    }
}

fn continuity_presentation(state: &InferenceUiState) -> (&'static str, &'static str, String) {
    let endpoint = state
        .endpoint
        .as_ref()
        .map(ServerEndpoint::authority)
        .unwrap_or_else(|| "no endpoint".to_owned());
    match (state.reachable, state.stale, state.snapshot.is_some()) {
        (None, _, _) if state.endpoint.is_none() => (
            "NOT MONITORED",
            "",
            "Run a streaming probe to attach request-bound inference evidence.".to_owned(),
        ),
        (None, _, _) => (
            "CHECKING",
            "",
            format!("Checking endpoint continuity for {endpoint}."),
        ),
        (Some(false), _, _) => (
            "DISCONNECTED",
            "error",
            format!(
                "{endpoint} is not accepting connections; prior live request evidence is stale."
            ),
        ),
        (Some(true), true, true) => (
            "RECONNECTED · EVIDENCE STALE",
            "stale",
            format!(
                "{endpoint} is reachable again, but old request metrics remain stale until a new streaming probe succeeds."
            ),
        ),
        (Some(true), false, true) => (
            "REACHABLE · REQUEST LIVE",
            "",
            format!(
                "{endpoint} is reachable and the latest request evidence has uninterrupted continuity."
            ),
        ),
        (Some(true), _, false) => (
            "REACHABLE · NO REQUEST EVIDENCE",
            "",
            format!(
                "{endpoint} is reachable, but no successful telemetry request has been captured."
            ),
        ),
    }
}

#[allow(non_snake_case)]
pub fn InferenceTelemetryPanel() -> Element {
    let mut state = use_signal_sync(InferenceUiState::default);
    let _monitor = use_hook(move || InferenceMonitorWorker::spawn(state));
    let mut host = use_signal(|| "127.0.0.1".to_owned());
    let mut port = use_signal(|| "8080".to_owned());
    let mut api_key = use_signal(String::new);
    let mut allow_non_loopback = use_signal(|| false);

    let snapshot = state.read().clone();
    let host_value = host();
    let port_value = port();
    let api_key_value = api_key();
    let allow_non_loopback_value = allow_non_loopback();
    let endpoint_validation = build_endpoint(
        &host_value,
        &port_value,
        &api_key_value,
        allow_non_loopback_value,
    );
    let can_probe = !snapshot.running && endpoint_validation.is_ok();
    let (continuity_label, continuity_class, continuity_detail) =
        continuity_presentation(&snapshot);

    rsx! {
        style { dangerous_inner_html: INFERENCE_UI_CSS }
        section { class: "tm-panel wide tm-inference",
            div { class: "tm-panel-head",
                h2 { "INFERENCE REQUEST EVIDENCE" }
                span { class: "tm-source", "llama.cpp /completion · client TTFT" }
            }
            div { class: "tm-inference-body",
                div { class: "tm-probe-fields",
                    div { class: "tm-probe-field",
                        label { "HOST" }
                        input {
                            class: "tm-probe-input",
                            value: "{host_value}",
                            disabled: snapshot.running,
                            oninput: move |event| host.set(event.value()),
                        }
                    }
                    div { class: "tm-probe-field",
                        label { "PORT" }
                        input {
                            class: "tm-probe-input",
                            value: "{port_value}",
                            disabled: snapshot.running,
                            oninput: move |event| port.set(event.value()),
                        }
                    }
                    div { class: "tm-probe-field key",
                        label { "API KEY · OPTIONAL · MEMORY ONLY" }
                        input {
                            class: "tm-probe-input",
                            r#type: "password",
                            value: "{api_key_value}",
                            disabled: snapshot.running,
                            oninput: move |event| api_key.set(event.value()),
                        }
                    }
                }

                div { class: "tm-probe-actions",
                    button {
                        class: if allow_non_loopback_value { "tm-probe-button magenta" } else { "tm-probe-button" },
                        disabled: snapshot.running,
                        onclick: move |_| {
                            let enabled = allow_non_loopback();
                            allow_non_loopback.set(!enabled);
                        },
                        if allow_non_loopback_value { "LAN OPT-IN ON" } else { "LAN OPT-IN OFF" }
                    }
                    button {
                        class: "tm-probe-button primary",
                        disabled: !can_probe,
                        onclick: move |_| {
                            match build_endpoint(&host(), &port(), &api_key(), allow_non_loopback()) {
                                Ok(endpoint) => {
                                    let worker_state = state;
                                    thread::spawn(move || run_probe(worker_state, endpoint));
                                }
                                Err(error) => state.write().notice = Some((false, error)),
                            }
                        },
                        if snapshot.running { "PROBING…" } else { "RUN STREAMING PROBE" }
                    }
                    if let Err(error) = endpoint_validation.as_ref() {
                        span { class: "tm-source", "BLOCKED: {error}" }
                    }
                }

                div { class: "tm-probe-help",
                    "This is an explicit 4-token inference request, not passive polling. Endpoint monitoring after the probe is TCP-only; a reconnect never upgrades old request metrics back to live without a new successful inference request."
                }

                div { class: "tm-continuity {continuity_class}",
                    span { class: "tm-state {continuity_class}", "{continuity_label}" }
                    span { "{continuity_detail}" }
                }

                if let Some((success, message)) = snapshot.notice.as_ref() {
                    div { class: if *success { "tm-probe-notice" } else { "tm-probe-notice error" }, "{message}" }
                }

                if let Some(inference) = snapshot.snapshot.as_ref() {
                    div { class: "tm-inference-identity",
                        strong { "REQUEST " }
                        "{inference.identity.request_id} · endpoint {inference.identity.endpoint}"
                        if let Some(model) = inference.identity.reported_model.as_ref() {
                            " · model {model}"
                        }
                        if let Some(events) = snapshot.event_count {
                            " · {events} SSE events"
                        }
                    }
                    div { class: "tm-metrics",
                        {metric_card(
                            "PROMPT RATE",
                            present_metric(&inference.prompt_tps, snapshot.stale, |value| format!("{value:.2} tok/s")),
                            metric_source(&inference.prompt_tps),
                        )}
                        {metric_card(
                            "DECODE RATE",
                            present_metric(&inference.decode_tps, snapshot.stale, |value| format!("{value:.2} tok/s")),
                            metric_source(&inference.decode_tps),
                        )}
                        {metric_card(
                            "TTFT",
                            present_metric(&inference.ttft_ms, snapshot.stale, |value| format!("{value:.2} ms")),
                            metric_source(&inference.ttft_ms),
                        )}
                        {metric_card(
                            "REQUEST LATENCY",
                            present_metric(&inference.request_latency_ms, snapshot.stale, |value| format!("{value:.2} ms")),
                            metric_source(&inference.request_latency_ms),
                        )}
                        {metric_card(
                            "CONTEXT USED",
                            present_metric(&inference.context_tokens, snapshot.stale, |value| format!("{value} tok")),
                            metric_source(&inference.context_tokens),
                        )}
                        {metric_card(
                            "MTP GENERATED",
                            present_metric(&inference.mtp_generated_tokens, snapshot.stale, |value| format!("{value} tok")),
                            metric_source(&inference.mtp_generated_tokens),
                        )}
                        {metric_card(
                            "MTP ACCEPTED",
                            present_metric(&inference.mtp_accepted_tokens, snapshot.stale, |value| format!("{value} tok")),
                            metric_source(&inference.mtp_accepted_tokens),
                        )}
                        {metric_card(
                            "MTP ACCEPTANCE",
                            present_metric(&inference.mtp_acceptance_rate, snapshot.stale, |value| format!("{:.1}%", value * 100.0)),
                            metric_source(&inference.mtp_acceptance_rate),
                        )}
                        {metric_card(
                            "MTP MEAN RUN",
                            present_metric(&inference.mtp_mean_run_length, snapshot.stale, |value| format!("{value:.2} tok")),
                            metric_source(&inference.mtp_mean_run_length),
                        )}
                    }
                } else {
                    div { class: "tm-empty",
                        "No request-bound inference evidence yet. Run the streaming probe. Prompt/decode/TTFT/MTP values remain unavailable until a real request supplies them; nothing is zero-filled."
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::inference_telemetry::{InferenceMetricSource, InferenceMetricUnit};

    use super::*;

    fn metric(state: TelemetryState<u64>) -> InferenceMetric<u64> {
        InferenceMetric {
            state,
            unit: InferenceMetricUnit::Tokens,
            source: InferenceMetricSource {
                provider: "llama.cpp-mtp".to_owned(),
                field: "timings.draft_n".to_owned(),
            },
            observed_at_unix_ms: 1234,
        }
    }

    #[test]
    fn continuity_break_marks_live_request_metric_stale() {
        let metric = metric(TelemetryState::Live { value: 12 });
        let presentation = present_metric(&metric, true, |value| value.to_string());
        assert_eq!(presentation.value, "12");
        assert_eq!(presentation.state_label, "STALE");
        assert!(presentation.detail.contains("continuity was interrupted"));
    }

    #[test]
    fn continuity_break_does_not_turn_unsupported_mtp_into_stale() {
        let metric = metric(TelemetryState::Unavailable {
            reason: "runtime explicitly reported non-MTP mode".to_owned(),
        });
        let presentation = present_metric(&metric, true, |value| value.to_string());
        assert_eq!(presentation.value, "Unavailable");
        assert_eq!(presentation.state_label, "UNAVAILABLE");
    }

    #[test]
    fn monitored_endpoint_never_retains_api_key() {
        let endpoint = build_endpoint("127.0.0.1", "8080", "top-secret", false).unwrap();
        let monitored = endpoint_without_secret(&endpoint);
        assert_eq!(endpoint.api_key.as_deref(), Some("top-secret"));
        assert!(monitored.api_key.is_none());
        assert_eq!(monitored.authority(), "127.0.0.1:8080");
    }

    #[test]
    fn failed_probe_reachability_distinguishes_transport_from_http_failure() {
        assert_eq!(
            failed_probe_reachability(&StreamingInferenceProbeError::Connect {
                endpoint: "127.0.0.1:8080".to_owned(),
                message: "refused".to_owned(),
            }),
            Some(false)
        );
        assert_eq!(
            failed_probe_reachability(&StreamingInferenceProbeError::HttpRejected {
                status_code: 401,
            }),
            Some(true)
        );
        assert_eq!(
            failed_probe_reachability(&StreamingInferenceProbeError::Busy {
                model: "Qwen3.8-27B".to_owned(),
            }),
            Some(true)
        );
    }

    #[test]
    fn reconnect_cannot_clear_stale_without_a_new_probe() {
        let mut state = InferenceUiState {
            reachable: Some(false),
            stale: true,
            ..InferenceUiState::default()
        };
        apply_reachability(&mut state, true);
        assert_eq!(state.reachable, Some(true));
        assert!(state.stale);
    }

    #[test]
    fn endpoint_validation_rejects_invalid_port_and_header_injection() {
        assert!(build_endpoint("127.0.0.1", "0", "", false).is_err());
        assert!(build_endpoint("127.0.0.1", "8080", "key\r\nInjected: yes", false).is_err());
    }
}
