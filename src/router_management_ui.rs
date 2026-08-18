use std::{fs, thread, time::Duration};

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    llama::LlamaInstallation,
    model_store::ModelStore,
    paths::AppPaths,
    persistence::Database,
    router::{
        RouterFeatureEvidence, RouterFeatureState, RouterModelPhase, RouterRegistry, RouterRole,
    },
    router_observability::{
        EvidenceAvailability, RouterModelObservability, RouterObservabilitySnapshot,
        RouterObservabilityTracker, RouterSnapshotFreshness, discover_router_observability,
    },
    router_operations::{
        RouterOperationCancellation, RouterOperationController, RouterOperationState,
    },
    server_readiness::ServerEndpoint,
};

const REFRESH_TIMEOUT: Duration = Duration::from_secs(5);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(120);
const PREFERENCES_FILE: &str = "router-control.json";

const CSS: &str = r#"
.rm-page{min-height:100vh;padding:30px 34px 92px;color:#f6eaff;background:radial-gradient(circle at 80% 8%,rgba(255,0,190,.13),transparent 34%),radial-gradient(circle at 7% 82%,rgba(0,255,255,.08),transparent 36%),#07000e;font-family:"Cascadia Mono","Cascadia Code",Consolas,monospace;box-sizing:border-box}.rm-page *{box-sizing:border-box}.rm-header{display:flex;justify-content:space-between;gap:24px;align-items:flex-start;padding-bottom:18px;border-bottom:1px solid rgba(0,255,255,.42)}.rm-kicker{color:#00ffff;font-size:9px;font-weight:900;letter-spacing:.15em}.rm-header h1{margin:7px 0 8px;font-size:clamp(26px,3vw,40px)}.rm-header p,.rm-muted{margin:0;color:#a996bb;font-size:10px;line-height:1.65}.rm-fresh{text-align:right;min-width:190px}.rm-fresh strong{display:block;margin:5px 0;font-size:18px}.rm-badge{display:inline-flex;align-items:center;min-height:22px;padding:0 7px;border:1px solid rgba(0,255,255,.45);color:#76ffe6;font-size:8px;font-weight:900;letter-spacing:.07em;text-transform:uppercase}.rm-badge.warn{border-color:#ffd36b;color:#ffd36b}.rm-badge.error{border-color:#ff3d7f;color:#ff7ba9}.rm-notice{margin-top:12px;padding:9px 11px;border:1px solid rgba(117,255,226,.48);background:rgba(0,20,18,.6);color:#baffed;font-size:9px;line-height:1.55;overflow-wrap:anywhere}.rm-notice.error{border-color:rgba(255,50,110,.58);background:rgba(40,0,18,.58);color:#ff91b5}.rm-panel{margin-top:14px;min-width:0;border:1px solid rgba(0,255,255,.32);background:linear-gradient(180deg,rgba(29,5,47,.83),rgba(7,0,15,.92))}.rm-panel-head{display:flex;justify-content:space-between;align-items:center;gap:12px;padding:12px 14px;border-bottom:1px solid rgba(0,255,255,.25)}.rm-panel-head h2{margin:4px 0 0;font-size:16px}.rm-panel-body{padding:13px;min-width:0}.rm-fields{display:grid;grid-template-columns:minmax(0,1fr) 130px;gap:9px}.rm-field.wide{grid-column:1/-1}.rm-field label{display:block;margin-bottom:5px;color:#9b80a9;font-size:8px;letter-spacing:.08em;text-transform:uppercase}.rm-input,.rm-select{width:100%;min-height:34px;padding:7px 9px;border:1px solid rgba(0,255,255,.3);border-radius:0;background:#030008;color:#f6eaff;font:inherit;font-size:10px}.rm-input:focus-visible,.rm-select:focus-visible,.rm-button:focus-visible{outline:2px solid #ff00ff;outline-offset:2px}.rm-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:11px}.rm-button{min-height:34px;padding:0 12px;border:1px solid #00dbe7;border-radius:0;background:transparent;color:#00f5ff;font:inherit;font-size:8px;font-weight:900;letter-spacing:.08em;text-transform:uppercase;cursor:pointer}.rm-button:hover:not(:disabled),.rm-button.primary{background:#00ffff;color:#050009}.rm-button.magenta{border-color:#ff00d4;color:#ff55e7}.rm-button.danger{border-color:#ff356f;color:#ff739f}.rm-button:disabled{opacity:.32;cursor:not-allowed}.rm-runtime,.rm-detail{margin-top:10px;padding:9px;border-left:2px solid #00ffff;background:rgba(0,0,0,.34);color:#cbb9d7;font-size:9px;line-height:1.55;overflow-wrap:anywhere;word-break:break-word;white-space:pre-wrap}.rm-detail{border-left-color:#785b91;color:#a996bb}.rm-two{display:grid;grid-template-columns:minmax(0,1.2fr) minmax(300px,.8fr);gap:12px}.rm-operation{padding:12px;border:1px solid rgba(106,74,126,.55);background:#090011}.rm-operation strong{display:block;margin:5px 0;font-size:13px}.rm-operation pre{margin:8px 0 0;color:#a996bb;font:inherit;font-size:8px;line-height:1.55;white-space:pre-wrap;overflow-wrap:anywhere}.rm-support{display:grid;gap:6px;margin-top:10px}.rm-support-row{display:grid;grid-template-columns:80px minmax(0,1fr);gap:8px;font-size:8px;line-height:1.45}.rm-support-row span{color:#887597}.rm-support-row strong{overflow-wrap:anywhere}.rm-live{color:#76ffe6}.rm-warn{color:#ffd36b}.rm-error{color:#ff7ba9}.rm-model-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px}.rm-card{min-width:0;border:1px solid rgba(106,74,126,.55);background:rgba(0,0,0,.28)}.rm-card.stale{border-style:dashed;opacity:.72}.rm-card-head{padding:10px 11px;border-bottom:1px solid rgba(106,74,126,.38)}.rm-card-head strong{display:block;font-size:12px;overflow-wrap:anywhere}.rm-aliases{display:flex;flex-wrap:wrap;gap:4px;margin-top:7px}.rm-alias{padding:3px 5px;border:1px solid rgba(255,0,212,.38);color:#ff77eb;font-size:7px}.rm-stats{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:1px;background:rgba(106,74,126,.22)}.rm-stat{min-width:0;padding:9px 10px;background:#090011}.rm-stat span{display:block;color:#887597;font-size:7px;text-transform:uppercase;letter-spacing:.05em}.rm-stat strong{display:block;margin-top:5px;font-size:10px;overflow-wrap:anywhere}.rm-stat small{display:block;margin-top:4px;color:#806d8d;font-size:7px;line-height:1.4;overflow-wrap:anywhere}.rm-card-actions{padding:10px}.rm-empty{padding:44px 14px;text-align:center;color:#8e789b;font-size:9px}@media(max-width:980px){.rm-page{padding:22px 22px 92px}.rm-header{flex-direction:column}.rm-fresh{text-align:left;min-width:0}.rm-two,.rm-model-grid{grid-template-columns:1fr}}@media(max-width:620px){.rm-page{padding-left:14px;padding-right:14px}.rm-fields{grid-template-columns:1fr}.rm-field.wide{grid-column:auto}.rm-stats,.rm-support-row{grid-template-columns:1fr}}@media(prefers-reduced-motion:reduce){.rm-page *,.rm-page *::before,.rm-page *::after{transition:none!important;animation:none!important}}
"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RouterControlPreferences {
    pub host: String,
    pub port: u16,
    pub allow_non_loopback: bool,
    pub preferred_model: Option<String>,
}

