use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use dioxus::prelude::*;
use rfd::FileDialog;

use crate::{
    gguf::ModelInfo,
    llama::LlamaInstallation,
    paths::AppPaths,
    persistence::Database,
    server_command::{ServerLaunchSettings, ServerLaunchSpec, build_server_launch_spec},
    server_console::{hide_managed_console_window, request_graceful_console_interrupt},
    server_logs::{
        DEFAULT_SERVER_LOG_DISK_RETENTION_BYTES, DEFAULT_SERVER_LOG_RETENTION_BYTES,
        ServerLifecyclePhase, ServerLifecycleTracker, ServerLogCapture, ServerLogSeverity,
        ServerLogSnapshot, ServerLogStream,
    },
    server_process::{GracefulStopOutcome, ManagedProcessState, ServerProcessSupervisor},
    server_readiness::{
        PortAvailability, ReadinessPolicy, ServerEndpoint, ServerReadinessError,
        ServerReadinessEvidence, check_port_available, require_port_available,
        wait_for_server_ready,
    },
};

const SERVER_UI_CSS: &str = r#"
.sv-page{min-height:100vh;padding:30px 34px 92px;color:#f6eaff;background:radial-gradient(circle at 82% 10%,rgba(255,0,190,.14),transparent 35%),radial-gradient(circle at 9% 78%,rgba(0,255,255,.07),transparent 37%),#07000e;font-family:"Cascadia Mono","Cascadia Code",Consolas,monospace;box-sizing:border-box}.sv-page *{box-sizing:border-box}.sv-header{display:flex;justify-content:space-between;gap:24px;align-items:flex-start;padding-bottom:18px;border-bottom:1px solid rgba(0,255,255,.42)}.sv-kicker{color:#00ffff;font-size:9px;font-weight:900;letter-spacing:.15em}.sv-header h1{margin:7px 0 8px;font-size:clamp(26px,3vw,40px)}.sv-header p,.sv-muted{margin:0;color:#a996bb;font-size:10px;line-height:1.65}.sv-phase{text-align:right;min-width:220px}.sv-phase strong{display:block;margin-top:5px;font-size:18px}.sv-badge{display:inline-flex;align-items:center;min-height:22px;padding:0 7px;border:1px solid rgba(0,255,255,.45);color:#76ffe6;font-size:8px;font-weight:900;letter-spacing:.07em;text-transform:uppercase}.sv-badge.warn{border-color:#ffd36b;color:#ffd36b}.sv-badge.error{border-color:#ff3d7f;color:#ff7ba9}.sv-notice{margin:15px 0 0;padding:10px 12px;border:1px solid rgba(117,255,226,.48);background:rgba(0,20,18,.6);color:#baffed;font-size:10px;overflow-wrap:anywhere}.sv-notice.error{border-color:rgba(255,50,110,.58);background:rgba(40,0,18,.58);color:#ff91b5}.sv-grid{display:grid;grid-template-columns:minmax(0,1.15fr) minmax(330px,.85fr);gap:12px;margin-top:15px}.sv-panel{min-width:0;border:1px solid rgba(0,255,255,.32);background:linear-gradient(180deg,rgba(29,5,47,.83),rgba(7,0,15,.92))}.sv-panel-head{display:flex;justify-content:space-between;align-items:center;gap:12px;padding:12px 14px;border-bottom:1px solid rgba(0,255,255,.25)}.sv-panel-head h2{margin:4px 0 0;font-size:16px}.sv-panel-body{min-width:0;padding:13px}.sv-fields{display:grid;grid-template-columns:minmax(0,1fr) 150px;gap:9px}.sv-field{min-width:0}.sv-field.wide{grid-column:1/-1}.sv-field label{display:block;margin-bottom:5px;color:#9b80a9;font-size:8px;letter-spacing:.08em;text-transform:uppercase}.sv-input{width:100%;min-height:34px;padding:7px 9px;border:1px solid rgba(0,255,255,.3);border-radius:0;background:#030008;color:#f6eaff;font:inherit;font-size:10px}.sv-input:focus-visible,.sv-button:focus-visible{outline:2px solid #ff00ff;outline-offset:2px}.sv-path,.sv-command,.sv-detail{min-width:0;padding:9px;border-left:2px solid #00ffff;background:rgba(0,0,0,.34);color:#cbb9d7;font-size:9px;line-height:1.55;overflow-wrap:anywhere;word-break:break-word;white-space:pre-wrap}.sv-command{margin-top:10px;border-left-color:#ff00d4;color:#f5dcff}.sv-detail{border-left-color:#785b91}.sv-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:12px}.sv-button{min-height:34px;padding:0 12px;border:1px solid #00dbe7;border-radius:0;background:transparent;color:#00f5ff;font:inherit;font-size:8px;font-weight:900;letter-spacing:.08em;text-transform:uppercase;cursor:pointer}.sv-button:hover:not(:disabled),.sv-button.primary{background:#00ffff;color:#050009}.sv-button.magenta{border-color:#ff00d4;color:#ff55e7}.sv-button.magenta:hover:not(:disabled){background:#ff00d4;color:#070009}.sv-button.danger{border-color:#ff356f;color:#ff739f}.sv-button.danger.confirm{background:#ff245c;color:#090007}.sv-button:disabled{opacity:.34;cursor:not-allowed}.sv-status-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:8px;margin-top:12px}.sv-stat{min-width:0;padding:10px;border:1px solid rgba(106,74,126,.5);background:rgba(0,0,0,.26)}.sv-stat span{display:block;color:#887597;font-size:7px;text-transform:uppercase}.sv-stat strong{display:block;margin-top:5px;font-size:11px;overflow-wrap:anywhere}.sv-readiness{display:grid;gap:7px}.sv-evidence{padding:9px;border:1px solid rgba(100,72,118,.45);background:rgba(0,0,0,.25);font-size:9px;line-height:1.55;overflow-wrap:anywhere;word-break:break-word}.sv-log-panel{margin-top:12px}.sv-log-toolbar{display:flex;justify-content:space-between;align-items:center;gap:10px;flex-wrap:wrap;margin-bottom:8px}.sv-log-list{height:330px;overflow:auto;border:1px solid rgba(0,255,255,.22);background:#020006;padding:7px}.sv-log{display:grid;grid-template-columns:54px 44px minmax(0,1fr);gap:7px;padding:5px 4px;border-bottom:1px solid rgba(88,60,105,.24);font-size:8px;line-height:1.45}.sv-log .stream{color:#00e8ff}.sv-log.stderr .stream{color:#ff7cb0}.sv-log.warning{background:rgba(255,211,107,.04)}.sv-log.error{background:rgba(255,80,130,.05)}.sv-log.fatal{background:rgba(255,20,70,.1);border-left:2px solid #ff245c}.sv-log .seq{color:#725d80}.sv-log .text{min-width:0;white-space:pre-wrap;overflow-wrap:anywhere;word-break:break-word}.sv-empty{padding:46px 14px;text-align:center;color:#8e789b;font-size:9px}.sv-warning{margin-top:8px;padding:8px;border-left:2px solid #ffd36b;background:rgba(255,211,107,.05);color:#e5c778;font-size:8px;line-height:1.5;overflow-wrap:anywhere}.sv-disk-error{margin-top:7px;color:#ff7ba9;font-size:8px;overflow-wrap:anywhere}@media(max-width:980px){.sv-page{padding:22px 22px 92px}.sv-header{flex-direction:column}.sv-phase{text-align:left;min-width:0}.sv-grid{grid-template-columns:1fr}.sv-status-grid{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:620px){.sv-fields{grid-template-columns:1fr}.sv-field.wide{grid-column:auto}.sv-status-grid{grid-template-columns:1fr}.sv-log{grid-template-columns:44px minmax(0,1fr)}.sv-log .seq{display:none}}@media(prefers-reduced-motion:reduce){.sv-page *,.sv-page *::before,.sv-page *::after{transition:none!important;animation:none!important}}
"#;

type ServerStateSignal = Signal<ServerUiState, SyncStorage>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerOperation {
    Idle,
    Starting,
    Stopping,
}

impl ServerOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Starting => "STARTING",
            Self::Stopping => "STOPPING",
        }
    }
}

