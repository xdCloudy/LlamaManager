use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    llama::{LlamaInstallation, now_ms},
    model_store::ModelStore,
    router::{RouterModelPhase, RouterRegistry, discover_router_registry},
    router_observability::{EvidenceAvailability, discover_router_observability},
    router_operations::{
        RouterOperationCancellation, RouterOperationController, RouterOperationError,
    },
    server_readiness::ServerEndpoint,
};

const SWITCH_BENCHMARK_SCHEMA_VERSION: u32 = 1;
const MAX_FIRST_TOKEN_RESPONSE_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetHistoryState {
    FirstLoadInRun,
    PreviouslyLoadedInRun { prior_loads: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEvidence {
    pub target_history: TargetHistoryState,
    pub os_page_cache_known: bool,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveRequestEvictionExercise {
    UnsupportedBySelectedRuntime { reason: String },
    EvidenceAvailableNotExercised { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterSwitchBenchmarkEnvelope {
    pub schema_version: u32,
    pub installation_id: String,
    pub server_sha256: String,
    pub server_version: Option<String>,
    pub endpoint_authority: String,
    pub model_a_router_id: String,
    pub model_a_library_id: String,
    pub model_a_sha256: String,
    pub model_b_router_id: String,
    pub model_b_library_id: String,
    pub model_b_sha256: String,
    pub router_settings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterSwitchPhaseTimings {
    /// Source unload/eviction operation including authoritative unload reconciliation.
    pub unload_or_evict_ms: u128,
    /// Target load operation through the point where the operation controller confirms ready.
    pub load_to_ready_ms: u128,
    /// Separate post-load live-registry confirmation. On synchronous router loads this is the
    /// independently measurable readiness phase after the load-to-ready operation returns.
    pub readiness_confirmation_ms: u128,
    /// Time from request dispatch until the first streamed completion payload is observed.
    pub first_token_ms: Option<u128>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterSwitchLeg {
    pub source_model: String,
    pub target_model: String,
    pub cache: CacheEvidence,
    pub timings: RouterSwitchPhaseTimings,
    pub final_registry: RouterRegistry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterSwitchBenchmarkPhase {
    Baseline,
    UnloadOrEvict,
    Load,
    Readiness,
    FirstToken,
    Recovery,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterSwitchRecoveryEvidence {
    pub attempted: bool,
    pub recovered: bool,
    pub model: String,
    pub message: String,
    pub registry: Option<RouterRegistry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RouterSwitchBenchmarkOutcome {
    Succeeded,
    Failed {
        phase: RouterSwitchBenchmarkPhase,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterSwitchBenchmarkRun {
    pub id: String,
    pub envelope: RouterSwitchBenchmarkEnvelope,
    pub started_at_unix_ms: u128,
    pub finished_at_unix_ms: u128,
    pub legs: Vec<RouterSwitchLeg>,
    pub outcome: RouterSwitchBenchmarkOutcome,
    pub recovery: Option<RouterSwitchRecoveryEvidence>,
    pub active_request_eviction: ActiveRequestEvictionExercise,
}

impl RouterSwitchBenchmarkRun {
    pub fn succeeded(&self) -> bool {
        matches!(self.outcome, RouterSwitchBenchmarkOutcome::Succeeded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterSwitchBenchmarkConfig {
    pub model_a: String,
    pub model_b: String,
    pub timeout: Duration,
    pub first_token_prompt: String,
    pub router_settings: Vec<String>,
}

impl RouterSwitchBenchmarkConfig {
    pub fn new(model_a: impl Into<String>, model_b: impl Into<String>) -> Self {
        Self {
            model_a: model_a.into(),
            model_b: model_b.into(),
            timeout: Duration::from_secs(120),
            first_token_prompt: "Reply with OK".into(),
            router_settings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterSwitchComparison {
    pub left_run_id: String,
    pub right_run_id: String,
    pub a_to_b_total_delta_ms: i128,
    pub b_to_a_total_delta_ms: i128,
    pub a_to_b_first_token_delta_ms: Option<i128>,
    pub b_to_a_first_token_delta_ms: Option<i128>,
}

#[derive(Debug, Error)]
pub enum RouterSwitchBenchmarkError {
    #[error("model `{model}` is not present in the live router registry")]
    ModelNotFound { model: String },

    #[error("model `{model}` is not mapped to exactly one M2 library model")]
    ModelIdentityUnproven { model: String },

    #[error("model `{model}` maps to missing M2 library record `{model_id}`")]
    LibraryModelMissing { model: String, model_id: String },

    #[error("selected llama.cpp installation has no llama-server evidence")]
    MissingServerEvidence,

    #[error("router switch benchmark requires two different model identities")]
    SameModel,

    #[error("router operation failed during {phase:?}: {message}")]
    PhaseFailure {
        phase: RouterSwitchBenchmarkPhase,
        message: String,
    },

    #[error("router registry discovery failed: {0}")]
    Discovery(String),

    #[error("first-token transport failed: {0}")]
    FirstTokenTransport(String),

    #[error("first-token request returned HTTP {status_code}: {body_excerpt}")]
    FirstTokenHttp {
        status_code: u16,
        body_excerpt: String,
    },

    #[error("first-token stream closed before token payload evidence was observed")]
    FirstTokenMissing,

    #[error("switch benchmark persistence failed: {0}")]
    Persistence(String),

    #[error("benchmark runs use incompatible comparison envelopes")]
    IncompatibleEnvelope,

    #[error("failed benchmark samples cannot be compared as successful timing runs")]
    FailedSampleComparison,
}

#[derive(Debug, Clone)]
pub struct RouterSwitchBenchmarkStore {
    path: PathBuf,
}

impl RouterSwitchBenchmarkStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, RouterSwitchBenchmarkError> {
        let store = Self { path: path.into() };
        store.initialize()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connection(&self) -> Result<Connection, RouterSwitchBenchmarkError> {
        let connection = Connection::open(&self.path)
            .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?;
        Ok(connection)
    }

    fn initialize(&self) -> Result<(), RouterSwitchBenchmarkError> {
        self.connection()?
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS router_switch_benchmark_runs(
                    id TEXT PRIMARY KEY,
                    envelope_json TEXT NOT NULL,
                    run_json TEXT NOT NULL,
                    succeeded INTEGER NOT NULL CHECK(succeeded IN (0,1)),
                    started_at_unix_ms TEXT NOT NULL,
                    finished_at_unix_ms TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_router_switch_benchmark_envelope
                    ON router_switch_benchmark_runs(envelope_json, started_at_unix_ms);",
            )
            .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?;
        Ok(())
    }

    pub fn save(&self, run: &RouterSwitchBenchmarkRun) -> Result<(), RouterSwitchBenchmarkError> {
        let envelope_json = serde_json::to_string(&run.envelope)
            .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?;
        let run_json = serde_json::to_string(run)
            .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?;
        self.connection()?
            .execute(
                "INSERT INTO router_switch_benchmark_runs(
                    id, envelope_json, run_json, succeeded,
                    started_at_unix_ms, finished_at_unix_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                    envelope_json = excluded.envelope_json,
                    run_json = excluded.run_json,
                    succeeded = excluded.succeeded,
                    started_at_unix_ms = excluded.started_at_unix_ms,
                    finished_at_unix_ms = excluded.finished_at_unix_ms",
                params![
                    run.id,
                    envelope_json,
                    run_json,
                    (if run.succeeded() { 1_i64 } else { 0_i64 }),
                    run.started_at_unix_ms.to_string(),
                    run.finished_at_unix_ms.to_string(),
                ],
            )
            .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?;
        Ok(())
    }

    pub fn get(
        &self,
        id: &str,
    ) -> Result<Option<RouterSwitchBenchmarkRun>, RouterSwitchBenchmarkError> {
        let json: Option<String> = self
            .connection()?
            .query_row(
                "SELECT run_json FROM router_switch_benchmark_runs WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?;
        json.map(|json| {
            serde_json::from_str(&json)
                .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))
        })
        .transpose()
    }

    pub fn comparable_runs(
        &self,
        envelope: &RouterSwitchBenchmarkEnvelope,
    ) -> Result<Vec<RouterSwitchBenchmarkRun>, RouterSwitchBenchmarkError> {
        let envelope_json = serde_json::to_string(envelope)
            .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?;
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT run_json FROM router_switch_benchmark_runs
                 WHERE envelope_json = ?1
                 ORDER BY CAST(started_at_unix_ms AS INTEGER), id",
            )
            .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?;
        let rows = statement
            .query_map([envelope_json], |row| row.get::<_, String>(0))
            .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?;
        rows.map(|row| {
            let json =
                row.map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?;
            serde_json::from_str(&json)
                .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))
        })
        .collect()
    }
}

pub fn compare_switch_runs(
    left: &RouterSwitchBenchmarkRun,
    right: &RouterSwitchBenchmarkRun,
) -> Result<RouterSwitchComparison, RouterSwitchBenchmarkError> {
    if left.envelope != right.envelope {
        return Err(RouterSwitchBenchmarkError::IncompatibleEnvelope);
    }
    if !left.succeeded() || !right.succeeded() || left.legs.len() != 2 || right.legs.len() != 2 {
        return Err(RouterSwitchBenchmarkError::FailedSampleComparison);
    }

    Ok(RouterSwitchComparison {
        left_run_id: left.id.clone(),
        right_run_id: right.id.clone(),
        a_to_b_total_delta_ms: leg_total_ms(&right.legs[0]) as i128
            - leg_total_ms(&left.legs[0]) as i128,
        b_to_a_total_delta_ms: leg_total_ms(&right.legs[1]) as i128
            - leg_total_ms(&left.legs[1]) as i128,
        a_to_b_first_token_delta_ms: timing_delta(
            left.legs[0].timings.first_token_ms,
            right.legs[0].timings.first_token_ms,
        ),
        b_to_a_first_token_delta_ms: timing_delta(
            left.legs[1].timings.first_token_ms,
            right.legs[1].timings.first_token_ms,
        ),
    })
}

fn timing_delta(left: Option<u128>, right: Option<u128>) -> Option<i128> {
    Some(right? as i128 - left? as i128)
}

fn leg_total_ms(leg: &RouterSwitchLeg) -> u128 {
    leg.timings.unload_or_evict_ms
        + leg.timings.load_to_ready_ms
        + leg.timings.readiness_confirmation_ms
        + leg.timings.first_token_ms.unwrap_or(0)
}

pub fn run_switch_round_trip(
    installation: &LlamaInstallation,
    endpoint: &ServerEndpoint,
    model_store: &ModelStore,
    config: &RouterSwitchBenchmarkConfig,
) -> RouterSwitchBenchmarkRun {
    let started_at_unix_ms = now_ms();
    let id = Uuid::new_v4().to_string();

    let setup = prepare_envelope(installation, endpoint, model_store, config);
    let (envelope, active_request_eviction) = match setup {
        Ok(value) => value,
        Err(error) => {
            return failed_without_envelope(
                id,
                installation,
                endpoint,
                config,
                started_at_unix_ms,
                RouterSwitchBenchmarkPhase::Baseline,
                error.to_string(),
            );
        }
    };

    let controller = RouterOperationController::new();
    let mut loaded_counts = std::collections::BTreeMap::<String, u32>::new();
    let mut legs = Vec::new();

    if let Err(error) = establish_baseline(
        &controller,
        installation,
        endpoint,
        model_store,
        config,
        &mut loaded_counts,
    ) {
        let recovery = recover_a(&controller, installation, endpoint, model_store, config);
        return RouterSwitchBenchmarkRun {
            id,
            envelope,
            started_at_unix_ms,
            finished_at_unix_ms: now_ms(),
            legs,
            outcome: RouterSwitchBenchmarkOutcome::Failed {
                phase: RouterSwitchBenchmarkPhase::Baseline,
                message: error.to_string(),
            },
            recovery: Some(recovery),
            active_request_eviction,
        };
    }

    for (source, target) in [
        (config.model_a.as_str(), config.model_b.as_str()),
        (config.model_b.as_str(), config.model_a.as_str()),
    ] {
        match run_leg(
            &controller,
            installation,
            endpoint,
            model_store,
            config,
            source,
            target,
            &mut loaded_counts,
        ) {
            Ok(leg) => legs.push(leg),
            Err((phase, message)) => {
                let recovery = recover_a(&controller, installation, endpoint, model_store, config);
                return RouterSwitchBenchmarkRun {
                    id,
                    envelope,
                    started_at_unix_ms,
                    finished_at_unix_ms: now_ms(),
                    legs,
                    outcome: RouterSwitchBenchmarkOutcome::Failed { phase, message },
                    recovery: Some(recovery),
                    active_request_eviction,
                };
            }
        }
    }

    RouterSwitchBenchmarkRun {
        id,
        envelope,
        started_at_unix_ms,
        finished_at_unix_ms: now_ms(),
        legs,
        outcome: RouterSwitchBenchmarkOutcome::Succeeded,
        recovery: None,
        active_request_eviction,
    }
}

fn failed_without_envelope(
    id: String,
    installation: &LlamaInstallation,
    endpoint: &ServerEndpoint,
    config: &RouterSwitchBenchmarkConfig,
    started_at_unix_ms: u128,
    phase: RouterSwitchBenchmarkPhase,
    message: String,
) -> RouterSwitchBenchmarkRun {
    let server = installation.server.as_ref();
    RouterSwitchBenchmarkRun {
        id,
        envelope: RouterSwitchBenchmarkEnvelope {
            schema_version: SWITCH_BENCHMARK_SCHEMA_VERSION,
            installation_id: installation.id.clone(),
            server_sha256: server
                .map(|server| server.sha256.clone())
                .unwrap_or_default(),
            server_version: server.and_then(|server| {
                (!server.version_output.trim().is_empty())
                    .then(|| server.version_output.trim().to_string())
            }),
            endpoint_authority: endpoint.authority(),
            model_a_router_id: config.model_a.clone(),
            model_a_library_id: String::new(),
            model_a_sha256: String::new(),
            model_b_router_id: config.model_b.clone(),
            model_b_library_id: String::new(),
            model_b_sha256: String::new(),
            router_settings: config.router_settings.clone(),
        },
        started_at_unix_ms,
        finished_at_unix_ms: now_ms(),
        legs: Vec::new(),
        outcome: RouterSwitchBenchmarkOutcome::Failed { phase, message },
        recovery: None,
        active_request_eviction: ActiveRequestEvictionExercise::UnsupportedBySelectedRuntime {
            reason: "benchmark setup failed before active-request evidence could be evaluated"
                .into(),
        },
    }
}

fn prepare_envelope(
    installation: &LlamaInstallation,
    endpoint: &ServerEndpoint,
    model_store: &ModelStore,
    config: &RouterSwitchBenchmarkConfig,
) -> Result<
    (RouterSwitchBenchmarkEnvelope, ActiveRequestEvictionExercise),
    RouterSwitchBenchmarkError,
> {
    if config.model_a == config.model_b {
        return Err(RouterSwitchBenchmarkError::SameModel);
    }
    let server = installation
        .server
        .as_ref()
        .ok_or(RouterSwitchBenchmarkError::MissingServerEvidence)?;
    let registry =
        discover_router_registry(installation, endpoint, Some(model_store), config.timeout)
            .map_err(|error| RouterSwitchBenchmarkError::Discovery(error.to_string()))?;
    let a = registry
        .models
        .iter()
        .find(|model| model.id == config.model_a)
        .ok_or_else(|| RouterSwitchBenchmarkError::ModelNotFound {
            model: config.model_a.clone(),
        })?;
    let b = registry
        .models
        .iter()
        .find(|model| model.id == config.model_b)
        .ok_or_else(|| RouterSwitchBenchmarkError::ModelNotFound {
            model: config.model_b.clone(),
        })?;
    let (a_library_id, a_sha) = library_identity(a, model_store)?;
    let (b_library_id, b_sha) = library_identity(b, model_store)?;

    let active_request_eviction = match discover_router_observability(
        installation,
        endpoint,
        Some(model_store),
        config.timeout,
    ) {
        Ok(snapshot) => {
            let observed = snapshot
                .models
                .iter()
                .any(|model| model.active_requests.availability == EvidenceAvailability::Observed);
            if observed {
                ActiveRequestEvictionExercise::EvidenceAvailableNotExercised {
                    reason: "selected runtime exposes active-request evidence; the benchmark records it but does not manufacture concurrent traffic during timing samples".into(),
                }
            } else {
                ActiveRequestEvictionExercise::UnsupportedBySelectedRuntime {
                    reason: "selected router does not expose active-request count in its model registry; eviction-failure timing cannot be attributed safely".into(),
                }
            }
        }
        Err(error) => ActiveRequestEvictionExercise::UnsupportedBySelectedRuntime {
            reason: format!("active-request observability probe unavailable: {error}"),
        },
    };

    Ok((
        RouterSwitchBenchmarkEnvelope {
            schema_version: SWITCH_BENCHMARK_SCHEMA_VERSION,
            installation_id: installation.id.clone(),
            server_sha256: server.sha256.clone(),
            server_version: (!server.version_output.trim().is_empty())
                .then(|| server.version_output.trim().to_string()),
            endpoint_authority: endpoint.authority(),
            model_a_router_id: config.model_a.clone(),
            model_a_library_id: a_library_id,
            model_a_sha256: a_sha,
            model_b_router_id: config.model_b.clone(),
            model_b_library_id: b_library_id,
            model_b_sha256: b_sha,
            router_settings: config.router_settings.clone(),
        },
        active_request_eviction,
    ))
}

fn library_identity(
    model: &crate::router::RouterModel,
    store: &ModelStore,
) -> Result<(String, String), RouterSwitchBenchmarkError> {
    let model_id = model.library_link.model_id.clone().ok_or_else(|| {
        RouterSwitchBenchmarkError::ModelIdentityUnproven {
            model: model.id.clone(),
        }
    })?;
    let record = store
        .get_model(&model_id)
        .map_err(|error| RouterSwitchBenchmarkError::Persistence(error.to_string()))?
        .ok_or_else(|| RouterSwitchBenchmarkError::LibraryModelMissing {
            model: model.id.clone(),
            model_id: model_id.clone(),
        })?;
    Ok((model_id, record.sha256))
}

fn establish_baseline(
    controller: &RouterOperationController,
    installation: &LlamaInstallation,
    endpoint: &ServerEndpoint,
    model_store: &ModelStore,
    config: &RouterSwitchBenchmarkConfig,
    loaded_counts: &mut std::collections::BTreeMap<String, u32>,
) -> Result<(), RouterSwitchBenchmarkError> {
    let cancellation = RouterOperationCancellation::new();
    controller
        .unload_model(
            installation,
            endpoint,
            Some(model_store),
            &config.model_b,
            config.timeout,
            &cancellation,
        )
        .map_err(|error| phase_operation_error(RouterSwitchBenchmarkPhase::Baseline, error))?;
    controller
        .load_model(
            installation,
            endpoint,
            model_store,
            &config.model_a,
            config.timeout,
            &cancellation,
        )
        .map_err(|error| phase_operation_error(RouterSwitchBenchmarkPhase::Baseline, error))?;
    loaded_counts.insert(config.model_a.clone(), 1);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_leg(
    controller: &RouterOperationController,
    installation: &LlamaInstallation,
    endpoint: &ServerEndpoint,
    model_store: &ModelStore,
    config: &RouterSwitchBenchmarkConfig,
    source: &str,
    target: &str,
    loaded_counts: &mut std::collections::BTreeMap<String, u32>,
) -> Result<RouterSwitchLeg, (RouterSwitchBenchmarkPhase, String)> {
    let cancellation = RouterOperationCancellation::new();

    let unload_started = Instant::now();
    controller
        .unload_model(
            installation,
            endpoint,
            Some(model_store),
            source,
            config.timeout,
            &cancellation,
        )
        .map_err(|error| (RouterSwitchBenchmarkPhase::UnloadOrEvict, error.to_string()))?;
    let unload_or_evict_ms = unload_started.elapsed().as_millis();

    let prior_loads = loaded_counts.get(target).copied().unwrap_or(0);
    let cache = CacheEvidence {
        target_history: if prior_loads == 0 {
            TargetHistoryState::FirstLoadInRun
        } else {
            TargetHistoryState::PreviouslyLoadedInRun { prior_loads }
        },
        os_page_cache_known: false,
        note: "process-level model history is observed; operating-system page-cache residency is not inferred"
            .into(),
    };

    let load_started = Instant::now();
    controller
        .load_model(
            installation,
            endpoint,
            model_store,
            target,
            config.timeout,
            &cancellation,
        )
        .map_err(|error| (RouterSwitchBenchmarkPhase::Load, error.to_string()))?;
    let load_to_ready_ms = load_started.elapsed().as_millis();
    *loaded_counts.entry(target.to_string()).or_default() += 1;

    let readiness_started = Instant::now();
    let registry =
        discover_router_registry(installation, endpoint, Some(model_store), config.timeout)
            .map_err(|error| (RouterSwitchBenchmarkPhase::Readiness, error.to_string()))?;
    let readiness_confirmation_ms = readiness_started.elapsed().as_millis();
    let target_state = registry
        .models
        .iter()
        .find(|model| model.id == target)
        .ok_or_else(|| {
            (
                RouterSwitchBenchmarkPhase::Readiness,
                format!("target {target} disappeared from live registry"),
            )
        })?;
    if !is_ready(&target_state.status.phase) || target_state.status.failed {
        return Err((
            RouterSwitchBenchmarkPhase::Readiness,
            format!(
                "target {target} was not ready after load: {:?}, failed={}",
                target_state.status.phase, target_state.status.failed
            ),
        ));
    }

    let first_token_ms =
        measure_first_token(endpoint, target, &config.first_token_prompt, config.timeout)
            .map_err(|error| (RouterSwitchBenchmarkPhase::FirstToken, error.to_string()))?;

    Ok(RouterSwitchLeg {
        source_model: source.into(),
        target_model: target.into(),
        cache,
        timings: RouterSwitchPhaseTimings {
            unload_or_evict_ms,
            load_to_ready_ms,
            readiness_confirmation_ms,
            first_token_ms: Some(first_token_ms),
            notes: vec![
                "load_to_ready_ms is intentionally combined because the #39 controller returns only after authoritative router readiness reconciliation".into(),
                "readiness_confirmation_ms is a second independent live registry confirmation after load-to-ready completed".into(),
            ],
        },
        final_registry: registry,
    })
}

fn recover_a(
    controller: &RouterOperationController,
    installation: &LlamaInstallation,
    endpoint: &ServerEndpoint,
    model_store: &ModelStore,
    config: &RouterSwitchBenchmarkConfig,
) -> RouterSwitchRecoveryEvidence {
    let cancellation = RouterOperationCancellation::new();
    match controller.load_model(
        installation,
        endpoint,
        model_store,
        &config.model_a,
        config.timeout,
        &cancellation,
    ) {
        Ok(evidence) => RouterSwitchRecoveryEvidence {
            attempted: true,
            recovered: evidence
                .registry
                .models
                .iter()
                .find(|model| model.id == config.model_a)
                .is_some_and(|model| is_ready(&model.status.phase) && !model.status.failed),
            model: config.model_a.clone(),
            message: "reloaded baseline model A after benchmark failure".into(),
            registry: Some(evidence.registry),
        },
        Err(error) => RouterSwitchRecoveryEvidence {
            attempted: true,
            recovered: false,
            model: config.model_a.clone(),
            message: error.to_string(),
            registry: discover_router_registry(
                installation,
                endpoint,
                Some(model_store),
                config.timeout,
            )
            .ok(),
        },
    }
}

fn phase_operation_error(
    phase: RouterSwitchBenchmarkPhase,
    error: RouterOperationError,
) -> RouterSwitchBenchmarkError {
    RouterSwitchBenchmarkError::PhaseFailure {
        phase,
        message: error.to_string(),
    }
}

fn is_ready(phase: &RouterModelPhase) -> bool {
    matches!(phase, RouterModelPhase::Loaded | RouterModelPhase::Sleeping)
}

pub fn measure_first_token(
    endpoint: &ServerEndpoint,
    model: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<u128, RouterSwitchBenchmarkError> {
    let mut stream = connect(endpoint, timeout)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| RouterSwitchBenchmarkError::FirstTokenTransport(error.to_string()))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| RouterSwitchBenchmarkError::FirstTokenTransport(error.to_string()))?;

    let body = json!({
        "model": model,
        "prompt": prompt,
        "n_predict": 1,
        "temperature": 0,
        "stream": true
    })
    .to_string();
    let mut request = format!(
        "POST /completion HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: text/event-stream\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        endpoint.authority(),
        body.len()
    );
    if let Some(api_key) = endpoint.api_key.as_ref() {
        request.push_str("Authorization: Bearer ");
        request.push_str(api_key);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.push_str(&body);

    let started = Instant::now();
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|error| RouterSwitchBenchmarkError::FirstTokenTransport(error.to_string()))?;

    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut header_end = None;
    loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|error| RouterSwitchBenchmarkError::FirstTokenTransport(error.to_string()))?;
        if read == 0 {
            return Err(RouterSwitchBenchmarkError::FirstTokenMissing);
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > MAX_FIRST_TOKEN_RESPONSE_BYTES {
            return Err(RouterSwitchBenchmarkError::FirstTokenTransport(
                "first-token response exceeded bounded evidence limit".into(),
            ));
        }

        if header_end.is_none() {
            header_end = find_bytes(&bytes, b"\r\n\r\n").map(|index| index + 4);
            if let Some(end) = header_end {
                let status = parse_status(&bytes[..end])?;
                if !(200..=299).contains(&status) {
                    return Err(RouterSwitchBenchmarkError::FirstTokenHttp {
                        status_code: status,
                        body_excerpt: String::from_utf8_lossy(&bytes[end..])
                            .chars()
                            .take(2048)
                            .collect(),
                    });
                }
            }
        }

        if let Some(end) = header_end {
            let body = String::from_utf8_lossy(&bytes[end..]);
            if body.contains("\"content\"") || body.contains("data: {") {
                return Ok(started.elapsed().as_millis());
            }
        }
    }
}

fn connect(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<TcpStream, RouterSwitchBenchmarkError> {
    let addresses = resolve_addresses(endpoint)?;
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(RouterSwitchBenchmarkError::FirstTokenTransport(
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "no resolved address accepted the connection".into()),
    ))
}

fn resolve_addresses(
    endpoint: &ServerEndpoint,
) -> Result<Vec<SocketAddr>, RouterSwitchBenchmarkError> {
    if endpoint.port == 0 {
        return Err(RouterSwitchBenchmarkError::FirstTokenTransport(
            "server port must be in 1..=65535".into(),
        ));
    }
    let addresses: Vec<_> = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| RouterSwitchBenchmarkError::FirstTokenTransport(error.to_string()))?
        .collect();
    if addresses.is_empty() {
        return Err(RouterSwitchBenchmarkError::FirstTokenTransport(
            "server host resolved to no addresses".into(),
        ));
    }
    if !endpoint.allow_non_loopback && addresses.iter().any(|address| !address.ip().is_loopback()) {
        return Err(RouterSwitchBenchmarkError::FirstTokenTransport(
            "non-loopback target requires explicit opt-in".into(),
        ));
    }
    Ok(addresses)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_status(headers: &[u8]) -> Result<u16, RouterSwitchBenchmarkError> {
    let text = String::from_utf8_lossy(headers);
    text.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| {
            RouterSwitchBenchmarkError::FirstTokenTransport(
                "invalid HTTP status line during first-token probe".into(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn envelope() -> RouterSwitchBenchmarkEnvelope {
        RouterSwitchBenchmarkEnvelope {
            schema_version: SWITCH_BENCHMARK_SCHEMA_VERSION,
            installation_id: "runtime".into(),
            server_sha256: "server-sha".into(),
            server_version: Some("b10472".into()),
            endpoint_authority: "127.0.0.1:8080".into(),
            model_a_router_id: "a".into(),
            model_a_library_id: "lib-a".into(),
            model_a_sha256: "sha-a".into(),
            model_b_router_id: "b".into(),
            model_b_library_id: "lib-b".into(),
            model_b_sha256: "sha-b".into(),
            router_settings: vec!["--models-max=1".into()],
        }
    }

    fn registry() -> RouterRegistry {
        serde_json::from_value(json!({
            "endpoint": "127.0.0.1:8080",
            "role": "router",
            "static_capabilities": {
                "server_sha256": null,
                "server_version": null,
                "router_cli_observed": true,
                "models_dir": true,
                "models_preset": false,
                "models_max": true,
                "models_autoload": true,
                "observed_options": []
            },
            "endpoints": {
                "props": {"state":"supported","reason":"fixture"},
                "list_models": {"state":"supported","reason":"fixture"},
                "reload_models": {"state":"unknown","reason":"fixture"},
                "load_model": {"state":"unknown","reason":"fixture"},
                "unload_model": {"state":"unknown","reason":"fixture"},
                "model_events": {"state":"unknown","reason":"fixture"}
            },
            "models": [],
            "observed_at_unix_ms": 1
        }))
        .unwrap()
    }

    fn successful_run(
        id: &str,
        envelope: RouterSwitchBenchmarkEnvelope,
    ) -> RouterSwitchBenchmarkRun {
        let leg = |source: &str, target: &str, first_token: u128| RouterSwitchLeg {
            source_model: source.into(),
            target_model: target.into(),
            cache: CacheEvidence {
                target_history: TargetHistoryState::FirstLoadInRun,
                os_page_cache_known: false,
                note: "unknown".into(),
            },
            timings: RouterSwitchPhaseTimings {
                unload_or_evict_ms: 10,
                load_to_ready_ms: 20,
                readiness_confirmation_ms: 5,
                first_token_ms: Some(first_token),
                notes: Vec::new(),
            },
            final_registry: registry(),
        };
        RouterSwitchBenchmarkRun {
            id: id.into(),
            envelope,
            started_at_unix_ms: 1,
            finished_at_unix_ms: 2,
            legs: vec![leg("a", "b", 30), leg("b", "a", 40)],
            outcome: RouterSwitchBenchmarkOutcome::Succeeded,
            recovery: None,
            active_request_eviction: ActiveRequestEvictionExercise::UnsupportedBySelectedRuntime {
                reason: "fixture".into(),
            },
        }
    }

    #[test]
    fn persisted_runs_round_trip_and_filter_by_exact_envelope() {
        let temp = tempdir().unwrap();
        let store = RouterSwitchBenchmarkStore::open(temp.path().join("bench.sqlite")).unwrap();
        let run = successful_run("one", envelope());
        store.save(&run).unwrap();
        assert_eq!(store.get("one").unwrap(), Some(run.clone()));
        assert_eq!(store.comparable_runs(&run.envelope).unwrap(), vec![run]);
    }

    #[test]
    fn comparison_rejects_incompatible_envelopes() {
        let left = successful_run("left", envelope());
        let mut other = envelope();
        other.server_sha256 = "different-runtime".into();
        let right = successful_run("right", other);
        assert!(matches!(
            compare_switch_runs(&left, &right),
            Err(RouterSwitchBenchmarkError::IncompatibleEnvelope)
        ));
    }

    #[test]
    fn failed_samples_persist_as_failed_and_are_not_compared() {
        let temp = tempdir().unwrap();
        let store = RouterSwitchBenchmarkStore::open(temp.path().join("bench.sqlite")).unwrap();
        let mut failed = successful_run("failed", envelope());
        failed.outcome = RouterSwitchBenchmarkOutcome::Failed {
            phase: RouterSwitchBenchmarkPhase::Load,
            message: "HTTP 500".into(),
        };
        store.save(&failed).unwrap();
        let reloaded = store.get("failed").unwrap().unwrap();
        assert!(!reloaded.succeeded());
        assert!(matches!(
            compare_switch_runs(&reloaded, &reloaded),
            Err(RouterSwitchBenchmarkError::FailedSampleComparison)
        ));
    }

    #[test]
    fn comparison_reports_phase_deltas_for_matching_envelopes() {
        let left = successful_run("left", envelope());
        let mut right = successful_run("right", envelope());
        right.legs[0].timings.first_token_ms = Some(35);
        right.legs[1].timings.first_token_ms = Some(38);
        let comparison = compare_switch_runs(&left, &right).unwrap();
        assert_eq!(comparison.a_to_b_first_token_delta_ms, Some(5));
        assert_eq!(comparison.b_to_a_first_token_delta_ms, Some(-2));
    }
}
