use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use dioxus::prelude::*;

use crate::{
    gpu_telemetry::{
        GpuAdapterTelemetry, GpuTelemetryProvider, GpuTelemetrySnapshot,
        NvidiaGpuTelemetryProvider,
    },
    hardware_telemetry::{
        HardwareTelemetryProvider, HardwareTelemetrySnapshot, TelemetryReading, TelemetryState,
        WindowsHardwareTelemetryProvider,
    },
};

const TELEMETRY_CADENCE: Duration = Duration::from_secs(1);

const TELEMETRY_UI_CSS: &str = r#"
.tm-page{min-height:100vh;padding:30px 34px 92px;color:#f6eaff;background:radial-gradient(circle at 80% 8%,rgba(255,0,190,.13),transparent 34%),radial-gradient(circle at 8% 75%,rgba(0,255,255,.08),transparent 38%),#07000e;font-family:"Cascadia Mono","Cascadia Code",Consolas,monospace;box-sizing:border-box}.tm-page *{box-sizing:border-box}.tm-header{display:flex;justify-content:space-between;gap:24px;align-items:flex-start;padding-bottom:18px;border-bottom:1px solid rgba(0,255,255,.42)}.tm-kicker{color:#00ffff;font-size:9px;font-weight:900;letter-spacing:.15em}.tm-header h1{margin:7px 0 8px;font-size:clamp(26px,3vw,40px)}.tm-header p{max-width:760px;margin:0;color:#aa98ba;font-size:10px;line-height:1.65}.tm-live{display:flex;align-items:center;gap:8px;min-height:28px;padding:0 9px;border:1px solid rgba(0,255,255,.38);color:#8fffee;font-size:8px;font-weight:900;letter-spacing:.08em}.tm-live-dot{width:7px;height:7px;background:#00ffff;box-shadow:0 0 10px #00ffff}.tm-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:12px;margin-top:15px}.tm-panel{min-width:0;border:1px solid rgba(0,255,255,.30);background:linear-gradient(180deg,rgba(29,5,47,.82),rgba(7,0,15,.94))}.tm-panel.wide{grid-column:1/-1}.tm-panel-head{display:flex;justify-content:space-between;gap:12px;align-items:center;padding:11px 13px;border-bottom:1px solid rgba(0,255,255,.22)}.tm-panel-head h2{margin:0;font-size:13px}.tm-source{color:#7e6a8b;font-size:7px;overflow-wrap:anywhere;text-align:right}.tm-metrics{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px;padding:12px}.tm-metric{min-width:0;padding:10px;border:1px solid rgba(102,72,120,.50);background:rgba(0,0,0,.26)}.tm-metric-label{color:#927da0;font-size:7px;letter-spacing:.07em;text-transform:uppercase}.tm-metric-value{margin-top:5px;font-size:18px;font-weight:850;overflow-wrap:anywhere}.tm-metric-meta{margin-top:4px;color:#7f6d8b;font-size:7px;line-height:1.45;overflow-wrap:anywhere}.tm-state{display:inline-flex;margin-top:7px;padding:3px 5px;border:1px solid rgba(117,255,226,.45);color:#8fffee;font-size:7px;font-weight:900;letter-spacing:.06em;text-transform:uppercase}.tm-state.stale{border-color:#ffd36b;color:#ffd36b}.tm-state.error{border-color:#ff3d7f;color:#ff7ba9}.tm-state.unavailable{border-color:#7b6888;color:#a795b3}.tm-gpu-list{display:grid;gap:10px;padding:12px}.tm-gpu{min-width:0;border:1px solid rgba(255,0,212,.26);background:rgba(12,0,20,.44)}.tm-gpu-title{display:flex;justify-content:space-between;gap:12px;padding:10px 11px;border-bottom:1px solid rgba(255,0,212,.20)}.tm-gpu-title strong{font-size:11px;overflow-wrap:anywhere}.tm-gpu-title span{color:#9d86aa;font-size:7px;text-align:right}.tm-empty{padding:24px 14px;color:#917f9e;font-size:9px;line-height:1.6}.tm-note{margin-top:12px;padding:10px 12px;border-left:2px solid #ff00d4;background:rgba(255,0,212,.045);color:#a995b8;font-size:8px;line-height:1.6}.tm-note strong{color:#f1d5ff}@media(max-width:980px){.tm-page{padding:22px 22px 92px}.tm-header{flex-direction:column}.tm-grid{grid-template-columns:1fr 1fr}.tm-panel.wide{grid-column:1/-1}}@media(max-width:650px){.tm-grid{grid-template-columns:1fr}.tm-panel.wide{grid-column:auto}.tm-metrics{grid-template-columns:1fr}.tm-live{align-self:flex-start}}@media(prefers-reduced-motion:reduce){.tm-page *,.tm-page *::before,.tm-page *::after{transition:none!important;animation:none!important}}
"#;

#[derive(Debug, Clone, Default)]
struct TelemetryUiState {
    hardware: Option<HardwareTelemetrySnapshot>,
    gpu: Option<GpuTelemetrySnapshot>,
    sample_count: u64,
}

type TelemetryStateSignal = Signal<TelemetryUiState, SyncStorage>;

struct TelemetryUiWorker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl TelemetryUiWorker {
    fn spawn(mut state: TelemetryStateSignal) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let mut hardware = WindowsHardwareTelemetryProvider::new(TELEMETRY_CADENCE);
            let mut gpu = NvidiaGpuTelemetryProvider::new();
            let mut sample_count = 0_u64;

            while !worker_stop.load(Ordering::Acquire) {
                let hardware = hardware.sample(None);
                let gpu = gpu.sample();
                sample_count = sample_count.saturating_add(1);
                state.set(TelemetryUiState {
                    hardware: Some(hardware),
                    gpu: Some(gpu),
                    sample_count,
                });
                thread::park_timeout(TELEMETRY_CADENCE);
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for TelemetryUiWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
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

fn present_reading<T>(
    reading: &TelemetryReading<T>,
    format_live: impl FnOnce(&T) -> String,
) -> MetricPresentation {
    present_state(&reading.state, format_live)
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
            detail: "Current provider sample".to_owned(),
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

fn bytes(value: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    if value >= 1024 * 1024 * 1024 {
        format!("{:.2} GiB", value as f64 / GIB)
    } else {
        format!("{:.1} MiB", value as f64 / MIB)
    }
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

fn gpu_card(adapter: &GpuAdapterTelemetry) -> Element {
    let title = adapter
        .identity
        .name
        .clone()
        .unwrap_or_else(|| format!("GPU {}", adapter.identity.index));
    let identity = adapter
        .identity
        .uuid
        .as_deref()
        .map(|uuid| format!("UUID {uuid}"))
        .unwrap_or_else(|| format!("volatile index {}", adapter.identity.index));

    let utilization = present_state(&adapter.gpu_utilization_percent.state, |value| {
        format!("{value}%")
    });
    let memory_used = present_state(&adapter.memory_used_bytes.state, |value| bytes(*value));
    let memory_total = present_state(&adapter.memory_total_bytes.state, |value| bytes(*value));
    let temperature = present_state(&adapter.temperature_celsius.state, |value| {
        format!("{value} °C")
    });
    let graphics_clock = present_state(&adapter.graphics_clock_mhz.state, |value| {
        format!("{value} MHz")
    });
    let power = present_state(&adapter.power_milliwatts.state, |value| {
        format!("{:.1} W", *value as f64 / 1000.0)
    });

    rsx! {
        article { class: "tm-gpu",
            div { class: "tm-gpu-title",
                strong { "{title}" }
                span { "{identity}" }
            }
            div { class: "tm-metrics",
                {metric_card("GPU UTIL", utilization, adapter.gpu_utilization_percent.source.api.clone())}
                {metric_card("VRAM USED", memory_used, adapter.memory_used_bytes.source.api.clone())}
                {metric_card("VRAM TOTAL", memory_total, adapter.memory_total_bytes.source.api.clone())}
                {metric_card("TEMPERATURE", temperature, adapter.temperature_celsius.source.api.clone())}
                {metric_card("GRAPHICS CLOCK", graphics_clock, adapter.graphics_clock_mhz.source.api.clone())}
                {metric_card("POWER", power, adapter.power_milliwatts.source.api.clone())}
            }
        }
    }
}

#[allow(non_snake_case)]
pub fn TelemetryView() -> Element {
    let state = use_signal_sync(TelemetryUiState::default);
    let _worker = use_hook(move || TelemetryUiWorker::spawn(state));
    let snapshot = state.read().clone();

    let hardware = snapshot.hardware.as_ref();
    let cpu = hardware.map(|item| {
        present_reading(&item.cpu.total_usage_percent, |value| format!("{value:.1}%"))
    });
    let ram_used = hardware.map(|item| {
        present_reading(&item.memory.used_physical_bytes, |value| bytes(*value))
    });
    let ram_available = hardware.map(|item| {
        present_reading(&item.memory.available_physical_bytes, |value| bytes(*value))
    });
    let logical = hardware.map(|item| {
        present_reading(&item.cpu.logical_processor_count, |value| value.to_string())
    });

    rsx! {
        style { dangerous_inner_html: TELEMETRY_UI_CSS }
        main { class: "tm-page",
            header { class: "tm-header",
                div {
                    div { class: "tm-kicker", "> LLAMAWAVE / TELEMETRY" }
                    h1 { "LIVE SYSTEM TELEMETRY" }
                    p { "Provider-backed Windows and NVIDIA telemetry. Missing, stale, and failed evidence is labelled explicitly; this view never converts unsupported data into zero." }
                }
                div { class: "tm-live",
                    span { class: "tm-live-dot" }
                    if snapshot.sample_count == 0 {
                        "STARTING PROVIDERS"
                    } else {
                        "1 HZ · SAMPLE {snapshot.sample_count}"
                    }
                }
            }

            div { class: "tm-grid",
                section { class: "tm-panel",
                    div { class: "tm-panel-head",
                        h2 { "CPU" }
                        span { class: "tm-source", "windows-native" }
                    }
                    div { class: "tm-metrics",
                        if let Some(metric) = cpu {
                            {metric_card("TOTAL USAGE", metric, hardware.unwrap().cpu.total_usage_percent.source.api.clone())}
                        } else {
                            div { class: "tm-empty", "Waiting for the first Windows CPU sample." }
                        }
                        if let Some(metric) = logical {
                            {metric_card("LOGICAL PROCESSORS", metric, hardware.unwrap().cpu.logical_processor_count.source.api.clone())}
                        }
                    }
                }

                section { class: "tm-panel",
                    div { class: "tm-panel-head",
                        h2 { "MEMORY" }
                        span { class: "tm-source", "windows-native" }
                    }
                    div { class: "tm-metrics",
                        if let Some(metric) = ram_used {
                            {metric_card("RAM USED", metric, hardware.unwrap().memory.used_physical_bytes.source.api.clone())}
                        } else {
                            div { class: "tm-empty", "Waiting for the first Windows memory sample." }
                        }
                        if let Some(metric) = ram_available {
                            {metric_card("RAM AVAILABLE", metric, hardware.unwrap().memory.available_physical_bytes.source.api.clone())}
                        }
                    }
                }

                section { class: "tm-panel",
                    div { class: "tm-panel-head",
                        h2 { "INFERENCE" }
                        span { class: "tm-source", "request-bound evidence" }
                    }
                    div { class: "tm-empty",
                        "No active inference stream is attached to this surface yet. Prompt/decode/TTFT/MTP values remain unavailable rather than inferred from unrelated counters."
                    }
                }

                section { class: "tm-panel wide",
                    div { class: "tm-panel-head",
                        h2 { "GPU ADAPTERS" }
                        span { class: "tm-source", "nvidia-nvml · capability-gated" }
                    }
                    if let Some(gpu) = snapshot.gpu.as_ref() {
                        if gpu.adapters.is_empty() {
                            div { class: "tm-empty", "No NVIDIA adapter telemetry is currently available. Unsupported provider state is preserved; no synthetic GPU is shown." }
                        } else {
                            div { class: "tm-gpu-list",
                                for adapter in &gpu.adapters {
                                    {gpu_card(adapter)}
                                }
                            }
                        }
                    } else {
                        div { class: "tm-empty", "Waiting for the first NVML provider sample." }
                    }
                }
            }

            div { class: "tm-note",
                strong { "TRUTHFULNESS CONTRACT · " }
                "Live values come from provider samples with source APIs attached. Stale values retain their last observation and reason. Error/unavailable states stay visible. Inference charts and alert presentation are intentionally not claimed by this foundation slice."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_and_error_states_never_present_as_zero() {
        let unavailable: TelemetryState<u32> = TelemetryState::Unavailable {
            reason: "provider unsupported".to_owned(),
        };
        let error: TelemetryState<u32> = TelemetryState::Error {
            message: "provider failed".to_owned(),
        };

        let unavailable = present_state(&unavailable, |value| value.to_string());
        let error = present_state(&error, |value| value.to_string());

        assert_eq!(unavailable.value, "Unavailable");
        assert_eq!(unavailable.state_label, "UNAVAILABLE");
        assert_eq!(error.value, "Error");
        assert_eq!(error.state_label, "ERROR");
    }

    #[test]
    fn stale_state_retains_last_value_and_reason() {
        let stale = TelemetryState::Stale {
            last_value: Some(73_u32),
            last_observed_at_unix_ms: Some(1234),
            reason: "provider disconnected".to_owned(),
        };
        let stale = present_state(&stale, |value| format!("{value} °C"));

        assert_eq!(stale.value, "73 °C");
        assert_eq!(stale.state_label, "STALE");
        assert!(stale.detail.contains("provider disconnected"));
        assert!(stale.detail.contains("1234"));
    }

    #[test]
    fn byte_formatter_is_human_readable_without_changing_evidence() {
        assert_eq!(bytes(1024 * 1024 * 1024), "1.00 GiB");
        assert_eq!(bytes(512 * 1024 * 1024), "512.0 MiB");
    }
}