#[derive(Debug, Clone)]
struct ServerUiState {
    paths: Option<AppPaths>,
    installation: Option<LlamaInstallation>,
    model: Option<ModelInfo>,
    supervisor: Arc<Mutex<ServerProcessSupervisor>>,
    cancellation: Arc<AtomicBool>,
    lifecycle: ServerLifecycleTracker,
    readiness: Option<ServerReadinessEvidence>,
    logs: Option<ServerLogSnapshot>,
    active_logs: Option<ServerLogCapture>,
    process_owned: bool,
    operation: ServerOperation,
    generation: u64,
    host: String,
    port: String,
    api_key: String,
    allow_non_loopback: bool,
    last_command: Option<String>,
    log_path: Option<PathBuf>,
    console_warning: Option<String>,
    force_confirm: bool,
    notice: Option<(bool, String)>,
}

impl ServerUiState {
    fn load() -> Self {
        let supervisor = Arc::new(Mutex::new(ServerProcessSupervisor::new()));
        let cancellation = Arc::new(AtomicBool::new(false));
        let mut lifecycle = ServerLifecycleTracker::default();
        let mut notice = None;

        let paths = match AppPaths::detect() {
            Ok(paths) => Some(paths),
            Err(error) => {
                notice = Some((
                    false,
                    format!("Could not resolve application paths: {error}"),
                ));
                None
            }
        };

        let (installation, model) = match paths.as_ref() {
            Some(paths) => match Database::open(paths.database.clone()) {
                Ok(db) => match (db.latest_installation(), db.latest_model()) {
                    (Ok(installation), Ok(model)) => (installation, model),
                    (installation, model) => {
                        notice = Some((
                            false,
                            format!(
                                "Could not reload persisted runtime/model evidence: installation={:?}, model={:?}",
                                installation.err(),
                                model.err()
                            ),
                        ));
                        (None, None)
                    }
                },
                Err(error) => {
                    notice = Some((
                        false,
                        format!("Could not open application database: {error}"),
                    ));
                    (None, None)
                }
            },
            None => (None, None),
        };

        match check_port_available(&ServerEndpoint::loopback(8080)) {
            Ok(PortAvailability::InUse) => {
                lifecycle.reconcile_after_restart(None, true);
                if notice.is_none() {
                    notice = Some((
                        false,
                        "127.0.0.1:8080 is already occupied. It is UNKNOWN/unowned, not assumed to be a managed LlamaWave server. Choose another port or stop the external process."
                            .into(),
                    ));
                }
            }
            Ok(PortAvailability::Available) => lifecycle.reconcile_after_restart(None, false),
            Err(error) => {
                if notice.is_none() {
                    notice = Some((
                        false,
                        format!("Could not reconcile default server port: {error}"),
                    ));
                }
            }
        }

        Self {
            paths,
            installation,
            model,
            supervisor,
            cancellation,
            lifecycle,
            readiness: None,
            logs: None,
            active_logs: None,
            process_owned: false,
            operation: ServerOperation::Idle,
            generation: 0,
            host: "127.0.0.1".into(),
            port: "8080".into(),
            api_key: String::new(),
            allow_non_loopback: false,
            last_command: None,
            log_path: None,
            console_warning: None,
            force_confirm: false,
            notice,
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedLaunch {
    spec: ServerLaunchSpec,
    endpoint: ServerEndpoint,
    paths: AppPaths,
    secrets: Vec<String>,
}

fn prepare_launch(state: &ServerUiState) -> Result<PreparedLaunch, String> {
    let installation = state.installation.as_ref().ok_or_else(|| {
        "No persisted llama.cpp installation is selected. Select one in CORE LAB first.".to_string()
    })?;
    if installation.server.is_none() {
        return Err("The selected llama.cpp installation does not contain llama-server.".into());
    }

    let model = state.model.as_ref().ok_or_else(|| {
        "No persisted GGUF model is selected. Add/select one in MODEL LIBRARY or CORE LAB first."
            .to_string()
    })?;
    if !model.path.is_file() {
        return Err(format!(
            "The selected GGUF is not present at {}. Repair/relink it in MODEL LIBRARY first.",
            model.path.display()
        ));
    }

    let paths = state
        .paths
        .clone()
        .ok_or_else(|| "Application storage paths are unavailable.".to_string())?;
    let host = state.host.trim();
    if host.is_empty() {
        return Err("Host cannot be empty.".into());
    }
    let port = state
        .port
        .trim()
        .parse::<u16>()
        .map_err(|_| "Port must be an integer in 1..=65535.".to_string())?;
    if port == 0 {
        return Err("Port must be in 1..=65535.".into());
    }

    let api_key = state.api_key.trim().to_string();
    let api_key_option = (!api_key.is_empty()).then_some(api_key.clone());
    let settings = ServerLaunchSettings {
        model: model.path.clone(),
        host: Some(host.to_string()),
        port: Some(port),
        api_key: api_key_option.clone(),
        ..ServerLaunchSettings::default()
    };
    let spec =
        build_server_launch_spec(installation, &settings).map_err(|error| error.to_string())?;
    let endpoint = ServerEndpoint {
        host: host.to_string(),
        port,
        api_key: api_key_option,
        allow_non_loopback: state.allow_non_loopback,
    };

    Ok(PreparedLaunch {
        spec,
        endpoint,
        paths,
        secrets: (!api_key.is_empty())
            .then_some(api_key)
            .into_iter()
            .collect(),
    })
}

fn refresh_sources(mut state: ServerStateSignal) {
    let snapshot = state.read().clone();
    let Some(paths) = snapshot.paths else {
        state.write().notice = Some((false, "Application storage paths are unavailable.".into()));
        return;
    };

    thread::spawn(move || {
        let result = Database::open(paths.database.clone())
            .and_then(|db| Ok((db.latest_installation()?, db.latest_model()?)));
        let mut current = state.write();
        match result {
            Ok((installation, model)) => {
                current.installation = installation;
                current.model = model;
                current.notice = Some((
                    true,
                    "Reloaded persisted runtime and model evidence.".into(),
                ));
            }
            Err(error) => current.notice = Some((false, error.to_string())),
        }
    });
}

fn run_start(mut state: ServerStateSignal) {
    let snapshot = state.read().clone();
    if snapshot.process_owned || snapshot.operation != ServerOperation::Idle {
        return;
    }

    {
        let mut current = state.write();
        current.operation = ServerOperation::Starting;
        current.force_confirm = false;
        current.readiness = None;
        current.console_warning = None;
        current.cancellation.store(false, Ordering::SeqCst);
        current.notice = Some((
            true,
            "Validating launch command and endpoint before spawn…".into(),
        ));
    }

    let prepared = match prepare_launch(&snapshot) {
        Ok(prepared) => prepared,
        Err(error) => {
            fail_start(state, error);
            return;
        }
    };
    if let Err(error) = require_port_available(&prepared.endpoint) {
        fail_start(state, error.to_string());
        return;
    }

    let log_path = prepared
        .paths
        .logs
        .join(format!("managed-server-{}.log", prepared.endpoint.port));
    let logs = match ServerLogCapture::with_disk(
        log_path.clone(),
        DEFAULT_SERVER_LOG_RETENTION_BYTES,
        DEFAULT_SERVER_LOG_DISK_RETENTION_BYTES,
        prepared.secrets.clone(),
    ) {
        Ok(logs) => logs,
        Err(error) => {
            fail_start(
                state,
                format!("Could not initialize bounded server log capture: {error}"),
            );
            return;
        }
    };

    let supervisor = snapshot.supervisor.clone();
    let identity = {
        let mut supervisor = match supervisor.lock() {
            Ok(supervisor) => supervisor,
            Err(_) => {
                fail_start(
                    state,
                    "Server process supervisor mutex was poisoned.".into(),
                );
                return;
            }
        };
        match supervisor.start_server_with_log_capture(&prepared.spec, logs.clone()) {
            Ok(identity) => identity.clone(),
            Err(error) => {
                fail_start(state, error.to_string());
                return;
            }
        }
    };

    let console_warning = hide_managed_console_window(identity.pid)
        .err()
        .map(|error| format!("Managed console could not be hidden automatically: {error}"));
    let generation = {
        let mut current = state.write();
        current.generation = current.generation.saturating_add(1);
        current.process_owned = true;
        current.lifecycle.mark_starting(identity.clone());
        current.active_logs = Some(logs.clone());
        current.logs = Some(logs.snapshot());
        current.last_command = Some(prepared.spec.diagnostic_command());
        current.log_path = Some(log_path);
        current.console_warning = console_warning;
        current.notice = Some((
            true,
            format!(
                "Managed llama-server pid {} started. Waiting for health + real minimal inference readiness.",
                identity.pid
            ),
        ));
        current.generation
    };

    spawn_log_pump(state, logs.clone(), generation);
    spawn_process_monitor(state, supervisor.clone(), generation);

    let readiness = {
        let mut supervisor = match supervisor.lock() {
            Ok(supervisor) => supervisor,
            Err(_) => {
                fail_start(
                    state,
                    "Server process supervisor mutex was poisoned during readiness.".into(),
                );
                return;
            }
        };
        let process = match supervisor.process_mut() {
            Ok(process) => process,
            Err(error) => {
                fail_start(state, error.to_string());
                return;
            }
        };
        wait_for_server_ready(
            process,
            &prepared.endpoint,
            &ReadinessPolicy::default(),
            &snapshot.cancellation,
        )
    };

    let mut current = state.write();
    current.logs = Some(logs.snapshot());
    if current.operation != ServerOperation::Stopping {
        current.operation = ServerOperation::Idle;
    }
    match readiness {
        Ok(evidence) => {
            current.lifecycle.mark_ready();
            current.readiness = Some(evidence.clone());
            current.notice = Some((
                true,
                format!(
                    "READY after {} probe attempt(s): health observed and minimal inference returned HTTP {}.",
                    evidence.attempts, evidence.inference.status_code
                ),
            ));
        }
        Err(ServerReadinessError::ProcessExited { evidence }) => {
            current.process_owned = false;
            current.lifecycle.mark_exit(evidence.clone());
            current.notice = Some((
                false,
                format!("llama-server exited before readiness: {evidence:?}"),
            ));
        }
        Err(ServerReadinessError::Cancelled) => {
            current.notice = Some((
                true,
                "Readiness polling was cancelled; the requested stop operation is reconciling the process."
                    .into(),
            ));
        }
        Err(error) => {
            current.lifecycle.mark_failed(error.to_string());
            current.notice = Some((
                false,
                format!(
                    "Server process exists but readiness/inference failed: {error}. Inspect logs, then STOP or FORCE KILL."
                ),
            ));
        }
    }
}

fn fail_start(mut state: ServerStateSignal, message: String) {
    let mut current = state.write();
    current.operation = ServerOperation::Idle;
    current.lifecycle.mark_failed(message.clone());
    current.notice = Some((false, message));
}

fn spawn_log_pump(mut state: ServerStateSignal, logs: ServerLogCapture, generation: u64) {
    thread::spawn(move || {
        loop {
            let keep_running = {
                let current = state.read();
                current.generation == generation && current.process_owned
            };
            state.write().logs = Some(logs.snapshot());
            if !keep_running {
                break;
            }
            thread::sleep(Duration::from_millis(180));
        }
    });
}

fn spawn_process_monitor(
    mut state: ServerStateSignal,
    supervisor: Arc<Mutex<ServerProcessSupervisor>>,
    generation: u64,
) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(250));
            let active = {
                let current = state.read();
                current.generation == generation && current.process_owned
            };
            if !active {
                break;
            }

            let observed = match supervisor.lock() {
                Ok(mut supervisor) => supervisor.state(),
                Err(_) => {
                    let mut current = state.write();
                    current.operation = ServerOperation::Idle;
                    current.lifecycle.mark_failed(
                        "Server process supervisor mutex was poisoned while monitoring.",
                    );
                    current.notice = Some((
                        false,
                        "Server process supervisor mutex was poisoned while monitoring.".into(),
                    ));
                    break;
                }
            };

            match observed {
                Ok(Some(ManagedProcessState::Running(_))) => {}
                Ok(Some(ManagedProcessState::Exited { evidence, .. })) => {
                    let mut current = state.write();
                    current.process_owned = false;
                    current.operation = ServerOperation::Idle;
                    let was_stopping =
                        current.lifecycle.snapshot().phase == ServerLifecyclePhase::Stopping;
                    current.lifecycle.mark_exit(evidence.clone());
                    if !was_stopping {
                        current.notice = Some((
                            false,
                            format!(
                                "Managed llama-server exited unexpectedly: {evidence:?}. Retained logs remain available below."
                            ),
                        ));
                    }
                    break;
                }
                Ok(None) => break,
                Err(error) => {
                    let mut current = state.write();
                    current.operation = ServerOperation::Idle;
                    current.lifecycle.mark_failed(error.to_string());
                    current.notice = Some((false, error.to_string()));
                    break;
                }
            }
        }
    });
}