impl Default for RouterControlPreferences {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8080,
            allow_non_loopback: false,
            preferred_model: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreferredModelVerification {
    NotConfigured,
    NeedsLiveReconciliation,
    Unsupported { reason: String },
    Missing { model: String },
    NotReady { model: String, phase: String },
    Verified { model: String },
}

pub fn verify_preferred_model(
    preferences: &RouterControlPreferences,
    tracker: &RouterObservabilityTracker,
) -> PreferredModelVerification {
    let Some(preferred) = preferences.preferred_model.as_ref() else {
        return PreferredModelVerification::NotConfigured;
    };
    if !tracker.is_live() {
        return PreferredModelVerification::NeedsLiveReconciliation;
    }
    let Some(snapshot) = tracker.current.as_ref() else {
        return PreferredModelVerification::NeedsLiveReconciliation;
    };
    if snapshot.registry.role != RouterRole::Router {
        return PreferredModelVerification::Unsupported {
            reason: "selected endpoint is not a router".into(),
        };
    }
    if !snapshot.registry.static_capabilities.models_autoload {
        return PreferredModelVerification::Unsupported {
            reason: "runtime does not advertise models-autoload; startup/default behavior remains N/A rather than inferred".into(),
        };
    }
    let Some(model) = snapshot.registry.models.iter().find(|model| {
        model.id == *preferred || model.routing_targets.iter().any(|target| target == preferred)
    }) else {
        return PreferredModelVerification::Missing {
            model: preferred.clone(),
        };
    };
    if model.status.failed {
        return PreferredModelVerification::NotReady {
            model: preferred.clone(),
            phase: "FAILED".into(),
        };
    }
    match &model.status.phase {
        RouterModelPhase::Loaded | RouterModelPhase::Sleeping => {
            PreferredModelVerification::Verified {
                model: preferred.clone(),
            }
        }
        phase => PreferredModelVerification::NotReady {
            model: preferred.clone(),
            phase: format!("{phase:?}"),
        },
    }
}

fn preferences_path(paths: &AppPaths) -> std::path::PathBuf {
    paths.config.join(PREFERENCES_FILE)
}

fn load_preferences(paths: &AppPaths) -> Result<RouterControlPreferences, String> {
    let path = preferences_path(paths);
    if !path.is_file() {
        return Ok(RouterControlPreferences::default());
    }
    let bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn save_preferences(paths: &AppPaths, preferences: &RouterControlPreferences) -> Result<(), String> {
    let path = preferences_path(paths);
    let payload = serde_json::to_vec_pretty(preferences).map_err(|error| error.to_string())?;
    fs::write(&path, payload).map_err(|error| format!("write {}: {error}", path.display()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestedAction {
    Reload,
    Load,
    Unload,
    Preload,
    Switch,
}

impl RequestedAction {
    fn label(self) -> &'static str {
        match self {
            Self::Reload => "RELOAD REGISTRY",
            Self::Load => "LOAD",
            Self::Unload => "UNLOAD",
            Self::Preload => "PRELOAD",
            Self::Switch => "SWITCH",
        }
    }
}

type ControlSignal = Signal<ControlState, SyncStorage>;

#[derive(Debug, Clone)]
struct ControlState {
    paths: Option<AppPaths>,
    installation: Option<LlamaInstallation>,
    preferences: RouterControlPreferences,
    port_text: String,
    api_key: String,
    tracker: RouterObservabilityTracker,
    controller: RouterOperationController,
    cancellation: RouterOperationCancellation,
    operation_state: RouterOperationState,
    pending_action: Option<RequestedAction>,
    selected_source: Option<String>,
    selected_target: Option<String>,
    notice: Option<(bool, String)>,
}

impl ControlState {
    fn load() -> Self {
        let mut notice = None;
        let paths = match AppPaths::detect() {
            Ok(paths) => Some(paths),
            Err(error) => {
                notice = Some((false, format!("Could not resolve application paths: {error}")));
                None
            }
        };
        let preferences = if let Some(paths) = paths.as_ref() {
            match load_preferences(paths) {
                Ok(value) => value,
                Err(error) => {
                    notice = Some((false, error));
                    RouterControlPreferences::default()
                }
            }
        } else {
            RouterControlPreferences::default()
        };
        let installation = paths.as_ref().and_then(|paths| {
            match Database::open(paths.database.clone()).and_then(|db| db.latest_installation()) {
                Ok(value) => value,
                Err(error) => {
                    notice = Some((false, format!("Could not reload persisted runtime evidence: {error}")));
                    None
                }
            }
        });
        Self {
            paths,
            installation,
            port_text: preferences.port.to_string(),
            preferences,
            api_key: String::new(),
            tracker: RouterObservabilityTracker::default(),
            controller: RouterOperationController::new(),
            cancellation: RouterOperationCancellation::new(),
            operation_state: RouterOperationState::Idle,
            pending_action: None,
            selected_source: None,
            selected_target: None,
            notice,
        }
    }

    fn busy(&self) -> bool {
        self.pending_action.is_some()
            || matches!(&self.operation_state, RouterOperationState::Running(_))
            || self.tracker.loading
    }
}

fn endpoint_from_state(state: &ControlState) -> Result<ServerEndpoint, String> {
    let host = state.preferences.host.trim();
    if host.is_empty() {
        return Err("Host cannot be empty.".into());
    }
    let port = state
        .port_text
        .trim()
        .parse::<u16>()
        .map_err(|_| "Port must be an integer in 1..=65535.".to_string())?;
    if port == 0 {
        return Err("Port must be in 1..=65535.".into());
    }
    let api_key = state.api_key.trim();
    Ok(ServerEndpoint {
        host: host.to_string(),
        port,
        api_key: (!api_key.is_empty()).then(|| api_key.to_string()),
        allow_non_loopback: state.preferences.allow_non_loopback,
    })
}

fn open_store(paths: &AppPaths) -> Result<ModelStore, String> {
    ModelStore::open(paths.database.clone()).map_err(|error| error.to_string())
}

fn reconcile_selection(state: &mut ControlState, snapshot: &RouterObservabilitySnapshot) {
    let valid = |selected: Option<&str>| {
        selected.is_some_and(|selected| snapshot.models.iter().any(|model| model.model.id == selected))
    };
    if !valid(state.selected_target.as_deref()) {
        state.selected_target = snapshot.models.first().map(|model| model.model.id.clone());
    }
    if !valid(state.selected_source.as_deref()) {
        state.selected_source = snapshot
            .models
            .iter()
            .find(|model| {
                matches!(
                    &model.model.status.phase,
                    RouterModelPhase::Loaded | RouterModelPhase::Sleeping
                ) && !model.model.status.failed
            })
            .map(|model| model.model.id.clone())
            .or_else(|| snapshot.models.first().map(|model| model.model.id.clone()));
    }
}

fn refresh_runtime(mut state: ControlSignal) {
    let Some(paths) = state.read().paths.clone() else {
        state.write().notice = Some((false, "Application storage paths are unavailable.".into()));
        return;
    };
    thread::spawn(move || {
        let result = Database::open(paths.database.clone()).and_then(|db| db.latest_installation());
        let mut current = state.write();
        match result {
            Ok(installation) => {
                current.installation = installation;
                current.notice = Some((true, "Reloaded persisted llama.cpp runtime evidence.".into()));
            }
            Err(error) => current.notice = Some((false, error.to_string())),
        }
    });
}

fn refresh_router(mut state: ControlSignal) {
    let snapshot = state.read().clone();
    let Some(installation) = snapshot.installation.clone() else {
        state.write().notice = Some((false, "No persisted llama.cpp installation is selected. Select one in CORE LAB first.".into()));
        return;
    };
    let endpoint = match endpoint_from_state(&snapshot) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            state.write().notice = Some((false, error));
            return;
        }
    };
    let paths = snapshot.paths.clone();
    {
        let mut current = state.write();
        current.tracker.begin_refresh();
        current.notice = None;
    }
    thread::spawn(move || {
        let result = (|| -> Result<RouterObservabilitySnapshot, String> {
            let store = paths.as_ref().map(open_store).transpose()?;
            discover_router_observability(&installation, &endpoint, store.as_ref(), REFRESH_TIMEOUT)
                .map_err(|error| error.to_string())
        })();
        let mut current = state.write();
        if let Ok(snapshot) = result.as_ref() {
            reconcile_selection(&mut current, snapshot);
        }
        let error = result.as_ref().err().cloned();
        current.tracker.reconcile(result);
        current.notice = match error {
            Some(error) => Some((false, error)),
            None => Some((true, "Live router state reconciled. Retained prior state is no longer treated as current.".into())),
        };
    });
}

fn run_action(
    mut state: ControlSignal,
    action: RequestedAction,
    source: Option<String>,
    target: Option<String>,
) {
    let snapshot = state.read().clone();
    if snapshot.busy() {
        state.write().notice = Some((false, "Another router operation or refresh is already running.".into()));
        return;
    }
    let Some(installation) = snapshot.installation.clone() else {
        state.write().notice = Some((false, "No persisted llama.cpp runtime is selected.".into()));
        return;
    };
    let Some(paths) = snapshot.paths.clone() else {
        state.write().notice = Some((false, "Application storage paths are unavailable.".into()));
        return;
    };
    let endpoint = match endpoint_from_state(&snapshot) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            state.write().notice = Some((false, error));
            return;
        }
    };
    let controller = snapshot.controller.clone();
    let cancellation = snapshot.cancellation.clone();
    cancellation.reset();
    {
        let mut current = state.write();
        current.pending_action = Some(action);
        current.notice = None;
    }

