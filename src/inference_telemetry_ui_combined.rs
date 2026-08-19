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
    inference_telemetry_ui_legacy::InferenceTelemetryPanel as RequestInferenceTelemetryPanel,
    passive_inference_telemetry::{
        PassiveInferenceTelemetrySnapshot, poll_passive_inference_telemetry,
    },
    server_readiness::ServerEndpoint,
};

const PASSIVE_CADENCE: Duration = Duration::from_secs(1);
const PASSIVE_TIMEOUT: Duration = Duration::from_millis(750);

const PASSIVE_UI_CSS: &str = r#"
.tm-passive-body{padding:12px}.tm-passive-fields{display:grid;grid-template-columns:minmax(0,1fr) 110px minmax(180px,.8fr);gap:8px}.tm-passive-field{min-width:0}.tm-passive-field label{display:block;margin-bottom:5px;color:#927da0;font-size:7px;letter-spacing:.07em;text-transform:uppercase}.tm-passive-input{width:100%;min-height:32px;padding:6px 8px;border:1px solid rgba(0,255,255,.30);border-radius:0;background:#030008;color:#f6eaff;font:inherit;font-size:9px}.tm-passive-input:focus-visible,.tm-passive-button:focus-visible{outline:2px solid #ff00ff;outline-offset:2px}.tm-passive-actions{display:flex;align-items:center;flex-wrap:wrap;gap:8px;margin-top:9px}.tm-passive-button{min-height:32px;padding:0 10px;border:1px solid #00dbe7;border-radius:0;background:transparent;color:#00f5ff;font:inherit;font-size:8px;font-weight:900;letter-spacing:.07em;text-transform:uppercase;cursor:pointer}.tm-passive-button:hover:not(:disabled),.tm-passive-button.primary{background:#00ffff;color:#050009}.tm-passive-button.magenta{border-color:#ff00d4;color:#ff55e7}.tm-passive-button.stop{border-color:#ff3d7f;color:#ff7ba9}.tm-passive-button:disabled{opacity:.34;cursor:not-allowed}.tm-passive-status{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-top:10px;padding:8px 9px;border-left:2px solid #00ffff;background:rgba(0,255,255,.035);font-size:8px;line-height:1.5}.tm-passive-status.stale{border-left-color:#ffd36b}.tm-passive-status.error{border-left-color:#ff3d7f}.tm-passive-help{margin-top:8px;color:#887595;font-size:7px;line-height:1.5}.tm-passive-warning{margin-top:8px;padding:7px 9px;border:1px solid rgba(255,211,107,.35);color:#ffd36b;background:rgba(45,30,0,.18);font-size:7px;line-height:1.5;overflow-wrap:anywhere}.tm-passive .tm-metrics{grid-template-columns:repeat(4,minmax(0,1fr))}@media(max-width:1100px){.tm-passive .tm-metrics{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:980px){.tm-passive-fields{grid-template-columns:1fr 110px}.tm-passive-field.key{grid-column:1/-1}}@media(max-width:650px){.tm-passive-fields{grid-template-columns:1fr}.tm-passive-field.key{grid-column:auto}.tm-passive .tm-metrics{grid-template-columns:1fr}}
"#;

#[derive(Debug, Clone, Default)]
struct PassiveUiState {
    snapshot: Option<PassiveInferenceTelemetrySnapshot>,
    error: Option<String>,
    stale: bool,
    sample_count: u64,
}

type PassiveStateSignal = Signal<PassiveUiState, SyncStorage>;
type PassiveTargetSignal = Signal<Option<ServerEndpoint>, SyncStorage>;

#[derive(Clone)]
struct PassivePollingWorker {
    _inner: Arc<PassivePollingWorkerInner>,
}

struct PassivePollingWorkerInner {
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl PassivePollingWorker {
    fn spawn(mut state: PassiveStateSignal, target: PassiveTargetSignal) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                let endpoint = target.read().clone();
                if let Some(endpoint) = endpoint {
                    let result = poll_passive_inference_telemetry(&endpoint, PASSIVE_TIMEOUT);
                    if target.read().as_ref() == Some(&endpoint) {
                        let mut current = state.write();
                        match result {
                            Ok(snapshot) => {
                                current.snapshot = Some(snapshot);
                                current.error = None;
                                current.stale = false;
                                current.sample_count = current.sample_count.saturating_add(1);
                            }
                            Err(error) => {
                                current.stale = current.snapshot.is_some();
                                current.error = Some(error);
                            }
                        }
                    }
                }
                thread::park_timeout(PASSIVE_CADENCE);
            }
        });

        Self {
            _inner: Arc::new(PassivePollingWorkerInner {
                stop,
                handle: Mutex::new(Some(handle)),
            }),
        }
    }
}