fn run_graceful_stop(mut state: ServerStateSignal, restart: bool) {
    let snapshot = state.read().clone();
    if !snapshot.process_owned || snapshot.operation == ServerOperation::Stopping {
        return;
    }
    snapshot.cancellation.store(true, Ordering::SeqCst);
    {
        let mut current = state.write();
        current.operation = ServerOperation::Stopping;
        current.force_confirm = false;
        current.lifecycle.mark_stopping();
        current.notice = Some((
            true,
            "Requesting llama-server's Windows Ctrl+C graceful shutdown; bounded wait is 5 seconds."
                .into(),
        ));
    }

    let outcome = {
        let mut supervisor = match snapshot.supervisor.lock() {
            Ok(supervisor) => supervisor,
            Err(_) => {
                stop_failed(
                    state,
                    "Server process supervisor mutex was poisoned; force-kill may be required."
                        .into(),
                );
                return;
            }
        };
        let process = match supervisor.process_mut() {
            Ok(process) => process,
            Err(error) => {
                stop_failed(state, error.to_string());
                return;
            }
        };
        process.wait_for_cooperative_exit(Duration::from_secs(5), |identity| {
            request_graceful_console_interrupt(identity.pid).map_err(|error| error.to_string())
        })
    };

    match outcome {
        Ok(GracefulStopOutcome::Exited(evidence)) => {
            if let Ok(mut supervisor) = snapshot.supervisor.lock() {
                let _ = supervisor.clear_exited();
            }
            let final_logs = snapshot
                .active_logs
                .as_ref()
                .map(ServerLogCapture::snapshot);
            {
                let mut current = state.write();
                current.process_owned = false;
                current.operation = ServerOperation::Idle;
                current.force_confirm = false;
                current.lifecycle.mark_exit(evidence.clone());
                if let Some(logs) = final_logs {
                    current.logs = Some(logs);
                }
                current.notice = Some((
                    true,
                    format!("Managed server stopped through graceful Ctrl+C: {evidence:?}"),
                ));
            }
            if restart {
                state.write().cancellation.store(false, Ordering::SeqCst);
                run_start(state);
            }
        }
        Ok(GracefulStopOutcome::GracePeriodExpired { waited }) => {
            stop_failed(
                state,
                format!(
                    "Graceful stop timed out after {:.1}s. The process is still owned and was NOT reported stopped. Confirm FORCE KILL if you want to terminate it.",
                    waited.as_secs_f64()
                ),
            );
        }
        Err(error) => {
            stop_failed(
                state,
                format!(
                    "Graceful stop request failed without claiming success: {error}. Confirm FORCE KILL if needed."
                ),
            );
        }
    }
}

