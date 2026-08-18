#![cfg(windows)]

use std::{
    env, fs,
    hint::black_box,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use llamamanager::{
    benchmark::run_default_benchmark,
    gguf::inspect_gguf,
    gpu_telemetry::{GpuTelemetryProvider, NvidiaGpuTelemetryProvider},
    hardware_telemetry::{HardwareTelemetryProvider, WindowsHardwareTelemetryProvider},
    llama::inspect_installation,
    telemetry_overhead::{
        OverheadBudget, OverheadPhase, PollTimingSample, TelemetryOverheadMeasurement,
        TelemetryOverheadRecorder, capture_current_process_resources,
    },
};
use serde::Serialize;
use serde_json::json;

const DEFAULT_PHASE_SECONDS: u64 = 10;
const MIN_PHASE_SECONDS: u64 = 3;
const MAX_PHASE_SECONDS: u64 = 120;
const POLL_CADENCE: Duration = Duration::from_secs(1);

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required environment variable {name} is missing"))
}

fn phase_duration() -> Duration {
    let seconds = env::var("LLAMAMANAGER_TELEMETRY_OVERHEAD_SECONDS")
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("LLAMAMANAGER_TELEMETRY_OVERHEAD_SECONDS must be u64"))
        })
        .unwrap_or(DEFAULT_PHASE_SECONDS)
        .clamp(MIN_PHASE_SECONDS, MAX_PHASE_SECONDS);
    Duration::from_secs(seconds)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn run_monitor_phase(
    phase: OverheadPhase,
    duration: Duration,
    budget: OverheadBudget,
) -> TelemetryOverheadMeasurement {
    let mut hardware = WindowsHardwareTelemetryProvider::new(POLL_CADENCE);
    let mut gpu = NvidiaGpuTelemetryProvider::new();
    let start_resources = capture_current_process_resources().unwrap();
    let mut recorder =
        TelemetryOverheadRecorder::new(phase, POLL_CADENCE, budget, start_resources).unwrap();
    let phase_started = Instant::now();

    while phase_started.elapsed() < duration {
        let poll_started_at_unix_ms = now_unix_ms();
        let poll_started = Instant::now();
        let hardware_snapshot = hardware.sample(None);
        let gpu_snapshot = gpu.sample();
        black_box(&hardware_snapshot);
        black_box(&gpu_snapshot);
        let poll_duration = poll_started.elapsed();
        let resources = capture_current_process_resources().unwrap();
        recorder
            .record_poll(
                PollTimingSample::from_duration(poll_started_at_unix_ms, poll_duration),
                resources,
            )
            .unwrap();

        let remaining = duration.saturating_sub(phase_started.elapsed());
        if remaining.is_zero() {
            break;
        }
        thread::sleep(POLL_CADENCE.saturating_sub(poll_started.elapsed()).min(remaining));
    }

    recorder
        .finish(capture_current_process_resources().unwrap())
        .unwrap()
}

#[derive(Debug, Serialize)]
struct RealOverheadEvidence {
    schema: &'static str,
    github_sha: Option<String>,
    runner_os: Option<String>,
    phase_seconds: u64,
    polling_cadence_ms: u64,
    llama_release_tag: Option<String>,
    bench_sha256: String,
    model_path: PathBuf,
    model_sha256: String,
    budget_policy: &'static str,
    idle: TelemetryOverheadMeasurement,
    active_inference: TelemetryOverheadMeasurement,
    completed_benchmark_runs: usize,
}

#[test]
#[ignore = "requires real Windows llama.cpp + GGUF and intentionally measures idle/active telemetry overhead"]
fn measures_real_telemetry_overhead_idle_and_under_inference() {
    let llama_root = PathBuf::from(required_env("LLAMAMANAGER_REAL_LLAMA_ROOT"));
    let model_path = PathBuf::from(required_env("LLAMAMANAGER_REAL_BENCH_MODEL"));
    let evidence_dir = PathBuf::from(required_env("LLAMAMANAGER_REAL_EVIDENCE_DIR"));
    fs::create_dir_all(&evidence_dir).unwrap();

    let installation = inspect_installation(&llama_root).unwrap();
    let model = inspect_gguf(&model_path).unwrap();
    let bench = installation
        .bench
        .as_ref()
        .expect("real overhead validation requires llama-bench.exe");
    let duration = phase_duration();
    let budget = OverheadBudget::default();

    let idle = run_monitor_phase(OverheadPhase::Idle, duration, budget);

    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker_installation = installation.clone();
    let worker_model = model.clone();
    let benchmark_worker = thread::spawn(move || -> Result<usize, String> {
        let mut completed = 0_usize;
        loop {
            let run = run_default_benchmark(&worker_installation, &worker_model)
                .map_err(|error| format!("active inference benchmark failed: {error}"))?;
            if run.samples.is_empty() {
                return Err("active inference benchmark produced zero samples".to_owned());
            }
            completed += 1;
            if worker_stop.load(Ordering::Acquire) {
                return Ok(completed);
            }
        }
    });

    let active_inference = run_monitor_phase(OverheadPhase::ActiveInference, duration, budget);
    stop.store(true, Ordering::Release);
    let completed_benchmark_runs = benchmark_worker
        .join()
        .expect("active inference benchmark worker panicked")
        .unwrap();

    let evidence = RealOverheadEvidence {
        schema: "llamamanager.telemetry-overhead.v1",
        github_sha: env::var("GITHUB_SHA").ok(),
        runner_os: env::var("RUNNER_OS").ok(),
        phase_seconds: duration.as_secs(),
        polling_cadence_ms: u64::try_from(POLL_CADENCE.as_millis()).unwrap(),
        llama_release_tag: env::var("LLAMAMANAGER_LLAMA_RELEASE_TAG").ok(),
        bench_sha256: bench.sha256.clone(),
        model_path: model.path.clone(),
        model_sha256: model.sha256.clone(),
        budget_policy: "v1 engineering target: idle <=1% total host CPU, active <=2%, peak private growth <=64 MiB, p95 polling <=25% of cadence",
        idle,
        active_inference,
        completed_benchmark_runs,
    };

    let evidence_path = evidence_dir.join("telemetry-overhead.json");
    fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "evidence": evidence_path,
            "idle_cpu_percent_total_capacity": evidence.idle.process_cpu_percent_total_capacity,
            "idle_peak_private_growth_bytes": evidence.idle.peak_private_growth_bytes,
            "idle_p95_poll_ms": evidence.idle.p95_poll_duration_ms,
            "idle_within_budget": evidence.idle.assessment.within_budget,
            "active_cpu_percent_total_capacity": evidence.active_inference.process_cpu_percent_total_capacity,
            "active_peak_private_growth_bytes": evidence.active_inference.peak_private_growth_bytes,
            "active_p95_poll_ms": evidence.active_inference.p95_poll_duration_ms,
            "active_within_budget": evidence.active_inference.assessment.within_budget,
            "completed_benchmark_runs": evidence.completed_benchmark_runs,
        }))
        .unwrap()
    );

    assert!(
        evidence.completed_benchmark_runs > 0,
        "active inference phase must complete at least one real llama-bench run"
    );
    assert!(
        evidence.idle.assessment.within_budget,
        "idle telemetry overhead exceeded the documented budget: {:?}",
        evidence.idle.assessment.violations
    );
    assert!(
        evidence.active_inference.assessment.within_budget,
        "active-inference telemetry overhead exceeded the documented budget: {:?}",
        evidence.active_inference.assessment.violations
    );
}