    thread::spawn(move || {
        let operation_controller = controller.clone();
        let operation_cancellation = cancellation.clone();
        let operation_installation = installation.clone();
        let operation_endpoint = endpoint.clone();
        let operation_paths = paths.clone();
        let operation_source = source.clone();
        let operation_target = target.clone();

        let handle = thread::spawn(move || -> Result<(), String> {
            let store = open_store(&operation_paths)?;
            match action {
                RequestedAction::Reload => operation_controller
                    .reload_registry(
                        &operation_installation,
                        &operation_endpoint,
                        Some(&store),
                        OPERATION_TIMEOUT,
                        &operation_cancellation,
                    )
                    .map(|_| ()),
                RequestedAction::Load => operation_controller
                    .load_model(
                        &operation_installation,
                        &operation_endpoint,
                        &store,
                        operation_target
                            .as_deref()
                            .ok_or_else(|| "No target model selected.".to_string())?,
                        OPERATION_TIMEOUT,
                        &operation_cancellation,
                    )
                    .map(|_| ()),
                RequestedAction::Unload => operation_controller
                    .unload_model(
                        &operation_installation,
                        &operation_endpoint,
                        Some(&store),
                        operation_source
                            .as_deref()
                            .ok_or_else(|| "No source model selected.".to_string())?,
                        OPERATION_TIMEOUT,
                        &operation_cancellation,
                    )
                    .map(|_| ()),
                RequestedAction::Preload => operation_controller
                    .preload_model(
                        &operation_installation,
                        &operation_endpoint,
                        &store,
                        operation_target
                            .as_deref()
                            .ok_or_else(|| "No target model selected.".to_string())?,
                        OPERATION_TIMEOUT,
                        &operation_cancellation,
                    )
                    .map(|_| ()),
                RequestedAction::Switch => operation_controller
                    .switch_model(
                        &operation_installation,
                        &operation_endpoint,
                        &store,
                        operation_source
                            .as_deref()
                            .ok_or_else(|| "No source model selected.".to_string())?,
                        operation_target
                            .as_deref()
                            .ok_or_else(|| "No target model selected.".to_string())?,
                        OPERATION_TIMEOUT,
                        &operation_cancellation,
                    )
                    .map(|_| ()),
            }
            .map_err(|error| error.to_string())
        });

        while !handle.is_finished() {
            state.write().operation_state = controller.state();
            thread::sleep(Duration::from_millis(75));
        }
        let result = handle
            .join()
            .map_err(|_| "router operation worker panicked".to_string())
            .and_then(|result| result);
        let final_operation = controller.state();

        let refreshed = match open_store(&paths) {
            Ok(store) => discover_router_observability(
                &installation,
                &endpoint,
                Some(&store),
                REFRESH_TIMEOUT,
            )
            .map_err(|error| error.to_string()),
            Err(error) => Err(error),
        };

        let mut current = state.write();
        current.pending_action = None;
        current.operation_state = final_operation;
        if let Ok(snapshot) = refreshed.as_ref() {
            reconcile_selection(&mut current, snapshot);
        }
        current.tracker.reconcile(refreshed);
        current.notice = match result {
            Ok(()) => Some((true, format!("{} completed and live state was reconciled.", action.label()))),
            Err(error) => Some((false, format!("{} failed: {error}. Controller evidence is retained below.", action.label()))),
        };
    });
}