fn stop_failed(mut state: ServerStateSignal, message: String) {
    let mut current = state.write();
    current.operation = ServerOperation::Idle;
    current.force_confirm = true;
    if current.readiness.is_some() {
        current.lifecycle.mark_ready();
    } else {
        current.lifecycle.mark_failed(message.clone());
    }
    current.notice = Some((false, message));
}

fn run_force_kill(mut state: ServerStateSignal) {
    let snapshot = state.read().clone();
    if !snapshot.process_owned {
        return;
    }
    snapshot.cancellation.store(true, Ordering::SeqCst);
    state.write().operation = ServerOperation::Stopping;
    state.write().lifecycle.mark_stopping();

    let result = match snapshot.supervisor.lock() {
        Ok(mut supervisor) => supervisor
            .process_mut()
            .and_then(|process| process.force_kill()),
        Err(_) => {
            stop_failed(
                state,
                "Server process supervisor mutex was poisoned.".into(),
            );
            return;
        }
    };

    let mut current = state.write();
    current.operation = ServerOperation::Idle;
    current.force_confirm = false;
    match result {
        Ok(evidence) => {
            if let Ok(mut supervisor) = snapshot.supervisor.lock() {
                let _ = supervisor.clear_exited();
            }
            current.process_owned = false;
            current.lifecycle.mark_exit(evidence.clone());
            current.logs = snapshot
                .active_logs
                .as_ref()
                .map(ServerLogCapture::snapshot);
            current.notice = Some((
                true,
                format!("Managed process tree FORCE KILLED explicitly: {evidence:?}"),
            ));
        }
        Err(error) => current.notice = Some((false, format!("Force kill failed: {error}"))),
    }
}

