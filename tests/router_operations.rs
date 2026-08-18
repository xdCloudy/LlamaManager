use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use llamamanager::{
    gguf::ModelInfo,
    llama::{LlamaInstallation, ToolEvidence},
    model_store::{FileFingerprint, ModelStore},
    persistence::Database,
    router_operations::{
        RouterOperationCancellation, RouterOperationController, RouterOperationError,
        RouterOperationKind, RouterOperationState,
    },
    server_readiness::ServerEndpoint,
};
use serde_json::json;
use tempfile::tempdir;

#[derive(Clone)]
struct ScriptedResponse {
    method: &'static str,
    path: &'static str,
    status: u16,
    body: String,
    delay: Duration,
}

fn scripted_router(responses: Vec<ScriptedResponse>) -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        for scripted in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 2048];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0, "client closed before completing HTTP headers");
                request.extend_from_slice(&chunk[..read]);
            }
            let request = String::from_utf8_lossy(&request);
            let first_line = request.lines().next().unwrap_or_default();
            assert_eq!(
                first_line,
                format!("{} {} HTTP/1.1", scripted.method, scripted.path)
            );
            if scripted.method == "POST" {
                assert!(request.contains("Content-Type: application/json"));
            }
            if !scripted.delay.is_zero() {
                thread::sleep(scripted.delay);
            }
            let reason = match scripted.status {
                200 => "OK",
                400 => "Bad Request",
                401 => "Unauthorized",
                404 => "Not Found",
                _ => "Error",
            };
            let response = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                scripted.status,
                reason,
                scripted.body.len(),
                scripted.body
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        }
    });
    (port, handle)
}

fn response(method: &'static str, path: &'static str, body: serde_json::Value) -> ScriptedResponse {
    ScriptedResponse {
        method,
        path,
        status: 200,
        body: body.to_string(),
        delay: Duration::ZERO,
    }
}

fn installation() -> LlamaInstallation {
    LlamaInstallation {
        id: "installation-test".into(),
        name: "fixture".into(),
        root_path: PathBuf::from(r"C:\llama fixture"),
        server: Some(ToolEvidence {
            path: PathBuf::from(r"C:\llama fixture\llama-server.exe"),
            sha256: "server-sha".into(),
            version_output: "b10472 fixture".into(),
            help_output: "--model -m --models-dir PATH --models-max N --models-autoload".into(),
            device_output: String::new(),
        }),
        bench: None,
        fit_params: None,
        backend: Some("CPU".into()),
        capabilities: BTreeSet::new(),
        discovered_at_unix_ms: 1,
    }
}

fn endpoint(port: u16) -> ServerEndpoint {
    ServerEndpoint {
        host: "127.0.0.1".into(),
        port,
        api_key: None,
        allow_non_loopback: false,
    }
}

fn model_info(id: &str, path: &Path, sha256: &str) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        path: path.to_path_buf(),
        file_size: 4,
        sha256: sha256.into(),
        gguf_version: 3,
        tensor_count: 1,
        metadata_count: 1,
        name: Some(id.into()),
        architecture: Some("llama".into()),
        context_length: Some(4096),
        quantization_version: Some(2),
        general_type: None,
        file_type: Some(2),
        parameter_count: Some(1_000_000),
        tensor_type_counts: BTreeMap::from([(2_u32, 1)]),
        metadata: BTreeMap::new(),
        inspected_at_unix_ms: 1,
    }
}

fn store_with_models(models: &[(&str, &Path, &str)]) -> ModelStore {
    let temp = tempdir().unwrap().keep();
    let database = temp.join("library.sqlite");
    Database::open(&database).unwrap();
    let store = ModelStore::open(&database).unwrap();
    for (id, path, sha) in models {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"GGUF").unwrap();
        store
            .save_model_with_location(
                &model_info(id, path, sha),
                &FileFingerprint {
                    file_size: 4,
                    modified_at_unix_ms: None,
                    edge_sha256: format!("edge-{id}"),
                },
            )
            .unwrap();
    }
    store
}

fn router_model(id: &str, path: &Path, phase: &str) -> serde_json::Value {
    json!({
        "id": id,
        "aliases": [id],
        "status": {
            "value": phase,
            "args": ["llama-server.exe", "--model", path.to_string_lossy()]
        },
        "architecture": {
            "input_modalities": ["text"],
            "output_modalities": ["text"]
        }
    })
}

fn registry(models: Vec<serde_json::Value>) -> serde_json::Value {
    json!({"object": "list", "data": models})
}

fn props() -> serde_json::Value {
    json!({"role": "router", "max_instances": 2, "models_autoload": true})
}

