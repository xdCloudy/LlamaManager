#![cfg(windows)]

use std::{env, fs, path::PathBuf, thread, time::Duration};

use llamamanager::hardware_telemetry::{
    HardwareTelemetryProvider, TelemetryState, WindowsHardwareTelemetryProvider,
};

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required environment variable {name} is missing"))
}

#[test]
#[ignore = "requires a real Windows runtime and writes hardware telemetry evidence"]
fn validates_real_windows_hardware_telemetry() {
    let evidence_dir = PathBuf::from(required_env("LLAMAMANAGER_REAL_EVIDENCE_DIR"));
    fs::create_dir_all(&evidence_dir).unwrap();

    let mut provider = WindowsHardwareTelemetryProvider::new(Duration::from_millis(300));
    let pid = std::process::id();

    let first = provider.sample(Some(pid));
    assert!(matches!(
        first.cpu.total_usage_percent.state,
        TelemetryState::Unavailable { .. }
    ));
    assert!(matches!(
        first.cpu.per_logical_processor_usage_percent.state,
        TelemetryState::Unavailable { .. }
    ));

    thread::sleep(provider.cadence().interval());
    let second = provider.sample(Some(pid));

    let total_cpu = match &second.cpu.total_usage_percent.state {
        TelemetryState::Live { value } => *value,
        other => panic!("expected live total CPU usage after the second sample, got {other:?}"),
    };
    assert!((0.0..=100.0).contains(&total_cpu));

    let per_cpu = match &second.cpu.per_logical_processor_usage_percent.state {
        TelemetryState::Live { value } => value,
        other => panic!("expected live per-CPU usage after the second sample, got {other:?}"),
    };
    assert!(!per_cpu.is_empty());
    assert!(per_cpu.iter().all(|value| (0.0..=100.0).contains(value)));

    let logical = match second.cpu.logical_processor_count.state {
        TelemetryState::Live { value } => value,
        ref other => panic!("expected live logical processor count, got {other:?}"),
    };
    let physical = match second.cpu.physical_core_count.state {
        TelemetryState::Live { value } => value,
        ref other => panic!("expected live physical core count, got {other:?}"),
    };
    assert!(logical > 0);
    assert!(physical > 0);
    assert!(physical <= logical);
    assert_eq!(per_cpu.len(), logical as usize);

    let total_memory = match second.memory.total_physical_bytes.state {
        TelemetryState::Live { value } => value,
        ref other => panic!("expected live total physical memory, got {other:?}"),
    };
    let available_memory = match second.memory.available_physical_bytes.state {
        TelemetryState::Live { value } => value,
        ref other => panic!("expected live available physical memory, got {other:?}"),
    };
    let used_memory = match second.memory.used_physical_bytes.state {
        TelemetryState::Live { value } => value,
        ref other => panic!("expected live used physical memory, got {other:?}"),
    };
    assert!(total_memory > 0);
    assert!(available_memory <= total_memory);
    assert_eq!(used_memory, total_memory - available_memory);

    let process = match &second.managed_process_memory.state {
        TelemetryState::Live { value } => value,
        other => panic!("expected live current-process memory evidence, got {other:?}"),
    };
    assert_eq!(process.pid, pid);
    assert!(process.working_set_bytes > 0);
    assert!(process.peak_working_set_bytes >= process.working_set_bytes);
    assert!(process.private_bytes > 0);

    fs::write(
        evidence_dir.join("hardware-telemetry.json"),
        serde_json::to_vec_pretty(&second).unwrap(),
    )
    .unwrap();
}