fn export_logs(state: &ServerUiState) -> Result<PathBuf, String> {
    let logs = state
        .logs
        .as_ref()
        .ok_or_else(|| "There are no retained server logs to export.".to_string())?;
    let default_dir = state
        .paths
        .as_ref()
        .map(|paths| paths.exports.clone())
        .unwrap_or_else(|| PathBuf::from("."));
    let Some(path) = FileDialog::new()
        .set_title("Export redacted managed server log")
        .set_directory(default_dir)
        .set_file_name("managed-server-redacted.log")
        .save_file()
    else {
        return Err("Export cancelled.".into());
    };
    let secrets: Vec<String> = (!state.api_key.trim().is_empty())
        .then_some(state.api_key.trim().to_string())
        .into_iter()
        .collect();
    logs.export_redacted(&path, &secrets)
        .map_err(|error| error.to_string())?;
    Ok(path)
}

fn phase_class(phase: ServerLifecyclePhase) -> &'static str {
    match phase {
        ServerLifecyclePhase::Ready | ServerLifecyclePhase::Stopped => "sv-badge",
        ServerLifecyclePhase::Starting
        | ServerLifecyclePhase::Stopping
        | ServerLifecyclePhase::Unknown => "sv-badge warn",
        ServerLifecyclePhase::Failed | ServerLifecyclePhase::Crashed => "sv-badge error",
    }
}

