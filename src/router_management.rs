use std::{fs, path::Path, thread, time::Duration};

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
        RouterOperationCancellation, RouterOperationController, RouterOperationKind,
        RouterOperationState,
    },
    server_readiness::ServerEndpoint,
};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(120);
const REFRESH_TIMEOUT: Duration = Duration::from_secs(5);
const PREFERENCES_FILE: &str = "router-control.json";

const ROUTER_CONTROL_CSS: &str = r#"
.rc-page{min-height:100vh;padding:30px 34px 92px;color:#f6eaff;background:radial-gradient(circle at 80% 8%,rgba(255,0,190,.13),transparent 34%),radial-gradient(circle at 7% 82%,rgba(0,255,255,.08),transparent 36%),#07000e;font-family:"Cascadia Mono","Cascadia Code",Consolas,monospace;box-sizing:border-box}.rc-page *{box-sizing:border-box}.rc-header{display:flex;justify-content:space-between;gap:24px;align-items:flex-start;padding-bottom:18px;border-bottom:1px solid rgba(0,255,255,.42)}.rc-kicker{color:#00ffff;font-size:9px;font-weight:900;letter-spacing:.15em}.rc-header h1{margin:7px 0 8px;font-size:clamp(26px,3vw,40px)}.rc-header p,.rc-muted{margin:0;color:#a996bb;font-size:10px;line-height:1.65}.rc-fresh{text-align:right;min-width:210px}.rc-fresh strong{display:block;margin-top:5px;font-size:18px}.rc-badge{display:inline-flex;align-items:center;min-height:22px;padding:0 7px;border:1px solid rgba(0,255,255,.45);color:#76ffe6;font-size:8px;font-weight:900;letter-spacing:.07em;text-transform:uppercase}.rc-badge.warn{border-color:#ffd36b;color:#ffd36b}.rc-badge.error{border-color:#ff3d7f;color:#ff7ba9}.rc-notice{margin-top:12px;padding:9px 11px;border:1px solid rgba(117,255,226,.48);background:rgba(0,20,18,.6);color:#baffed;font-size:9px;line-height:1.55;overflow-wrap:anywhere}.rc-notice.error{border-color:rgba(255,50,110,.58);background:rgba(40,0,18,.58);color:#ff91b5}.rc-panel{margin-top:14px;min-width:0;border:1px solid rgba(0,255,255,.32);background:linear-gradient(180deg,rgba(29,5,47,.83),rgba(7,0,15,.92))}.rc-panel-head{display:flex;justify-content:space-between;align-items:center;gap:12px;padding:12px 14px;border-bottom:1px solid rgba(0,255,255,.25)}.rc-panel-head h2{margin:4px 0 0;font-size:16px}.rc-panel-body{padding:13px;min-width:0}.rc-fields{display:grid;grid-template-columns:minmax(0,1fr) 130px;gap:9px}.rc-field.wide{grid-column:1/-1}.rc-field label{display:block;margin-bottom:5px;color:#9b80a9;font-size:8px;letter-spacing:.08em;text-transform:uppercase}.rc-input,.rc-select{width:100%;min-height:34px;padding:7px 9px;border:1px solid rgba(0,255,255,.3);border-radius:0;background:#030008;color:#f6eaff;font:inherit;font-size:10px}.rc-input:focus-visible,.rc-select:focus-visible,.rc-button:focus-visible{outline:2px solid #ff00ff;outline-offset:2px}.rc-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:11px}.rc-button{min-height:34px;padding:0 12px;border:1px solid #00dbe7;border-radius:0;background:transparent;color:#00f5ff;font:inherit;font-size:8px;font-weight:900;letter-spacing:.08em;text-transform:uppercase;cursor:pointer}.rc-button:hover:not(:disabled),.rc-button.primary{background:#00ffff;color:#050009}.rc-button.magenta{border-color:#ff00d4;color:#ff55e7}.rc-button.danger{border-color:#ff356f;color:#ff739f}.rc-button:disabled{opacity:.32;cursor:not-allowed}.rc-runtime,.rc-detail{margin-top:10px;padding:9px;border-left:2px solid #00ffff;background:rgba(0,0,0,.34);color:#cbb9d7;font-size:9px;line-height:1.55;overflow-wrap:anywhere;word-break:break-word;white-space:pre-wrap}.rc-detail{border-left-color:#785b91;color:#a996bb}.rc-control-grid{display:grid;grid-template-columns:minmax(0,1.2fr) minmax(300px,.8fr);gap:12px;margin-top:14px}.rc-operation{padding:12px;border:1px solid rgba(106,74,126,.55);background:#090011}.rc-operation strong{display:block;margin:5px 0;font-size:13px}.rc-operation pre{margin:8px 0 0;color:#a996bb;font:inherit;font-size:8px;line-height:1.55;white-space:pre-wrap;overflow-wrap:anywhere}.rc-support{display:grid;gap:6px;margin-top:8px}.rc-support-row{display:grid;grid-template-columns:90px minmax(0,1fr);gap:8px;font-size:8px;line-height:1.45}.rc-support-row span{color:#887597}.rc-support-row strong{overflow-wrap:anywhere}.rc-live{color:#76ffe6}.rc-warn{color:#ffd36b}.rc-error{color:#ff7ba9}.rc-model-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px}.rc-card{min-width:0;border:1px solid rgba(106,74,126,.55);background:rgba(0,0,0,.28)}.rc-card.stale{border-style:dashed;opacity:.72}.rc-card-head{padding:10px 11px;border-bottom:1px solid rgba(106,74,126,.38)}.rc-card-head strong{display:block;font-size:12px;overflow-wrap:anywhere}.rc-aliases{display:flex;flex-wrap:wrap;gap:4px;margin-top:7px}.rc-alias{padding:3px 5px;border:1px solid rgba(255,0,212,.38);color:#ff77eb;font-size:7px}.rc-stat-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:1px;background:rgba(106,74,126,.22)}.rc-stat{min-width:0;padding:9px 10px;background:#090011}.rc-stat span{display:block;color:#887597;font-size:7px;text-transform:uppercase;letter-spacing:.05em}.rc-stat strong{display:block;margin-top:5px;font-size:10px;overflow-wrap:anywhere}.rc-stat small{display:block;margin-top:4px;color:#806d8d;font-size:7px;line-height:1.4;overflow-wrap:anywhere}.rc-card-actions{display:flex;flex-wrap:wrap;gap:6px;padding:10px}.rc-card-actions .rc-button{min-height:28px;padding:0 8px;font-size:7px}.rc-disabled-reason{padding:0 10px 10px;color:#8e789b;font-size:7px;line-height:1.45}.rc-empty{padding:44px 14px;text-align:center;color:#8e789b;font-size:9px}.rc-preference{padding:12px;border:1px solid rgba(106,74,126,.55);background:#090011}.rc-preference strong{display:block;margin:5px 0;font-size:12px}.rc-preference .rc-actions{margin-top:8px}@media(max-width:980px){.rc-page{padding:22px 22px 92px}.rc-header{flex-direction:column}.rc-fresh{text-align:left;min-width:0}.rc-control-grid,.rc-model-grid{grid-template-columns:1fr}}@media(max-width:620px){.rc-fields{grid-template-columns:1fr}.rc-field.wide{grid-column:auto}.rc-stat-grid{grid-template-columns:1fr}.rc-support-row{grid-template-columns:1fr}.rc-page{padding-left:14px;padding-right:14px}}@media(prefers-reduced-motion:reduce){.rc-page *,.rc-page *::before,.rc-page *::after{transition:none!important;animation:none!important}}
"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RouterControlPreferences {
    pub host: String,
    pub port: u16,
    pub allow_non_loopback: bool,
    /// This is a persisted LlamaWave preference, not an upstream startup claim.
    /// A post-restart live snapshot must verify the target before the UI calls it verified.
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
            reason: "selected llama-server evidence does not advertise models-autoload; persistent startup behavior is not claimed".into(),
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

type RouterControlSignal = Signal<RouterControlState, SyncStorage>;

#[derive(Debug, Clone)]
struct RouterControlState {
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

impl RouterControlState {
    fn load() -> Self {
        let mut notice = None;
        let paths = match AppPaths::detect() {
            Ok(paths) => Some(paths),
            Err(error) => {
                notice = Some((false, format!("Could not resolve application paths: {error}")));
                None
            }
        };
        let preferences = paths
            .as_ref()
            .map(load_preferences)
            .transpose()
            .unwrap_or_else(|error| {
                notice = Some((false, error));
                Some(RouterControlPreferences::default())
            })
            .unwrap_or_default();
        let installation = paths.as_ref().and_then(|paths| {
            match Database::open(paths.database.clone()).and_then(|db| db.latest_installation()) {
                Ok(installation) => installation,
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
            || matches!(self.operation_state, RouterOperationState::Running(_))
            || self.tracker.loading
    }
}

fn endpoint_from_state(state: &RouterControlState) -> Result<ServerEndpoint, String> {
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

fn refresh_runtime(mut state: RouterControlSignal) {
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

fn reconcile_selection(state: &mut RouterControlState, snapshot: &RouterObservabilitySnapshot) {
    let ids: Vec<&str> = snapshot.models.iter().map(|model| model.model.id.as_str()).collect();
    if state
        .selected_target
        .as_deref()
        .is_none_or(|selected| !ids.contains(&selected))
    {
        state.selected_target = ids.first().map(|value| (*value).to_string());
    }
    if state
        .selected_source
        .as_deref()
        .is_none_or(|selected| !ids.contains(&selected))
    {
        state.selected_source = snapshot
            .models
            .iter()
            .find(|model| {
                matches!(model.model.status.phase, RouterModelPhase::Loaded | RouterModelPhase::Sleeping)
                    && !model.model.status.failed
            })
            .map(|model| model.model.id.clone())
            .or_else(|| ids.first().map(|value| (*value).to_string()));
    }
}

fn refresh_router(mut state: RouterControlSignal) {
    let snapshot = state.read().clone();
    let Some(installation) = snapshot.installation.clone() else {
        state.write().notice = Some((false, "No persisted llama.cpp installation is selected. Select one in CORE LAB first.".into()));
        return;
    };
    if installation.server.is_none() {
        state.write().notice = Some((false, "The selected llama.cpp installation does not contain llama-server.".into()));
        return;
    }
    let endpoint = match endpoint_from_state(&snapshot) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            let mut current = state.write();
            current.tracker.reconcile(Err(error.clone()));
            current.notice = Some((false, error));
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
            None => Some((true, "Live router state reconciled. Previous snapshots are not assumed current.".into())),
        };
    });
}

fn monitor_operation(mut state: RouterControlSignal, controller: RouterOperationController) {
    thread::spawn(move || {
        let mut observed_non_idle = false;
        for _ in 0..2400 {
            thread::sleep(Duration::from_millis(50));
            let observed = controller.state();
            if !matches!(observed, RouterOperationState::Idle) {
                observed_non_idle = true;
                state.write().operation_state = observed.clone();
            }
            if observed_non_idle && !matches!(observed, RouterOperationState::Running(_)) {
                break;
            }
        }
    });
}

fn run_action(
    mut state: RouterControlSignal,
    action: RequestedAction,
    source: Option<String>,
    target: Option<String>,
) {
    let snapshot = state.read().clone();
    if snapshot.pending_action.is_some() || matches!(snapshot.operation_state, RouterOperationState::Running(_)) {
        state.write().notice = Some((false, "Another router operation is already running.".into()));
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

    monitor_operation(state, controller.clone());
    thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let store = open_store(&paths)?;
            match action {
                RequestedAction::Reload => controller
                    .reload_registry(&installation, &endpoint, Some(&store), CONTROL_TIMEOUT, &cancellation)
                    .map(|_| ()),
                RequestedAction::Load => controller
                    .load_model(
                        &installation,
                        &endpoint,
                        &store,
                        target.as_deref().ok_or_else(|| "No target model selected.".to_string())?,
                        CONTROL_TIMEOUT,
                        &cancellation,
                    )
                    .map(|_| ()),
                RequestedAction::Unload => controller
                    .unload_model(
                        &installation,
                        &endpoint,
                        Some(&store),
                        source.as_deref().ok_or_else(|| "No source model selected.".to_string())?,
                        CONTROL_TIMEOUT,
                        &cancellation,
                    )
                    .map(|_| ()),
                RequestedAction::Preload => controller
                    .preload_model(
                        &installation,
                        &endpoint,
                        &store,
                        target.as_deref().ok_or_else(|| "No target model selected.".to_string())?,
                        CONTROL_TIMEOUT,
                        &cancellation,
                    )
                    .map(|_| ()),
                RequestedAction::Switch => controller
                    .switch_model(
                        &installation,
                        &endpoint,
                        &store,
                        source.as_deref().ok_or_else(|| "No source model selected.".to_string())?,
                        target.as_deref().ok_or_else(|| "No target model selected.".to_string())?,
                        CONTROL_TIMEOUT,
                        &cancellation,
                    )
                    .map(|_| ()),
            }
            .map_err(|error| error.to_string())
        })();

        let refresh = discover_router_observability(&installation, &endpoint, open_store(&paths).ok().as_ref(), REFRESH_TIMEOUT)
            .map_err(|error| error.to_string());
        let final_operation = controller.state();
        let mut current = state.write();
        current.pending_action = None;
        current.operation_state = final_operation;
        if let Ok(snapshot) = refresh.as_ref() {
            reconcile_selection(&mut current, snapshot);
        }
        current.tracker.reconcile(refresh);
        current.notice = match result {
            Ok(()) => Some((true, format!("{} completed and live router state was reconciled.", action.label()))),
            Err(error) => Some((false, format!("{} failed: {error}. Last controller evidence is retained below.", action.label()))),
        };
    });
}

fn persist_endpoint(mut state: RouterControlSignal) {
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
            state.write().notice = Some((true, "Router endpoint preferences persisted. API keys are intentionally never persisted.".into()));
        }
        Err(error) => state.write().notice = Some((false, error)),
    }
}

