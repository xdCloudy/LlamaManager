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
    llama::inspect_installation,
    model_library::scan_root,
    model_store::ModelStore,
    persistence::Database,
    router::{RouterModelPhase, RouterRegistry, RouterRole, discover_router_registry},
    router_operations::{
        RouterOperationCancellation, RouterOperationController, RouterOperationError,
        RouterOperationKind, RouterOperationState,
    },
    server_readiness::ServerEndpoint,
};
use serde_json::json;
use tempfile::tempdir;

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

fn model_phase<'a>(registry: &'a RouterRegistry, model_id: &str) -> &'a RouterModelPhase {
    &registry
        .models
        .iter()
        .find(|model| model.id == model_id)
        .unwrap_or_else(|| panic!("model {model_id} missing from reconciled registry"))
        .status
        .phase
}

fn is_ready(phase: &RouterModelPhase) -> bool {
    matches!(phase, RouterModelPhase::Loaded | RouterModelPhase::Sleeping)
}

#[test]
#[ignore = "requires pinned real Windows llama.cpp binaries and two published GGUF models"]
fn validates_real_router_load_unload_reload_preload_and_switch() {
    let llama_root = PathBuf::from(required_env("LLAMAMANAGER_REAL_LLAMA_ROOT"));
    let model_a = PathBuf::from(required_env("LLAMAMANAGER_REAL_MODEL"));
    let model_b = PathBuf::from(required_env("LLAMAMANAGER_REAL_MODEL_V2"));
    let evidence_dir = PathBuf::from(required_env("LLAMAMANAGER_REAL_EVIDENCE_DIR"));
    fs::create_dir_all(&evidence_dir).unwrap();

    let model_root = model_a
        .parent()
        .expect("primary model must have a containing directory");
    assert_eq!(
        model_b.parent(),
        Some(model_root),
        "real router operation models must share one --models-dir root"
    );

    let installation =
        inspect_installation(&llama_root).expect("inspect pinned llama.cpp installation");
    let server = installation
        .server
        .as_ref()
        .expect("pinned runtime must expose llama-server.exe");

    let library_temp = tempdir().expect("create temporary M2 model library");
    let database_path = library_temp.path().join("router-operations.sqlite");
    Database::open(&database_path).expect("initialize base persistence schema");
    let store = ModelStore::open(&database_path).expect("initialize M2 model library schema");
    let scan = scan_root(&store, model_root, &AtomicBool::new(false), |_| {})
        .expect("scan real router models into M2 library");
    assert_eq!(scan.progress.errors, 0, "real model scan must be clean");
    assert!(
        scan.progress.models_saved + scan.progress.reused_unchanged >= 2,
        "real operation validation requires two M2-backed models"
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
        .expect("start pinned llama-server in router mode");
    let mut child = ChildGuard(child);

    let discovery_deadline = Instant::now() + Duration::from_secs(30);
    let initial = loop {
        if let Some(status) = child
            .0
            .try_wait()
            .expect("inspect real router process during startup")
        {
            panic!("real llama-server router exited before operations: {status}");
        }

        match discover_router_registry(
            &installation,
            &endpoint,
            Some(&store),
            Duration::from_secs(2),
        ) {
            Ok(registry) if registry.models.len() >= 2 => break registry,
            Ok(_) if Instant::now() < discovery_deadline => {
                thread::sleep(Duration::from_millis(200));
            }
            Err(_) if Instant::now() < discovery_deadline => {
                thread::sleep(Duration::from_millis(200));
            }
            Ok(registry) => panic!(
                "real router registry did not expose two models before timeout: {:?}",
                registry
                    .models
                    .iter()
                    .map(|model| &model.id)
                    .collect::<Vec<_>>()
            ),
            Err(error) => panic!("real router discovery failed before operations: {error}"),
        }
    };

    assert_eq!(initial.role, RouterRole::Router);
    let model_a_id = model_id_for_path(&initial, &model_a);
    let model_b_id = model_id_for_path(&initial, &model_b);
    assert_ne!(model_a_id, model_b_id);

    let timeout = Duration::from_secs(120);
    let controller = RouterOperationController::new();
    let cancellation = RouterOperationCancellation::new();

    let reload = controller
        .reload_registry(
            &installation,
            &endpoint,
            Some(&store),
            timeout,
            &cancellation,
        )
        .expect("real GET /models?reload=1 must reconcile");
    assert_eq!(reload.kind, RouterOperationKind::ReloadRegistry);
    assert_eq!(reload.http_statuses, vec![200]);

    let load = controller
        .load_model(
            &installation,
            &endpoint,
            &store,
            &model_a_id,
            timeout,
            &cancellation,
        )
        .expect("real POST /models/load must reach ready state");
    assert!(is_ready(model_phase(&load.registry, &model_a_id)));
    assert_eq!(load.http_statuses, vec![200]);

    let unload = controller
        .unload_model(
            &installation,
            &endpoint,
            Some(&store),
            &model_a_id,
            timeout,
            &cancellation,
        )
        .expect("real POST /models/unload must reach unloaded state");
    assert!(matches!(
        model_phase(&unload.registry, &model_a_id),
        RouterModelPhase::Unloaded
    ));
    assert_eq!(unload.http_statuses, vec![200]);

    let preload = controller
        .preload_model(
            &installation,
            &endpoint,
            &store,
            &model_a_id,
            timeout,
            &cancellation,
        )
        .expect("real preload must use load semantics and reach ready state");
    assert!(is_ready(model_phase(&preload.registry, &model_a_id)));
    assert_eq!(preload.http_statuses, vec![200]);

    let switch = controller
        .switch_model(
            &installation,
            &endpoint,
            &store,
            &model_a_id,
            &model_b_id,
            timeout,
            &cancellation,
        )
        .expect("real switch must leave target ready after reconciliation");
    assert!(is_ready(model_phase(&switch.registry, &model_b_id)));
    assert!(
        switch.http_statuses.iter().all(|status| *status == 200),
        "all real switch mutations must return HTTP 200: {:?}",
        switch.http_statuses
    );

    let final_unload = controller
        .unload_model(
            &installation,
            &endpoint,
            Some(&store),
            &model_b_id,
            timeout,
            &cancellation,
        )
        .expect("real target unload must reconcile after switch");
    assert!(matches!(
        model_phase(&final_unload.registry, &model_b_id),
        RouterModelPhase::Unloaded
    ));

    let cancelled = RouterOperationCancellation::new();
    cancelled.cancel();
    let cancellation_error = controller
        .load_model(
            &installation,
            &endpoint,
            &store,
            &model_a_id,
            timeout,
            &cancelled,
        )
        .expect_err("pre-cancelled real operation must not mutate router state");
    assert!(matches!(
        cancellation_error,
        RouterOperationError::Cancelled
    ));
    assert!(matches!(
        controller.state(),
        RouterOperationState::Cancelled(_)
    ));

    let final_registry = discover_router_registry(
        &installation,
        &endpoint,
        Some(&store),
        Duration::from_secs(5),
    )
    .expect("capture final real router registry");

    let evidence = json!({
        "github_sha": env::var("GITHUB_SHA").ok(),
        "runner_os": env::var("RUNNER_OS").ok(),
        "llama_release_tag": env::var("LLAMAMANAGER_LLAMA_RELEASE_TAG").ok(),
        "server_path": server.path,
        "server_sha256": server.sha256,
        "router_argv": argv,
        "model_a": model_a,
        "model_b": model_b,
        "model_a_id": model_a_id,
        "model_b_id": model_b_id,
        "reload": reload,
        "load": load,
        "unload": unload,
        "preload": preload,
        "switch": switch,
        "final_unload": final_unload,
        "cancelled_before_mutation": true,
        "final_registry": final_registry,
        "dynamic_default_model_mutation_supported": false,
        "dynamic_default_model_reason": "pinned b10472 exposes router startup policy through CLI/props but no dynamic default-model mutation route; #39 applies persistence/restart verification only where supported"
    });
    fs::write(
        evidence_dir.join("router-operations.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();

    child
        .0
        .kill()
        .expect("stop real router after operation validation");
    let status = child.0.wait().expect("wait for stopped real router");
    assert!(
        !status.success(),
        "explicit test cleanup should terminate router"
    );
}