fn severity_class(severity: ServerLogSeverity, _stream: ServerLogStream) -> &'static str {
    match severity {
        ServerLogSeverity::Fatal => "sv-log fatal stderr",
        ServerLogSeverity::Error => "sv-log error stderr",
        ServerLogSeverity::Warning => "sv-log warning",
        ServerLogSeverity::Info => "sv-log",
    }
}

#[allow(non_snake_case)]
pub fn ServerLifecycleView() -> Element {
    let mut state = use_signal_sync(ServerUiState::load);
    let snapshot = state.read().clone();
    let phase = snapshot.lifecycle.snapshot().phase;
    let preview = prepare_launch(&snapshot).map(|prepared| prepared.spec.diagnostic_command());
    let can_start =
        !snapshot.process_owned && snapshot.operation == ServerOperation::Idle && preview.is_ok();
    let can_stop = snapshot.process_owned && snapshot.operation != ServerOperation::Stopping;
    let can_restart = snapshot.process_owned && snapshot.operation == ServerOperation::Idle;

    let mut visible_logs = snapshot
        .logs
        .as_ref()
        .map(|logs| {
            logs.entries
                .iter()
                .rev()
                .take(180)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    visible_logs.reverse();
    let log_summary = snapshot
        .logs
        .as_ref()
        .map(|logs| {
            format!(
                "{} entries shown · {} bytes retained · {} evicted",
                visible_logs.len(),
                logs.retained_bytes,
                logs.evicted_entries
            )
        })
        .unwrap_or_else(|| "No managed log capture yet.".into());

    rsx! {
        style { dangerous_inner_html: SERVER_UI_CSS }
        main { class: "sv-page",
            header { class: "sv-header",
                div {
                    div { class: "sv-kicker", "> LLAMAWAVE / SERVER LAB" }
                    h1 { "MANAGED LLAMA-SERVER" }
                    p { "Capability-backed launch, real health + inference readiness, bounded logs, explicit graceful/force stop semantics, and retained failure evidence." }
                }
                div { class: "sv-phase",
                    div { class: "sv-kicker", "LIFECYCLE" }
                    strong { "{phase:?}" }
                    span { class: phase_class(phase), "{snapshot.operation.label()}" }
                }
            }

            if let Some((success, message)) = snapshot.notice.as_ref() {
                div { class: if *success { "sv-notice" } else { "sv-notice error" }, "{message}" }
            }

            div { class: "sv-grid",
                section { class: "sv-panel",
                    div { class: "sv-panel-head",
                        div {
                            div { class: "sv-kicker", "LAUNCH / PRE-FLIGHT" }
                            h2 { "EXACT MANAGED COMMAND" }
                        }
                        button {
                            class: "sv-button",
                            disabled: snapshot.operation != ServerOperation::Idle,
                            onclick: move |_| refresh_sources(state),
                            "REFRESH SOURCES"
                        }
                    }
                    div { class: "sv-panel-body",
                        div { class: "sv-fields",
                            div { class: "sv-field",
                                label { "HOST" }
                                input {
                                    class: "sv-input",
                                    value: "{snapshot.host}",
                                    disabled: snapshot.process_owned,
                                    oninput: move |event| state.write().host = event.value(),
                                }
                            }
                            div { class: "sv-field",
                                label { "PORT" }
                                input {
                                    class: "sv-input",
                                    value: "{snapshot.port}",
                                    disabled: snapshot.process_owned,
                                    oninput: move |event| state.write().port = event.value(),
                                }
                            }
                            div { class: "sv-field wide",
                                label { "API KEY · OPTIONAL · DIAGNOSTIC COMMAND IS REDACTED" }
                                input {
                                    class: "sv-input",
                                    r#type: "password",
                                    value: "{snapshot.api_key}",
                                    disabled: snapshot.process_owned,
                                    oninput: move |event| state.write().api_key = event.value(),
                                }
                            }
                        }

                        div { class: "sv-actions",
                            button {
                                class: if snapshot.allow_non_loopback { "sv-button magenta" } else { "sv-button" },
                                disabled: snapshot.process_owned,
                                onclick: move |_| {
                                    let enabled = state.read().allow_non_loopback;
                                    state.write().allow_non_loopback = !enabled;
                                },
                                if snapshot.allow_non_loopback { "LAN OPT-IN ON" } else { "LAN OPT-IN OFF" }
                            }
                        }

                        div { class: "sv-path",
                            strong { "RUNTIME\n" }
                            if let Some(installation) = snapshot.installation.as_ref() {
                                if let Some(server) = installation.server.as_ref() {
                                    "{server.path.display()}"
                                } else {
                                    "selected runtime has no llama-server"
                                }
                            } else {
                                "no persisted runtime selected"
                            }
                            "\n\nMODEL\n"
                            if let Some(model) = snapshot.model.as_ref() {
                                "{model.path.display()}"
                            } else {
                                "no persisted model selected"
                            }
                        }

                        div { class: "sv-command",
                            strong { "PREVIEW\n" }
                            match preview.as_ref() {
                                Ok(command) => command.clone(),
                                Err(error) => format!("BLOCKED: {error}"),
                            }
                        }
                        if let Some(last) = snapshot.last_command.as_ref() {
                            div { class: "sv-detail", strong { "LAST LAUNCHED\n" } "{last}" }
                        }
                        if let Some(warning) = snapshot.console_warning.as_ref() {
                            div { class: "sv-warning", "{warning}" }
                        }

                        div { class: "sv-actions",
                            button {
                                class: "sv-button primary",
                                disabled: !can_start,
                                onclick: move |_| {
                                    let worker_state = state;
                                    thread::spawn(move || run_start(worker_state));
                                },
                                "START SERVER"
                            }
                            button {
                                class: "sv-button",
                                disabled: !can_stop,
                                onclick: move |_| {
                                    let worker_state = state;
                                    thread::spawn(move || run_graceful_stop(worker_state, false));
                                },
                                if phase == ServerLifecyclePhase::Starting { "CANCEL + STOP" } else { "STOP GRACEFULLY" }
                            }
                            button {
                                class: "sv-button magenta",
                                disabled: !can_restart,
                                onclick: move |_| {
                                    let worker_state = state;
                                    thread::spawn(move || run_graceful_stop(worker_state, true));
                                },
                                "RESTART"
                            }
                            button {
                                class: if snapshot.force_confirm { "sv-button danger confirm" } else { "sv-button danger" },
                                disabled: !snapshot.process_owned,
                                onclick: move |_| {
                                    if !state.read().force_confirm {
                                        let mut current = state.write();
                                        current.force_confirm = true;
                                        current.notice = Some((
                                            false,
                                            "FORCE KILL terminates the Windows Job Object process tree without graceful model/server cleanup. Click CONFIRM FORCE KILL to proceed."
                                                .into(),
                                        ));
                                    } else {
                                        let worker_state = state;
                                        thread::spawn(move || run_force_kill(worker_state));
                                    }
                                },
                                if snapshot.force_confirm { "CONFIRM FORCE KILL" } else { "FORCE KILL" }
                            }
                        }
                    }
                }

                aside { class: "sv-panel",
                    div { class: "sv-panel-head",
                        div {
                            div { class: "sv-kicker", "TRUTHFUL STATE" }
                            h2 { "READINESS + EVIDENCE" }
                        }
                    }
                    div { class: "sv-panel-body",
                        div { class: "sv-status-grid",
                            div { class: "sv-stat", span { "PHASE" } strong { "{phase:?}" } }
                            div { class: "sv-stat", span { "OWNERSHIP" } strong { if snapshot.process_owned { "MANAGED" } else { "NONE / UNKNOWN" } } }
                            div { class: "sv-stat", span { "ENDPOINT" } strong { "{snapshot.host}:{snapshot.port}" } }
                            div { class: "sv-stat", span { "LOG" } strong { if snapshot.log_path.is_some() { "RETAINED" } else { "NOT STARTED" } } }
                        }
                        div { class: "sv-readiness", style: "margin-top:10px",
                            if let Some(readiness) = snapshot.readiness.as_ref() {
                                div { class: "sv-evidence",
                                    strong { "REAL READINESS PASS" }
                                    div { "endpoint: {readiness.endpoint}" }
                                    div { {format!("attempts: {} · elapsed: {:.3}s", readiness.attempts, readiness.elapsed.as_secs_f64())} }
                                    div { "minimal inference: HTTP {readiness.inference.status_code} {readiness.inference.path}" }
                                    div { "authenticated: {readiness.authenticated}" }
                                }
                            } else if phase == ServerLifecyclePhase::Starting {
                                div { class: "sv-evidence", "Bounded health polling + real /completion inference is in progress. STOP cancels readiness before requesting graceful shutdown." }
                            } else {
                                div { class: "sv-evidence", "No completed readiness evidence for the current process." }
                            }
                            if let Some(exit) = snapshot.lifecycle.snapshot().last_exit.as_ref() {
                                div { class: "sv-evidence",
                                    strong { "LAST EXIT" }
                                    div { {format!("code={:?} · kind={:?} · observed={}", exit.code, exit.kind, exit.observed_at_unix_ms)} }
                                }
                            }
                            if let Some(detail) = snapshot.lifecycle.snapshot().detail.as_ref() {
                                div { class: "sv-evidence", "{detail}" }
                            }
                        }
                    }
                }
            }

            section { class: "sv-panel sv-log-panel",
                div { class: "sv-panel-head",
                    div {
                        div { class: "sv-kicker", "STDOUT + STDERR" }
                        h2 { "RETAINED SERVER LOG" }
                    }
                    button {
                        class: "sv-button",
                        disabled: snapshot.logs.is_none(),
                        onclick: move |_| {
                            let current = state.read().clone();
                            match export_logs(&current) {
                                Ok(path) => {
                                    state.write().notice = Some((
                                        true,
                                        format!("Exported redacted logs to {}", path.display()),
                                    ));
                                }
                                Err(error) if error == "Export cancelled." => {}
                                Err(error) => state.write().notice = Some((false, error)),
                            }
                        },
                        "EXPORT REDACTED LOG"
                    }
                }
                div { class: "sv-panel-body",
                    div { class: "sv-log-toolbar",
                        div { class: "sv-muted", "{log_summary}" }
                        if let Some(path) = snapshot.log_path.as_ref() {
                            div { class: "sv-muted", "{path.display()}" }
                        }
                    }
                    if let Some(logs) = snapshot.logs.as_ref() {
                        if let Some(error) = logs.disk_error.as_ref() {
                            div { class: "sv-disk-error", "DISK CAPTURE ERROR: {error}" }
                        }
                    }
                    div { class: "sv-log-list",
                        if visible_logs.is_empty() {
                            div { class: "sv-empty", "Start a managed server to stream bounded stdout/stderr here." }
                        } else {
                            for entry in visible_logs {
                                div { class: severity_class(entry.presentation_severity(), entry.stream),
                                    span { class: "seq", "#{entry.sequence}" }
                                    span {
                                        class: "stream",
                                        match entry.presentation_severity() {
                                            ServerLogSeverity::Info => "INFO",
                                            ServerLogSeverity::Warning => "WARN",
                                            ServerLogSeverity::Error => "ERR",
                                            ServerLogSeverity::Fatal => "FATAL",
                                        }
                                    }
                                    span { class: "text", "{entry.text}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