fn set_preferred_model(mut state: RouterControlSignal, model: String) {
    let snapshot = state.read().clone();
    let Some(paths) = snapshot.paths.as_ref() else {
        state.write().notice = Some((false, "Application storage paths are unavailable.".into()));
        return;
    };
    let mut preferences = snapshot.preferences.clone();
    preferences.preferred_model = Some(model.clone());
    match save_preferences(paths, &preferences) {
        Ok(()) => {
            state.write().preferences = preferences;
            state.write().notice = Some((true, format!("Preferred model `{model}` persisted. Startup behavior remains unverified until a live post-restart reconciliation proves it.")));
        }
        Err(error) => state.write().notice = Some((false, error)),
    }
}

fn feature_for<'a>(registry: &'a RouterRegistry, action: RequestedAction) -> &'a RouterFeatureEvidence {
    match action {
        RequestedAction::Reload => &registry.endpoints.reload_models,
        RequestedAction::Load | RequestedAction::Preload => &registry.endpoints.load_model,
        RequestedAction::Unload => &registry.endpoints.unload_model,
        RequestedAction::Switch => {
            if registry.endpoints.load_model.state != RouterFeatureState::Supported {
                &registry.endpoints.load_model
            } else {
                &registry.endpoints.unload_model
            }
        }
    }
}

