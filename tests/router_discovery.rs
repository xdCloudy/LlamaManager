use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    thread,
    time::Duration,
};

use llamamanager::{
    gguf::ModelInfo,
    llama::{LlamaInstallation, ToolEvidence},
    model_store::{FileFingerprint, ModelStore},
    router::{
        RouterDiscoveryError, RouterFeatureState, RouterLibraryLinkKind, RouterModelPhase,
        RouterRole, discover_router_registry,
    },
    server_readiness::ServerEndpoint,
};
use serde_json::json;
use tempfile::tempdir;

#[derive(Clone)]
struct FakeResponse {
    expected_path: &'static str,
    status: u16,
    body: String,
}

fn fake_router(responses: Vec<FakeResponse>) -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        for scripted in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0, "client closed before completing HTTP headers");
                request.extend_from_slice(&chunk[..read]);
            }
            let request = String::from_utf8_lossy(&request);
            let first_line = request.lines().next().unwrap_or_default();
            assert!(
                first_line.contains(scripted.expected_path),
                "expected request path {}, got {first_line}",
                scripted.expected_path
            );

            let reason = match scripted.status {
                200 => "OK",
                401 => "Unauthorized",
                403 => "Forbidden",
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

fn installation() -> LlamaInstallation {
    LlamaInstallation {
        id: "installation-test".into(),
        name: "fixture".into(),
        root_path: PathBuf::from(r"C:\llama fixture"),
        server: Some(ToolEvidence {
            path: PathBuf::from(r"C:\llama fixture\llama-server.exe"),
            sha256: "server-sha".into(),
            version_output: "b10472 fixture".into(),
            help_output: "--models-dir PATH --models-preset PATH --models-max N --models-autoload --no-models-autoload".into(),
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

fn model_info(path: PathBuf, sha256: &str) -> ModelInfo {
    ModelInfo {
        id: "model-library-id".into(),
        path,
        file_size: 4,
        sha256: sha256.into(),
        gguf_version: 3,
        tensor_count: 1,
        metadata_count: 0,
        name: Some("Library Model".into()),
        architecture: Some("llama".into()),
        context_length: Some(4096),
        quantization_version: None,
        general_type: None,
        file_type: None,
        parameter_count: None,
        tensor_type_counts: BTreeMap::new(),
        metadata: BTreeMap::new(),
        inspected_at_unix_ms: 1,
    }
}

#[test]
fn discovers_live_router_state_without_guessing_control_support() {
    let temp = tempdir().unwrap();
    let database = temp.path().join("library.sqlite");
    let store = ModelStore::open(&database).unwrap();
    let known_path = temp.path().join("known model.gguf");
    fs::write(&known_path, b"GGUF").unwrap();
    let sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    store
        .save_model_with_location(
            &model_info(known_path.clone(), sha),
            &FileFingerprint {
                file_size: 4,
                modified_at_unix_ms: None,
                edge_sha256: "edge".into(),
            },
        )
        .unwrap();

    let same_filename_wrong_path = temp.path().join("other").join("known model.gguf");
    let models = json!({
        "object": "list",
        "data": [
            {
                "id": "canonical-a",
                "alias": "alpha",
                "aliases": ["alpha", "route-a"],
                "path": known_path,
                "status": {
                    "value": "loaded",
                    "args": ["llama-server", "--model", "known model.gguf"]
                },
                "resident": true,
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"]
                }
            },
            {
                "id": "same-name-is-not-identity",
                "path": same_filename_wrong_path,
                "status": {
                    "value": "unloaded",
                    "failed": true,
                    "exit_code": 1
                }
            },
            {
                "id": "sha-backed",
                "sha256": sha,
                "status": {"value": "sleeping"}
            }
        ]
    })
    .to_string();
    let (port, server) = fake_router(vec![
        FakeResponse {
            expected_path: "/props",
            status: 200,
            body: json!({"role": "router"}).to_string(),
        },
        FakeResponse {
            expected_path: "/models",
            status: 200,
            body: models,
        },
    ]);

    let registry = discover_router_registry(
        &installation(),
        &endpoint(port),
        Some(&store),
        Duration::from_secs(2),
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(registry.role, RouterRole::Router);
    assert!(registry.static_capabilities.router_cli_observed);
    assert!(registry.static_capabilities.models_dir);
    assert!(registry.static_capabilities.models_preset);
    assert_eq!(
        registry.endpoints.list_models.state,
        RouterFeatureState::Supported
    );
    assert_eq!(
        registry.endpoints.load_model.state,
        RouterFeatureState::Unknown,
        "discovery must not fake support for a mutating endpoint"
    );
    assert_eq!(registry.models.len(), 3);

    let loaded = &registry.models[0];
    assert_eq!(loaded.status.phase, RouterModelPhase::Loaded);
    assert_eq!(loaded.resident, Some(true));
    assert_eq!(
        loaded.routing_targets,
        vec![
            "alpha".to_string(),
            "canonical-a".to_string(),
            "route-a".to_string()
        ]
    );
    assert_eq!(loaded.library_link.kind, RouterLibraryLinkKind::ExactPath);
    assert_eq!(
        loaded.library_link.model_id.as_deref(),
        Some("model-library-id")
    );

    let same_name = &registry.models[1];
    assert!(same_name.status.failed);
    assert_eq!(same_name.status.exit_code, Some(1));
    assert_eq!(same_name.library_link.kind, RouterLibraryLinkKind::Unmatched);
    assert_eq!(same_name.library_link.model_id, None);

    let sha_backed = &registry.models[2];
    assert_eq!(sha_backed.status.phase, RouterModelPhase::Sleeping);
    assert_eq!(sha_backed.resident, None, "residency must remain unknown");
    assert_eq!(sha_backed.library_link.kind, RouterLibraryLinkKind::Sha256);
    assert_eq!(
        sha_backed.library_link.model_id.as_deref(),
        Some("model-library-id")
    );
}

#[test]
fn single_model_server_is_not_misrepresented_as_router() {
    let (port, server) = fake_router(vec![FakeResponse {
        expected_path: "/props",
        status: 200,
        body: json!({"role": "model"}).to_string(),
    }]);

    let registry = discover_router_registry(
        &installation(),
        &endpoint(port),
        None,
        Duration::from_secs(2),
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(registry.role, RouterRole::SingleModel);
    assert!(registry.models.is_empty());
    assert_eq!(
        registry.endpoints.load_model.state,
        RouterFeatureState::Unsupported
    );
    assert_eq!(
        registry.endpoints.list_models.state,
        RouterFeatureState::Unsupported
    );
}

#[test]
fn authentication_failure_is_actionable() {
    let (port, server) = fake_router(vec![FakeResponse {
        expected_path: "/props",
        status: 401,
        body: json!({"error": "bad api key"}).to_string(),
    }]);

    let error = discover_router_registry(
        &installation(),
        &endpoint(port),
        None,
        Duration::from_secs(2),
    )
    .unwrap_err();
    server.join().unwrap();

    assert_eq!(
        error,
        RouterDiscoveryError::AuthenticationRejected {
            path: "/props".into(),
            status_code: 401
        }
    );
}

#[test]
fn version_drift_is_not_silently_coerced() {
    let (port, server) = fake_router(vec![FakeResponse {
        expected_path: "/props",
        status: 200,
        body: json!({"future_role_field": "router-v2"}).to_string(),
    }]);

    let error = discover_router_registry(
        &installation(),
        &endpoint(port),
        None,
        Duration::from_secs(2),
    )
    .unwrap_err();
    server.join().unwrap();

    match error {
        RouterDiscoveryError::ProtocolDrift { path, message, .. } => {
            assert_eq!(path, "/props");
            assert!(message.contains("role"));
        }
        other => panic!("expected protocol drift, got {other:?}"),
    }
}

#[test]
fn models_endpoint_shape_drift_is_actionable() {
    let (port, server) = fake_router(vec![
        FakeResponse {
            expected_path: "/props",
            status: 200,
            body: json!({"role": "router"}).to_string(),
        },
        FakeResponse {
            expected_path: "/models",
            status: 200,
            body: json!({"models_v2": []}).to_string(),
        },
    ]);

    let error = discover_router_registry(
        &installation(),
        &endpoint(port),
        None,
        Duration::from_secs(2),
    )
    .unwrap_err();
    server.join().unwrap();

    assert!(matches!(
        error,
        RouterDiscoveryError::ProtocolDrift { ref path, .. } if path == "/models"
    ));
}
