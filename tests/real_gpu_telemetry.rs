#![cfg(windows)]

use std::{env, fs, path::PathBuf};

use llamamanager::{
    gpu_telemetry::{GpuTelemetryProvider, GpuTelemetryUnit, NvidiaGpuTelemetryProvider},
    hardware_telemetry::TelemetryState,
};

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required environment variable {name} is missing"))
}

#[test]
#[ignore = "requires a real Windows NVIDIA GPU and writes GPU telemetry evidence"]
fn validates_real_nvidia_gpu_telemetry() {
    let evidence_dir = PathBuf::from(required_env("LLAMAMANAGER_REAL_EVIDENCE_DIR"));
    fs::create_dir_all(&evidence_dir).unwrap();

    let mut provider = NvidiaGpuTelemetryProvider::new();
    let snapshot = provider.sample();

    let reported_count = match &snapshot.provider_adapter_count {
        TelemetryState::Live { value } => *value,
        state => panic!("NVML provider did not become live on the NVIDIA host: {state:?}"),
    };
    assert!(reported_count > 0, "NVML reported zero NVIDIA adapters");
    assert!(
        !snapshot.adapters.is_empty(),
        "NVML reported adapters but returned no adapter telemetry"
    );

    let stable_adapter = snapshot
        .adapters
        .iter()
        .find(|adapter| adapter.identity.stable_for_evidence && adapter.identity.uuid.is_some())
        .expect("no NVIDIA adapter exposed a stable NVML UUID");

    assert_eq!(stable_adapter.identity.vendor, "nvidia");
    assert!(
        stable_adapter
            .identity
            .uuid
            .as_deref()
            .is_some_and(|uuid| !uuid.trim().is_empty()),
        "NVML UUID must be non-empty"
    );

    assert_eq!(
        stable_adapter.gpu_utilization_percent.unit,
        GpuTelemetryUnit::Percent
    );
    assert_eq!(
        stable_adapter.memory_utilization_percent.unit,
        GpuTelemetryUnit::Percent
    );
    assert_eq!(
        stable_adapter.memory_used_bytes.unit,
        GpuTelemetryUnit::Bytes
    );
    assert_eq!(
        stable_adapter.memory_total_bytes.unit,
        GpuTelemetryUnit::Bytes
    );
    assert_eq!(
        stable_adapter.temperature_celsius.unit,
        GpuTelemetryUnit::Celsius
    );
    assert_eq!(
        stable_adapter.graphics_clock_mhz.unit,
        GpuTelemetryUnit::Megahertz
    );
    assert_eq!(
        stable_adapter.memory_clock_mhz.unit,
        GpuTelemetryUnit::Megahertz
    );
    assert_eq!(
        stable_adapter.power_milliwatts.unit,
        GpuTelemetryUnit::Milliwatts
    );

    if let Some(value) = stable_adapter.gpu_utilization_percent.live_value() {
        assert!(*value <= 100, "GPU utilization exceeded 100%: {value}");
    }
    if let Some(value) = stable_adapter.memory_utilization_percent.live_value() {
        assert!(*value <= 100, "memory utilization exceeded 100%: {value}");
    }
    if let (Some(used), Some(total)) = (
        stable_adapter.memory_used_bytes.live_value(),
        stable_adapter.memory_total_bytes.live_value(),
    ) {
        assert!(
            used <= total,
            "NVML reported used VRAM greater than total VRAM: {used} > {total}"
        );
        assert!(*total > 0, "NVML total VRAM must be greater than zero");
    }

    let live_metric_count = [
        stable_adapter.gpu_utilization_percent.live_value().is_some(),
        stable_adapter
            .memory_utilization_percent
            .live_value()
            .is_some(),
        stable_adapter.memory_used_bytes.live_value().is_some(),
        stable_adapter.memory_total_bytes.live_value().is_some(),
        stable_adapter.temperature_celsius.live_value().is_some(),
        stable_adapter.graphics_clock_mhz.live_value().is_some(),
        stable_adapter.memory_clock_mhz.live_value().is_some(),
        stable_adapter.power_milliwatts.live_value().is_some(),
    ]
    .into_iter()
    .filter(|is_live| *is_live)
    .count();

    assert!(
        live_metric_count >= 2,
        "real NVIDIA validation requires at least two live NVML metrics; got {live_metric_count}"
    );

    fs::write(
        evidence_dir.join("gpu-telemetry.json"),
        serde_json::to_vec_pretty(&snapshot).unwrap(),
    )
    .unwrap();
}