fn action_support(snapshot: &RouterControlState, action: RequestedAction) -> (bool, String) {
    if snapshot.busy() {
        return (false, "another refresh or operation is running".into());
    }
    if !snapshot.tracker.is_live() {
        return (false, "controls require a live reconciled router snapshot; stale/failed state cannot authorize mutation".into());
    }
    let Some(current) = snapshot.tracker.current.as_ref() else {
        return (false, "no live router snapshot".into());
    };
    if current.registry.role != RouterRole::Router {
        return (false, "selected endpoint reports single-model server role".into());
    }
    let feature = feature_for(&current.registry, action);
    match feature.state {
        RouterFeatureState::Supported => (true, feature.reason.clone()),
        RouterFeatureState::Unsupported => (false, format!("unsupported: {}", feature.reason)),
        RouterFeatureState::Unknown => (false, format!("support unknown: {}", feature.reason)),
    }
}

fn freshness_class(freshness: RouterSnapshotFreshness) -> &'static str {
    match freshness {
        RouterSnapshotFreshness::Live => "rc-badge",
        RouterSnapshotFreshness::Loading | RouterSnapshotFreshness::Stale | RouterSnapshotFreshness::Empty => "rc-badge warn",
        RouterSnapshotFreshness::Failed => "rc-badge error",
    }
}

