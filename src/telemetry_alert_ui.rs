use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use dioxus::prelude::*;

use crate::{
    gpu_telemetry::{GpuAdapterIdentity, GpuTelemetryReading, GpuTelemetrySnapshot},
    hardware_telemetry::{HardwareTelemetrySnapshot, TelemetryReading, TelemetryState},
    telemetry_alerts::{
        AlertComparator, AlertEngine, AlertError, AlertEvaluation, AlertEvent, AlertEventKind,
        AlertPresentationState, AlertRule, AlertSeverity, AlertThreshold, AlertValueRange,
    },
    telemetry_history::{
        MetricSupport, SampleSource, SeriesIdentity, SeriesKey, TimeSeriesSample, TimeSeriesState,
    },
};

const CPU_RULE_ID: &str = "cpu-high-utilization";
const GPU_TEMP_RULE_ID: &str = "gpu-temperature-warning";

const ALERT_UI_CSS: &str = r#"
.tm-alert-body{padding:12px}.tm-alert-rules{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px}.tm-alert-rule{min-width:0;border:1px solid rgba(255,0,212,.26);background:rgba(12,0,20,.42)}.tm-alert-rule-head{display:flex;justify-content:space-between;gap:8px;padding:9px 10px;border-bottom:1px solid rgba(255,0,212,.18)}.tm-alert-rule-head strong{font-size:10px}.tm-alert-severity{font-size:7px;font-weight:900;letter-spacing:.07em;text-transform:uppercase;color:#ffd36b}.tm-alert-policy{padding:9px 10px;color:#9884a6;font-size:7px;line-height:1.55}.tm-alert-policy b{color:#e9d5f4}.tm-threshold-grid{display:grid;grid-template-columns:1fr 1fr;gap:8px;padding:0 10px 10px}.tm-threshold{padding:8px;border:1px solid rgba(0,255,255,.19);background:rgba(0,0,0,.26)}.tm-threshold-label{color:#8f7a9e;font-size:7px}.tm-threshold-value{margin:4px 0 7px;font-size:15px;font-weight:900}.tm-threshold-actions{display:flex;gap:5px}.tm-alert-btn{min-width:29px;min-height:27px;border:1px solid rgba(0,255,255,.45);border-radius:0;background:transparent;color:#00f5ff;font:inherit;font-size:9px;font-weight:900;cursor:pointer}.tm-alert-btn:hover{background:#00ffff;color:#050009}.tm-alert-error{margin:0 10px 10px;padding:7px 8px;border:1px solid rgba(255,61,127,.42);color:#ff91b5;background:rgba(45,0,18,.35);font-size:7px;line-height:1.5}.tm-alert-instances{display:grid;gap:7px;margin-top:12px}.tm-alert-instance{display:grid;grid-template-columns:minmax(150px,1fr) auto;gap:10px;padding:9px 10px;border:1px solid rgba(105,78,119,.42);background:rgba(0,0,0,.24)}.tm-alert-instance-name{font-size:8px;font-weight:800;overflow-wrap:anywhere}.tm-alert-instance-meta{margin-top:4px;color:#887595;font-size:7px;line-height:1.45}.tm-alert-state{align-self:start;padding:3px 5px;border:1px solid rgba(117,255,226,.45);color:#8fffee;font-size:7px;font-weight:900;letter-spacing:.06em;text-transform:uppercase}.tm-alert-state.active{border-color:#ff3d7f;color:#ff7ba9}.tm-alert-state.pending,.tm-alert-state.clearing{border-color:#ffd36b;color:#ffd36b}.tm-alert-state.suppressed{border-color:#7b6888;color:#a795b3}.tm-alert-history{margin-top:12px;border-top:1px solid rgba(0,255,255,.16)}.tm-alert-history-row{display:grid;grid-template-columns:74px minmax(0,1fr);gap:9px;padding:9px 0;border-bottom:1px solid rgba(99,71,115,.24);font-size:7px}.tm-alert-event-kind{font-weight:900;color:#8fffee}.tm-alert-event-kind.fired{color:#ff7ba9}.tm-alert-event-detail{color:#a58fb2;line-height:1.55;overflow-wrap:anywhere}.tm-alert-empty{padding:10px 0;color:#887595;font-size:8px;line-height:1.5}@media(max-width:780px){.tm-alert-rules{grid-template-columns:1fr}}@media(max-width:520px){.tm-threshold-grid{grid-template-columns:1fr}.tm-alert-instance{grid-template-columns:1fr}.tm-alert-history-row{grid-template-columns:1fr}}
"#;

#[derive(Debug, Clone, PartialEq)]
pub struct AlertInstanceView {
    pub rule_id: String,
    pub identity: String,
    pub state: AlertPresentationState,
    pub suppression_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TelemetryAlertSnapshot {
    pub rules: Vec<AlertRule>,
    pub instances: Vec<AlertInstanceView>,
    pub history: Vec<AlertEvent>,
    pub control_error: Option<String>,
    pub observation_error: Option<String>,
}

struct TelemetryAlertDashboardEngine {
    engine: AlertEngine,
    latest: HashMap<(String, String), AlertEvaluation>,
    control_error: Option<String>,
    observation_error: Option<String>,
}

impl TelemetryAlertDashboardEngine {
    fn new() -> Self {
        Self {
            engine: AlertEngine::new(default_rules())
                .expect("default telemetry alert rules are valid"),
            latest: HashMap::new(),
            control_error: None,
            observation_error: None,
        }
    }

    fn observe(
        &mut self,
        hardware: &HardwareTelemetrySnapshot,
        gpu: &GpuTelemetrySnapshot,
    ) -> TelemetryAlertSnapshot {
        self.observation_error = None;

        let cpu_key = cpu_key(&hardware.cpu.total_usage_percent);
        match cpu_sample(&cpu_key, &hardware.cpu.total_usage_percent)
            .and_then(|sample| self.observe_one(&cpu_key, &sample))
        {
            Ok(()) => {}
            Err(error) => self.observation_error = Some(error),
        }

        for adapter in &gpu.adapters {
            let key = gpu_temperature_key(&adapter.identity, &adapter.temperature_celsius);
            let result = gpu_temperature_sample(&key, &adapter.temperature_celsius)
                .and_then(|sample| self.observe_one(&key, &sample));
            if let Err(error) = result {
                self.observation_error = Some(error);
                break;
            }
        }

        self.snapshot()
    }

    fn observe_one(&mut self, key: &SeriesKey, sample: &TimeSeriesSample) -> Result<(), String> {
        let evaluations = self
            .engine
            .observe(key, sample)
            .map_err(|error| error.to_string())?;
        for evaluation in evaluations {
            let identity = evaluation.series_key.identity.disclosure();
            self.latest
                .insert((evaluation.rule_id.clone(), identity), evaluation);
        }
        Ok(())
    }

    fn adjust_threshold(
        &mut self,
        rule_id: &str,
        trigger_delta: f64,
        clear_delta: f64,
    ) -> Result<(), AlertError> {
        let rule = self
            .engine
            .rules()
            .iter()
            .find(|rule| rule.id == rule_id)
            .ok_or_else(|| AlertError::UnknownRule(rule_id.to_owned()))?;
        let threshold = AlertThreshold {
            trigger: rule.threshold.trigger + trigger_delta,
            clear: rule.threshold.clear + clear_delta,
        };
        match self.engine.update_threshold(rule_id, threshold) {
            Ok(()) => {
                self.control_error = None;
                Ok(())
            }
            Err(error) => {
                self.control_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn snapshot(&self) -> TelemetryAlertSnapshot {
        let mut instances = self
            .latest
            .values()
            .map(|evaluation| AlertInstanceView {
                rule_id: evaluation.rule_id.clone(),
                identity: evaluation.series_key.identity.disclosure(),
                state: evaluation.state,
                suppression_reason: evaluation.suppression_reason.clone(),
            })
            .collect::<Vec<_>>();
        instances.sort_by(|left, right| {
            left.rule_id
                .cmp(&right.rule_id)
                .then_with(|| left.identity.cmp(&right.identity))
        });

        TelemetryAlertSnapshot {
            rules: self.engine.rules().to_vec(),
            instances,
            history: self
                .engine
                .history()
                .iter()
                .rev()
                .take(12)
                .cloned()
                .collect(),
            control_error: self.control_error.clone(),
            observation_error: self.observation_error.clone(),
        }
    }
}

#[derive(Clone)]
pub struct TelemetryAlertController {
    inner: Arc<Mutex<TelemetryAlertDashboardEngine>>,
}

impl std::fmt::Debug for TelemetryAlertController {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelemetryAlertController")
            .finish_non_exhaustive()
    }
}

impl PartialEq for TelemetryAlertController {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Default for TelemetryAlertController {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TelemetryAlertDashboardEngine::new())),
        }
    }
}

impl TelemetryAlertController {
    pub fn observe(
        &self,
        hardware: &HardwareTelemetrySnapshot,
        gpu: &GpuTelemetrySnapshot,
    ) -> TelemetryAlertSnapshot {
        match self.inner.lock() {
            Ok(mut engine) => engine.observe(hardware, gpu),
            Err(poisoned) => poisoned.into_inner().snapshot(),
        }
    }

    pub fn adjust_threshold(
        &self,
        rule_id: &str,
        trigger_delta: f64,
        clear_delta: f64,
    ) -> Result<(), AlertError> {
        match self.inner.lock() {
            Ok(mut engine) => engine.adjust_threshold(rule_id, trigger_delta, clear_delta),
            Err(poisoned) => {
                poisoned
                    .into_inner()
                    .adjust_threshold(rule_id, trigger_delta, clear_delta)
            }
        }
    }
}

fn default_rules() -> Vec<AlertRule> {
    vec![
        AlertRule {
            id: CPU_RULE_ID.to_owned(),
            metric: "cpu.total_usage".to_owned(),
            source_provider: "windows-native".to_owned(),
            source_api: "NtQuerySystemInformation(SystemProcessorPerformanceInformation)"
                .to_owned(),
            severity: AlertSeverity::Warning,
            comparator: AlertComparator::Above,
            threshold: AlertThreshold {
                trigger: 90.0,
                clear: 80.0,
            },
            window_ms: 3_000,
            debounce_ms: 5_000,
            min_live_samples: 4,
            valid_value_range: Some(AlertValueRange {
                min: 0.0,
                max: 100.0,
            }),
            reason: "configured sustained host CPU utilization threshold".to_owned(),
        },
        AlertRule {
            id: GPU_TEMP_RULE_ID.to_owned(),
            metric: "gpu.temperature".to_owned(),
            source_provider: "nvidia-nvml".to_owned(),
            source_api: "nvmlDeviceGetTemperature(NVML_TEMPERATURE_GPU)".to_owned(),
            severity: AlertSeverity::Warning,
            comparator: AlertComparator::Above,
            threshold: AlertThreshold {
                trigger: 85.0,
                clear: 78.0,
            },
            window_ms: 3_000,
            debounce_ms: 5_000,
            min_live_samples: 4,
            valid_value_range: Some(AlertValueRange {
                min: -20.0,
                max: 150.0,
            }),
            reason: "configured sustained NVIDIA GPU temperature threshold".to_owned(),
        },
    ]
}

fn cpu_key(reading: &TelemetryReading<f64>) -> SeriesKey {
    SeriesKey::new(
        "cpu.total_usage",
        "percent",
        reading.source.provider.clone(),
        reading.source.api.clone(),
        SeriesIdentity::new("host", "local-windows"),
    )
}

fn gpu_temperature_key(
    identity: &GpuAdapterIdentity,
    reading: &GpuTelemetryReading<u32>,
) -> SeriesKey {
    let series_identity = match identity.uuid.as_ref() {
        Some(uuid) => SeriesIdentity::new("gpu-uuid", uuid.clone()),
        None => SeriesIdentity::new("gpu-index", identity.index.to_string()),
    };
    SeriesKey::new(
        "gpu.temperature",
        "celsius",
        reading.source.provider.clone(),
        reading.source.api.clone(),
        series_identity,
    )
}

fn cpu_sample(
    key: &SeriesKey,
    reading: &TelemetryReading<f64>,
) -> Result<TimeSeriesSample, String> {
    sample_from_state(key, reading.sampled_at_unix_ms, &reading.state, |value| {
        *value
    })
}

fn gpu_temperature_sample(
    key: &SeriesKey,
    reading: &GpuTelemetryReading<u32>,
) -> Result<TimeSeriesSample, String> {
    sample_from_state(key, reading.sampled_at_unix_ms, &reading.state, |value| {
        f64::from(*value)
    })
}

fn sample_from_state<T>(
    key: &SeriesKey,
    timestamp_unix_ms: u64,
    state: &TelemetryState<T>,
    convert: impl Fn(&T) -> f64,
) -> Result<TimeSeriesSample, String> {
    let source = SampleSource::from_key(key);
    match state {
        TelemetryState::Live { value } => {
            TimeSeriesSample::live(timestamp_unix_ms, convert(value), source)
                .map_err(|error| error.to_string())
        }
        TelemetryState::Stale {
            last_value,
            last_observed_at_unix_ms,
            reason,
        } => TimeSeriesSample::stale(
            timestamp_unix_ms,
            last_value.as_ref().map(convert),
            source,
            *last_observed_at_unix_ms,
            reason.clone(),
        )
        .map_err(|error| error.to_string()),
        TelemetryState::Unavailable { reason } => Ok(TimeSeriesSample {
            timestamp_unix_ms,
            value: None,
            source,
            support: MetricSupport::Unavailable,
            state: TimeSeriesState::Unavailable {
                reason: reason.clone(),
            },
        }),
        TelemetryState::Error { message } => Ok(TimeSeriesSample {
            timestamp_unix_ms,
            value: None,
            source,
            support: MetricSupport::Error,
            state: TimeSeriesState::Error {
                message: message.clone(),
            },
        }),
    }
}

fn severity_label(severity: AlertSeverity) -> &'static str {
    match severity {
        AlertSeverity::Info => "INFO",
        AlertSeverity::Warning => "WARNING",
        AlertSeverity::Critical => "CRITICAL",
    }
}

