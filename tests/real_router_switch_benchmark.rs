#![cfg(windows)]

use std::{
    env, fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::AtomicBool,
    thread,
    time::{Duration, Instant},
};

use llamamanager::{
    llama::{inspect_installation, sha256_file},
    model_library::scan_root,
    model_store::ModelStore,
    persistence::Database,
    router::{RouterModelPhase, RouterRegistry, RouterRole, discover_router_registry},
    router_operations::{RouterOperationCancellation, RouterOperationController},
    router_switch_benchmark::{
        ActiveRequestEvictionExercise, RouterSwitchBenchmarkConfig, RouterSwitchBenchmarkOutcome,
        RouterSwitchBenchmarkStore, TargetHistoryState, compare_switch_runs, measure_first_token,
        run_switch_round_trip,
    },
    server_readiness::ServerEndpoint,
};
use serde_json::json;
use tempfile::tempdir;
use walkdir::WalkDir;

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required environment variable {name} is missing"))
}

fn free_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral loopback port");
    listener.local_addr().expect("read local address").port()
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create ASCII benchmark runtime directory");
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.expect("walk pinned benchmark runtime");
        let relative = entry
            .path()
            .strip_prefix(source)
            .expect("derive benchmark runtime relative path");
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).expect("create benchmark runtime subdirectory");
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).expect("create benchmark runtime file parent");
            }
            fs::copy(entry.path(), &target).expect("copy pinned benchmark runtime file");
        }
    }
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn model_id_for_path(registry: &RouterRegistry, path: &Path) -> String {
    let expected = normalized_path(path);
    registry
        .models
        .iter()
        .find(|model| {
            model
                .status
                .args
                .iter()
                .any(|arg| arg.replace('/', "\\").to_ascii_lowercase() == expected)
        })
        .unwrap_or_else(|| {
            panic!(
                "real router registry did not expose exact argv identity for {}: {:?}",
                path.display(),
                registry
                    .models
                    .iter()
                    .map(|model| (&model.id, &model.status.args))
                    .collect::<Vec<_>>()
            )
        })
        .id
        .clone()
}

fn is_ready(phase: &RouterModelPhase) -> bool {
    matches!(phase, RouterModelPhase::Loaded | RouterModelPhase::Sleeping)
}

