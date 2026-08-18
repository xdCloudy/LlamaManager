#![cfg(windows)]

use std::{
    env, fs,
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use llamamanager::{
    llama::inspect_installation,
    router::{RouterFeatureState, RouterRole, discover_router_registry},
    server_readiness::ServerEndpoint,
};
use serde_json::json;

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

#[test]
#[ignore = "requires pinned real Windows llama.cpp binaries and published GGUF model"]
fn validates_real_router_role_and_live_model_registry() {
    let llama_root = PathBuf::from(required_env("LLAMAMANAGER_REAL_LLAMA_ROOT"));
    let model_path = PathBuf::from(required_env("LLAMAMANAGER_REAL_BENCH_MODEL"));
    let evidence_dir = PathBuf::from(required_env("LLAMAMANAGER_REAL_EVIDENCE_DIR"));
    fs::create_dir_all(&evidence_dir).unwrap();

    let installation =
        inspect_installation(&llama_root).expect("inspect pinned llama.cpp installation");
    let server = installation
        .server
        .as_ref()
        .expect("pinned runtime must expose llama-server.exe");
    let model_dir = model_path
        .parent()
        .expect("benchmark model must have a containing directory");
    let port = free_loopback_port();
    let endpoint = ServerEndpoint::loopback(port);

    let argv = vec![
        "--host".to_string(),
        endpoint.host.clone(),
        "--port".to_string(),
        port.to_string(),
        "--models-dir".to_string(),
        model_dir.to_string_lossy().to_string(),
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

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut last_error = None;
    let registry = loop {
        if let Some(status) = child
            .0
            .try_wait()
            .expect("inspect real router process during discovery")
        {
            panic!("real llama-server router exited before discovery: {status}");
        }

        match discover_router_registry(
            &installation,
            &endpoint,
            None,
            Duration::from_secs(2),
        ) {
            Ok(registry) => break registry,
            Err(error) if Instant::now() < deadline => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(200));
            }
            Err(error) => panic!(
                "real router discovery did not become ready before timeout; last error: {error}"
            ),
        }
    };

    assert_eq!(registry.role, RouterRole::Router);
    assert!(registry.static_capabilities.router_cli_observed);
    assert!(registry.static_capabilities.models_dir);
    assert!(registry.static_capabilities.models_max);
    assert_eq!(
        registry.endpoints.props.state,
        RouterFeatureState::Supported
    );
    assert_eq!(
        registry.endpoints.list_models.state,
        RouterFeatureState::Supported
    );
    assert_eq!(
        registry.endpoints.load_model.state,
        RouterFeatureState::Unknown,
        "discovery must not claim mutating support before #39 verifies it"
    );
    assert!(
        !registry.models.is_empty(),
        "router model registry must reflect the GGUF present in --models-dir"
    );

    let model_ids: Vec<_> = registry
        .models
        .iter()
        .map(|model| model.id.clone())
        .collect();
    let model_states: Vec<_> = registry
        .models
        .iter()
        .map(|model| format!("{:?}", model.status.phase))
        .collect();

    let evidence = json!({
        "github_sha": env::var("GITHUB_SHA").ok(),
        "runner_os": env::var("RUNNER_OS").ok(),
        "llama_release_tag": env::var("LLAMAMANAGER_LLAMA_RELEASE_TAG").ok(),
        "server_path": server.path,
        "server_sha256": server.sha256,
        "router_argv": argv,
        "endpoint": registry.endpoint,
        "role": format!("{:?}", registry.role),
        "router_cli_observed": registry.static_capabilities.router_cli_observed,
        "observed_options": registry.static_capabilities.observed_options,
        "model_count": registry.models.len(),
        "model_ids": model_ids,
        "model_states": model_states,
        "last_transient_discovery_error": last_error.map(|error| error.to_string())
    });
    fs::write(
        evidence_dir.join("router-discovery.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();

    child.0.kill().expect("stop real router after validation");
    let status = child.0.wait().expect("wait for stopped real router");
    assert!(!status.success(), "explicit test cleanup should terminate router");
}