fn state_label(state: AlertPresentationState) -> (&'static str, &'static str) {
    match state {
        AlertPresentationState::Inactive => ("INACTIVE", ""),
        AlertPresentationState::Pending => ("PENDING", "pending"),
        AlertPresentationState::Active => ("ACTIVE", "active"),
        AlertPresentationState::Suppressed => ("SUPPRESSED", "suppressed"),
        AlertPresentationState::Clearing => ("CLEARING", "clearing"),
    }
}

fn comparator_text(rule: &AlertRule) -> &'static str {
    match rule.comparator {
        AlertComparator::Above => "ABOVE",
        AlertComparator::Below => "BELOW",
    }
}

fn threshold_control(
    label: &'static str,
    value: f64,
    rule_id: String,
    trigger: bool,
    controller: TelemetryAlertController,
) -> Element {
    let decrement_controller = controller.clone();
    let decrement_rule = rule_id.clone();
    let increment_controller = controller;
    rsx! {
        div { class: "tm-threshold",
            div { class: "tm-threshold-label", "{label}" }
            div { class: "tm-threshold-value", "{value:.1}" }
            div { class: "tm-threshold-actions",
                button {
                    class: "tm-alert-btn",
                    title: "Decrease by 1",
                    onclick: move |_| {
                        let _ = if trigger {
                            decrement_controller.adjust_threshold(&decrement_rule, -1.0, 0.0)
                        } else {
                            decrement_controller.adjust_threshold(&decrement_rule, 0.0, -1.0)
                        };
                    },
                    "−"
                }
                button {
                    class: "tm-alert-btn",
                    title: "Increase by 1",
                    onclick: move |_| {
                        let _ = if trigger {
                            increment_controller.adjust_threshold(&rule_id, 1.0, 0.0)
                        } else {
                            increment_controller.adjust_threshold(&rule_id, 0.0, 1.0)
                        };
                    },
                    "+"
                }
            }
        }
    }
}