fn phase_text(phase: &RouterModelPhase, failed: bool) -> (String, &'static str) {
    if failed {
        return (format!("FAILED · {phase:?}"), "rc-error");
    }
    match phase {
        RouterModelPhase::Loaded => ("LOADED".into(), "rc-live"),
        RouterModelPhase::Sleeping => ("SLEEPING".into(), "rc-live"),
        RouterModelPhase::Loading | RouterModelPhase::Downloading => (format!("{phase:?}").to_uppercase(), "rc-warn"),
        RouterModelPhase::Unloaded => ("UNLOADED".into(), "rc-warn"),
        RouterModelPhase::Unknown(value) => (format!("UNKNOWN · {value}"), "rc-warn"),
    }
}

fn residency_text(model: &RouterModelObservability) -> (String, &'static str) {
    match (model.residency.availability, model.residency.value) {
        (EvidenceAvailability::Observed, Some(true)) => ("RESIDENT · OBSERVED".into(), "rc-live"),
        (EvidenceAvailability::Observed, Some(false)) => ("NOT RESIDENT · OBSERVED".into(), "rc-warn"),
        _ => ("UNKNOWN / UNAVAILABLE".into(), "rc-warn"),
    }
}

fn active_text(model: &RouterModelObservability) -> (String, &'static str) {
    match model.active_requests.value {
        Some(0) if model.active_requests.availability == EvidenceAvailability::Observed => ("0 · OBSERVED".into(), "rc-live"),
        Some(value) if model.active_requests.availability == EvidenceAvailability::Observed => (format!("{value} · ACTIVE"), "rc-error"),
        _ => ("UNKNOWN / UNAVAILABLE".into(), "rc-warn"),
    }
}

fn operation_view(state: &RouterOperationState) -> (String, &'static str, String) {
    match state {
        RouterOperationState::Idle => ("IDLE".into(), "rc-warn", "No router operation evidence has been recorded in this session.".into()),
        RouterOperationState::Running(progress) => (
            format!("RUNNING · {:?}", progress.kind),
            "rc-warn",
            format!("{}\nsource={:?}\ntarget={:?}\nstarted={}", progress.message, progress.source_model, progress.target_model, progress.started_at_unix_ms),
        ),
        RouterOperationState::Succeeded(evidence) => (
            format!("SUCCEEDED · {:?}", evidence.kind),
            "rc-live",
            format!("source={:?}\ntarget={:?}\nHTTP={:?}\nstarted={}\ncompleted={}\n{}", evidence.source_model, evidence.target_model, evidence.http_statuses, evidence.started_at_unix_ms, evidence.completed_at_unix_ms, evidence.notes.join("\n")),
        ),
        RouterOperationState::Failed(failure) => (
            format!("FAILED · {:?}", failure.kind),
            "rc-error",
            format!("{}\nsource={:?}\ntarget={:?}\nstarted={}\nfailed={}\nlast_registry={}", failure.message, failure.source_model, failure.target_model, failure.started_at_unix_ms, failure.failed_at_unix_ms, failure.last_registry.as_ref().map(|registry| registry.observed_at_unix_ms.to_string()).unwrap_or_else(|| "none".into())),
        ),
        RouterOperationState::Cancelled(failure) => (
            format!("CANCELLED · {:?}", failure.kind),
            "rc-warn",
            format!("{}\nsource={:?}\ntarget={:?}\nlast_registry={}", failure.message, failure.source_model, failure.target_model, failure.last_registry.as_ref().map(|registry| registry.observed_at_unix_ms.to_string()).unwrap_or_else(|| "none".into())),
        ),
    }
}

