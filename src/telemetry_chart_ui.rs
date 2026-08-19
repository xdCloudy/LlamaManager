use std::collections::HashMap;

use dioxus::prelude::*;

use crate::{
    gpu_telemetry::{GpuAdapterIdentity, GpuTelemetryReading, GpuTelemetrySnapshot},
    hardware_telemetry::{HardwareTelemetrySnapshot, TelemetryReading, TelemetryState},
    telemetry_history::{
        ChartOptions, ChartProjection, HistoryError, HistoryPolicy, SampleSource, SeriesIdentity,
        SeriesKey, SeriesPresentationState, TimeSeries, TimeSeriesSample,
    },
};

const CHART_WIDTH_PX: u32 = 960;
const CHART_HEIGHT_PX: u32 = 180;
const MISSING_SAMPLE_GAP_MS: u64 = 2_500;

const TELEMETRY_CHART_CSS: &str = r#"
.tm-history-list{display:grid;gap:10px;padding:12px}.tm-chart-card{min-width:0;border:1px solid rgba(0,255,255,.22);background:rgba(2,0,9,.48);overflow:hidden}.tm-chart-head{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:9px 10px;border-bottom:1px solid rgba(0,255,255,.15)}.tm-chart-head strong{font-size:10px;overflow-wrap:anywhere}.tm-chart-state{font-size:7px;font-weight:900;letter-spacing:.07em;text-transform:uppercase;color:#8fffee}.tm-chart-state.stale{color:#ffd36b}.tm-chart-state.error,.tm-chart-state.disconnected{color:#ff7ba9}.tm-chart-state.unavailable,.tm-chart-state.empty{color:#a795b3}.tm-chart-svg{width:100%;min-height:150px;padding:8px 8px 3px;overflow:hidden}.tm-chart-svg svg{display:block;width:100%;height:auto;min-height:140px;max-height:190px;background:linear-gradient(180deg,rgba(0,255,255,.025),rgba(255,0,212,.018));border:1px solid rgba(115,80,135,.22)}.tm-chart-svg .telemetry-segment{fill:none;stroke:#00f5ff;stroke-width:2;vector-effect:non-scaling-stroke}.tm-chart-svg .telemetry-stale{stroke:#ffd36b;stroke-dasharray:6 5}.tm-chart-svg .telemetry-point{fill:#00f5ff}.tm-chart-svg .telemetry-point.telemetry-stale{fill:#ffd36b}.tm-chart-svg .telemetry-gap{fill:rgba(255,0,212,.08)}.tm-chart-svg .telemetry-gap-disconnected,.tm-chart-svg .telemetry-gap-error{fill:rgba(255,61,127,.14)}.tm-chart-svg .telemetry-gap-unavailable,.tm-chart-svg .telemetry-gap-paused{fill:rgba(145,125,158,.12)}.tm-chart-svg .telemetry-gap-reset{fill:rgba(255,211,107,.13)}.tm-chart-meta{display:flex;justify-content:space-between;gap:10px;flex-wrap:wrap;padding:0 10px 9px;color:#806d8d;font-size:7px;line-height:1.45}.tm-chart-legend{display:flex;gap:10px;flex-wrap:wrap;padding:0 12px 12px;color:#8d789a;font-size:7px}.tm-chart-key{display:inline-flex;align-items:center;gap:5px}.tm-chart-swatch{width:16px;height:2px;background:#00f5ff}.tm-chart-swatch.stale{background:#ffd36b}.tm-chart-swatch.gap{height:8px;background:rgba(255,0,212,.28)}@media(max-width:650px){.tm-history-list{padding:8px}.tm-chart-svg{padding:6px 5px 2px}.tm-chart-svg svg{min-height:115px}.tm-chart-head{align-items:flex-start;flex-direction:column;gap:4px}}
"#;

#[derive(Debug, Clone, PartialEq)]
pub struct GpuHistoryChart {
    pub label: String,
    pub projection: ChartProjection,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TelemetryHistorySnapshot {
    pub cpu_total: Option<ChartProjection>,
    pub gpu_utilization: Vec<GpuHistoryChart>,
}

struct GpuHistorySeries {
    label: String,
    series: TimeSeries,
}

pub struct TelemetryHistoryEngine {
    policy: HistoryPolicy,
    cpu_total: Option<TimeSeries>,
    gpu_utilization: HashMap<String, GpuHistorySeries>,
}

impl Default for TelemetryHistoryEngine {
    fn default() -> Self {
        Self::new(HistoryPolicy::default()).expect("default telemetry history policy is valid")
    }
}

impl TelemetryHistoryEngine {
    pub fn new(policy: HistoryPolicy) -> Result<Self, HistoryError> {
        let policy = policy.validate()?;
        Ok(Self {
            policy,
            cpu_total: None,
            gpu_utilization: HashMap::new(),
        })
    }

    pub fn observe(
        &mut self,
        hardware: &HardwareTelemetrySnapshot,
        gpu: &GpuTelemetrySnapshot,
    ) -> Result<TelemetryHistorySnapshot, HistoryError> {
        self.observe_cpu(&hardware.cpu.total_usage_percent)?;
        for adapter in &gpu.adapters {
            self.observe_gpu(adapter)?;
        }
        self.snapshot()
    }

    fn observe_cpu(&mut self, reading: &TelemetryReading<f64>) -> Result<(), HistoryError> {
        if self.cpu_total.is_none() {
            let key = SeriesKey::new(
                "cpu.total_usage",
                "percent",
                reading.source.provider.clone(),
                reading.source.api.clone(),
                SeriesIdentity::new("host", "local-windows"),
            );
            self.cpu_total = Some(TimeSeries::new(key, self.policy)?);
        }
        let series = self.cpu_total.as_mut().expect("initialized above");
        let sample = f64_sample(series.key(), reading)?;
        series.push(sample)
    }

    fn observe_gpu(
        &mut self,
        adapter: &crate::gpu_telemetry::GpuAdapterTelemetry,
    ) -> Result<(), HistoryError> {
        let stable_id = gpu_stable_id(&adapter.identity);
        let label = gpu_label(&adapter.identity);
        if !self.gpu_utilization.contains_key(&stable_id) {
            let identity = gpu_series_identity(&adapter.identity);
            let reading = &adapter.gpu_utilization_percent;
            let key = SeriesKey::new(
                "gpu.utilization",
                "percent",
                reading.source.provider.clone(),
                reading.source.api.clone(),
                identity,
            );
            self.gpu_utilization.insert(
                stable_id.clone(),
                GpuHistorySeries {
                    label: label.clone(),
                    series: TimeSeries::new(key, self.policy)?,
                },
            );
        }

        let history = self
            .gpu_utilization
            .get_mut(&stable_id)
            .expect("GPU history initialized above");
        history.label = label;
        let sample = u32_sample(history.series.key(), &adapter.gpu_utilization_percent)?;
        history.series.push(sample)
    }

    fn snapshot(&self) -> Result<TelemetryHistorySnapshot, HistoryError> {
        let cpu_total = self
            .cpu_total
            .as_ref()
            .map(|series| series.project(chart_options()))
            .transpose()?;
        let mut gpu_utilization = self
            .gpu_utilization
            .values()
            .map(|history| {
                Ok(GpuHistoryChart {
                    label: history.label.clone(),
                    projection: history.series.project(chart_options())?,
                })
            })
            .collect::<Result<Vec<_>, HistoryError>>()?;
        gpu_utilization.sort_by(|left, right| left.label.cmp(&right.label));

        Ok(TelemetryHistorySnapshot {
            cpu_total,
            gpu_utilization,
        })
    }
}

fn chart_options() -> ChartOptions {
    ChartOptions {
        width_px: CHART_WIDTH_PX,
        height_px: CHART_HEIGHT_PX,
        missing_gap_after_ms: Some(MISSING_SAMPLE_GAP_MS),
    }
}

fn f64_sample(
    key: &SeriesKey,
    reading: &TelemetryReading<f64>,
) -> Result<TimeSeriesSample, HistoryError> {
    sample_from_state(key, reading.sampled_at_unix_ms, &reading.state, |value| {
        *value
    })
}

fn u32_sample(
    key: &SeriesKey,
    reading: &GpuTelemetryReading<u32>,
) -> Result<TimeSeriesSample, HistoryError> {
    sample_from_state(key, reading.sampled_at_unix_ms, &reading.state, |value| {
        f64::from(*value)
    })
}

fn sample_from_state<T>(
    key: &SeriesKey,
    timestamp_unix_ms: u64,
    state: &TelemetryState<T>,
    convert: impl Fn(&T) -> f64,
) -> Result<TimeSeriesSample, HistoryError> {
    let source = SampleSource::from_key(key);
    match state {
        TelemetryState::Live { value } => {
            TimeSeriesSample::live(timestamp_unix_ms, convert(value), source)
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
        ),
        TelemetryState::Unavailable { reason } => Ok(TimeSeriesSample::unavailable(
            timestamp_unix_ms,
            source,
            reason.clone(),
        )),
        TelemetryState::Error { message } => Ok(TimeSeriesSample::error(
            timestamp_unix_ms,
            source,
            message.clone(),
        )),
    }
}

fn gpu_stable_id(identity: &GpuAdapterIdentity) -> String {
    identity
        .uuid
        .as_ref()
        .map(|uuid| format!("uuid:{uuid}"))
        .unwrap_or_else(|| format!("volatile-index:{}", identity.index))
}

fn gpu_series_identity(identity: &GpuAdapterIdentity) -> SeriesIdentity {
    let (namespace, stable_id) = match identity.uuid.as_ref() {
        Some(uuid) => ("gpu-uuid", uuid.clone()),
        None => ("gpu-index", identity.index.to_string()),
    };
    let series = SeriesIdentity::new(namespace, stable_id);
    match identity.name.as_ref() {
        Some(name) => series.with_display_name(name.clone()),
        None => series,
    }
}

fn gpu_label(identity: &GpuAdapterIdentity) -> String {
    identity
        .name
        .clone()
        .unwrap_or_else(|| format!("GPU {}", identity.index))
}

fn presentation_label(state: SeriesPresentationState) -> (&'static str, &'static str) {
    match state {
        SeriesPresentationState::Empty => ("EMPTY", "empty"),
        SeriesPresentationState::Live => ("LIVE", ""),
        SeriesPresentationState::Stale => ("STALE", "stale"),
        SeriesPresentationState::Disconnected => ("DISCONNECTED", "disconnected"),
        SeriesPresentationState::Paused => ("PAUSED", "unavailable"),
        SeriesPresentationState::Unavailable => ("UNAVAILABLE", "unavailable"),
        SeriesPresentationState::Error => ("ERROR", "error"),
        SeriesPresentationState::Reset => ("RESET", "stale"),
    }
}

fn chart_card(title: String, projection: ChartProjection) -> Element {
    let svg = projection.to_svg();
    let identity = projection.identity_disclosure.clone();
    let (state_label, state_class) = presentation_label(projection.presentation_state);
    let gap_count = projection.gaps.len();
    let segment_count = projection.segments.len();

    rsx! {
        article { class: "tm-chart-card",
            div { class: "tm-chart-head",
                strong { "{title}" }
                span { class: "tm-chart-state {state_class}", "{state_label}" }
            }
            div { class: "tm-chart-svg", dangerous_inner_html: svg }
            div { class: "tm-chart-meta",
                span { "{identity}" }
                span { "{segment_count} segments · {gap_count} explicit/missing gaps" }
            }
        }
    }
}

pub fn render_history_panel(history: TelemetryHistorySnapshot, error: Option<String>) -> Element {
    rsx! {
        style { dangerous_inner_html: TELEMETRY_CHART_CSS }
        section { class: "tm-panel wide",
            div { class: "tm-panel-head",
                h2 { "LIVE HISTORY" }
                span { class: "tm-source", "bounded TimeSeries · 1 Hz · gap-aware" }
            }
            if let Some(error) = error {
                div { class: "tm-empty", "History projection error: {error}" }
            } else if history.cpu_total.is_none() && history.gpu_utilization.is_empty() {
                div { class: "tm-empty", "Waiting for enough provider samples to build telemetry history." }
            } else {
                div { class: "tm-history-list",
                    if let Some(cpu) = history.cpu_total {
                        {chart_card("CPU TOTAL UTILIZATION".to_owned(), cpu)}
                    }
                    for gpu in history.gpu_utilization {
                        {chart_card(format!("{} · GPU UTILIZATION", gpu.label), gpu.projection)}
                    }
                }
                div { class: "tm-chart-legend",
                    span { class: "tm-chart-key", span { class: "tm-chart-swatch" } "LIVE" }
                    span { class: "tm-chart-key", span { class: "tm-chart-swatch stale" } "STALE" }
                    span { class: "tm-chart-key", span { class: "tm-chart-swatch gap" } "DISCONNECT / UNAVAILABLE / ERROR / MISSING" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        gpu_telemetry::{GpuTelemetrySource, GpuTelemetryUnit},
        hardware_telemetry::{TelemetrySource, TelemetryUnit},
        telemetry_history::{ChartGapKind, ChartSegmentState, MetricSupport},
    };

    use super::*;

    fn cpu_reading(state: TelemetryState<f64>, timestamp: u64) -> TelemetryReading<f64> {
        TelemetryReading {
            state,
            unit: TelemetryUnit::Percent,
            source: TelemetrySource {
                provider: "windows-native".to_owned(),
                api: "NtQuerySystemInformation".to_owned(),
            },
            sampled_at_unix_ms: timestamp,
        }
    }

    fn gpu_reading(state: TelemetryState<u32>, timestamp: u64) -> GpuTelemetryReading<u32> {
        GpuTelemetryReading {
            state,
            unit: GpuTelemetryUnit::Percent,
            source: GpuTelemetrySource {
                provider: "nvidia-nvml".to_owned(),
                api: "nvmlDeviceGetUtilizationRates(gpu)".to_owned(),
            },
            sampled_at_unix_ms: timestamp,
        }
    }

    #[test]
    fn provider_stale_state_becomes_distinct_chart_segment() {
        let reading = cpu_reading(TelemetryState::Live { value: 20.0 }, 1_000);
        let key = SeriesKey::new(
            "cpu.total_usage",
            "percent",
            reading.source.provider.clone(),
            reading.source.api.clone(),
            SeriesIdentity::new("host", "local-windows"),
        );
        let mut series = TimeSeries::new(key.clone(), HistoryPolicy::default()).unwrap();
        series.push(f64_sample(&key, &reading).unwrap()).unwrap();
        let stale = cpu_reading(
            TelemetryState::Stale {
                last_value: Some(20.0),
                last_observed_at_unix_ms: Some(1_000),
                reason: "provider delayed".to_owned(),
            },
            2_000,
        );
        series.push(f64_sample(&key, &stale).unwrap()).unwrap();

        let projection = series.project(chart_options()).unwrap();
        assert_eq!(projection.segments.len(), 2);
        assert_eq!(projection.segments[0].state, ChartSegmentState::Live);
        assert_eq!(projection.segments[1].state, ChartSegmentState::Stale);
    }

    #[test]
    fn unsupported_gpu_state_is_gap_with_no_fake_zero() {
        let reading = gpu_reading(
            TelemetryState::Unavailable {
                reason: "metric unsupported".to_owned(),
            },
            2_000,
        );
        let key = SeriesKey::new(
            "gpu.utilization",
            "percent",
            reading.source.provider.clone(),
            reading.source.api.clone(),
            SeriesIdentity::new("gpu-uuid", "GPU-1"),
        );
        let sample = u32_sample(&key, &reading).unwrap();
        assert_eq!(sample.value, None);
        assert_eq!(sample.support, MetricSupport::Unavailable);

        let mut series = TimeSeries::new(key, HistoryPolicy::default()).unwrap();
        series.push(sample).unwrap();
        let projection = series.project(chart_options()).unwrap();
        assert!(projection.segments.is_empty());
        assert_eq!(projection.gaps.len(), 1);
        assert_eq!(projection.gaps[0].kind, ChartGapKind::Unavailable);
    }

    #[test]
    fn gpu_uuid_is_preferred_over_volatile_index() {
        let identity = GpuAdapterIdentity {
            vendor: "NVIDIA".to_owned(),
            index: 7,
            uuid: Some("GPU-stable".to_owned()),
            name: Some("RTX Test".to_owned()),
            stable_for_evidence: true,
            identity_note: None,
        };
        assert_eq!(gpu_stable_id(&identity), "uuid:GPU-stable");
        let series = gpu_series_identity(&identity);
        assert_eq!(series.namespace, "gpu-uuid");
        assert_eq!(series.stable_id, "GPU-stable");
        assert_eq!(series.display_name.as_deref(), Some("RTX Test"));
    }
}
