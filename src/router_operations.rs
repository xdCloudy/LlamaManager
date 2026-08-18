use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::{
    compatibility::{CompatibilityStatus, evaluate_compatibility},
    gguf::ModelInfo,
    llama::{LlamaInstallation, now_ms},
    model_store::ModelStore,
    router::{
        RouterDiscoveryError, RouterModel, RouterModelPhase, RouterRegistry, RouterRole,
        discover_router_registry,
    },
    server_readiness::ServerEndpoint,
};

const MAX_ROUTER_OPERATION_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterOperationKind {
    ReloadRegistry,
    Load,
    Unload,
    Preload,
    Switch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterCompatibilityEvidence {
    pub model_id: String,
    pub status: CompatibilityStatus,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterOperationEvidence {
    pub kind: RouterOperationKind,
    pub source_model: Option<String>,
    pub target_model: Option<String>,
    pub started_at_unix_ms: u128,
    pub completed_at_unix_ms: u128,
    pub compatibility: Option<RouterCompatibilityEvidence>,
    pub http_statuses: Vec<u16>,
    pub registry: RouterRegistry,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterOperationProgress {
    pub kind: RouterOperationKind,
    pub source_model: Option<String>,
    pub target_model: Option<String>,
    pub started_at_unix_ms: u128,
    pub message: String,
    pub last_registry: Option<RouterRegistry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterOperationFailure {
    pub kind: RouterOperationKind,
    pub source_model: Option<String>,
    pub target_model: Option<String>,
    pub started_at_unix_ms: u128,
    pub failed_at_unix_ms: u128,
    pub message: String,
    pub last_registry: Option<RouterRegistry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RouterOperationState {
    Idle,
    Running(RouterOperationProgress),
    Succeeded(RouterOperationEvidence),
    Failed(RouterOperationFailure),
    Cancelled(RouterOperationFailure),
}

impl Default for RouterOperationState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Default)]
pub struct RouterOperationCancellation {
    cancelled: Arc<AtomicBool>,
}

impl RouterOperationCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Default)]
pub struct RouterOperationController {
    state: Arc<Mutex<RouterOperationState>>,
}

impl RouterOperationController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> RouterOperationState {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| RouterOperationState::Failed(RouterOperationFailure {
                kind: RouterOperationKind::ReloadRegistry,
                source_model: None,
                target_model: None,
                started_at_unix_ms: 0,
                failed_at_unix_ms: now_ms(),
                message: "router operation state mutex is poisoned".into(),
                last_registry: None,
            }))
    }

    pub fn reload_registry(
        &self,
        installation: &LlamaInstallation,
        endpoint: &ServerEndpoint,
        model_store: Option<&ModelStore>,
        timeout: Duration,
        cancellation: &RouterOperationCancellation,
    ) -> Result<RouterOperationEvidence, RouterOperationError> {
        self.run(
            RouterOperationKind::ReloadRegistry,
            None,
            None,
            cancellation,
            |started_at| {
                cancelled(cancellation)?;
                let initial = discover_router_registry(installation, endpoint, model_store, timeout)?;
                require_router(&initial)?;
                self.progress("refreshing router model registry", Some(initial));

                let response = request(
                    endpoint,
                    "GET",
                    "/models?reload=1",
                    None,
                    timeout,
                )?;
                ensure_mutation_success("/models?reload=1", &response)?;
                cancelled(cancellation)?;

                let registry = discover_router_registry(installation, endpoint, model_store, timeout)?;
                require_router(&registry)?;
                Ok(RouterOperationEvidence {
                    kind: RouterOperationKind::ReloadRegistry,
                    source_model: None,
                    target_model: None,
                    started_at_unix_ms: started_at,
                    completed_at_unix_ms: now_ms(),
                    compatibility: None,
                    http_statuses: vec![response.status_code],
                    registry,
                    notes: vec![
                        "GET /models?reload=1 succeeded and the live registry was re-read"
                            .into(),
                    ],
                })
            },
        )
    }

    pub fn load_model(
        &self,
        installation: &LlamaInstallation,
        endpoint: &ServerEndpoint,
        model_store: &ModelStore,
        target_model: &str,
        timeout: Duration,
        cancellation: &RouterOperationCancellation,
    ) -> Result<RouterOperationEvidence, RouterOperationError> {
        self.load_like(
            RouterOperationKind::Load,
            installation,
            endpoint,
            model_store,
            target_model,
            timeout,
            cancellation,
        )
    }

    pub fn preload_model(
        &self,
        installation: &LlamaInstallation,
        endpoint: &ServerEndpoint,
        model_store: &ModelStore,
        target_model: &str,
        timeout: Duration,
        cancellation: &RouterOperationCancellation,
    ) -> Result<RouterOperationEvidence, RouterOperationError> {
        self.load_like(
            RouterOperationKind::Preload,
            installation,
            endpoint,
            model_store,
            target_model,
            timeout,
            cancellation,
        )
    }

    fn load_like(
        &self,
        kind: RouterOperationKind,
        installation: &LlamaInstallation,
        endpoint: &ServerEndpoint,
        model_store: &ModelStore,
        target_model: &str,
        timeout: Duration,
        cancellation: &RouterOperationCancellation,
    ) -> Result<RouterOperationEvidence, RouterOperationError> {
        self.run(
            kind,
            None,
            Some(target_model.to_string()),
            cancellation,
            |started_at| {
                let mut statuses = Vec::new();
                cancelled(cancellation)?;
                let registry =
                    discover_router_registry(installation, endpoint, Some(model_store), timeout)?;
                require_router(&registry)?;
                let target = registry_model(&registry, target_model)?;
                let compatibility = compatibility_for(target, installation, model_store)?;
                self.progress(
                    format!("compatibility checked for {target_model}"),
                    Some(registry.clone()),
                );

                if is_ready(&target.status.phase) && !target.status.failed {
                    return Ok(RouterOperationEvidence {
                        kind,
                        source_model: None,
                        target_model: Some(target_model.to_string()),
                        started_at_unix_ms: started_at,
                        completed_at_unix_ms: now_ms(),
                        compatibility: Some(compatibility),
                        http_statuses: statuses,
                        registry,
                        notes: vec![
                            "target was already loaded/sleeping; no duplicate load request was sent"
                                .into(),
                        ],
                    });
                }

                cancelled(cancellation)?;
                let response = request(
                    endpoint,
                    "POST",
                    "/models/load",
                    Some(&json!({"model": target_model}).to_string()),
                    timeout,
                )?;
                statuses.push(response.status_code);
                ensure_success_json("/models/load", &response)?;
                self.progress(
                    format!("load accepted for {target_model}; reconciling live registry"),
                    Some(registry),
                );

                let registry = wait_for_model(
                    self,
                    installation,
                    endpoint,
                    Some(model_store),
                    target_model,
                    timeout,
                    cancellation,
                    |phase| is_ready(phase),
                    "loaded or sleeping",
                )?;

                Ok(RouterOperationEvidence {
                    kind,
                    source_model: None,
                    target_model: Some(target_model.to_string()),
                    started_at_unix_ms: started_at,
                    completed_at_unix_ms: now_ms(),
                    compatibility: Some(compatibility),
                    http_statuses: statuses,
                    registry,
                    notes: vec![
                        "POST /models/load returned success and the live registry confirmed readiness"
                            .into(),
                    ],
                })
            },
        )
    }

    pub fn unload_model(
        &self,
        installation: &LlamaInstallation,
        endpoint: &ServerEndpoint,
        model_store: Option<&ModelStore>,
        target_model: &str,
        timeout: Duration,
        cancellation: &RouterOperationCancellation,
    ) -> Result<RouterOperationEvidence, RouterOperationError> {
        self.run(
            RouterOperationKind::Unload,
            Some(target_model.to_string()),
            None,
            cancellation,
            |started_at| {
                cancelled(cancellation)?;
                let registry = discover_router_registry(installation, endpoint, model_store, timeout)?;
                require_router(&registry)?;
                let target = registry_model(&registry, target_model)?;
                self.progress(
                    format!("current state for {target_model} is {:?}", target.status.phase),
                    Some(registry.clone()),
                );

                if matches!(target.status.phase, RouterModelPhase::Unloaded) {
                    return Ok(RouterOperationEvidence {
                        kind: RouterOperationKind::Unload,
                        source_model: Some(target_model.to_string()),
                        target_model: None,
                        started_at_unix_ms: started_at,
                        completed_at_unix_ms: now_ms(),
                        compatibility: None,
                        http_statuses: Vec::new(),
                        registry,
                        notes: vec![
                            "target was already unloaded; no duplicate unload request was sent"
                                .into(),
                        ],
                    });
                }

                cancelled(cancellation)?;
                let response = request(
                    endpoint,
                    "POST",
                    "/models/unload",
                    Some(&json!({"model": target_model}).to_string()),
                    timeout,
                )?;
                ensure_success_json("/models/unload", &response)?;
                self.progress(
                    format!("unload accepted for {target_model}; reconciling live registry"),
                    Some(registry),
                );

                let registry = wait_for_model(
                    self,
                    installation,
                    endpoint,
                    model_store,
                    target_model,
                    timeout,
                    cancellation,
                    |phase| matches!(phase, RouterModelPhase::Unloaded),
                    "unloaded",
                )?;

                Ok(RouterOperationEvidence {
                    kind: RouterOperationKind::Unload,
                    source_model: Some(target_model.to_string()),
                    target_model: None,
                    started_at_unix_ms: started_at,
                    completed_at_unix_ms: now_ms(),
                    compatibility: None,
                    http_statuses: vec![response.status_code],
                    registry,
                    notes: vec![
                        "POST /models/unload returned success and the live registry confirmed unloaded state"
                            .into(),
                    ],
                })
            },
        )
    }

    pub fn switch_model(
        &self,
        installation: &LlamaInstallation,
        endpoint: &ServerEndpoint,
        model_store: &ModelStore,
        source_model: &str,
        target_model: &str,
        timeout: Duration,
        cancellation: &RouterOperationCancellation,
    ) -> Result<RouterOperationEvidence, RouterOperationError> {
        self.run(
            RouterOperationKind::Switch,
            Some(source_model.to_string()),
            Some(target_model.to_string()),
            cancellation,
            |started_at| {
                cancelled(cancellation)?;
                let mut statuses = Vec::new();
                let initial =
                    discover_router_registry(installation, endpoint, Some(model_store), timeout)?;
                require_router(&initial)?;
                let target = registry_model(&initial, target_model)?;
                let compatibility = compatibility_for(target, installation, model_store)?;
                self.progress(
                    format!("target compatibility checked for {target_model}"),
                    Some(initial.clone()),
                );

                let mut registry = initial;
                if !is_ready(&registry_model(&registry, target_model)?.status.phase) {
                    cancelled(cancellation)?;
                    let response = request(
                        endpoint,
                        "POST",
                        "/models/load",
                        Some(&json!({"model": target_model}).to_string()),
                        timeout,
                    )?;
                    statuses.push(response.status_code);
                    ensure_success_json("/models/load", &response)?;
                    registry = wait_for_model(
                        self,
                        installation,
                        endpoint,
                        Some(model_store),
                        target_model,
                        timeout,
                        cancellation,
                        |phase| is_ready(phase),
                        "loaded or sleeping",
                    )?;
                }

                if source_model != target_model {
                    let source_phase = registry
                        .models
                        .iter()
                        .find(|model| model.id == source_model)
                        .map(|model| model.status.phase.clone());
                    if source_phase.as_ref().is_some_and(is_running) {
                        cancelled(cancellation)?;
                        let response = request(
                            endpoint,
                            "POST",
                            "/models/unload",
                            Some(&json!({"model": source_model}).to_string()),
                            timeout,
                        )?;
                        statuses.push(response.status_code);
                        ensure_success_json("/models/unload", &response)?;
                        registry = wait_for_model(
                            self,
                            installation,
                            endpoint,
                            Some(model_store),
                            source_model,
                            timeout,
                            cancellation,
                            |phase| matches!(phase, RouterModelPhase::Unloaded),
                            "unloaded",
                        )?;
                    }
                }

                let final_target = registry_model(&registry, target_model)?;
                if !is_ready(&final_target.status.phase) || final_target.status.failed {
                    return Err(RouterOperationError::Reconciliation {
                        message: format!(
                            "switch did not leave target {target_model} ready; observed {:?}, failed={}",
                            final_target.status.phase, final_target.status.failed
                        ),
                    });
                }

                Ok(RouterOperationEvidence {
                    kind: RouterOperationKind::Switch,
                    source_model: Some(source_model.to_string()),
                    target_model: Some(target_model.to_string()),
                    started_at_unix_ms: started_at,
                    completed_at_unix_ms: now_ms(),
                    compatibility: Some(compatibility),
                    http_statuses: statuses,
                    registry,
                    notes: vec![
                        "target readiness was confirmed before any remaining source unload request"
                            .into(),
                        "upstream router LRU may evict the source during target load when models_max is saturated; final registry is authoritative"
                            .into(),
                    ],
                })
            },
        )
    }

    fn run<F>(
        &self,
        kind: RouterOperationKind,
        source_model: Option<String>,
        target_model: Option<String>,
        cancellation: &RouterOperationCancellation,
        operation: F,
    ) -> Result<RouterOperationEvidence, RouterOperationError>
    where
        F: FnOnce(u128) -> Result<RouterOperationEvidence, RouterOperationError>,
    {
        let started_at = self.begin(kind, source_model.clone(), target_model.clone())?;
        let result = operation(started_at);
        match result {
            Ok(evidence) => {
                self.finish_success(evidence.clone());
                Ok(evidence)
            }
            Err(RouterOperationError::Cancelled) => {
                self.finish_failure(true, "operation cancelled".into());
                Err(RouterOperationError::Cancelled)
            }
            Err(error) => {
                self.finish_failure(false, error.to_string());
                Err(error)
            }
        }
    }

    fn begin(
        &self,
        kind: RouterOperationKind,
        source_model: Option<String>,
        target_model: Option<String>,
    ) -> Result<u128, RouterOperationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RouterOperationError::StatePoisoned)?;
        if let RouterOperationState::Running(progress) = &*state {
            return Err(RouterOperationError::Busy {
                active: progress.kind,
            });
        }
        let started_at = now_ms();
        *state = RouterOperationState::Running(RouterOperationProgress {
            kind,
            source_model,
            target_model,
            started_at_unix_ms: started_at,
            message: "operation started".into(),
            last_registry: None,
        });
        Ok(started_at)
    }

    fn progress(&self, message: impl Into<String>, registry: Option<RouterRegistry>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if let RouterOperationState::Running(progress) = &mut *state {
            progress.message = message.into();
            if let Some(registry) = registry {
                progress.last_registry = Some(registry);
            }
        }
    }

    fn finish_success(&self, evidence: RouterOperationEvidence) {
        if let Ok(mut state) = self.state.lock() {
            *state = RouterOperationState::Succeeded(evidence);
        }
    }

    fn finish_failure(&self, cancelled: bool, message: String) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let failure = match &*state {
            RouterOperationState::Running(progress) => RouterOperationFailure {
                kind: progress.kind,
                source_model: progress.source_model.clone(),
                target_model: progress.target_model.clone(),
                started_at_unix_ms: progress.started_at_unix_ms,
                failed_at_unix_ms: now_ms(),
                message,
                last_registry: progress.last_registry.clone(),
            },
            _ => return,
        };
        *state = if cancelled {
            RouterOperationState::Cancelled(failure)
        } else {
            RouterOperationState::Failed(failure)
        };
    }
}