fn preference_view(verification: &PreferredModelVerification) -> (String, &'static str, String) {
    match verification {
        PreferredModelVerification::NotConfigured => ("NOT SET".into(), "rc-warn", "No preferred model is persisted.".into()),
        PreferredModelVerification::NeedsLiveReconciliation => ("UNVERIFIED".into(), "rc-warn", "A live post-reconnect snapshot is required before any startup/default claim is made.".into()),
        PreferredModelVerification::Unsupported { reason } => ("N/A".into(), "rc-warn", reason.clone()),
        PreferredModelVerification::Missing { model } => ("MISSING".into(), "rc-error", format!("Preferred model `{model}` is not in the live registry.")),
        PreferredModelVerification::NotReady { model, phase } => ("NOT READY".into(), "rc-error", format!("Preferred model `{model}` reconciled as {phase}; prior loaded state is not retained as truth.")),
        PreferredModelVerification::Verified { model } => ("VERIFIED LIVE".into(), "rc-live", format!("Preferred model `{model}` is loaded/sleeping in the current post-reconciliation registry.")),
    }
}

#[allow(non_snake_case)]
pub fn RouterManagementView() -> Element {
    let mut state = use_signal_sync(RouterControlState::load);
    let snapshot = state.read().clone();
    let freshness = snapshot.tracker.freshness();
    let current = snapshot.tracker.current.as_ref();
    let busy = snapshot.busy();
    let (reload_ok, reload_reason) = action_support(&snapshot, RequestedAction::Reload);
    let (load_ok, load_reason) = action_support(&snapshot, RequestedAction::Load);
    let (unload_ok, unload_reason) = action_support(&snapshot, RequestedAction::Unload);
    let (preload_ok, preload_reason) = action_support(&snapshot, RequestedAction::Preload);
    let (switch_ok, switch_reason) = action_support(&snapshot, RequestedAction::Switch);
    let (operation_label, operation_class, operation_detail) = operation_view(&snapshot.operation_state);
    let verification = verify_preferred_model(&snapshot.preferences, &snapshot.tracker);
    let (preference_label, preference_class, preference_detail) = preference_view(&verification);
    let source_value = snapshot.selected_source.clone().unwrap_or_default();
    let target_value = snapshot.selected_target.clone().unwrap_or_default();

    rsx! {
        style { dangerous_inner_html: ROUTER_CONTROL_CSS }
        main { class: "rc-page",
            header { class: "rc-header",
                div {
                    div { class: "rc-kicker", "> LLAMAWAVE / ROUTER CONTROL" }
                    h1 { "CONTROL THE ROUTER. KEEP THE EVIDENCE." }
                    p { "Live model actions are enabled only from supported, reconciled router capability evidence. Stale state cannot authorize mutations, and unavailable residency/active-request fields stay unknown." }
                }
                div { class: "rc-fresh",
                    div { class: "rc-kicker", "CANONICAL SNAPSHOT" }
                    strong { "{freshness:?}" }
                    span { class: freshness_class(freshness), "{freshness:?}" }
                }
            }

            if let Some((success, message)) = snapshot.notice.as_ref() {
                div { class: if *success { "rc-notice" } else { "rc-notice error" }, "{message}" }
            }
            if freshness == RouterSnapshotFreshness::Stale {
                div { class: "rc-notice error", "STALE · the last successful snapshot is retained only as evidence. All mutation controls remain disabled until live reconciliation succeeds." }
            }

            section { class: "rc-panel",
                div { class: "rc-panel-head",
                    div {
                        div { class: "rc-kicker", "CONNECTION + RECONCILIATION" }
                        h2 { "LIVE ROUTER ENDPOINT" }
                    }
                    button { class: "rc-button", disabled: busy, onclick: move |_| refresh_runtime(state), "REFRESH RUNTIME" }
                }
                div { class: "rc-panel-body",
                    div { class: "rc-fields",
                        div { class: "rc-field",
                            label { "HOST" }
                            input { class: "rc-input", value: "{snapshot.preferences.host}", disabled: busy, oninput: move |event| state.write().preferences.host = event.value() }
                        }
                        div { class: "rc-field",
                            label { "PORT" }
                            input { class: "rc-input", value: "{snapshot.port_text}", disabled: busy, oninput: move |event| state.write().port_text = event.value() }
                        }
                        div { class: "rc-field wide",
                            label { "API KEY · SESSION ONLY · NEVER PERSISTED" }
                            input { class: "rc-input", r#type: "password", value: "{snapshot.api_key}", disabled: busy, oninput: move |event| state.write().api_key = event.value() }
                        }
                    }
                    div { class: "rc-actions",
                        button {
                            class: if snapshot.preferences.allow_non_loopback { "rc-button magenta" } else { "rc-button" },
                            disabled: busy,
                            onclick: move |_| {
                                let enabled = state.read().preferences.allow_non_loopback;
                                state.write().preferences.allow_non_loopback = !enabled;
                            },
                            if snapshot.preferences.allow_non_loopback { "LAN OPT-IN ON" } else { "LAN OPT-IN OFF" }
                        }
                        button { class: "rc-button", disabled: busy, onclick: move |_| persist_endpoint(state), "SAVE ENDPOINT" }
                        button { class: "rc-button primary", disabled: busy || snapshot.installation.is_none(), onclick: move |_| refresh_router(state), if snapshot.tracker.loading { "RECONCILING..." } else { "RECONCILE LIVE STATE" } }
                        button {
                            class: "rc-button",
                            disabled: !reload_ok,
                            title: "{reload_reason}",
                            onclick: move |_| run_action(state, RequestedAction::Reload, None, None),
                            "RELOAD REGISTRY"
                        }
                    }
                    div { class: "rc-runtime",
                        strong { "SELECTED LLAMA.CPP EVIDENCE\n" }
                        if let Some(installation) = snapshot.installation.as_ref() {
                            if let Some(server) = installation.server.as_ref() {
                                "{server.path.display()}\nSHA-256 {server.sha256}"
                            } else { "selected installation has no llama-server" }
                        } else { "no persisted runtime selected" }
                    }
                }
            }

            div { class: "rc-control-grid",
                section { class: "rc-panel", style: "margin-top:0",
                    div { class: "rc-panel-head",
                        div { div { class: "rc-kicker", "MODEL SWITCHING" } h2 { "SOURCE → TARGET" } }
                    }
                    div { class: "rc-panel-body",
                        div { class: "rc-fields",
                            div { class: "rc-field",
                                label { "SOURCE" }
                                select { class: "rc-select", value: "{source_value}", disabled: busy || current.is_none(), onchange: move |event| state.write().selected_source = Some(event.value()),
                                    if let Some(current) = current { for model in current.models.iter() { option { value: "{model.model.id}", "{model.model.id}" } } }
                                }
                            }
                            div { class: "rc-field",
                                label { "TARGET" }
                                select { class: "rc-select", value: "{target_value}", disabled: busy || current.is_none(), onchange: move |event| state.write().selected_target = Some(event.value()),
                                    if let Some(current) = current { for model in current.models.iter() { option { value: "{model.model.id}", "{model.model.id}" } } }
                                }
                            }
                        }
                        div { class: "rc-actions",
                            button { class: "rc-button primary", disabled: !switch_ok || source_value.is_empty() || target_value.is_empty(), title: "{switch_reason}", onclick: move |_| run_action(state, RequestedAction::Switch, state.read().selected_source.clone(), state.read().selected_target.clone()), "SWITCH MODEL" }
                            button { class: "rc-button", disabled: !load_ok || target_value.is_empty(), title: "{load_reason}", onclick: move |_| run_action(state, RequestedAction::Load, None, state.read().selected_target.clone()), "LOAD TARGET" }
                            button { class: "rc-button", disabled: !preload_ok || target_value.is_empty(), title: "{preload_reason}", onclick: move |_| run_action(state, RequestedAction::Preload, None, state.read().selected_target.clone()), "PRELOAD TARGET" }
                            button { class: "rc-button danger", disabled: !unload_ok || source_value.is_empty(), title: "{unload_reason}", onclick: move |_| run_action(state, RequestedAction::Unload, state.read().selected_source.clone(), None), "UNLOAD SOURCE" }
                            if matches!(snapshot.operation_state, RouterOperationState::Running(_)) {
                                button { class: "rc-button danger", onclick: move |_| state.read().cancellation.cancel(), "CANCEL" }
                            }
                        }
                        div { class: "rc-support",
                            {support_row("LOAD", load_ok, &load_reason)}
                            {support_row("UNLOAD", unload_ok, &unload_reason)}
                            {support_row("PRELOAD", preload_ok, &preload_reason)}
                            {support_row("SWITCH", switch_ok, &switch_reason)}
                        }
                    }
                }

                section { class: "rc-panel", style: "margin-top:0",
                    div { class: "rc-panel-head", div { div { class: "rc-kicker", "OPERATION EVIDENCE" } h2 { "CURRENT / LAST ACTION" } } }
                    div { class: "rc-panel-body",
                        div { class: "rc-operation",
                            div { class: "rc-kicker", "CONTROLLER STATE" }
                            strong { class: operation_class, "{operation_label}" }
                            if let Some(action) = snapshot.pending_action { span { class: "rc-muted", "UI DISPATCH · {action.label()}" } }
                            pre { "{operation_detail}" }
                        }
                        if matches!(snapshot.operation_state, RouterOperationState::Failed(_) | RouterOperationState::Cancelled(_)) {
                            div { class: "rc-actions",
                                button { class: "rc-button primary", disabled: busy, onclick: move |_| refresh_router(state), "RECOVER / RECONCILE" }
                            }
                        }
                    }
                }
            }

            section { class: "rc-panel",
                div { class: "rc-panel-head",
                    div { div { class: "rc-kicker", "PERSISTENCE + RESTART TRUTH" } h2 { "PREFERRED MODEL" } }
                }
                div { class: "rc-panel-body",
                    div { class: "rc-preference",
                        div { class: "rc-kicker", "POST-RESTART VERIFICATION" }
                        strong { class: preference_class, "{preference_label}" }
                        p { class: "rc-muted", "{preference_detail}" }
                        if let Some(model) = snapshot.preferences.preferred_model.as_ref() { div { class: "rc-detail", "PERSISTED TARGET · {model}" } }
                        p { class: "rc-muted", style: "margin-top:8px", "LlamaWave persists the preferred target, but never treats persistence as proof of upstream startup behavior. A live reconnect/restart snapshot must confirm readiness, and unsupported models-autoload capability remains N/A." }
                    }
                }
            }

            section { class: "rc-panel",
                div { class: "rc-panel-head",
                    div { div { class: "rc-kicker", "CANONICAL LIVE REGISTRY" } h2 { "MODELS + ROUTING EVIDENCE" } }
                    if let Some(current) = current { span { class: "rc-muted", "observed {current.observed_at_unix_ms} · {current.models.len()} models" } }
                }
                div { class: "rc-panel-body",
                    if let Some(current) = current {
                        if current.registry.role != RouterRole::Router {
                            div { class: "rc-empty", "Selected endpoint reports single-model server role. Router controls are disabled by contract." }
                        } else if current.models.is_empty() {
                            div { class: "rc-empty", "Router returned an empty model registry." }
                        } else {
                            div { class: "rc-model-grid",
                                for observed in current.models.iter() {
                                    {
                                        let model_id = observed.model.id.clone();
                                        let (phase, phase_class) = phase_text(&observed.model.status.phase, observed.model.status.failed);
                                        let (resident, resident_class) = residency_text(observed);
                                        let (active, active_class) = active_text(observed);
                                        let aliases: Vec<String> = observed.model.routing_targets.iter().filter(|target| *target != &observed.model.id).cloned().collect();
                                        let stale = freshness != RouterSnapshotFreshness::Live;
                                        rsx! {
                                            article { class: if stale { "rc-card stale" } else { "rc-card" },
                                                div { class: "rc-card-head",
                                                    strong { "{observed.model.id}" }
                                                    if aliases.is_empty() { div { class: "rc-muted", "NO ALIAS EVIDENCE" } } else { div { class: "rc-aliases", for alias in aliases.iter() { span { class: "rc-alias", "ALIAS · {alias}" } } }
                                                }
                                                div { class: "rc-stat-grid",
                                                    div { class: "rc-stat", span { "ROUTER PHASE" } strong { class: phase_class, "{phase}" } small { "direct /models state" } }
                                                    div { class: "rc-stat", span { "RESIDENCY" } strong { class: resident_class, "{resident}" } small { "{observed.residency.reason}" } }
                                                    div { class: "rc-stat", span { "ACTIVE REQUESTS" } strong { class: active_class, "{active}" } small { "{observed.active_requests.reason}" } }
                                                    div { class: "rc-stat", span { "M2 IDENTITY" } strong { "{observed.model.library_link.kind:?}" } small { "{observed.model.library_link.reason}" } }
                                                }
                                                div { class: "rc-card-actions",
                                                    button { class: "rc-button", disabled: !load_ok, title: "{load_reason}", onclick: { let model_id = model_id.clone(); move |_| run_action(state, RequestedAction::Load, None, Some(model_id.clone())) }, "LOAD" }
                                                    button { class: "rc-button", disabled: !preload_ok, title: "{preload_reason}", onclick: { let model_id = model_id.clone(); move |_| run_action(state, RequestedAction::Preload, None, Some(model_id.clone())) }, "PRELOAD" }
                                                    button { class: "rc-button danger", disabled: !unload_ok, title: "{unload_reason}", onclick: { let model_id = model_id.clone(); move |_| run_action(state, RequestedAction::Unload, Some(model_id.clone()), None) }, "UNLOAD" }
                                                    button { class: "rc-button magenta", disabled: busy, onclick: { let model_id = model_id.clone(); move |_| set_preferred_model(state, model_id.clone()) }, "SET PREFERRED" }
                                                }
                                                if !load_ok || !unload_ok { div { class: "rc-disabled-reason", "Controls remain disabled whenever live capability evidence is unsupported/unknown or the snapshot is stale." } }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(error) = current.supplemental_error.as_ref() { div { class: "rc-detail", "SUPPLEMENTAL OBSERVABILITY ERROR · {error}. Canonical registry remains live; unsupported fields stay unavailable." } }
                    } else if snapshot.tracker.loading {
                        div { class: "rc-empty", "Reconciling live router evidence. No success state is shown until the canonical snapshot succeeds." }
                    } else {
                        div { class: "rc-empty", "No router snapshot yet. Reconcile a live router endpoint to enable evidence-backed controls." }
                    }
                }
            }
        }
    }
}

fn support_row(label: &str, enabled: bool, reason: &str) -> Element {
    rsx! {
        div { class: "rc-support-row",
            span { "{label}" }
            strong { class: if enabled { "rc-live" } else { "rc-warn" }, "{reason}" }
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
        router_observability::{EvidenceValue, RouterObservabilitySnapshot},
    };

    fn feature(state: RouterFeatureState) -> RouterFeatureEvidence {
        RouterFeatureEvidence {
            state,
            reason: "fixture".into(),
        }
    }

    fn snapshot(phase: RouterModelPhase, models_autoload: bool) -> RouterObservabilitySnapshot {
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
                    models_autoload,
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
                residency: EvidenceValue { availability: EvidenceAvailability::Unavailable, value: None, reason: "fixture".into() },
                active_requests: EvidenceValue { availability: EvidenceAvailability::Unavailable, value: None, reason: "fixture".into() },
                last_used_ms: EvidenceValue { availability: EvidenceAvailability::Unavailable, value: None, reason: "fixture".into() },
                lru_rank: EvidenceValue { availability: EvidenceAvailability::Unavailable, value: None, reason: "fixture".into() },
                evictable: EvidenceValue { availability: EvidenceAvailability::Unavailable, value: None, reason: "fixture".into() },
            }],
            supplemental_error: None,
            observed_at_unix_ms: 1,
        }
    }

    #[test]
    fn preferences_persist_without_api_key_surface() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_root(crate::paths::StorageMode::Portable, temp.path().join("router prefs")).unwrap();
        let prefs = RouterControlPreferences {
            host: "localhost".into(),
            port: 9999,
            allow_non_loopback: false,
            preferred_model: Some("alpha".into()),
        };
        save_preferences(&paths, &prefs).unwrap();
        assert_eq!(load_preferences(&paths).unwrap(), prefs);
        let raw = fs::read_to_string(preferences_path(&paths)).unwrap();
        assert!(!raw.to_ascii_lowercase().contains("api_key"));
    }

    #[test]
    fn preferred_model_requires_live_post_restart_reconciliation() {
        let prefs = RouterControlPreferences { preferred_model: Some("alpha".into()), ..RouterControlPreferences::default() };
        let mut tracker = RouterObservabilityTracker::default();
        assert_eq!(verify_preferred_model(&prefs, &tracker), PreferredModelVerification::NeedsLiveReconciliation);
        tracker.reconcile(Ok(snapshot(RouterModelPhase::Loaded, true)));
        assert_eq!(verify_preferred_model(&prefs, &tracker), PreferredModelVerification::Verified { model: "alpha".into() });
        tracker.reconcile(Err("router restarted".into()));
        assert_eq!(verify_preferred_model(&prefs, &tracker), PreferredModelVerification::NeedsLiveReconciliation);
    }

    #[test]
    fn unsupported_autoload_is_never_presented_as_verified_startup_behavior() {
        let prefs = RouterControlPreferences { preferred_model: Some("alpha".into()), ..RouterControlPreferences::default() };
        let mut tracker = RouterObservabilityTracker::default();
        tracker.reconcile(Ok(snapshot(RouterModelPhase::Loaded, false)));
        assert!(matches!(verify_preferred_model(&prefs, &tracker), PreferredModelVerification::Unsupported { .. }));
    }

    #[test]
    fn post_restart_unloaded_target_is_not_ready_even_when_preference_survives() {
        let prefs = RouterControlPreferences { preferred_model: Some("alpha".into()), ..RouterControlPreferences::default() };
        let mut tracker = RouterObservabilityTracker::default();
        tracker.reconcile(Ok(snapshot(RouterModelPhase::Unloaded, true)));
        assert!(matches!(verify_preferred_model(&prefs, &tracker), PreferredModelVerification::NotReady { .. }));
    }
}