#[component]
pub fn TelemetryAlertPanel(
    snapshot: TelemetryAlertSnapshot,
    controller: TelemetryAlertController,
) -> Element {
    rsx! {
        style { dangerous_inner_html: ALERT_UI_CSS }
        section { class: "tm-panel wide",
            div { class: "tm-panel-head",
                h2 { "EVIDENCE-BACKED ALERTS" }
                span { class: "tm-source", "hysteresis · sustained windows · stale suppression" }
            }
            div { class: "tm-alert-body",
                div { class: "tm-alert-rules",
                    for rule in &snapshot.rules {
                        article { class: "tm-alert-rule",
                            div { class: "tm-alert-rule-head",
                                strong { "{rule.id}" }
                                span { class: "tm-alert-severity", "{severity_label(rule.severity)}" }
                            }
                            div { class: "tm-alert-policy",
                                b { "{rule.metric}" }
                                " · {comparator_text(rule)} trigger · {rule.window_ms} ms window · {rule.min_live_samples} live samples · {rule.debounce_ms} ms debounce"
                                br {}
                                "SOURCE: {rule.source_provider} · {rule.source_api}"
                                br {}
                                "WHY: {rule.reason}"
                            }
                            div { class: "tm-threshold-grid",
                                {threshold_control(
                                    "TRIGGER",
                                    rule.threshold.trigger,
                                    rule.id.clone(),
                                    true,
                                    controller.clone(),
                                )}
                                {threshold_control(
                                    "CLEAR",
                                    rule.threshold.clear,
                                    rule.id.clone(),
                                    false,
                                    controller.clone(),
                                )}
                            }
                        }
                    }
                }

                if let Some(error) = snapshot.control_error.as_ref() {
                    div { class: "tm-alert-error", "Threshold update rejected: {error}" }
                }
                if let Some(error) = snapshot.observation_error.as_ref() {
                    div { class: "tm-alert-error", "Alert observation error: {error}" }
                }

                div { class: "tm-alert-instances",
                    if snapshot.instances.is_empty() {
                        div { class: "tm-alert-empty", "No alert instances have been evaluated yet. Provider samples will populate CPU/GPU identities automatically." }
                    } else {
                        for instance in &snapshot.instances {
                            {
                                let (label, class) = state_label(instance.state);
                                rsx! {
                                    div { class: "tm-alert-instance",
                                        div {
                                            div { class: "tm-alert-instance-name", "{instance.rule_id} · {instance.identity}" }
                                            if let Some(reason) = instance.suppression_reason.as_ref() {
                                                div { class: "tm-alert-instance-meta", "SUPPRESSED BECAUSE: {reason}" }
                                            } else {
                                                div { class: "tm-alert-instance-meta", "Evaluation is based only on current supported live evidence." }
                                            }
                                        }
                                        span { class: "tm-alert-state {class}", "{label}" }
                                    }
                                }
                            }
                        }
                    }
                }

                div { class: "tm-alert-history",
                    if snapshot.history.is_empty() {
                        div { class: "tm-alert-empty", "No fired/resolved transitions recorded in this session." }
                    } else {
                        for event in &snapshot.history {
                            {
                                let kind_class = if event.kind == AlertEventKind::Fired { "fired" } else { "" };
                                let kind = if event.kind == AlertEventKind::Fired { "FIRED" } else { "RESOLVED" };
                                rsx! {
                                    div { class: "tm-alert-history-row",
                                        div { class: "tm-alert-event-kind {kind_class}", "{kind}" }
                                        div { class: "tm-alert-event-detail",
                                            "{event.rule_id} · {event.evidence.identity_disclosure} · observed {event.occurred_at_unix_ms} ms UNIX · trigger {event.evidence.trigger_threshold:.1} · clear {event.evidence.clear_threshold:.1} · {event.evidence.samples.len()} retained evidence samples · {event.evidence.reason}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        gpu_telemetry::GpuTelemetryUnit,
        hardware_telemetry::{TelemetrySource, TelemetryUnit},
    };

    use super::*;

    fn cpu_reading(timestamp: u64, value: f64) -> TelemetryReading<f64> {
        TelemetryReading {
            state: TelemetryState::Live { value },
            unit: TelemetryUnit::Percent,
            source: TelemetrySource {
                provider: "windows-native".to_owned(),
                api: "NtQuerySystemInformation(SystemProcessorPerformanceInformation)".to_owned(),
            },
            sampled_at_unix_ms: timestamp,
        }
    }

    fn gpu_temp_reading(state: TelemetryState<u32>, timestamp: u64) -> GpuTelemetryReading<u32> {
        GpuTelemetryReading {
            state,
            unit: GpuTelemetryUnit::Celsius,
            source: crate::gpu_telemetry::GpuTelemetrySource {
                provider: "nvidia-nvml".to_owned(),
                api: "nvmlDeviceGetTemperature(NVML_TEMPERATURE_GPU)".to_owned(),
            },
            sampled_at_unix_ms: timestamp,
        }
    }

    fn gpu_identity(name: &str) -> GpuAdapterIdentity {
        GpuAdapterIdentity {
            vendor: "NVIDIA".to_owned(),
            index: 0,
            uuid: Some("GPU-1".to_owned()),
            name: Some(name.to_owned()),
            stable_for_evidence: true,
            identity_note: None,
        }
    }

    #[test]
    fn default_rules_are_valid_and_source_exact() {
        let rules = default_rules();
        assert_eq!(rules.len(), 2);
        for rule in &rules {
            rule.validate().unwrap();
        }
        assert_eq!(
            rules[0].source_api,
            "NtQuerySystemInformation(SystemProcessorPerformanceInformation)"
        );
        assert_eq!(
            rules[1].source_api,
            "nvmlDeviceGetTemperature(NVML_TEMPERATURE_GPU)"
        );
    }

    #[test]
    fn cpu_rule_fires_and_resolves_with_sustained_same_provider_samples() {
        let mut dashboard = TelemetryAlertDashboardEngine::new();
        let key = cpu_key(&cpu_reading(1_000, 95.0));
        for (timestamp, value) in [(1_000, 95.0), (2_000, 96.0), (3_000, 94.0), (4_000, 95.0)] {
            let reading = cpu_reading(timestamp, value);
            dashboard
                .observe_one(&key, &cpu_sample(&key, &reading).unwrap())
                .unwrap();
        }
        assert_eq!(dashboard.engine.history().len(), 1);
        assert_eq!(dashboard.engine.history()[0].kind, AlertEventKind::Fired);

        for (timestamp, value) in [
            (9_000, 75.0),
            (10_000, 76.0),
            (11_000, 74.0),
            (12_000, 75.0),
        ] {
            let reading = cpu_reading(timestamp, value);
            dashboard
                .observe_one(&key, &cpu_sample(&key, &reading).unwrap())
                .unwrap();
        }
        assert_eq!(dashboard.engine.history().len(), 2);
        assert_eq!(dashboard.engine.history()[1].kind, AlertEventKind::Resolved);
    }

    #[test]
    fn gpu_display_name_change_does_not_change_alert_identity() {
        let reading = gpu_temp_reading(TelemetryState::Live { value: 70 }, 1_000);
        let first = gpu_temperature_key(&gpu_identity("RTX Original"), &reading);
        let renamed = gpu_temperature_key(&gpu_identity("RTX Renamed"), &reading);
        assert_eq!(first, renamed);
        assert_eq!(first.identity.namespace, "gpu-uuid");
        assert_eq!(first.identity.stable_id, "GPU-1");
        assert!(first.identity.display_name.is_none());
    }

    #[test]
    fn stale_gpu_temperature_is_suppressed_and_never_fires() {
        let identity = gpu_identity("RTX Test");
        let reading = gpu_temp_reading(
            TelemetryState::Stale {
                last_value: Some(99),
                last_observed_at_unix_ms: Some(1_000),
                reason: "adapter disappeared".to_owned(),
            },
            2_000,
        );
        let key = gpu_temperature_key(&identity, &reading);
        let mut dashboard = TelemetryAlertDashboardEngine::new();
        dashboard
            .observe_one(&key, &gpu_temperature_sample(&key, &reading).unwrap())
            .unwrap();
        let snapshot = dashboard.snapshot();
        assert_eq!(snapshot.instances.len(), 1);
        assert_eq!(
            snapshot.instances[0].state,
            AlertPresentationState::Suppressed
        );
        assert!(snapshot.history.is_empty());
    }

    #[test]
    fn invalid_user_threshold_edit_is_rejected_without_changing_rule() {
        let mut dashboard = TelemetryAlertDashboardEngine::new();
        let before = dashboard
            .engine
            .rules()
            .iter()
            .find(|rule| rule.id == CPU_RULE_ID)
            .unwrap()
            .threshold;
        assert!(dashboard.adjust_threshold(CPU_RULE_ID, -15.0, 0.0).is_err());
        let after = dashboard
            .engine
            .rules()
            .iter()
            .find(|rule| rule.id == CPU_RULE_ID)
            .unwrap()
            .threshold;
        assert_eq!(before, after);
        assert!(dashboard.control_error.is_some());
    }
}