#[derive(Debug, Error)]
pub enum RouterOperationError {
    #[error("another router operation is already running: {active:?}")]
    Busy { active: RouterOperationKind },

    #[error("router operation state mutex is poisoned")]
    StatePoisoned,

    #[error(transparent)]
    Discovery(#[from] RouterDiscoveryError),

    #[error("selected endpoint is a single-model server, not a router")]
    NotRouter,

    #[error("router model `{model}` is not present in the live registry")]
    ModelNotFound { model: String },

    #[error("router model `{model}` cannot be bound to one M2 model identity from exact path, SHA-256, or exact argv evidence")]
    ModelIdentityUnproven { model: String },

    #[error("router model `{model}` maps ambiguously to M2 model identities: {candidates:?}")]
    ModelIdentityAmbiguous {
        model: String,
        candidates: Vec<String>,
    },

    #[error("router model `{model}` maps to missing M2 model record `{model_id}`")]
    LibraryModelMissing { model: String, model_id: String },

    #[error("router model `{model}` compatibility is {status:?}; load is blocked: {reasons:?}")]
    CompatibilityBlocked {
        model: String,
        status: CompatibilityStatus,
        reasons: Vec<String>,
    },

    #[error("router model-store lookup failed: {message}")]
    ModelStore { message: String },

    #[error("router operation transport failed for {path}: {message}")]
    Transport { path: String, message: String },

    #[error("router operation endpoint {path} is unsupported (HTTP {status_code})")]
    EndpointUnsupported { path: String, status_code: u16 },

    #[error("router operation authentication failed at {path} (HTTP {status_code})")]
    AuthenticationRejected { path: String, status_code: u16 },

    #[error("router operation {path} returned HTTP {status_code}: {body_excerpt}")]
    HttpFailure {
        path: String,
        status_code: u16,
        body_excerpt: String,
    },

    #[error("router operation {path} did not return {{\"success\":true}}: {body_excerpt}")]
    ProtocolDrift { path: String, body_excerpt: String },

    #[error("router operation cancelled")]
    Cancelled,

    #[error("timed out waiting for router model `{model}` to become {expected}; last phase={last_phase}")]
    Timeout {
        model: String,
        expected: String,
        last_phase: String,
    },

    #[error("router post-operation reconciliation failed: {message}")]
    Reconciliation { message: String },
}

fn compatibility_for(
    router_model: &RouterModel,
    installation: &LlamaInstallation,
    model_store: &ModelStore,
) -> Result<RouterCompatibilityEvidence, RouterOperationError> {
    let model = resolve_library_model(router_model, model_store)?;
    let result = evaluate_compatibility(&model, installation, None);
    let reasons: Vec<String> = result
        .reasons
        .iter()
        .map(|reason| format!("{}: {}", reason.code, reason.message))
        .collect();

    if matches!(
        result.status,
        CompatibilityStatus::Incompatible | CompatibilityStatus::Unknown
    ) {
        return Err(RouterOperationError::CompatibilityBlocked {
            model: router_model.id.clone(),
            status: result.status,
            reasons,
        });
    }

    Ok(RouterCompatibilityEvidence {
        model_id: model.id,
        status: result.status,
        reasons,
    })
}

fn resolve_library_model(
    router_model: &RouterModel,
    model_store: &ModelStore,
) -> Result<ModelInfo, RouterOperationError> {
    if let Some(model_id) = router_model.library_link.model_id.as_ref() {
        return model_store
            .get_model(model_id)
            .map_err(|error| RouterOperationError::ModelStore {
                message: error.to_string(),
            })?
            .ok_or_else(|| RouterOperationError::LibraryModelMissing {
                model: router_model.id.clone(),
                model_id: model_id.clone(),
            });
    }

    let records = model_store
        .list_model_records()
        .map_err(|error| RouterOperationError::ModelStore {
            message: error.to_string(),
        })?;
    let mut candidates = Vec::new();
    for record in records {
        let exact_path = router_model
            .path
            .as_ref()
            .is_some_and(|path| path == &record.model.path);
        let exact_sha = router_model
            .sha256
            .as_deref()
            .is_some_and(|sha| sha.eq_ignore_ascii_case(&record.model.sha256));
        let path_text = record.model.path.to_string_lossy();
        let exact_argv = router_model
            .status
            .args
            .iter()
            .any(|arg| arg == path_text.as_ref());
        if exact_path || exact_sha || exact_argv {
            candidates.push(record.model);
        }
    }

    match candidates.len() {
        0 => Err(RouterOperationError::ModelIdentityUnproven {
            model: router_model.id.clone(),
        }),
        1 => Ok(candidates.remove(0)),
        _ => Err(RouterOperationError::ModelIdentityAmbiguous {
            model: router_model.id.clone(),
            candidates: candidates.into_iter().map(|model| model.id).collect(),
        }),
    }
}

fn wait_for_model<F>(
    controller: &RouterOperationController,
    installation: &LlamaInstallation,
    endpoint: &ServerEndpoint,
    model_store: Option<&ModelStore>,
    model_id: &str,
    timeout: Duration,
    cancellation: &RouterOperationCancellation,
    reached: F,
    expected: &str,
) -> Result<RouterRegistry, RouterOperationError>
where
    F: Fn(&RouterModelPhase) -> bool,
{
    let deadline = Instant::now() + timeout;
    let mut last_phase = "unknown".to_string();
    loop {
        if cancellation.is_cancelled() {
            if let Ok(registry) =
                discover_router_registry(installation, endpoint, model_store, Duration::from_secs(2))
            {
                controller.progress("operation cancelled; final live registry captured", Some(registry));
            }
            return Err(RouterOperationError::Cancelled);
        }

        let registry = discover_router_registry(
            installation,
            endpoint,
            model_store,
            Duration::from_secs(2).min(timeout),
        )?;
        require_router(&registry)?;
        let model = registry_model(&registry, model_id)?;
        last_phase = format!("{:?}", model.status.phase);
        controller.progress(
            format!("{model_id}: observed {last_phase}; waiting for {expected}"),
            Some(registry.clone()),
        );

        if model.status.failed {
            return Err(RouterOperationError::Reconciliation {
                message: format!(
                    "router model {model_id} entered failed state while waiting for {expected}; exit_code={:?}",
                    model.status.exit_code
                ),
            });
        }
        if reached(&model.status.phase) {
            return Ok(registry);
        }
        if Instant::now() >= deadline {
            return Err(RouterOperationError::Timeout {
                model: model_id.to_string(),
                expected: expected.to_string(),
                last_phase,
            });
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn require_router(registry: &RouterRegistry) -> Result<(), RouterOperationError> {
    if registry.role == RouterRole::Router {
        Ok(())
    } else {
        Err(RouterOperationError::NotRouter)
    }
}

fn registry_model<'a>(
    registry: &'a RouterRegistry,
    model_id: &str,
) -> Result<&'a RouterModel, RouterOperationError> {
    registry
        .models
        .iter()
        .find(|model| model.id == model_id || model.routing_targets.iter().any(|id| id == model_id))
        .ok_or_else(|| RouterOperationError::ModelNotFound {
            model: model_id.to_string(),
        })
}

fn is_ready(phase: &RouterModelPhase) -> bool {
    matches!(phase, RouterModelPhase::Loaded | RouterModelPhase::Sleeping)
}

fn is_running(phase: &RouterModelPhase) -> bool {
    matches!(
        phase,
        RouterModelPhase::Downloading
            | RouterModelPhase::Loading
            | RouterModelPhase::Loaded
            | RouterModelPhase::Sleeping
    )
}

fn cancelled(cancellation: &RouterOperationCancellation) -> Result<(), RouterOperationError> {
    if cancellation.is_cancelled() {
        Err(RouterOperationError::Cancelled)
    } else {
        Ok(())
    }
}

#[derive(Debug)]
struct OperationHttpResponse {
    status_code: u16,
    body: String,
}

fn request(
    endpoint: &ServerEndpoint,
    method: &str,
    path: &str,
    body: Option<&str>,
    timeout: Duration,
) -> Result<OperationHttpResponse, RouterOperationError> {
    let addresses: Vec<_> = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| RouterOperationError::Transport {
            path: path.into(),
            message: error.to_string(),
        })?
        .collect();
    if addresses.is_empty() {
        return Err(RouterOperationError::Transport {
            path: path.into(),
            message: "host resolved to no addresses".into(),
        });
    }
    if !endpoint.allow_non_loopback && addresses.iter().any(|address| !address.ip().is_loopback()) {
        return Err(RouterOperationError::Transport {
            path: path.into(),
            message: "non-loopback router target requires explicit opt-in".into(),
        });
    }

    let mut last_error = None;
    let mut stream = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(candidate) => {
                stream = Some(candidate);
                break;
            }
            Err(error) => last_error = Some(error),
        }
    }
    let mut stream = stream.ok_or_else(|| RouterOperationError::Transport {
        path: path.into(),
        message: last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "connection failed".into()),
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| RouterOperationError::Transport {
            path: path.into(),
            message: error.to_string(),
        })?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| RouterOperationError::Transport {
            path: path.into(),
            message: error.to_string(),
        })?;

    let body = body.unwrap_or("");
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n",
        endpoint.authority()
    );
    if let Some(api_key) = endpoint.api_key.as_ref() {
        request.push_str("Authorization: Bearer ");
        request.push_str(api_key);
        request.push_str("\r\n");
    }
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    request.push_str(body);
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|error| RouterOperationError::Transport {
            path: path.into(),
            message: error.to_string(),
        })?;

    let mut bytes = Vec::new();
    stream
        .take(MAX_ROUTER_OPERATION_RESPONSE_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| RouterOperationError::Transport {
            path: path.into(),
            message: error.to_string(),
        })?;
    let response = String::from_utf8_lossy(&bytes);
    let status_line = response
        .lines()
        .next()
        .ok_or_else(|| RouterOperationError::Transport {
            path: path.into(),
            message: "empty HTTP response".into(),
        })?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| RouterOperationError::Transport {
            path: path.into(),
            message: format!("invalid HTTP status line: {status_line}"),
        })?;
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or("")
        .to_string();
    Ok(OperationHttpResponse { status_code, body })
}

fn ensure_mutation_success(
    path: &str,
    response: &OperationHttpResponse,
) -> Result<(), RouterOperationError> {
    match response.status_code {
        200..=299 => Ok(()),
        401 | 403 => Err(RouterOperationError::AuthenticationRejected {
            path: path.into(),
            status_code: response.status_code,
        }),
        404 | 405 => Err(RouterOperationError::EndpointUnsupported {
            path: path.into(),
            status_code: response.status_code,
        }),
        status_code => Err(RouterOperationError::HttpFailure {
            path: path.into(),
            status_code,
            body_excerpt: response.body.chars().take(4096).collect(),
        }),
    }
}

fn ensure_success_json(
    path: &str,
    response: &OperationHttpResponse,
) -> Result<(), RouterOperationError> {
    ensure_mutation_success(path, response)?;
    let body: Value = serde_json::from_str(&response.body).map_err(|_| {
        RouterOperationError::ProtocolDrift {
            path: path.into(),
            body_excerpt: response.body.chars().take(4096).collect(),
        }
    })?;
    if body.get("success").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(RouterOperationError::ProtocolDrift {
            path: path.into(),
            body_excerpt: response.body.chars().take(4096).collect(),
        })
    }
}