fn persist_preferences(mut state: ControlSignal) {
    let snapshot = state.read().clone();
    let Some(paths) = snapshot.paths.as_ref() else {
        state.write().notice = Some((false, "Application storage paths are unavailable.".into()));
        return;
    };
    let port = match snapshot.port_text.trim().parse::<u16>() {
        Ok(port) if port > 0 => port,
        _ => {
            state.write().notice = Some((false, "Port must be an integer in 1..=65535.".into()));
            return;
        }
    };
    let mut preferences = snapshot.preferences.clone();
    preferences.port = port;
    match save_preferences(paths, &preferences) {
        Ok(()) => {
            state.write().preferences = preferences;
            state.write().notice = Some((true, "Endpoint preferences persisted. API keys are session-only and were not written.".into()));
        }
        Err(error) => state.write().notice = Some((false, error)),
    }
}

fn set_preferred(mut state: ControlSignal) {
    let snapshot = state.read().clone();
    let Some(model) = snapshot.selected_target.clone() else {
        state.write().notice = Some((false, "Select a target model first.".into()));
        return;
    };
    let Some(paths) = snapshot.paths.as_ref() else {
        state.write().notice = Some((false, "Application storage paths are unavailable.".into()));
        return;
    };
    let mut preferences = snapshot.preferences.clone();
    preferences.preferred_model = Some(model.clone());
    match save_preferences(paths, &preferences) {
        Ok(()) => {
            state.write().preferences = preferences;
            state.write().notice = Some((true, format!("Preferred target `{model}` persisted. It remains unverified until live post-restart reconciliation proves readiness.")));
        }
        Err(error) => state.write().notice = Some((false, error)),
    }
}

fn feature_for(registry: &RouterRegistry, action: RequestedAction) -> &RouterFeatureEvidence {
    match action {
        RequestedAction::Reload => &registry.endpoints.reload_models,
        RequestedAction::Load | RequestedAction::Preload => &registry.endpoints.load_model,
        RequestedAction::Unload => &registry.endpoints.unload_model,
        RequestedAction::Switch => {
            if registry.endpoints.load_model.state == RouterFeatureState::Supported {
                &registry.endpoints.unload_model
            } else {
                &registry.endpoints.load_model
            }
        }
    }
}

fn action_support(state: &ControlState, action: RequestedAction) -> (bool, String) {
    if state.busy() {
        return (false, "another refresh or operation is running".into());
    }
    if !state.tracker.is_live() {
        return (false, "requires a live reconciled router snapshot; stale state cannot authorize mutation".into());
    }
    let Some(snapshot) = state.tracker.current.as_ref() else {
        return (false, "no live router snapshot".into());
    };
    if snapshot.registry.role != RouterRole::Router {
        return (false, "selected endpoint is a single-model server".into());
    }
    let feature = feature_for(&snapshot.registry, action);
    match feature.state {
        RouterFeatureState::Supported => (true, feature.reason.clone()),
        RouterFeatureState::Unsupported => (false, format!("unsupported: {}", feature.reason)),
        RouterFeatureState::Unknown => (false, format!("support unknown: {}", feature.reason)),
    }
}

