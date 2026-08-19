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
    passive_inference_metrics::{
        PassiveInferenceMetricsSnapshot, poll_passive_inference_metrics,
    },
    server_readiness::ServerEndpoint,
    streaming_inference_probe::{
        StreamingInferenceProbeError, check_endpoint_reachable, probe_llama_cpp_streaming,
    },
};

const PASSIVE_CADENCE: Duration = Duration::from_secs(1);
const PASSIVE_TIMEOUT: Duration = Duration::from_millis(750);
const REACHABILITY_TIMEOUT: Duration = Duration::from_millis(250);
const INFERENCE_PROBE_TIMEOUT: Duration = Duration::from_secs(20);

const INFERENCE_UI_CSS: &str = r#"
.tm-inference-body{padding:12px}.tm-probe-fields{display:grid;grid-template-columns:minmax(0,1fr) 110px minmax(180px,.8fr);gap:8px}.tm-probe-field{min-width:0}.tm-probe-field label{display:block;margin-bottom:5px;color:#927da0;font-size:7px;letter-spacing:.07em;text-transform:uppercase}.tm-probe-input{width:100%;min-height:32px;padding:6px 8px;border:1px solid rgba(0,255,255,.30);border-radius:0;background:#030008;color:#f6eaff;font:inherit;font-size:9px}.tm-probe-input:focus-visible,.tm-probe-button:focus-visible{outline:2px solid #ff00ff;outline-offset:2px}.tm-probe-actions{display:flex;align-items:center;flex-wrap:wrap;gap:8px;margin-top:9px}.tm-probe-button{min-height:32px;padding:0 10px;border:1px solid #00dbe7;border-radius:0;background:transparent;color:#00f5ff;font:inherit;font-size:8px;font-weight:900;letter-spacing:.07em;text-transform:uppercase;cursor:pointer}.tm-probe-button:hover:not(:disabled),.tm-probe-button.primary{background:#00ffff;color:#050009}.tm-probe-button.magenta{border-color:#ff00d4;color:#ff55e7}.tm-probe-button:disabled{opacity:.34;cursor:not-allowed}.tm-runtime-banner{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-top:10px;padding:8px 9px;border-left:2px solid #00ffff;background:rgba(0,255,255,.035);font-size:8px;line-height:1.5}.tm-runtime-banner.stale{border-left-color:#ffd36b}.tm-runtime-banner.error{border-left-color:#ff3d7f}.tm-probe-notice{margin-top:9px;padding:8px 9px;border:1px solid rgba(117,255,226,.42);color:#baffed;background:rgba(0,20,18,.45);font-size:8px;line-height:1.5;overflow-wrap:anywhere}.tm-probe-notice.error{border-color:rgba(255,61,127,.48);color:#ff91b5;background:rgba(40,0,18,.48)}.tm-inference-identity{margin-top:10px;padding:8px 9px;border:1px solid rgba(255,0,212,.24);background:rgba(20,0,28,.34);color:#a995b8;font-size:8px;line-height:1.55;overflow-wrap:anywhere;word-break:break-word}.tm-probe-help{margin-top:8px;color:#887595;font-size:7px;line-height:1.5}.tm-runtime-metrics{padding-top:10px}.tm-runtime-metrics .tm-metrics,.tm-request-metrics .tm-metrics{padding:10px 0 0;grid-template-columns:repeat(3,minmax(0,1fr))}.tm-runtime-subhead{display:flex;align-items:center;justify-content:space-between;gap:10px;margin-top:12px;padding-top:11px;border-top:1px solid rgba(0,255,255,.16)}.tm-runtime-subhead strong{font-size:9px;letter-spacing:.05em}.tm-runtime-subhead span{color:#7e6a8b;font-size:7px;text-align:right}@media(max-width:980px){.tm-probe-fields{grid-template-columns:1fr 110px}.tm-probe-field.key{grid-column:1/-1}.tm-runtime-metrics .tm-metrics,.tm-request-metrics .tm-metrics{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:650px){.tm-probe-fields{grid-template-columns:1fr}.tm-probe-field.key{grid-column:auto}.tm-runtime-metrics .tm-metrics,.tm-request-metrics .tm-metrics{grid-template-columns:1fr}}
"#;

#[derive(Clone, Default)]
struct InferenceUiState {
    monitor_endpoint: Option<ServerEndpoint>,
    public_endpoint: Option<ServerEndpoint>,
    passive: Option<PassiveInferenceMetricsSnapshot>,
    passive_stale: bool,
    passive_error: Option<String>,
    request: Option<InferenceTelemetrySnapshot>,
    request_stale: bool,
    reachable: Option<bool>,
    probe_running: bool,
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
                    (current.monitor_endpoint.clone(), current.generation)
                };

                if let Some(endpoint) = endpoint {
                    let passive = poll_passive_inference_metrics(&endpoint, PASSIVE_TIMEOUT);
                    let reachable = passive.is_ok()
                        || check_endpoint_reachable(&endpoint, REACHABILITY_TIMEOUT).is_ok();
                    let mut current = state.write();
                    if current.generation == generation
                        && current.monitor_endpoint.as_ref() == Some(&endpoint)
                    {
                        current.reachable = Some(reachable);
                        match passive {
                            Ok(snapshot) => {
                                let source_changed = current
                                    .passive
                                    .as_ref()
                                    .is_some_and(|previous| {
                                        previous.source_endpoint != snapshot.source_endpoint
                                    });
                                if source_changed && current.request.is_some() {
                                    current.request_stale = true;
                                }
                                current.passive = Some(snapshot);
                                current.passive_stale = false;
                                current.passive_error = None;
                            }
                            Err(error) => {
                                current.passive_stale = current.passive.is_some();
                                current.passive_error = Some(error.to_string());
                                if !reachable && current.request.is_some() {
                                    current.request_stale = true;
                                }
                            }
                        }
                    }
                }

                thread::park_timeout(PASSIVE_CADENCE);
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
            detail: "Latest request evidence".to_owned(),
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

fn present_request_metric<T>(
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
            "Last request evidence observed {} ms UNIX; runtime continuity changed. Run a new 4-token probe to refresh request-bound evidence.",
            metric.observed_at_unix_ms
        );
    }
    presentation
}

