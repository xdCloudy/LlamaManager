use llamamanager::{
    gpu_telemetry::{GpuTelemetryProvider, GpuTelemetryUnit, NvidiaGpuTelemetryProvider},
    hardware_telemetry::TelemetryState,
};

#[test]
fn gpu_provider_preserves_units_and_live_value_ranges() {
    let mut provider = NvidiaGpuTelemetryProvider::new();
    let snapshot = provider.sample();

    match snapshot.provider_adapter_count {
        TelemetryState::Live { value } => {
            assert!(value as usize >= snapshot.adapters.len());
        }
        TelemetryState::Unavailable { .. }
        | TelemetryState::Error { .. }
        | TelemetryState::Stale { .. } => {}
    }

    for adapter in &snapshot.adapters {
        assert_eq!(
            adapter.gpu_utilization_percent.unit,
            GpuTelemetryUnit::Percent
        );
        assert_eq!(
            adapter.memory_utilization_percent.unit,
            GpuTelemetryUnit::Percent
        );
        assert_eq!(adapter.memory_used_bytes.unit, GpuTelemetryUnit::Bytes);
        assert_eq!(adapter.memory_total_bytes.unit, GpuTelemetryUnit::Bytes);
        assert_eq!(
            adapter.temperature_celsius.unit,
            GpuTelemetryUnit::Celsius
        );
        assert_eq!(
            adapter.graphics_clock_mhz.unit,
            GpuTelemetryUnit::Megahertz
        );
        assert_eq!(
            adapter.memory_clock_mhz.unit,
            GpuTelemetryUnit::Megahertz
        );
        assert_eq!(adapter.power_milliwatts.unit, GpuTelemetryUnit::Milliwatts);

        if let Some(value) = adapter.gpu_utilization_percent.live_value() {
            assert!(*value <= 100, "GPU utilization exceeded 100%: {value}");
        }
        if let Some(value) = adapter.memory_utilization_percent.live_value() {
            assert!(*value <= 100, "memory utilization exceeded 100%: {value}");
        }
        if let (Some(used), Some(total)) = (
            adapter.memory_used_bytes.live_value(),
            adapter.memory_total_bytes.live_value(),
        ) {
            assert!(
                used <= total,
                "NVML reported used VRAM greater than total VRAM: {used} > {total}"
            );
        }

        assert_eq!(adapter.gpu_utilization_percent.source.provider, "nvidia-nvml");
        assert_eq!(adapter.memory_used_bytes.source.provider, "nvidia-nvml");
        assert_eq!(adapter.temperature_celsius.source.provider, "nvidia-nvml");
        assert_eq!(adapter.graphics_clock_mhz.source.provider, "nvidia-nvml");
        assert_eq!(adapter.power_milliwatts.source.provider, "nvidia-nvml");
    }
}