fn freshness_class(value: RouterSnapshotFreshness) -> &'static str {
    match value {
        RouterSnapshotFreshness::Live => "rm-badge",
        RouterSnapshotFreshness::Failed => "rm-badge error",
        _ => "rm-badge warn",
    }
}

fn phase_text(model: &RouterModelObservability) -> (String, &'static str) {
    if model.model.status.failed {
        return (format!("FAILED · {:?}", model.model.status.phase), "rm-error");
    }
    match &model.model.status.phase {
        RouterModelPhase::Loaded => ("LOADED".into(), "rm-live"),
        RouterModelPhase::Sleeping => ("SLEEPING".into(), "rm-live"),
        RouterModelPhase::Unloaded => ("UNLOADED".into(), "rm-warn"),
        RouterModelPhase::Loading => ("LOADING".into(), "rm-warn"),
        RouterModelPhase::Downloading => ("DOWNLOADING".into(), "rm-warn"),
        RouterModelPhase::Unknown(value) => (format!("UNKNOWN · {value}"), "rm-warn"),
    }
}

fn residency_text(model: &RouterModelObservability) -> (String, &'static str) {
    match (model.residency.availability, model.residency.value) {
        (EvidenceAvailability::Observed, Some(true)) => ("RESIDENT · OBSERVED".into(), "rm-live"),
        (EvidenceAvailability::Observed, Some(false)) => ("NOT RESIDENT · OBSERVED".into(), "rm-warn"),
        _ => ("UNKNOWN / UNAVAILABLE".into(), "rm-warn"),
    }
}

fn active_text(model: &RouterModelObservability) -> (String, &'static str) {
    match (model.active_requests.availability, model.active_requests.value) {
        (EvidenceAvailability::Observed, Some(0)) => ("0 · OBSERVED".into(), "rm-live"),
        (EvidenceAvailability::Observed, Some(value)) => (format!("{value} · ACTIVE"), "rm-error"),
        _ => ("UNKNOWN / UNAVAILABLE".into(), "rm-warn"),
    }
}

fn operation_text(state: &RouterOperationState) -> (String, &'static str, String) {
    match state {
        RouterOperationState::Idle => (
            "IDLE".into(),
            "rm-warn",
            "No controller evidence recorded in this session.".into(),
        ),
        RouterOperationState::Running(progress) => (
            format!("RUNNING · {:?}", progress.kind),
            "rm-warn",
            format!(
                "{}\nsource={:?}\ntarget={:?}\nstarted={}",
                progress.message,
                progress.source_model,
                progress.target_model,
                progress.started_at_unix_ms
            ),
        ),
        RouterOperationState::Succeeded(evidence) => (
            format!("SUCCEEDED · {:?}", evidence.kind),
            "rm-live",
            format!(
                "source={:?}\ntarget={:?}\nHTTP={:?}\nstarted={}\ncompleted={}\n{}",
                evidence.source_model,
                evidence.target_model,
                evidence.http_statuses,
                evidence.started_at_unix_ms,
                evidence.completed_at_unix_ms,
                evidence.notes.join("\n")
            ),
        ),
        RouterOperationState::Failed(failure) => (
            format!("FAILED · {:?}", failure.kind),
            "rm-error",
            format!(
                "{}\nsource={:?}\ntarget={:?}\nlast_registry={}",
                failure.message,
                failure.source_model,
                failure.target_model,
                failure
                    .last_registry
                    .as_ref()
                    .map(|registry| registry.observed_at_unix_ms.to_string())
                    .unwrap_or_else(|| "none".into())
            ),
        ),
        RouterOperationState::Cancelled(failure) => (
            format!("CANCELLED · {:?}", failure.kind),
            "rm-warn",
            format!(
                "{}\nsource={:?}\ntarget={:?}",
                failure.message, failure.source_model, failure.target_model
            ),
        ),
    }
}

fn verification_text(value: &PreferredModelVerification) -> (String, &'static str, String) {
    match value {
        PreferredModelVerification::NotConfigured => (
            "NOT SET".into(),
            "rm-warn",
            "No preferred model is persisted.".into(),
        ),
        PreferredModelVerification::NeedsLiveReconciliation => (
            "UNVERIFIED".into(),
            "rm-warn",
            "A live post-reconnect snapshot is required before any readiness claim.".into(),
        ),
        PreferredModelVerification::Unsupported { reason } => {
            ("N/A".into(), "rm-warn", reason.clone())
        }
        PreferredModelVerification::Missing { model } => (
            "MISSING".into(),
            "rm-error",
            format!("Preferred model `{model}` is absent from the live registry."),
        ),
        PreferredModelVerification::NotReady { model, phase } => (
            "NOT READY".into(),
            "rm-error",
            format!("Preferred model `{model}` reconciled as {phase}."),
        ),
        PreferredModelVerification::Verified { model } => (
            "VERIFIED LIVE".into(),
            "rm-live",
            format!("Preferred model `{model}` is ready in the current live registry."),
        ),
    }
}

fn support_row(label: &str, enabled: bool, reason: &str) -> Element {
    rsx! {
        div { class: "rm-support-row",
            span { "{label}" }
            strong { class: if enabled { "rm-live" } else { "rm-warn" }, "{reason}" }
        }
    }
}