fn request_metric_source<T>(metric: &InferenceMetric<T>) -> String {
    format!("{} · {}", metric.source.provider, metric.source.field)
}

fn metric_card(label: String, metric: MetricPresentation, source: String) -> Element {
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

fn passive_metric(
    value: Option<f64>,
    stale: bool,
    format_value: impl FnOnce(f64) -> String,
    detail: &str,
) -> MetricPresentation {
    match value {
        Some(value) => MetricPresentation {
            value: format_value(value),
            state_label: if stale { "STALE" } else { "LIVE" },
            state_class: if stale { "stale" } else { "" },
            detail: detail.to_owned(),
        },
        None => MetricPresentation {
            value: "Unavailable".to_owned(),
            state_label: "UNAVAILABLE",
            state_class: "unavailable",
            detail: "This llama.cpp /metrics response did not expose this field; no zero was synthesized."
                .to_owned(),
        },
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

fn attach_monitor(mut state: InferenceStateSignal, endpoint: ServerEndpoint) {
    let public_endpoint = endpoint_without_secret(&endpoint);
    let mut current = state.write();
    let endpoint_changed = current
        .public_endpoint
        .as_ref()
        .is_some_and(|previous| previous.authority() != public_endpoint.authority());
    if endpoint_changed {
        current.passive = None;
        current.passive_stale = false;
        current.passive_error = None;
        if current.request.is_some() {
            current.request_stale = true;
        }
    }
    current.monitor_endpoint = Some(endpoint);
    current.public_endpoint = Some(public_endpoint.clone());
    current.reachable = None;
    current.generation = current.generation.saturating_add(1);
    current.notice = Some((
        true,
        format!(
            "Passive /metrics monitoring attached to {}. Runtime counters can update without consuming an inference slot.",
            public_endpoint.authority()
        ),
    ));
}

fn failed_probe_reachability(error: &StreamingInferenceProbeError) -> Option<bool> {
    match error {
        StreamingInferenceProbeError::Connect { .. }
        | StreamingInferenceProbeError::HostResolution { .. } => Some(false),
        StreamingInferenceProbeError::InvalidPort
        | StreamingInferenceProbeError::InvalidApiKey
        | StreamingInferenceProbeError::NonLoopbackDenied { .. }
        | StreamingInferenceProbeError::Io { .. } => None,
        StreamingInferenceProbeError::ResponseTooLarge { .. }
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
    attach_monitor(state, endpoint.clone());
    let public_endpoint = endpoint_without_secret(&endpoint);
    {
        let mut current = state.write();
        if current.probe_running {
            return;
        }
        current.probe_running = true;
        current.notice = Some((
            true,
            format!(
                "4-token request-bound probe in progress against {}.",
                public_endpoint.authority()
            ),
        ));
    }

    let result = probe_llama_cpp_streaming(&endpoint, INFERENCE_PROBE_TIMEOUT);
    let mut current = state.write();
    if current.public_endpoint.as_ref() != Some(&public_endpoint) {
        current.probe_running = false;
        return;
    }
    current.probe_running = false;
    current.generation = current.generation.saturating_add(1);

    match result {
        Ok(evidence) => {
            current.reachable = Some(true);
            current.request_stale = false;
            current.event_count = Some(evidence.event_count);
            current.notice = Some((
                true,
                format!(
                    "Request-bound evidence captured: HTTP {} · {} SSE events · TTFT {:.2} ms · latency {:.2} ms.",
                    evidence.status_code,
                    evidence.event_count,
                    evidence.ttft_ms,
                    evidence.request_latency_ms
                ),
            ));
            current.request = Some(evidence.snapshot);
        }
        Err(StreamingInferenceProbeError::Busy { model }) => {
            current.reachable = Some(true);
            current.notice = Some((
                true,
                format!(
                    "Model {model} is busy. Passive /metrics monitoring remains active; TTFT and exact request-bound fields will refresh when a slot is free."
                ),
            ));
        }
        Err(error) => {
            if let Some(reachable) = failed_probe_reachability(&error) {
                current.reachable = Some(reachable);
                if !reachable && current.request.is_some() {
                    current.request_stale = true;
                }
            }
            current.notice = Some((false, format!("4-token request probe failed: {error}")));
        }
    }
}

fn monitor_presentation(state: &InferenceUiState) -> (&'static str, &'static str, String) {
    let endpoint = state
        .public_endpoint
        .as_ref()
        .map(ServerEndpoint::authority)
        .unwrap_or_else(|| "no endpoint".to_owned());
    if state.public_endpoint.is_none() {
        return (
            "NOT MONITORED",
            "",
            "Attach passive monitoring or run the request probe to begin.".to_owned(),
        );
    }
    if state.reachable == Some(false) {
        return (
            "DISCONNECTED",
            "error",
            format!("{endpoint} is not accepting connections; retained evidence is stale."),
        );
    }
    if state.passive_stale {
        return (
            "PASSIVE STALE",
            "stale",
            format!(
                "{endpoint} is reachable, but the latest /metrics poll failed; the last runtime values are retained and labelled stale."
            ),
        );
    }
    if let Some(passive) = state.passive.as_ref() {
        let model = passive.model.as_deref().unwrap_or("direct runtime");
        return (
            "PASSIVE LIVE",
            "",
            format!(
                "Polling {model} at {} every 1 s without submitting inference work.",
                passive.source_endpoint
            ),
        );
    }
    if state.reachable == Some(true) {
        return (
            "REACHABLE · METRICS PENDING",
            "",
            format!("{endpoint} is reachable; waiting for a usable /metrics sample."),
        );
    }
    (
        "CHECKING",
        "",
        format!("Resolving runtime metrics behind {endpoint}."),
    )
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
    let can_probe = !snapshot.probe_running && endpoint_validation.is_ok();
    let can_attach = endpoint_validation.is_ok();
    let (monitor_label, monitor_class, monitor_detail) = monitor_presentation(&snapshot);

    rsx! {
        style { dangerous_inner_html: INFERENCE_UI_CSS }
        section { class: "tm-panel wide tm-inference",
            div { class: "tm-panel-head",
                h2 { "INFERENCE TELEMETRY" }
                span { class: "tm-source", "llama.cpp /metrics · /completion · client TTFT" }
            }
            div { class: "tm-inference-body",
                div { class: "tm-probe-fields",
                    div { class: "tm-probe-field",
                        label { "HOST" }
                        input {
                            class: "tm-probe-input",
                            value: "{host_value}",
                            disabled: snapshot.probe_running,
                            oninput: move |event| host.set(event.value()),
                        }
                    }
                    div { class: "tm-probe-field",
                        label { "PORT" }
                        input {
                            class: "tm-probe-input",
                            value: "{port_value}",
                            disabled: snapshot.probe_running,
                            oninput: move |event| port.set(event.value()),
                        }
                    }
                    div { class: "tm-probe-field key",
                        label { "API KEY · OPTIONAL · MEMORY ONLY" }
                        input {
                            class: "tm-probe-input",
                            r#type: "password",
                            value: "{api_key_value}",
                            disabled: snapshot.probe_running,
                            oninput: move |event| api_key.set(event.value()),
                        }
                    }
                }

                div { class: "tm-probe-actions",
                    button {
                        class: if allow_non_loopback_value { "tm-probe-button magenta" } else { "tm-probe-button" },
                        disabled: snapshot.probe_running,
                        onclick: move |_| {
                            let enabled = allow_non_loopback();
                            allow_non_loopback.set(!enabled);
                        },
                        if allow_non_loopback_value { "LAN OPT-IN ON" } else { "LAN OPT-IN OFF" }
                    }
                    button {
                        class: "tm-probe-button",
                        disabled: !can_attach,
                        onclick: move |_| {
                            match build_endpoint(&host(), &port(), &api_key(), allow_non_loopback()) {
                                Ok(endpoint) => attach_monitor(state, endpoint),
                                Err(error) => state.write().notice = Some((false, error)),
                            }
                        },
                        "ATTACH PASSIVE MONITOR"
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
                        if snapshot.probe_running { "PROBING…" } else { "RUN 4-TOKEN PROBE" }
                    }
                    if let Err(error) = endpoint_validation.as_ref() {
                        span { class: "tm-source", "BLOCKED: {error}" }
                    }
                }

                div { class: "tm-probe-help",
                    "Passive /metrics polling does not consume an inference slot and stays useful while a one-slot model is busy. The 4-token probe exists only for request-bound evidence such as TTFT, request latency, and exact per-request timings. Router child ports are discovered automatically."
                }

                div { class: "tm-runtime-banner {monitor_class}",
                    span { class: "tm-state {monitor_class}", "{monitor_label}" }
                    span { "{monitor_detail}" }
                }

                if let Some((success, message)) = snapshot.notice.as_ref() {
                    div { class: if *success { "tm-probe-notice" } else { "tm-probe-notice error" }, "{message}" }
                }

                if let Some(error) = snapshot.passive_error.as_ref() {
                    div { class: "tm-probe-help",
                        "LATEST /METRICS POLL: {error}"
                    }
                }

                if let Some(passive) = snapshot.passive.as_ref() {
                    div { class: "tm-runtime-metrics",
                        div { class: "tm-inference-identity",
                            strong { "PASSIVE RUNTIME " }
                            if let Some(model) = passive.model.as_ref() {
                                "{model} · "
                            }
                            "source {passive.source_endpoint} · observed {passive.observed_at_unix_ms} ms UNIX"
                            if let Some(mode) = passive.speculative_type.as_ref() {
                                " · speculative mode {mode}"
                            }
                        }
                        div { class: "tm-metrics",
                            {metric_card(
                                "PROMPT RATE".to_owned(),
                                passive_metric(passive.prompt_tps, snapshot.passive_stale, |value| format!("{value:.2} tok/s"), "llama.cpp process/runtime gauge; does not require a new request from LlamaWave."),
                                "llama.cpp /metrics · llamacpp:prompt_tokens_seconds".to_owned(),
                            )}
                            {metric_card(
                                "DECODE RATE".to_owned(),
                                passive_metric(passive.decode_tps, snapshot.passive_stale, |value| format!("{value:.2} tok/s"), "llama.cpp process/runtime gauge; remains pollable while the inference slot is occupied."),
                                "llama.cpp /metrics · llamacpp:predicted_tokens_seconds".to_owned(),
                            )}
                            {metric_card(
                                "REQUESTS PROCESSING".to_owned(),
                                passive_metric(passive.requests_processing, snapshot.passive_stale, |value| format!("{value:.0}"), "Current llama.cpp processing-request gauge."),
                                "llama.cpp /metrics · llamacpp:requests_processing".to_owned(),
                            )}
                            {metric_card(
                                "REQUESTS DEFERRED".to_owned(),
                                passive_metric(passive.requests_deferred, snapshot.passive_stale, |value| format!("{value:.0}"), "Current llama.cpp deferred-request gauge."),
                                "llama.cpp /metrics · llamacpp:requests_deferred".to_owned(),
                            )}
                            {metric_card(
                                "PROMPT TOKENS TOTAL".to_owned(),
                                passive_metric(passive.prompt_tokens_total, snapshot.passive_stale, |value| format!("{value:.0} tok"), "Cumulative runtime counter since the child process started."),
                                "llama.cpp /metrics · llamacpp:prompt_tokens_total".to_owned(),
                            )}
                            {metric_card(
                                "DECODE TOKENS TOTAL".to_owned(),
                                passive_metric(passive.decode_tokens_total, snapshot.passive_stale, |value| format!("{value:.0} tok"), "Cumulative runtime counter since the child process started."),
                                "llama.cpp /metrics · llamacpp:tokens_predicted_total".to_owned(),
                            )}
                            {metric_card(
                                "CACHED PROMPT TOTAL".to_owned(),
                                passive_metric(passive.cached_prompt_tokens_total, snapshot.passive_stale, |value| format!("{value:.0} tok"), "Cumulative cached-prompt counter; missing stays unavailable rather than zero."),
                                "llama.cpp /metrics · llamacpp:prompt_tokens_cached_total".to_owned(),
                            )}
                            {metric_card(
                                "BUSY SLOTS / DECODE".to_owned(),
                                passive_metric(passive.busy_slots_per_decode, snapshot.passive_stale, |value| format!("{value:.2}"), "llama.cpp runtime gauge."),
                                "llama.cpp /metrics · llamacpp:n_busy_slots_per_decode".to_owned(),
                            )}
                            {metric_card(
                                if passive.is_mtp() { "MTP DRAFTED TOTAL" } else { "SPEC DRAFTED TOTAL" }.to_owned(),
                                passive_metric(passive.speculative_draft_tokens_total, snapshot.passive_stale, |value| format!("{value:.0} tok"), "Cumulative speculative draft-token counter. It is labelled MTP only when router args explicitly report an MTP spec type."),
                                "llama.cpp /metrics · llamacpp:spec_decode_num_draft_tokens_total".to_owned(),
                            )}
                            {metric_card(
                                if passive.is_mtp() { "MTP ACCEPTED TOTAL" } else { "SPEC ACCEPTED TOTAL" }.to_owned(),
                                passive_metric(passive.speculative_accepted_tokens_total, snapshot.passive_stale, |value| format!("{value:.0} tok"), "Cumulative speculative accepted-token counter."),
                                "llama.cpp /metrics · llamacpp:spec_decode_num_accepted_tokens_total".to_owned(),
                            )}
                            {metric_card(
                                if passive.is_mtp() { "MTP ACCEPTANCE" } else { "SPEC ACCEPTANCE" }.to_owned(),
                                passive_metric(passive.speculative_acceptance_rate, snapshot.passive_stale, |value| format!("{:.1}%", value * 100.0), "Derived from cumulative accepted / drafted counters; unavailable until the runtime exposes a nonzero draft count."),
                                "llama.cpp /metrics · accepted_total / draft_total".to_owned(),
                            )}
                            {metric_card(
                                "SPEC DRAFTS TOTAL".to_owned(),
                                passive_metric(passive.speculative_drafts_total, snapshot.passive_stale, |value| format!("{value:.0}"), "Cumulative speculative draft-operation counter."),
                                "llama.cpp /metrics · llamacpp:spec_decode_num_drafts_total".to_owned(),
                            )}
                        }
                    }
                } else {
                    div { class: "tm-empty",
                        "No passive runtime sample yet. Attach passive monitoring; this path uses /metrics and does not wait for an inference slot."
                    }
                }

                div { class: "tm-runtime-subhead",
                    strong { "REQUEST-BOUND EVIDENCE" }
                    span { "4-token /completion probe · TTFT and exact request timings" }
                }

                if let Some(inference) = snapshot.request.as_ref() {
                    div { class: "tm-request-metrics",
                        div { class: "tm-inference-identity",
                            strong { "REQUEST " }
                            "{inference.identity.request_id} · endpoint {inference.identity.endpoint}"
                            if let Some(model) = inference.identity.reported_model.as_ref() {
                                " · model {model}"
                            }
                            if let Some(events) = snapshot.event_count {
                                " · {events} SSE events"
                            }
                            if snapshot.request_stale {
                                " · STALE"
                            }
                        }
                        div { class: "tm-metrics",
                            {metric_card(
                                "PROMPT RATE".to_owned(),
                                present_request_metric(&inference.prompt_tps, snapshot.request_stale, |value| format!("{value:.2} tok/s")),
                                request_metric_source(&inference.prompt_tps),
                            )}
                            {metric_card(
                                "DECODE RATE".to_owned(),
                                present_request_metric(&inference.decode_tps, snapshot.request_stale, |value| format!("{value:.2} tok/s")),
                                request_metric_source(&inference.decode_tps),
                            )}
                            {metric_card(
                                "TTFT".to_owned(),
                                present_request_metric(&inference.ttft_ms, snapshot.request_stale, |value| format!("{value:.2} ms")),
                                request_metric_source(&inference.ttft_ms),
                            )}
                            {metric_card(
                                "REQUEST LATENCY".to_owned(),
                                present_request_metric(&inference.request_latency_ms, snapshot.request_stale, |value| format!("{value:.2} ms")),
                                request_metric_source(&inference.request_latency_ms),
                            )}
                            {metric_card(
                                "CONTEXT USED".to_owned(),
                                present_request_metric(&inference.context_tokens, snapshot.request_stale, |value| format!("{value} tok")),
                                request_metric_source(&inference.context_tokens),
                            )}
                            {metric_card(
                                "MTP GENERATED".to_owned(),
                                present_request_metric(&inference.mtp_generated_tokens, snapshot.request_stale, |value| format!("{value} tok")),
                                request_metric_source(&inference.mtp_generated_tokens),
                            )}
                            {metric_card(
                                "MTP ACCEPTED".to_owned(),
                                present_request_metric(&inference.mtp_accepted_tokens, snapshot.request_stale, |value| format!("{value} tok")),
                                request_metric_source(&inference.mtp_accepted_tokens),
                            )}
                            {metric_card(
                                "MTP ACCEPTANCE".to_owned(),
                                present_request_metric(&inference.mtp_acceptance_rate, snapshot.request_stale, |value| format!("{:.1}%", value * 100.0)),
                                request_metric_source(&inference.mtp_acceptance_rate),
                            )}
                            {metric_card(
                                "MTP MEAN RUN".to_owned(),
                                present_request_metric(&inference.mtp_mean_run_length, snapshot.request_stale, |value| format!("{value:.2} tok")),
                                request_metric_source(&inference.mtp_mean_run_length),
                            )}
                        }
                    }
                } else {
                    div { class: "tm-empty",
                        "No request-bound evidence yet. This is expected while a one-slot model is busy: passive prompt/decode/MTP runtime metrics can still remain live above, while TTFT and request latency stay unavailable until a real probe can run."
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passive_missing_metric_is_unavailable_not_zero() {
        let metric = passive_metric(None, false, |value| value.to_string(), "unused");
        assert_eq!(metric.value, "Unavailable");
        assert_eq!(metric.state_label, "UNAVAILABLE");
    }

    #[test]
    fn passive_poll_failure_retains_value_as_stale() {
        let metric = passive_metric(Some(3.39), true, |value| format!("{value:.2}"), "runtime");
        assert_eq!(metric.value, "3.39");
        assert_eq!(metric.state_label, "STALE");
    }

    #[test]
    fn monitored_public_endpoint_never_retains_api_key() {
        let endpoint = build_endpoint("127.0.0.1", "8080", "top-secret", false).unwrap();
        let public = endpoint_without_secret(&endpoint);
        assert_eq!(endpoint.api_key.as_deref(), Some("top-secret"));
        assert!(public.api_key.is_none());
    }

    #[test]
    fn busy_probe_is_known_reachable() {
        assert_eq!(
            failed_probe_reachability(&StreamingInferenceProbeError::Busy {
                model: "Qwen3.8-27B".to_owned(),
            }),
            Some(true)
        );
    }
}
