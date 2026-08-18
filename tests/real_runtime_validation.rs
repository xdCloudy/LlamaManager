#![cfg(windows)]

use std::{env, fs, path::PathBuf};

use llamamanager::{
    benchmark::run_default_benchmark,
    error::LlamaManagerError,
    gguf::inspect_gguf,
    llama::inspect_installation,
    persistence::Database,
};
use serde_json::json;

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required environment variable {name} is missing"))
}

#[test]
#[ignore = "requires pinned real Windows llama.cpp binaries and a published GGUF model"]
fn validates_real_windows_runtime_end_to_end() {
    let llama_root = PathBuf::from(required_env("LLAMAMANAGER_REAL_LLAMA_ROOT"));
    let model_path = PathBuf::from(required_env("LLAMAMANAGER_REAL_MODEL"));
    let evidence_dir = PathBuf::from(required_env("LLAMAMANAGER_REAL_EVIDENCE_DIR"));
    let expected_model_sha = required_env("LLAMAMANAGER_REAL_MODEL_SHA256").to_ascii_lowercase();

    fs::create_dir_all(&evidence_dir).unwrap();

    let root_text = llama_root.to_string_lossy();
    let model_text = model_path.to_string_lossy();
    assert!(root_text.contains(' '), "runtime path must exercise spaces");
    assert!(root_text.contains('外'), "runtime path must exercise Unicode");
    assert!(model_text.contains(' '), "model path must exercise spaces");
    assert!(model_text.contains('模'), "model path must exercise Unicode");

    // #13: inspect a real, external llama.cpp installation and retain exact
    // executable identities plus upstream-produced help/version evidence.
    let installation = inspect_installation(&llama_root).unwrap();
    let server = installation
        .server
        .as_ref()
        .expect("real release must expose llama-server.exe");
    let bench = installation
        .bench
        .as_ref()
        .expect("real release must expose llama-bench.exe");
    assert_eq!(server.sha256.len(), 64);
    assert_eq!(bench.sha256.len(), 64);
    assert!(!server.help_output.trim().is_empty());
    assert!(!bench.help_output.trim().is_empty());
    assert!(!installation.capabilities.is_empty());

    // Missing and non-executable installations must fail rather than being
    // accepted as synthetic capability evidence.
    let negative_root = evidence_dir.join("negative runtime cases");
    fs::create_dir_all(&negative_root).unwrap();
    let empty_root = negative_root.join("empty");
    fs::create_dir_all(&empty_root).unwrap();
    assert!(matches!(
        inspect_installation(&empty_root),
        Err(LlamaManagerError::NoLlamaBinaries(_))
    ));

    let fake_root = negative_root.join("fake executable");
    fs::create_dir_all(&fake_root).unwrap();
    fs::write(fake_root.join("llama-bench.exe"), b"not a Windows executable").unwrap();
    assert!(inspect_installation(&fake_root).is_err());

    // #14: inspect a published GGUF rather than a generated test fixture and
    // pin its externally published identity.
    let model = inspect_gguf(&model_path).unwrap();
    assert_eq!(model.sha256.to_ascii_lowercase(), expected_model_sha);
    assert!((2..=3).contains(&model.gguf_version));
    assert!(model.tensor_count > 0);
    assert!(model.metadata_count > 0);
    assert!(model.architecture.is_some());

    let corrupt_path = evidence_dir.join("corrupt truncated 模型.gguf");
    fs::write(&corrupt_path, b"GGUF\x03\x00").unwrap();
    assert!(inspect_gguf(&corrupt_path).is_err());
    assert!(inspect_gguf(&evidence_dir.join("missing.gguf")).is_err());

    // #15: execute the actual upstream llama-bench binary through product code,
    // retain stdout/stderr/argv/exit status, then persist and reload history.
    let run = run_default_benchmark(&installation, &model).unwrap();
    assert_eq!(run.exit_code, Some(0));
    assert!(!run.arguments.is_empty());
    assert!(!run.stdout.trim().is_empty());
    assert!(!run.samples.is_empty());
    assert_eq!(run.bench_sha256, bench.sha256);
    assert_eq!(run.model_sha256, model.sha256);

    let db_path = evidence_dir.join("runtime-validation.sqlite");
    let database = Database::open(&db_path).unwrap();
    database.save_installation(&installation).unwrap();
    database.save_model(&model).unwrap();
    database.save_benchmark(&run).unwrap();
    drop(database);

    let reopened = Database::open(&db_path).unwrap();
    let persisted_installation = reopened.latest_installation().unwrap().unwrap();
    let persisted_model = reopened.latest_model().unwrap().unwrap();
    let history = reopened.recent_benchmarks(10).unwrap();
    assert_eq!(persisted_installation.id, installation.id);
    assert_eq!(persisted_model.id, model.id);
    assert!(history.iter().any(|item| item.id == run.id));

    // A real process failure must remain a typed failure. Pointing the same
    // valid model identity at a missing file causes upstream llama-bench to
    // return non-zero without requiring a fake executable.
    let mut missing_model = model.clone();
    missing_model.path = evidence_dir.join("does not exist 模型.gguf");
    assert!(matches!(
        run_default_benchmark(&installation, &missing_model),
        Err(LlamaManagerError::ProcessFailed { .. })
    ));

    fs::write(
        evidence_dir.join("installation.json"),
        serde_json::to_vec_pretty(&installation).unwrap(),
    )
    .unwrap();
    fs::write(
        evidence_dir.join("model.json"),
        serde_json::to_vec_pretty(&model).unwrap(),
    )
    .unwrap();
    fs::write(
        evidence_dir.join("benchmark-run.json"),
        serde_json::to_vec_pretty(&run).unwrap(),
    )
    .unwrap();

    let summary = json!({
        "github_sha": env::var("GITHUB_SHA").ok(),
        "runner_os": env::var("RUNNER_OS").ok(),
        "runner_image": env::var("ImageOS").ok(),
        "llama_release_tag": env::var("LLAMAMANAGER_LLAMA_RELEASE_TAG").ok(),
        "llama_archive_sha256": env::var("LLAMAMANAGER_LLAMA_ARCHIVE_SHA256").ok(),
        "runtime_root": llama_root,
        "server_sha256": server.sha256,
        "bench_sha256": bench.sha256,
        "detected_backend": installation.backend,
        "capability_count": installation.capabilities.len(),
        "model_path": model_path,
        "model_sha256": model.sha256,
        "gguf_version": model.gguf_version,
        "architecture": model.architecture,
        "tensor_count": model.tensor_count,
        "metadata_count": model.metadata_count,
        "benchmark_exit_code": run.exit_code,
        "benchmark_arguments": run.arguments,
        "benchmark_sample_count": run.samples.len(),
        "prompt_tps": run.prompt_tps(),
        "decode_tps": run.decode_tps(),
        "persistence_reopened": true,
        "negative_missing_installation_rejected": true,
        "negative_non_executable_rejected": true,
        "negative_corrupt_gguf_rejected": true,
        "negative_missing_gguf_rejected": true,
        "negative_benchmark_nonzero_typed": true
    });
    fs::write(
        evidence_dir.join("summary.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
}