#[allow(non_snake_case)]
pub fn RouterManagementView() -> Element {
    let mut state = use_signal_sync(ControlState::load);
    let snapshot = state.read().clone();
    let freshness = snapshot.tracker.freshness();
    let current = snapshot.tracker.current.as_ref();
    let busy = snapshot.busy();
    let source_value = snapshot.selected_source.clone().unwrap_or_default();
    let target_value = snapshot.selected_target.clone().unwrap_or_default();
    let (reload_ok, reload_reason) = action_support(&snapshot, RequestedAction::Reload);
    let (load_ok, load_reason) = action_support(&snapshot, RequestedAction::Load);
    let (unload_ok, unload_reason) = action_support(&snapshot, RequestedAction::Unload);
    let (preload_ok, preload_reason) = action_support(&snapshot, RequestedAction::Preload);
    let (switch_ok, switch_reason) = action_support(&snapshot, RequestedAction::Switch);
    let (operation_label, operation_class, operation_detail) =
        operation_text(&snapshot.operation_state);
    let verification = verify_preferred_model(&snapshot.preferences, &snapshot.tracker);
    let (verification_label, verification_class, verification_detail) =
        verification_text(&verification);

    rsx! {
        style { dangerous_inner_html: CSS }
        main { class: "rm-page",
            header { class: "rm-header",
                div {
                    div { class: "rm-kicker", "> LLAMAWAVE / ROUTER CONTROL" }
                    h1 { "CONTROL THE ROUTER. KEEP THE EVIDENCE." }
                    p { "Mutations are enabled only from live supported capabilities. Stale snapshots cannot authorize operations; unavailable router evidence remains unknown." }
                }
                div { class: "rm-fresh",
                    div { class: "rm-kicker", "CANONICAL SNAPSHOT" }
                    strong { "{freshness:?}" }
                    span { class: freshness_class(freshness), "{freshness:?}" }
                }
            }

            if let Some((success, message)) = snapshot.notice.as_ref() {
                div { class: if *success { "rm-notice" } else { "rm-notice error" }, "{message}" }
            }
            if freshness == RouterSnapshotFreshness::Stale {
                div { class: "rm-notice error", "STALE SNAPSHOT · retained only for diagnostics. Mutation controls are disabled until live reconciliation succeeds." }
            }

            section { class: "rm-panel",
                div { class: "rm-panel-head",
                    div {
                        div { class: "rm-kicker", "CONNECTION + RECONCILIATION" }
                        h2 { "LIVE ROUTER ENDPOINT" }
                    }
                    button { class: "rm-button", disabled: busy, onclick: move |_| refresh_runtime(state), "REFRESH RUNTIME" }
                }
                div { class: "rm-panel-body",
                    div { class: "rm-fields",
                        div { class: "rm-field",
                            label { "HOST" }
                            input {
                                class: "rm-input",
                                value: "{snapshot.preferences.host}",
                                disabled: busy,
                                oninput: move |event| state.write().preferences.host = event.value(),
                            }
                        }
                        div { class: "rm-field",
                            label { "PORT" }
                            input {
                                class: "rm-input",
                                value: "{snapshot.port_text}",
                                disabled: busy,
                                oninput: move |event| state.write().port_text = event.value(),
                            }
                        }
                        div { class: "rm-field wide",
                            label { "API KEY · SESSION ONLY · NEVER PERSISTED" }
                            input {
                                class: "rm-input",
                                r#type: "password",
                                value: "{snapshot.api_key}",
                                disabled: busy,
                                oninput: move |event| state.write().api_key = event.value(),
                            }
                        }
                    }
                    div { class: "rm-actions",
                        button {
                            class: if snapshot.preferences.allow_non_loopback { "rm-button magenta" } else { "rm-button" },
                            disabled: busy,
                            onclick: move |_| {
                                let enabled = state.read().preferences.allow_non_loopback;
                                state.write().preferences.allow_non_loopback = !enabled;
                            },
                            if snapshot.preferences.allow_non_loopback { "LAN OPT-IN ON" } else { "LAN OPT-IN OFF" }
                        }
                        button { class: "rm-button", disabled: busy, onclick: move |_| persist_preferences(state), "SAVE ENDPOINT" }
                        button { class: "rm-button primary", disabled: busy || snapshot.installation.is_none(), onclick: move |_| refresh_router(state), "RECONCILE LIVE STATE" }
                        button { class: "rm-button", disabled: !reload_ok, title: "{reload_reason}", onclick: move |_| run_action(state, RequestedAction::Reload, None, None), "RELOAD REGISTRY" }
                    }
                    div { class: "rm-runtime",
                        strong { "SELECTED LLAMA.CPP EVIDENCE\n" }
                        if let Some(installation) = snapshot.installation.as_ref() {
                            if let Some(server) = installation.server.as_ref() {
                                "{server.path.display()}\nSHA-256 {server.sha256}"
                            } else {
                                "selected installation has no llama-server"
                            }
                        } else {
                            "no persisted runtime selected"
                        }
                    }
                }
            }

            div { class: "rm-two",
                section { class: "rm-panel",
                    div { class: "rm-panel-head",
                        div { div { class: "rm-kicker", "MODEL SWITCHING" } h2 { "SOURCE → TARGET" } }
                    }
                    div { class: "rm-panel-body",
                        div { class: "rm-fields",
                            div { class: "rm-field",
                                label { "SOURCE" }
                                select {
                                    class: "rm-select",
                                    value: "{source_value}",
                                    disabled: busy || current.is_none(),
                                    onchange: move |event| state.write().selected_source = Some(event.value()),
                                    if let Some(current) = current {
                                        for model in current.models.iter() {
                                            option { value: "{model.model.id}", "{model.model.id}" }
                                        }
                                    }
                                }
                            }
                            div { class: "rm-field",
                                label { "TARGET" }
                                select {
                                    class: "rm-select",
                                    value: "{target_value}",
                                    disabled: busy || current.is_none(),
                                    onchange: move |event| state.write().selected_target = Some(event.value()),
                                    if let Some(current) = current {
                                        for model in current.models.iter() {
                                            option { value: "{model.model.id}", "{model.model.id}" }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "rm-actions",
                            button {
                                class: "rm-button primary",
                                disabled: !switch_ok || source_value.is_empty() || target_value.is_empty(),
                                title: "{switch_reason}",
                                onclick: move |_| run_action(state, RequestedAction::Switch, state.read().selected_source.clone(), state.read().selected_target.clone()),
                                "SWITCH"
                            }
                            button {
                                class: "rm-button",
                                disabled: !load_ok || target_value.is_empty(),
                                title: "{load_reason}",
                                onclick: move |_| run_action(state, RequestedAction::Load, None, state.read().selected_target.clone()),
                                "LOAD TARGET"
                            }
                            button {
                                class: "rm-button",
                                disabled: !preload_ok || target_value.is_empty(),
                                title: "{preload_reason}",
                                onclick: move |_| run_action(state, RequestedAction::Preload, None, state.read().selected_target.clone()),
                                "PRELOAD TARGET"
                            }
                            button {
                                class: "rm-button danger",
                                disabled: !unload_ok || source_value.is_empty(),
                                title: "{unload_reason}",
                                onclick: move |_| run_action(state, RequestedAction::Unload, state.read().selected_source.clone(), None),
                                "UNLOAD SOURCE"
                            }
                            button {
                                class: "rm-button magenta",
                                disabled: busy || target_value.is_empty(),
                                onclick: move |_| set_preferred(state),
                                "SET PREFERRED"
                            }
                            if matches!(&snapshot.operation_state, RouterOperationState::Running(_)) {
                                button { class: "rm-button danger", onclick: move |_| state.read().cancellation.cancel(), "CANCEL" }
                            }
                        }
                        div { class: "rm-support",
                            {support_row("LOAD", load_ok, &load_reason)}
                            {support_row("UNLOAD", unload_ok, &unload_reason)}
                            {support_row("PRELOAD", preload_ok, &preload_reason)}
                            {support_row("SWITCH", switch_ok, &switch_reason)}
                        }
                    }
                }

                section { class: "rm-panel",
                    div { class: "rm-panel-head",
                        div { div { class: "rm-kicker", "OPERATION EVIDENCE" } h2 { "CURRENT / LAST ACTION" } }
                    }
                    div { class: "rm-panel-body",
                        div { class: "rm-operation",
                            div { class: "rm-kicker", "CONTROLLER STATE" }
                            strong { class: operation_class, "{operation_label}" }
                            if let Some(action) = snapshot.pending_action {
                                span { class: "rm-muted", "UI DISPATCH · {action.label()}" }
                            }
                            pre { "{operation_detail}" }
                        }
                        if matches!(&snapshot.operation_state, RouterOperationState::Failed(_) | RouterOperationState::Cancelled(_)) {
                            div { class: "rm-actions",
                                button { class: "rm-button primary", disabled: busy, onclick: move |_| refresh_router(state), "RECOVER / RECONCILE" }
                            }
                        }
                    }
                }
            }

            section { class: "rm-panel",
                div { class: "rm-panel-head",
                    div { div { class: "rm-kicker", "PERSISTENCE + RESTART TRUTH" } h2 { "PREFERRED MODEL" } }
                }
                div { class: "rm-panel-body",
                    div { class: "rm-operation",
                        div { class: "rm-kicker", "POST-RESTART VERIFICATION" }
                        strong { class: verification_class, "{verification_label}" }
                        p { class: "rm-muted", "{verification_detail}" }
                        if let Some(model) = snapshot.preferences.preferred_model.as_ref() {
                            div { class: "rm-detail", "PERSISTED TARGET · {model}" }
                        }
                        p { class: "rm-muted", style: "margin-top:8px", "Persistence is not treated as proof of upstream startup behavior. The pinned runtime has no dynamic default-model mutation route; unsupported behavior remains N/A and reconnect/restart must reconcile live state." }
                    }
                }
            }

            section { class: "rm-panel",
                div { class: "rm-panel-head",
                    div { div { class: "rm-kicker", "CANONICAL LIVE REGISTRY" } h2 { "MODELS + ROUTING EVIDENCE" } }
                    if let Some(current) = current {
                        span { class: "rm-muted", "observed {current.observed_at_unix_ms} · {current.models.len()} models" }
                    }
                }
                div { class: "rm-panel-body",
                    if let Some(current) = current {
                        if current.registry.role != RouterRole::Router {
                            div { class: "rm-empty", "Selected endpoint reports single-model server role. Router controls are disabled." }
                        } else if current.models.is_empty() {
                            div { class: "rm-empty", "Router returned an empty model registry." }
                        } else {
                            div { class: "rm-model-grid",
                                for observed in current.models.iter() {
                                    {
                                        let (phase, phase_class) = phase_text(observed);
                                        let (residency, residency_class) = residency_text(observed);
                                        let (active, active_class) = active_text(observed);
                                        let stale = freshness != RouterSnapshotFreshness::Live;
                                        rsx! {
                                            article { class: if stale { "rm-card stale" } else { "rm-card" },
                                                div { class: "rm-card-head",
                                                    strong { "{observed.model.id}" }
                                                    div { class: "rm-aliases",
                                                        for alias in observed.model.routing_targets.iter().filter(|target| *target != &observed.model.id) {
                                                            span { class: "rm-alias", "ALIAS · {alias}" }
                                                        }
                                                    }
                                                }
                                                div { class: "rm-stats",
                                                    div { class: "rm-stat", span { "ROUTER PHASE" } strong { class: phase_class, "{phase}" } small { "direct /models state" } }
                                                    div { class: "rm-stat", span { "RESIDENCY" } strong { class: residency_class, "{residency}" } small { "{observed.residency.reason}" } }
                                                    div { class: "rm-stat", span { "ACTIVE REQUESTS" } strong { class: active_class, "{active}" } small { "{observed.active_requests.reason}" } }
                                                    div { class: "rm-stat", span { "M2 IDENTITY" } strong { "{observed.model.library_link.kind:?}" } small { "{observed.model.library_link.reason}" } }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(error) = current.supplemental_error.as_ref() {
                            div { class: "rm-detail", "SUPPLEMENTAL OBSERVABILITY ERROR · {error}. Canonical registry remains live; unavailable fields stay unavailable." }
                        }
                    } else if snapshot.tracker.loading {
                        div { class: "rm-empty", "Reconciling live router evidence. No success state is shown until the canonical snapshot succeeds." }
                    } else {
                        div { class: "rm-empty", "No router snapshot yet. Reconcile a live endpoint to enable evidence-backed controls." }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        router::{
            RouterEndpointCapabilities, RouterFeatureEvidence, RouterLibraryLink,
            RouterLibraryLinkKind, RouterModel, RouterModelStatus, RouterStaticCapabilities,
        },
        router_observability::EvidenceValue,
    };

    fn feature(state: RouterFeatureState) -> RouterFeatureEvidence {
        RouterFeatureEvidence {
            state,
            reason: "fixture".into(),
        }
    }

    fn snapshot(phase: RouterModelPhase, autoload: bool) -> RouterObservabilitySnapshot {
        let model = RouterModel {
            id: "alpha".into(),
            routing_targets: vec!["alpha".into(), "alias-a".into()],
            path: None,
            sha256: None,
            status: RouterModelStatus {
                phase,
                failed: false,
                exit_code: None,
                args: Vec::new(),
                progress: None,
            },
            resident: None,
            input_modalities: vec!["text".into()],
            output_modalities: vec!["text".into()],
            library_link: RouterLibraryLink {
                kind: RouterLibraryLinkKind::Unmatched,
                model_id: None,
                candidates: Vec::new(),
                reason: "fixture".into(),
            },
        };
        RouterObservabilitySnapshot {
            registry: RouterRegistry {
                endpoint: "127.0.0.1:8080".into(),
                role: RouterRole::Router,
                static_capabilities: RouterStaticCapabilities {
                    server_sha256: None,
                    server_version: None,
                    router_cli_observed: true,
                    models_dir: true,
                    models_preset: false,
                    models_max: true,
                    models_autoload: autoload,
                    observed_options: BTreeSet::new(),
                },
                endpoints: RouterEndpointCapabilities {
                    props: feature(RouterFeatureState::Supported),
                    list_models: feature(RouterFeatureState::Supported),
                    reload_models: feature(RouterFeatureState::Supported),
                    load_model: feature(RouterFeatureState::Supported),
                    unload_model: feature(RouterFeatureState::Supported),
                    model_events: feature(RouterFeatureState::Unknown),
                },
                models: vec![model.clone()],
                observed_at_unix_ms: 1,
            },
            models: vec![RouterModelObservability {
                model,
                residency: EvidenceValue {
                    availability: EvidenceAvailability::Unavailable,
                    value: None,
                    reason: "fixture".into(),
                },
                active_requests: EvidenceValue {
                    availability: EvidenceAvailability::Unavailable,
                    value: None,
                    reason: "fixture".into(),
                },
                last_used_ms: EvidenceValue {
                    availability: EvidenceAvailability::Unavailable,
                    value: None,
                    reason: "fixture".into(),
                },
                lru_rank: EvidenceValue {
                    availability: EvidenceAvailability::Unavailable,
                    value: None,
                    reason: "fixture".into(),
                },
                evictable: EvidenceValue {
                    availability: EvidenceAvailability::Unavailable,
                    value: None,
                    reason: "fixture".into(),
                },
            }],
            supplemental_error: None,
            observed_at_unix_ms: 1,
        }
    }

    #[test]
    fn preferences_persist_without_api_key_field() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_root(
            crate::paths::StorageMode::Portable,
            temp.path().join("router prefs"),
        )
        .unwrap();
        let preferences = RouterControlPreferences {
            host: "localhost".into(),
            port: 9999,
            allow_non_loopback: false,
            preferred_model: Some("alpha".into()),
        };
        save_preferences(&paths, &preferences).unwrap();
        assert_eq!(load_preferences(&paths).unwrap(), preferences);
        let raw = fs::read_to_string(preferences_path(&paths)).unwrap();
        assert!(!raw.to_ascii_lowercase().contains("api_key"));
    }

    #[test]
    fn preferred_model_needs_fresh_reconciliation_after_disconnect() {
        let preferences = RouterControlPreferences {
            preferred_model: Some("alpha".into()),
            ..RouterControlPreferences::default()
        };
        let mut tracker = RouterObservabilityTracker::default();
        assert_eq!(
            verify_preferred_model(&preferences, &tracker),
            PreferredModelVerification::NeedsLiveReconciliation
        );
        tracker.reconcile(Ok(snapshot(RouterModelPhase::Loaded, true)));
        assert_eq!(
            verify_preferred_model(&preferences, &tracker),
            PreferredModelVerification::Verified {
                model: "alpha".into()
            }
        );
        tracker.reconcile(Err("router restarted".into()));
        assert_eq!(
            verify_preferred_model(&preferences, &tracker),
            PreferredModelVerification::NeedsLiveReconciliation
        );
    }

    #[test]
    fn unsupported_autoload_is_not_presented_as_verified_startup_behavior() {
        let preferences = RouterControlPreferences {
            preferred_model: Some("alpha".into()),
            ..RouterControlPreferences::default()
        };
        let mut tracker = RouterObservabilityTracker::default();
        tracker.reconcile(Ok(snapshot(RouterModelPhase::Loaded, false)));
        assert!(matches!(
            verify_preferred_model(&preferences, &tracker),
            PreferredModelVerification::Unsupported { .. }
        ));
    }

    #[test]
    fn post_restart_unloaded_target_is_not_ready() {
        let preferences = RouterControlPreferences {
            preferred_model: Some("alpha".into()),
            ..RouterControlPreferences::default()
        };
        let mut tracker = RouterObservabilityTracker::default();
        tracker.reconcile(Ok(snapshot(RouterModelPhase::Unloaded, true)));
        assert!(matches!(
            verify_preferred_model(&preferences, &tracker),
            PreferredModelVerification::NotReady { .. }
        ));
    }
}