#[test]
#[ignore = "requires pinned real Windows llama.cpp binaries and two published GGUF models"]
fn validates_real_a_b_a_switch_timing_persistence_comparison_and_failure_recovery() {
    let source_llama_root = PathBuf::from(required_env("LLAMAMANAGER_REAL_LLAMA_ROOT"));
    let source_model_a = PathBuf::from(required_env("LLAMAMANAGER_REAL_MODEL"));
    let source_model_b = PathBuf::from(required_env("LLAMAMANAGER_REAL_MODEL_V2"));
    let expected_model_a_sha = required_env("LLAMAMANAGER_REAL_MODEL_SHA256");
    let expected_model_b_sha = required_env("LLAMAMANAGER_REAL_MODEL_V2_SHA256");
    let evidence_dir = PathBuf::from(required_env("LLAMAMANAGER_REAL_EVIDENCE_DIR"));
    fs::create_dir_all(&evidence_dir).unwrap();

    // b10472 on Windows mangles Unicode paths while spawning router child servers. Preserve the
    // exact pinned runtime identity but execute from an ASCII temporary clone so this test measures
    // switch semantics rather than the upstream path-encoding limitation.
    let test_temp = tempdir().expect("create temporary router switch benchmark workspace");
    let llama_root = test_temp.path().join("router benchmark runtime");
    copy_tree(&source_llama_root, &llama_root);

    let source_installation =
        inspect_installation(&source_llama_root).expect("inspect source pinned llama.cpp runtime");
    let installation =
        inspect_installation(&llama_root).expect("inspect ASCII pinned llama.cpp runtime clone");
    let source_server = source_installation
        .server
        .as_ref()
        .expect("source pinned runtime must expose llama-server.exe");
    let server = installation
        .server
        .as_ref()
        .expect("ASCII runtime clone must expose llama-server.exe");
    assert_eq!(
        server.sha256, source_server.sha256,
        "benchmark runtime clone must preserve selected llama-server identity"
    );

    let model_root = test_temp.path().join("Router Benchmark Models");
    fs::create_dir_all(&model_root).expect("create ASCII router benchmark model directory");
    let model_a = model_root.join("stories 15M benchmark A.gguf");
    let model_b = model_root.join("TinyLlama benchmark B.gguf");
    fs::copy(&source_model_a, &model_a).expect("copy primary benchmark GGUF");
    fs::copy(&source_model_b, &model_b).expect("copy secondary benchmark GGUF");
    assert_eq!(
        sha256_file(&model_a).expect("hash benchmark model A"),
        expected_model_a_sha,
        "benchmark model A must preserve published identity"
    );
    assert_eq!(
        sha256_file(&model_b).expect("hash benchmark model B"),
        expected_model_b_sha,
        "benchmark model B must preserve published identity"
    );

    let database_path = test_temp.path().join("router-switch-benchmark.sqlite");
    Database::open(&database_path).expect("initialize base persistence schema");
    let model_store =
        ModelStore::open(&database_path).expect("initialize M2 model library schema");
    let scan = scan_root(&model_store, &model_root, &AtomicBool::new(false), |_| {})
        .expect("scan real switch benchmark models into M2 library");
    assert_eq!(scan.progress.errors, 0, "real model scan must be clean");
    assert!(
        scan.progress.models_saved + scan.progress.reused_unchanged >= 2,
        "switch benchmark validation requires two M2-backed models"
    );

    let port = free_loopback_port();
    let endpoint = ServerEndpoint::loopback(port);
    let argv = vec![
        "--host".to_string(),
        endpoint.host.clone(),
        "--port".to_string(),
        port.to_string(),
        "--models-dir".to_string(),
        model_root.to_string_lossy().to_string(),
        "--models-max".to_string(),
        "1".to_string(),
    ];

    let child = Command::new(&server.path)
        .args(&argv)
        .current_dir(&llama_root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start pinned llama-server in router mode for switch benchmark");
    let mut child = ChildGuard(child);

    let discovery_deadline = Instant::now() + Duration::from_secs(30);
    let initial = loop {
        if let Some(status) = child
            .0
            .try_wait()
            .expect("inspect real router process during benchmark startup")
        {
            panic!("real llama-server router exited before switch benchmark: {status}");
        }

        match discover_router_registry(
            &installation,
            &endpoint,
            Some(&model_store),
            Duration::from_secs(2),
        ) {
            Ok(registry) if registry.models.len() >= 2 => break registry,
            Ok(_) | Err(_) if Instant::now() < discovery_deadline => {
                thread::sleep(Duration::from_millis(200));
            }
            Ok(registry) => panic!(
                "real router registry did not expose two benchmark models before timeout: {:?}",
                registry
                    .models
                    .iter()
                    .map(|model| &model.id)
                    .collect::<Vec<_>>()
            ),
            Err(error) => panic!("real router discovery failed before switch benchmark: {error}"),
        }
    };

    assert_eq!(initial.role, RouterRole::Router);
    let model_a_id = model_id_for_path(&initial, &model_a);
    let model_b_id = model_id_for_path(&initial, &model_b);
    assert_ne!(model_a_id, model_b_id);

    let mut config = RouterSwitchBenchmarkConfig::new(&model_a_id, &model_b_id);
    config.timeout = Duration::from_secs(120);
    config.router_settings = argv.clone();

    let benchmark_store = RouterSwitchBenchmarkStore::open(&database_path)
        .expect("initialize switch benchmark persistence");

    let first_run = run_switch_round_trip(&installation, &endpoint, &model_store, &config);
    assert!(
        matches!(first_run.outcome, RouterSwitchBenchmarkOutcome::Succeeded),
        "first real A->B->A benchmark must succeed: {:?}",
        first_run.outcome
    );
    assert_eq!(first_run.legs.len(), 2);
    assert_eq!(first_run.legs[0].source_model, model_a_id);
    assert_eq!(first_run.legs[0].target_model, model_b_id);
    assert_eq!(first_run.legs[1].source_model, model_b_id);
    assert_eq!(first_run.legs[1].target_model, model_a_id);
    assert!(matches!(
        first_run.legs[0].cache.target_history,
        TargetHistoryState::FirstLoadInRun
    ));
    assert!(matches!(
        first_run.legs[1].cache.target_history,
        TargetHistoryState::PreviouslyLoadedInRun { prior_loads: 1 }
    ));
    assert!(!first_run.legs[0].cache.os_page_cache_known);
    assert!(!first_run.legs[1].cache.os_page_cache_known);
    assert!(
        first_run
            .legs
            .iter()
            .all(|leg| leg.timings.first_token_ms.is_some())
    );
    assert_eq!(first_run.envelope.server_sha256, server.sha256);
    assert_eq!(first_run.envelope.model_a_sha256, expected_model_a_sha);
    assert_eq!(first_run.envelope.model_b_sha256, expected_model_b_sha);
    assert_eq!(first_run.envelope.router_settings, argv);
    assert!(matches!(
        first_run.active_request_eviction,
        ActiveRequestEvictionExercise::UnsupportedBySelectedRuntime { .. }
    ));
    benchmark_store
        .save(&first_run)
        .expect("persist first real switch benchmark");
    assert_eq!(
        benchmark_store.get(&first_run.id).unwrap(),
        Some(first_run.clone())
    );

    let second_run = run_switch_round_trip(&installation, &endpoint, &model_store, &config);
    assert!(
        matches!(second_run.outcome, RouterSwitchBenchmarkOutcome::Succeeded),
        "second real A->B->A benchmark must succeed: {:?}",
        second_run.outcome
    );
    benchmark_store
        .save(&second_run)
        .expect("persist second real switch benchmark");
    let comparison = compare_switch_runs(&first_run, &second_run)
        .expect("identical real benchmark envelopes must be comparable");
    let comparable = benchmark_store
        .comparable_runs(&first_run.envelope)
        .expect("load exact-envelope switch benchmark history");
    assert_eq!(comparable.len(), 2);

    // Exercise a real failed B load after the successful timing samples. Corrupting B after the
    // M2 scan keeps the persisted compatibility/identity envelope stable while forcing the actual
    // router child load to fail. The failure must stay failed, then A must be recoverable.
    fs::write(&model_b, b"not a valid gguf")
        .expect("corrupt benchmark model B for recovery validation");
    let controller = RouterOperationController::new();
    let cancellation = RouterOperationCancellation::new();
    let failed_b = controller
        .load_model(
            &installation,
            &endpoint,
            &model_store,
            &model_b_id,
            Duration::from_secs(60),
            &cancellation,
        )
        .expect_err("corrupt real model B load must fail");
    let post_failure_registry = discover_router_registry(
        &installation,
        &endpoint,
        Some(&model_store),
        Duration::from_secs(5),
    )
    .expect("capture truthful router registry after failed B load");
    let failed_b_state = post_failure_registry
        .models
        .iter()
        .find(|model| model.id == model_b_id)
        .expect("failed B must remain represented in registry");
    assert!(
        failed_b_state.status.failed
            || matches!(failed_b_state.status.phase, RouterModelPhase::Unloaded),
        "failed B load must not become fake-ready: {:?}",
        failed_b_state.status
    );

    let recovery = controller
        .load_model(
            &installation,
            &endpoint,
            &model_store,
            &model_a_id,
            Duration::from_secs(120),
            &RouterOperationCancellation::new(),
        )
        .expect("baseline model A must recover after failed B load");
    let recovered_a = recovery
        .registry
        .models
        .iter()
        .find(|model| model.id == model_a_id)
        .expect("recovered A must exist in registry");
    assert!(is_ready(&recovered_a.status.phase));
    assert!(!recovered_a.status.failed);
    let recovery_first_token_ms = measure_first_token(
        &endpoint,
        &model_a_id,
        "Reply with OK",
        Duration::from_secs(30),
    )
    .expect("recovered A must produce a real first token");

    let evidence = json!({
        "github_sha": env::var("GITHUB_SHA").ok(),
        "runner_os": env::var("RUNNER_OS").ok(),
        "llama_release_tag": env::var("LLAMAMANAGER_LLAMA_RELEASE_TAG").ok(),
        "source_runtime_root_unicode_path": source_llama_root,
        "runtime_root": llama_root,
        "source_server_path": source_server.path,
        "server_path": server.path,
        "server_sha256": server.sha256,
        "router_argv": config.router_settings,
        "source_model_a_unicode_path": source_model_a,
        "source_model_b_unicode_path": source_model_b,
        "model_a": model_a,
        "model_b": model_b,
        "model_a_id": model_a_id,
        "model_b_id": model_b_id,
        "first_run": first_run,
        "second_run": second_run,
        "comparison": comparison,
        "failed_b_error": failed_b.to_string(),
        "post_failure_registry": post_failure_registry,
        "recovery": recovery,
        "recovery_first_token_ms": recovery_first_token_ms,
        "active_request_eviction_exercised": false,
        "active_request_eviction_reason": "pinned b10472 does not expose active-request count in /models; #41 records the gate as unsupported rather than manufacturing concurrent-request evidence",
        "upstream_unicode_path_workaround": "runtime and GGUF bytes were copied to ASCII temporary paths after exact SHA-256 identity verification because pinned b10472 Windows router child spawning does not preserve Unicode paths"
    });
    fs::write(
        evidence_dir.join("router-switch-benchmark.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();

    child
        .0
        .kill()
        .expect("stop real router after switch benchmark validation");
    let status = child.0.wait().expect("wait for stopped real router");
    assert!(
        !status.success(),
        "explicit test cleanup should terminate router"
    );
}
