use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    thread,
    time::Duration,
};

use llamamanager::{
    llama::{LlamaInstallation, ToolEvidence},
    router::{RouterModelPhase, RouterRole},
    router_observability::{
        EvidenceAvailability, RouterEvictionSafety, RouterObservabilityTracker,
        RouterSnapshotFreshness, discover_router_observability,
    },
    server_readiness::ServerEndpoint,
};

fn installation() -> LlamaInstallation {
    LlamaInstallation {
        id: "fake-router".into(),
        name: "fake-router".into(),
        root_path: PathBuf::from("C:/fake"),
        server: Some(ToolEvidence {
            path: PathBuf::from("C:/fake/llama-server.exe"),
            sha256: "abc123".into(),
            version_output: "fake b10472".into(),
            help_output: "--models-dir --models-max --models-autoload".into(),
            device_output: String::new(),
        }),
        bench: None,
        fit_params: None,
        backend: None,
        capabilities: BTreeSet::new(),
        discovered_at_unix_ms: 0,
    }
}

fn start_fake_router(models_body: &'static str) -> (ServerEndpoint, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0_u8; 8192];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            let body = if path == "/props" {
                r#"{"role":"router","max_instances":2,"models_autoload":true}"#
            } else if path == "/models" {
                models_body
            } else {
                r#"{"error":"unexpected path"}"#
            };
            let status = if path == "/props" || path == "/models" {
                "200 OK"
            } else {
                "404 Not Found"
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        }
    });
    (ServerEndpoint::loopback(port), handle)
}

#[test]
fn fake_router_exposes_active_request_lru_residency_and_alias_evidence() {
    let (endpoint, handle) = start_fake_router(
        r#"{"data":[
            {"id":"alpha","aliases":["a","chat"],"resident":true,"active_requests":2,"last_used_ms":100,"lru_rank":1,"evictable":true,"status":{"value":"loaded","args":["llama-server","-m","alpha.gguf"]}},
            {"id":"beta","aliases":["b"],"resident":false,"active_requests":0,"last_used_ms":50,"lru_rank":2,"evictable":false,"status":{"value":"unloaded","args":["llama-server","-m","beta.gguf"]}}
        ]}"#,
    );

    let snapshot =
        discover_router_observability(&installation(), &endpoint, None, Duration::from_secs(2))
            .unwrap();
    handle.join().unwrap();

    assert_eq!(snapshot.registry.role, RouterRole::Router);
    assert_eq!(snapshot.models.len(), 2);

    let alpha = snapshot
        .models
        .iter()
        .find(|model| model.model.id == "alpha")
        .unwrap();
    assert_eq!(alpha.model.status.phase, RouterModelPhase::Loaded);
    assert_eq!(
        alpha.model.routing_targets,
        vec!["a".to_string(), "alpha".to_string(), "chat".to_string()]
    );
    assert_eq!(alpha.residency.value, Some(true));
    assert_eq!(alpha.active_requests.value, Some(2));
    assert_eq!(alpha.last_used_ms.value, Some(100));
    assert_eq!(alpha.lru_rank.value, Some(1));
    assert_eq!(alpha.evictable.value, Some(true));
    assert_eq!(
        alpha.eviction_safety(),
        RouterEvictionSafety::Busy { active_requests: 2 }
    );

    let beta = snapshot
        .models
        .iter()
        .find(|model| model.model.id == "beta")
        .unwrap();
    assert_eq!(beta.model.status.phase, RouterModelPhase::Unloaded);
    assert_eq!(beta.residency.value, Some(false));
    assert_eq!(beta.evictable.value, Some(false));
    assert!(matches!(
        beta.eviction_safety(),
        RouterEvictionSafety::NotApplicable { .. }
    ));
}

#[test]
fn fake_router_missing_observability_fields_remain_explicitly_unavailable() {
    let (endpoint, handle) = start_fake_router(
        r#"{"data":[{"id":"alpha","aliases":["a"],"status":{"value":"loading","args":[]}}]}"#,
    );

    let snapshot =
        discover_router_observability(&installation(), &endpoint, None, Duration::from_secs(2))
            .unwrap();
    handle.join().unwrap();

    let alpha = &snapshot.models[0];
    assert_eq!(alpha.model.status.phase, RouterModelPhase::Loading);
    assert_eq!(
        alpha.residency.availability,
        EvidenceAvailability::Unavailable
    );
    assert_eq!(
        alpha.active_requests.availability,
        EvidenceAvailability::Unavailable
    );
    assert_eq!(
        alpha.lru_rank.availability,
        EvidenceAvailability::Unavailable
    );
    assert_eq!(
        alpha.evictable.availability,
        EvidenceAvailability::Unavailable
    );
}

#[test]
fn tracker_retains_snapshot_but_marks_it_stale_after_disconnect() {
    let (endpoint, handle) =
        start_fake_router(r#"{"data":[{"id":"alpha","status":{"value":"loaded","args":[]}}]}"#);
    let snapshot =
        discover_router_observability(&installation(), &endpoint, None, Duration::from_secs(2))
            .unwrap();
    handle.join().unwrap();

    let mut tracker = RouterObservabilityTracker::default();
    tracker.reconcile(Ok(snapshot));
    assert_eq!(tracker.freshness(), RouterSnapshotFreshness::Live);
    tracker.begin_refresh();
    assert_eq!(tracker.freshness(), RouterSnapshotFreshness::Loading);
    tracker.reconcile(Err("connection refused".into()));
    assert_eq!(tracker.freshness(), RouterSnapshotFreshness::Stale);
    assert!(tracker.current.is_some());
}