#[test]
fn load_checks_compatibility_then_reconciles_loaded_state() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("target.gguf");
    let sha = "a".repeat(64);
    let store = store_with_models(&[("library-target", path.as_path(), sha.as_str())]);
    let (port, server) = scripted_router(vec![
        response("GET", "/props", props()),
        response(
            "GET",
            "/models",
            registry(vec![router_model("target", &path, "unloaded")]),
        ),
        response("POST", "/models/load", json!({"success": true})),
        response("GET", "/props", props()),
        response(
            "GET",
            "/models",
            registry(vec![router_model("target", &path, "loaded")]),
        ),
    ]);

    let controller = RouterOperationController::new();
    let evidence = controller
        .load_model(
            &installation(),
            &endpoint(port),
            &store,
            "target",
            Duration::from_secs(2),
            &RouterOperationCancellation::new(),
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(evidence.kind, RouterOperationKind::Load);
    assert_eq!(evidence.http_statuses, vec![200]);
    assert!(evidence.compatibility.is_some());
    assert!(matches!(
        controller.state(),
        RouterOperationState::Succeeded(_)
    ));
}

#[test]
fn unload_reconciles_actual_unloaded_state() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("target.gguf");
    let (port, server) = scripted_router(vec![
        response("GET", "/props", props()),
        response(
            "GET",
            "/models",
            registry(vec![router_model("target", &path, "loaded")]),
        ),
        response("POST", "/models/unload", json!({"success": true})),
        response("GET", "/props", props()),
        response(
            "GET",
            "/models",
            registry(vec![router_model("target", &path, "unloaded")]),
        ),
    ]);

    let controller = RouterOperationController::new();
    let evidence = controller
        .unload_model(
            &installation(),
            &endpoint(port),
            None,
            "target",
            Duration::from_secs(2),
            &RouterOperationCancellation::new(),
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(evidence.kind, RouterOperationKind::Unload);
    assert_eq!(evidence.http_statuses, vec![200]);
}

#[test]
fn reload_uses_real_reload_query_then_reconciles() {
    let (port, server) = scripted_router(vec![
        response("GET", "/props", props()),
        response("GET", "/models", registry(Vec::new())),
        response("GET", "/models?reload=1", registry(Vec::new())),
        response("GET", "/props", props()),
        response("GET", "/models", registry(Vec::new())),
    ]);

    let controller = RouterOperationController::new();
    let evidence = controller
        .reload_registry(
            &installation(),
            &endpoint(port),
            None,
            Duration::from_secs(2),
            &RouterOperationCancellation::new(),
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(evidence.kind, RouterOperationKind::ReloadRegistry);
    assert_eq!(evidence.http_statuses, vec![200]);
}

#[test]
fn switch_confirms_target_before_unloading_any_remaining_source() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source.gguf");
    let target = temp.path().join("target.gguf");
    let sha_a = "a".repeat(64);
    let sha_b = "b".repeat(64);
    let store = store_with_models(&[
        ("library-source", source.as_path(), sha_a.as_str()),
        ("library-target", target.as_path(), sha_b.as_str()),
    ]);
    let (port, server) = scripted_router(vec![
        response("GET", "/props", props()),
        response(
            "GET",
            "/models",
            registry(vec![
                router_model("source", &source, "loaded"),
                router_model("target", &target, "unloaded"),
            ]),
        ),
        response("POST", "/models/load", json!({"success": true})),
        response("GET", "/props", props()),
        response(
            "GET",
            "/models",
            registry(vec![
                router_model("source", &source, "loaded"),
                router_model("target", &target, "loaded"),
            ]),
        ),
        response("POST", "/models/unload", json!({"success": true})),
        response("GET", "/props", props()),
        response(
            "GET",
            "/models",
            registry(vec![
                router_model("source", &source, "unloaded"),
                router_model("target", &target, "loaded"),
            ]),
        ),
    ]);

    let controller = RouterOperationController::new();
    let evidence = controller
        .switch_model(
            &installation(),
            &endpoint(port),
            &store,
            "source",
            "target",
            Duration::from_secs(2),
            &RouterOperationCancellation::new(),
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(evidence.kind, RouterOperationKind::Switch);
    assert_eq!(evidence.http_statuses, vec![200, 200]);
}

#[test]
fn failed_target_load_is_explicit_and_registry_evidence_is_retained() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source.gguf");
    let target = temp.path().join("target.gguf");
    let sha_a = "a".repeat(64);
    let sha_b = "b".repeat(64);
    let store = store_with_models(&[
        ("library-source", source.as_path(), sha_a.as_str()),
        ("library-target", target.as_path(), sha_b.as_str()),
    ]);
    let failed_target = json!({
        "id": "target",
        "status": {
            "value": "unloaded",
            "failed": true,
            "exit_code": 7,
            "args": ["llama-server.exe", "--model", target.to_string_lossy()]
        }
    });
    let (port, server) = scripted_router(vec![
        response("GET", "/props", props()),
        response(
            "GET",
            "/models",
            registry(vec![
                router_model("source", &source, "loaded"),
                router_model("target", &target, "unloaded"),
            ]),
        ),
        response("POST", "/models/load", json!({"success": true})),
        response("GET", "/props", props()),
        response(
            "GET",
            "/models",
            registry(vec![
                router_model("source", &source, "loaded"),
                failed_target,
            ]),
        ),
    ]);

    let controller = RouterOperationController::new();
    let error = controller
        .switch_model(
            &installation(),
            &endpoint(port),
            &store,
            "source",
            "target",
            Duration::from_secs(2),
            &RouterOperationCancellation::new(),
        )
        .unwrap_err();
    server.join().unwrap();

    assert!(matches!(error, RouterOperationError::Reconciliation { .. }));
    match controller.state() {
        RouterOperationState::Failed(failure) => {
            let registry = failure
                .last_registry
                .expect("failed operation keeps registry evidence");
            assert_eq!(
                registry
                    .models
                    .iter()
                    .find(|model| model.id == "source")
                    .unwrap()
                    .status
                    .phase,
                llamamanager::router::RouterModelPhase::Loaded
            );
        }
        other => panic!("expected failed operation state, got {other:?}"),
    }
}

