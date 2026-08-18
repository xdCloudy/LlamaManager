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
    router::RouterModelPhase,
    router_management::{
        PreferredModelVerification, RouterControlPreferences, verify_preferred_model,
    },
    router_observability::{
        RouterObservabilitySnapshot, RouterObservabilityTracker, RouterSnapshotFreshness,
        discover_router_observability,
    },
    router_operations::{RouterOperationCancellation, RouterOperationController},
    server_readiness::ServerEndpoint,
};
use serde_json::json;
use tempfile::tempdir;
use walkdir::WalkDir;

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn spawn(server: &Path, cwd: &Path, argv: &[String]) -> Self {
        let child = Command::new(server)
            .args(argv)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start pinned llama-server in router mode");
        Self(Some(child))
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("router child must be running")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.stop();
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
    fs::create_dir_all(destination).expect("create ASCII runtime directory");
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.expect("walk pinned runtime");
        let relative = entry
            .path()
            .strip_prefix(source)
            .expect("derive runtime relative path");
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).expect("create runtime subdirectory");
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).expect("create runtime file parent");
            }
            fs::copy(entry.path(), target).expect("copy pinned runtime file");
        }
    }
}

fn wait_for_snapshot(
    child: &mut ChildGuard,
    installation: &llamamanager::llama::LlamaInstallation,
    endpoint: &ServerEndpoint,
    store: &ModelStore,
) -> RouterObservabilitySnapshot {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child
            .child_mut()
            .try_wait()
            .expect("inspect router process during reconciliation")
        {
            panic!("real router exited before reconciliation: {status}");
        }
        match discover_router_observability(
            installation,
            endpoint,
            Some(store),
            Duration::from_secs(2),
        ) {
            Ok(snapshot) if !snapshot.models.is_empty() => return snapshot,
            Ok(_) | Err(_) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(200));
            }
            Ok(snapshot) => panic!(
                "real router returned no models before timeout: {:?}",
                snapshot.registry.models
            ),
            Err(error) => panic!("real router reconciliation timed out: {error}"),
        }
    }
}

fn ready(phase: &RouterModelPhase) -> bool {
    matches!(phase, RouterModelPhase::Loaded | RouterModelPhase::Sleeping)
}