impl Drop for PassivePollingWorkerInner {
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

fn value_card(
    label: &'static str,
    value: Option<String>,
    source: &'static str,
    detail: String,
    stale: bool,
) -> Element {
    let (shown, state_label, state_class) = match value {
        Some(value) if stale => (value, "STALE", "stale"),
        Some(value) => (value, "LIVE", ""),
        None => ("Unavailable".to_owned(), "UNAVAILABLE", "unavailable"),
    };
    rsx! {
        div { class: "tm-metric",
            div { class: "tm-metric-label", "{label}" }
            div { class: "tm-metric-value", "{shown}" }
            div { class: "tm-metric-meta", "{detail}" }
            div { class: "tm-metric-meta", "SOURCE: {source}" }
            span { class: "tm-state {state_class}", "{state_label}" }
        }
    }
}

fn format_count(value: Option<f64>) -> Option<String> {
    value.map(|value| {
        if value.fract().abs() < f64::EPSILON {
            format!("{value:.0}")
        } else {
            format!("{value:.2}")
        }
    })
}

#[allow(non_snake_case)]
fn PassiveInferenceTelemetryPanel() -> Element {
    let mut state = use_signal_sync(PassiveUiState::default);
    let mut target = use_signal_sync(|| None::<ServerEndpoint>);
    let _worker = use_hook(move || PassivePollingWorker::spawn(state, target));
    let mut host = use_signal(|| "127.0.0.1".to_owned());
    let mut port = use_signal(|| "8080".to_owned());
    let mut api_key = use_signal(String::new);
    let mut allow_non_loopback = use_signal(|| false);

    let snapshot = state.read().clone();
    let active = target.read().is_some();
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

    let status = if active {
        if snapshot.error.is_some() && snapshot.snapshot.is_none() {
            ("POLL ERROR", "error")
        } else if snapshot.stale {
            ("POLL STALE", "stale")
        } else if snapshot.snapshot.is_some() {
            ("POLLING LIVE", "")
        } else {
            ("ATTACHING", "")
        }
    } else if snapshot.snapshot.is_some() {
        ("POLL STOPPED", "stale")
    } else {
        ("NOT POLLING", "")
    };

    rsx! {
        style { dangerous_inner_html: PASSIVE_UI_CSS }
        section { class: "tm-panel wide tm-passive",
            div { class: "tm-panel-head",
                h2 { "PASSIVE INFERENCE TELEMETRY" }
                span { class: "tm-source", "llama.cpp /metrics + /slots · no inference request" }
            }
            div { class: "tm-passive-body",
                div { class: "tm-passive-fields",
                    div { class: "tm-passive-field",
                        label { "HOST" }
                        input {
                            class: "tm-passive-input",
                            value: "{host_value}",
                            disabled: active,
                            oninput: move |event| host.set(event.value()),
                        }
                    }
                    div { class: "tm-passive-field",
                        label { "PORT" }
                        input {
                            class: "tm-passive-input",
                            value: "{port_value}",
                            disabled: active,
                            oninput: move |event| port.set(event.value()),
                        }
                    }
                    div { class: "tm-passive-field key",
                        label { "API KEY · OPTIONAL · MEMORY ONLY" }
                        input {
                            class: "tm-passive-input",
                            r#type: "password",
                            value: "{api_key_value}",
                            disabled: active,
                            oninput: move |event| api_key.set(event.value()),
                        }
                    }
                }

                div { class: "tm-passive-actions",
                    button {
                        class: if allow_non_loopback_value { "tm-passive-button magenta" } else { "tm-passive-button" },
                        disabled: active,
                        onclick: move |_| {
                            let enabled = allow_non_loopback();
                            allow_non_loopback.set(!enabled);
                        },
                        if allow_non_loopback_value { "LAN OPT-IN ON" } else { "LAN OPT-IN OFF" }
                    }
                    if active {
                        button {
                            class: "tm-passive-button stop",
                            onclick: move |_| {
                                target.set(None);
                                let mut current = state.write();
                                current.stale = current.snapshot.is_some();
                                current.error = None;
                            },
                            "STOP POLLING"
                        }
                    } else {
                        button {
                            class: "tm-passive-button primary",
                            disabled: endpoint_validation.is_err(),
                            onclick: move |_| {
                                match build_endpoint(&host(), &port(), &api_key(), allow_non_loopback()) {
                                    Ok(endpoint) => {
                                        state.set(PassiveUiState::default());
                                        target.set(Some(endpoint));
                                    }
                                    Err(error) => state.write().error = Some(error),
                                }
                            },
                            "START PASSIVE POLL"
                        }
                    }
                    if let Err(error) = endpoint_validation.as_ref() {
                        span { class: "tm-source", "BLOCKED: {error}" }
                    }
                }

                div { class: "tm-passive-help",
                    "This monitor only reads llama.cpp monitoring endpoints. It does not reserve a slot and remains usable while a single-slot model is busy. Router child ports are discovered automatically. TTFT and end-to-end request latency stay in the explicit request-evidence panel below because passive counters cannot truthfully supply them."
                }

                div { class: "tm-passive-status {status.1}",
                    span { class: "tm-state {status.1}", "{status.0}" }
                    if let Some(sample) = snapshot.snapshot.as_ref() {
                        span {
                            "{sample.logical_endpoint} · source {sample.source_endpoint}"
                            if let Some(model) = sample.model.as_ref() {
                                " · model {model}"
                            }
                            " · sample {snapshot.sample_count}"
                        }
                    } else {
                        span { "Passive monitoring is not producing evidence yet." }
                    }
                }

                if let Some(error) = snapshot.error.as_ref() {
                    div { class: "tm-passive-warning", "{error}" }
                }

                if let Some(sample) = snapshot.snapshot.as_ref() {
                    if sample.metrics_error.is_some() || sample.slots_error.is_some() {
                        div { class: "tm-passive-warning",
                            if let Some(error) = sample.metrics_error.as_ref() {
                                "METRICS PARTIAL: {error} "
                            }
                            if let Some(error) = sample.slots_error.as_ref() {
                                "SLOTS PARTIAL: {error}"
                            }
                        }
                    }
                    div { class: "tm-metrics",
                        {value_card(
                            "SERVER PROMPT RATE",
                            sample.prompt_tps.map(|value| format!("{value:.2} tok/s")),
                            "llama.cpp /metrics · prompt_tokens_seconds",
                            "Server-reported monitoring gauge; does not consume a slot.".to_owned(),
                            snapshot.stale,
                        )}
                        {value_card(
                            "SERVER DECODE RATE",
                            sample.decode_tps.map(|value| format!("{value:.2} tok/s")),
                            "llama.cpp /metrics · predicted_tokens_seconds",
                            "Server-reported monitoring gauge; separate from request TTFT.".to_owned(),
                            snapshot.stale,
                        )}
                        {value_card(
                            "PROCESSING REQUESTS",
                            format_count(sample.requests_processing),
                            "llama.cpp /metrics · requests_processing",
                            "Current server processing gauge.".to_owned(),
                            snapshot.stale,
                        )}
                        {value_card(
                            "BUSY SLOTS",
                            sample.busy_slots.zip(sample.total_slots).map(|(busy, total)| format!("{busy}/{total}")),
                            "llama.cpp /slots",
                            "Read without fail_on_no_slot, so a busy slot remains observable.".to_owned(),
                            snapshot.stale,
                        )}
                        {value_card(
                            "CURRENT DECODED",
                            sample.current_decoded_tokens.map(|value| format!("{value} tok")),
                            "llama.cpp /slots · next_token.n_decoded",
                            "Current slot progress where the server exposes it.".to_owned(),
                            snapshot.stale,
                        )}
                        {value_card(
                            "CONTEXT CAPACITY",
                            sample.context_capacity_tokens.map(|value| format!("{value} tok")),
                            "llama.cpp /slots · n_ctx",
                            "Per-slot context capacity from live monitoring state.".to_owned(),
                            snapshot.stale,
                        )}
                        {value_card(
                            "MTP DRAFTED · CUMULATIVE",
                            if sample.mtp_explicit { sample.speculative_draft_tokens_total.map(|value| format!("{value} tok")) } else { None },
                            "llama.cpp /metrics · spec_decode_num_draft_tokens_total",
                            if sample.mtp_explicit { "Runtime explicitly identifies MTP.".to_owned() } else { "Speculative mode was not explicitly identified as MTP.".to_owned() },
                            snapshot.stale,
                        )}
                        {value_card(
                            "MTP ACCEPTED · CUMULATIVE",
                            if sample.mtp_explicit { sample.speculative_accepted_tokens_total.map(|value| format!("{value} tok")) } else { None },
                            "llama.cpp /metrics · spec_decode_num_accepted_tokens_total",
                            if sample.mtp_explicit { "Runtime explicitly identifies MTP.".to_owned() } else { "Speculative mode was not explicitly identified as MTP.".to_owned() },
                            snapshot.stale,
                        )}
                        {value_card(
                            "MTP ACCEPTANCE · CUMULATIVE",
                            sample.mtp_acceptance_rate.map(|value| format!("{:.1}%", value * 100.0)),
                            "derived from llama.cpp MTP counters",
                            "Accepted draft tokens divided by drafted tokens; cumulative server evidence.".to_owned(),
                            snapshot.stale,
                        )}
                        {value_card(
                            "PROMPT TOKENS · CUMULATIVE",
                            sample.prompt_tokens_total.map(|value| format!("{value} tok")),
                            "llama.cpp /metrics · prompt_tokens_total",
                            "Server cumulative counter.".to_owned(),
                            snapshot.stale,
                        )}
                        {value_card(
                            "PREDICTED TOKENS · CUMULATIVE",
                            sample.predicted_tokens_total.map(|value| format!("{value} tok")),
                            "llama.cpp /metrics · tokens_predicted_total",
                            "Server cumulative counter.".to_owned(),
                            snapshot.stale,
                        )}
                        {value_card(
                            "DEFERRED REQUESTS",
                            format_count(sample.requests_deferred),
                            "llama.cpp /metrics · requests_deferred",
                            "Current server deferred-request gauge.".to_owned(),
                            snapshot.stale,
                        )}
                    }
                } else {
                    div { class: "tm-empty",
                        "Start passive polling to observe the loaded model even when every inference slot is occupied. No completion request is sent."
                    }
                }
            }
        }
    }
}

#[allow(non_snake_case)]
pub fn InferenceTelemetryPanel() -> Element {
    rsx! {
        PassiveInferenceTelemetryPanel {}
        RequestInferenceTelemetryPanel {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passive_endpoint_validation_keeps_api_key_memory_only() {
        let endpoint = build_endpoint("127.0.0.1", "8080", "secret", false).unwrap();
        assert_eq!(endpoint.authority(), "127.0.0.1:8080");
        assert_eq!(endpoint.api_key.as_deref(), Some("secret"));
    }

    #[test]
    fn passive_endpoint_validation_rejects_header_injection() {
        assert!(build_endpoint("127.0.0.1", "8080", "x\r\ny", false).is_err());
    }
}