#[test]
fn unknown_compatibility_blocks_load_before_post() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("unknown.gguf");
    let database_root = tempdir().unwrap();
    let database = database_root.path().join("library.sqlite");
    Database::open(&database).unwrap();
    let store = ModelStore::open(&database).unwrap();
    fs::write(&path, b"GGUF").unwrap();
    let mut info = model_info("library-unknown", &path, &"c".repeat(64));
    info.architecture = Some("future-architecture".into());
    store
        .save_model_with_location(
            &info,
            &FileFingerprint {
                file_size: 4,
                modified_at_unix_ms: None,
                edge_sha256: "edge".into(),
            },
        )
        .unwrap();

    let (port, server) = scripted_router(vec![
        response("GET", "/props", props()),
        response(
            "GET",
            "/models",
            registry(vec![router_model("unknown", &path, "unloaded")]),
        ),
    ]);
    let controller = RouterOperationController::new();
    let error = controller
        .load_model(
            &installation(),
            &endpoint(port),
            &store,
            "unknown",
            Duration::from_secs(2),
            &RouterOperationCancellation::new(),
        )
        .unwrap_err();
    server.join().unwrap();

    assert!(matches!(
        error,
        RouterOperationError::CompatibilityBlocked { .. }
    ));
}

#[test]
fn cancellation_before_mutation_is_explicit() {
    let controller = RouterOperationController::new();
    let cancellation = RouterOperationCancellation::new();
    cancellation.cancel();
    let error = controller
        .reload_registry(
            &installation(),
            &endpoint(9),
            None,
            Duration::from_millis(50),
            &cancellation,
        )
        .unwrap_err();
    assert!(matches!(error, RouterOperationError::Cancelled));
    assert!(matches!(
        controller.state(),
        RouterOperationState::Cancelled(_)
    ));
}

#[test]
fn duplicate_operation_is_rejected_while_first_is_running() {
    let delayed = ScriptedResponse {
        method: "GET",
        path: "/props",
        status: 200,
        body: props().to_string(),
        delay: Duration::from_millis(500),
    };
    let (port, server) = scripted_router(vec![
        delayed,
        response("GET", "/models", registry(Vec::new())),
        response("GET", "/models?reload=1", registry(Vec::new())),
        response("GET", "/props", props()),
        response("GET", "/models", registry(Vec::new())),
    ]);
    let controller = RouterOperationController::new();
    let worker_controller = controller.clone();
    let worker = thread::spawn(move || {
        worker_controller.reload_registry(
            &installation(),
            &endpoint(port),
            None,
            Duration::from_secs(2),
            &RouterOperationCancellation::new(),
        )
    });
    thread::sleep(Duration::from_millis(100));

    let error = controller
        .reload_registry(
            &installation(),
            &endpoint(port),
            None,
            Duration::from_secs(2),
            &RouterOperationCancellation::new(),
        )
        .unwrap_err();
    assert!(matches!(error, RouterOperationError::Busy { .. }));

    worker.join().unwrap().unwrap();
    server.join().unwrap();
}