#[test]
#[ignore = "requires pinned real Windows llama.cpp binaries and published GGUF model"]
fn validates_preferred_model_restart_and_reconnect_reconciliation() {
    let source_runtime = PathBuf::from(required_env("LLAMAMANAGER_REAL_LLAMA_ROOT"));
    let source_model = PathBuf::from(required_env("LLAMAMANAGER_REAL_BENCH_MODEL"));
    let expected_model_sha = required_env("LLAMAMANAGER_REAL_MODEL_SHA256");
    let evidence_dir = PathBuf::from(required_env("LLAMAMANAGER_REAL_EVIDENCE_DIR"));
    fs::create_dir_all(&evidence_dir).unwrap();

    let temp = tempdir().expect("create router-management workspace");
    let runtime_root = temp.path().join("router runtime ascii");
    copy_tree(&source_runtime, &runtime_root);
    let installation = inspect_installation(&runtime_root).expect("inspect ASCII pinned runtime");
    let server = installation
        .server
        .as_ref()
        .expect("pinned runtime must expose llama-server.exe");

    let model_root = temp.path().join("Router Models with spaces");
    fs::create_dir_all(&model_root).unwrap();
    let model_path = model_root.join("preferred model.gguf");
    fs::copy(&source_model, &model_path).expect("copy published model into ASCII router fixture");
    assert_eq!(
        sha256_file(&model_path).expect("hash router-management model"),
        expected_model_sha,
        "router-management fixture must preserve published model identity"
    );

    let database_path = temp.path().join("router-management.sqlite");
    Database::open(&database_path).expect("initialize persistence schema");
    let store = ModelStore::open(&database_path).expect("open M2 model store");
    let scan = scan_root(&store, &model_root, &AtomicBool::new(false), |_| {})
        .expect("scan router-management model into M2 library");
    assert_eq!(scan.progress.errors, 0);

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

    let mut first_process = ChildGuard::spawn(&server.path, &runtime_root, &argv);
    let first_snapshot = wait_for_snapshot(&mut first_process, &installation, &endpoint, &store);
    let model_id = first_snapshot.models[0].model.id.clone();
    let preferences = RouterControlPreferences {
        preferred_model: Some(model_id.clone()),
        ..RouterControlPreferences::default()
    };
    let mut tracker = RouterObservabilityTracker::default();
    tracker.reconcile(Ok(first_snapshot));
    assert_eq!(tracker.freshness(), RouterSnapshotFreshness::Live);

    let controller = RouterOperationController::new();
    let cancellation = RouterOperationCancellation::new();
    let loaded = controller
        .load_model(
            &installation,
            &endpoint,
            &store,
            &model_id,
            Duration::from_secs(120),
            &cancellation,
        )
        .expect("load preferred model through real router controller");
    assert!(
        loaded
            .registry
            .models
            .iter()
            .find(|model| model.id == model_id)
            .is_some_and(|model| ready(&model.status.phase))
    );
    tracker.reconcile(
        discover_router_observability(
            &installation,
            &endpoint,
            Some(&store),
            Duration::from_secs(5),
        )
        .map_err(|error| error.to_string()),
    );
    let before_restart = verify_preferred_model(&preferences, &tracker);
    assert!(matches!(
        before_restart,
        PreferredModelVerification::Verified { .. }
            | PreferredModelVerification::Unsupported { .. }
    ));

    first_process.stop();
    tracker.reconcile(Err("router process stopped for restart validation".into()));
    assert_eq!(tracker.freshness(), RouterSnapshotFreshness::Stale);
    assert_eq!(
        verify_preferred_model(&preferences, &tracker),
        PreferredModelVerification::NeedsLiveReconciliation,
        "persisted preference must not keep a stale ready claim after disconnect"
    );

    let mut second_process = ChildGuard::spawn(&server.path, &runtime_root, &argv);
    let restarted_snapshot = wait_for_snapshot(&mut second_process, &installation, &endpoint, &store);
    assert!(
        restarted_snapshot
            .models
            .iter()
            .any(|model| model.model.id == model_id),
        "router identity must reconcile back to the persisted preferred target"
    );
    tracker.reconcile(Ok(restarted_snapshot));
    assert_eq!(tracker.freshness(), RouterSnapshotFreshness::Live);
    let after_restart = verify_preferred_model(&preferences, &tracker);
    assert!(
        !matches!(after_restart, PreferredModelVerification::NeedsLiveReconciliation),
        "fresh restart evidence must replace the stale snapshot"
    );

    let restart_phase = tracker
        .current
        .as_ref()
        .and_then(|snapshot| snapshot.registry.models.iter().find(|model| model.id == model_id))
        .map(|model| format!("{:?}", model.status.phase));
    let evidence = json!({
        "github_sha": env::var("GITHUB_SHA").ok(),
        "runner_os": env::var("RUNNER_OS").ok(),
        "llama_release_tag": env::var("LLAMAMANAGER_LLAMA_RELEASE_TAG").ok(),
        "server_path": server.path,
        "server_sha256": server.sha256,
        "router_argv": argv,
        "preferred_model": model_id,
        "before_restart_verification": format!("{before_restart:?}"),
        "disconnect_freshness": "stale",
        "disconnect_verification": "needs_live_reconciliation",
        "after_restart_verification": format!("{after_restart:?}"),
        "after_restart_phase": restart_phase,
        "dynamic_default_model_mutation_supported": false,
        "dynamic_default_model_reason": "pinned b10472 exposes startup policy through CLI/props but no dynamic default-model mutation route; the UI therefore persists only a local preferred target and requires live post-restart reconciliation before readiness is claimed"
    });
    fs::write(
        evidence_dir.join("router-management.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();

    second_process.stop();
}