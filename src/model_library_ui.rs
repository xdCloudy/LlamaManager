use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use dioxus::prelude::*;
use rfd::FileDialog;

use crate::{
    compatibility::{CompatibilityResult, evaluate_compatibility},
    llama::LlamaInstallation,
    model_library::{
        ScanProgress, ScanReport, manual_add_model, manual_add_projector, relink_model,
        relink_projector, scan_root,
    },
    model_library_actions::{remove_model_from_library, remove_projector_from_library},
    model_store::{LocationState, ModelRecord, ModelStore, StoredProjector},
    paths::AppPaths,
    persistence::Database,
};

const MODEL_LIBRARY_CSS: &str = include_str!("../assets/model_library.css");

type UiResult<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LibraryActivity {
    Idle,
    Scanning,
    AddingModel,
    AddingProjector,
    Relinking,
    Associating,
    Refreshing,
}

impl LibraryActivity {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "READY",
            Self::Scanning => "SCANNING",
            Self::AddingModel => "READING MODEL",
            Self::AddingProjector => "READING PROJECTOR",
            Self::Relinking => "RELINKING",
            Self::Associating => "ASSOCIATING",
            Self::Refreshing => "REFRESHING",
        }
    }

    fn is_busy(self) -> bool {
        self != Self::Idle
    }
}

#[derive(Debug, Clone)]
struct ProjectorChoice {
    id: String,
    name: String,
    path: PathBuf,
    status: String,
    reasons: String,
}

#[derive(Debug, Clone)]
struct ModelItem {
    record: ModelRecord,
    compatibility: Option<CompatibilityResult>,
    compatibility_stale: bool,
    associated_projector_id: Option<String>,
    projector_choices: Vec<ProjectorChoice>,
}

#[derive(Debug, Clone)]
struct LibrarySnapshot {
    runtime: Option<LlamaInstallation>,
    models: Vec<ModelItem>,
    projectors: Vec<StoredProjector>,
    scan_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct LibraryUiState {
    db_path: Option<PathBuf>,
    runtime: Option<LlamaInstallation>,
    models: Vec<ModelItem>,
    projectors: Vec<StoredProjector>,
    scan_roots: Vec<PathBuf>,
    activity: LibraryActivity,
    scan_progress: Option<ScanProgress>,
    last_scan: Option<ScanReport>,
    cancel_token: Option<Arc<AtomicBool>>,
    notice: Option<(bool, String)>,
}

impl LibraryUiState {
    fn initialize() -> Self {
        let mut state = Self {
            db_path: None,
            runtime: None,
            models: Vec::new(),
            projectors: Vec::new(),
            scan_roots: Vec::new(),
            activity: LibraryActivity::Idle,
            scan_progress: None,
            last_scan: None,
            cancel_token: None,
            notice: None,
        };

        match AppPaths::detect() {
            Ok(paths) => {
                state.db_path = Some(paths.database.clone());
                match load_snapshot(&paths.database) {
                    Ok(snapshot) => state.apply(snapshot),
                    Err(error) => state.notice = Some((false, error)),
                }
            }
            Err(error) => state.notice = Some((false, error.to_string())),
        }

        state
    }

    fn apply(&mut self, snapshot: LibrarySnapshot) {
        self.runtime = snapshot.runtime;
        self.models = snapshot.models;
        self.projectors = snapshot.projectors;
        self.scan_roots = snapshot.scan_roots;
    }
}

fn load_snapshot(db_path: &Path) -> UiResult<LibrarySnapshot> {
    let store = ModelStore::open(db_path.to_path_buf()).map_err(|error| error.to_string())?;
    let database = Database::open(db_path.to_path_buf()).map_err(|error| error.to_string())?;
    let runtime = database
        .latest_installation()
        .map_err(|error| error.to_string())?;

    let mut models = Vec::new();
    for record in store
        .list_model_records()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|record| !record.locations.is_empty())
    {
        let associated = store
            .associated_projector(&record.model.id)
            .map_err(|error| error.to_string())?;

        let (compatibility, compatibility_stale) = if let Some(installation) = runtime.as_ref() {
            let saved = store
                .load_compatibility(&record.model.id, &installation.id)
                .map_err(|error| error.to_string())?;
            let stale = saved
                .as_ref()
                .map(|result| result.is_stale(&record.model, installation))
                .unwrap_or(false);
            (saved, stale)
        } else {
            (None, false)
        };

        let projector_choices = store
            .projector_candidates(&record.model)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|candidate| ProjectorChoice {
                id: candidate.projector.id.clone(),
                name: candidate
                    .projector
                    .name
                    .clone()
                    .unwrap_or_else(|| candidate.projector.id.clone()),
                path: candidate.projector.path,
                status: format!("{:?}", candidate.pairing.status).to_ascii_uppercase(),
                reasons: candidate.pairing.reasons.join("; "),
            })
            .collect();

        models.push(ModelItem {
            record,
            compatibility,
            compatibility_stale,
            associated_projector_id: associated.map(|projector| projector.id),
            projector_choices,
        });
    }

    Ok(LibrarySnapshot {
        runtime,
        models,
        projectors: store.list_projectors().map_err(|error| error.to_string())?,
        scan_roots: store.list_scan_roots().map_err(|error| error.to_string())?,
    })
}

fn recompute_compatibility(db_path: &Path) -> UiResult<usize> {
    let store = ModelStore::open(db_path.to_path_buf()).map_err(|error| error.to_string())?;
    let database = Database::open(db_path.to_path_buf()).map_err(|error| error.to_string())?;
    let Some(installation) = database
        .latest_installation()
        .map_err(|error| error.to_string())?
    else {
        return Err(
            "No llama.cpp runtime is selected. Select one in the benchmark workspace first.".into(),
        );
    };

    let records = store
        .list_model_records()
        .map_err(|error| error.to_string())?;
    let count = records
        .iter()
        .filter(|record| !record.locations.is_empty())
        .count();
    for record in records
        .into_iter()
        .filter(|record| !record.locations.is_empty())
    {
        let projector = store
            .associated_projector(&record.model.id)
            .map_err(|error| error.to_string())?;
        let result = evaluate_compatibility(&record.model, &installation, projector.as_ref());
        store
            .save_compatibility(&result)
            .map_err(|error| error.to_string())?;
    }
    Ok(count)
}

fn recompute_if_runtime(db_path: &Path) -> UiResult<usize> {
    let database = Database::open(db_path.to_path_buf()).map_err(|error| error.to_string())?;
    if database
        .latest_installation()
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Ok(0);
    }
    recompute_compatibility(db_path)
}

fn finish_operation(
    mut state: Signal<LibraryUiState>,
    result: UiResult<String>,
    last_scan: Option<ScanReport>,
) {
    let db_path = state.read().db_path.clone();
    let snapshot = db_path
        .as_deref()
        .ok_or_else(|| "Application database path is unavailable.".to_string())
        .and_then(load_snapshot);

    let mut current = state.write();
    current.activity = LibraryActivity::Idle;
    current.cancel_token = None;
    current.scan_progress = None;
    if let Some(report) = last_scan {
        current.last_scan = Some(report);
    }

    match (result, snapshot) {
        (Ok(message), Ok(snapshot)) => {
            current.apply(snapshot);
            current.notice = Some((true, message));
        }
        (Err(error), Ok(snapshot)) => {
            current.apply(snapshot);
            current.notice = Some((false, error));
        }
        (Ok(_), Err(error)) | (Err(_), Err(error)) => {
            current.notice = Some((false, error));
        }
    }
}

fn begin_scan(mut state: Signal<LibraryUiState>) {
    if state.read().activity.is_busy() {
        return;
    }
    let Some(root) = FileDialog::new()
        .set_title("Scan a folder for GGUF models and projectors")
        .pick_folder()
    else {
        return;
    };
    let Some(db_path) = state.read().db_path.clone() else {
        state.write().notice = Some((false, "Application database path is unavailable.".into()));
        return;
    };

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut current = state.write();
        current.activity = LibraryActivity::Scanning;
        current.scan_progress = Some(ScanProgress::default());
        current.cancel_token = Some(cancel.clone());
        current.notice = None;
    }

    let mut worker_state = state;
    thread::spawn(move || {
        let store = match ModelStore::open(db_path.clone()) {
            Ok(store) => store,
            Err(error) => {
                finish_operation(worker_state, Err(error.to_string()), None);
                return;
            }
        };

        let report = scan_root(&store, &root, &cancel, |progress| {
            worker_state.write().scan_progress = Some(progress.clone());
        });

        match report {
            Ok(report) => {
                let was_cancelled = report.progress.cancelled;
                let summary = report.summary_line();
                let result = recompute_if_runtime(&db_path).map(|_| {
                    if was_cancelled {
                        format!("Scan cancelled safely. {summary}")
                    } else {
                        format!("Scan complete. {summary}")
                    }
                });
                finish_operation(worker_state, result, Some(report));
            }
            Err(error) => finish_operation(worker_state, Err(error.to_string()), None),
        }
    });
}

fn cancel_scan(state: Signal<LibraryUiState>) {
    if let Some(token) = state.read().cancel_token.clone() {
        token.store(true, Ordering::SeqCst);
    }
}

fn begin_add_model(mut state: Signal<LibraryUiState>) {
    if state.read().activity.is_busy() {
        return;
    }
    let Some(path) = FileDialog::new()
        .set_title("Add a GGUF model")
        .add_filter("GGUF model", &["gguf"])
        .pick_file()
    else {
        return;
    };
    let Some(db_path) = state.read().db_path.clone() else {
        return;
    };
    state.write().activity = LibraryActivity::AddingModel;
    state.write().notice = None;

    thread::spawn(move || {
        let result = ModelStore::open(db_path.clone())
            .map_err(|error| error.to_string())
            .and_then(|store| {
                manual_add_model(&store, &path)
                    .map_err(|error| error.to_string())
                    .map(|model| model.name.unwrap_or_else(|| model.id))
            })
            .and_then(|name| {
                recompute_if_runtime(&db_path)?;
                Ok(format!("Model added to the library: {name}"))
            });
        finish_operation(state, result, None);
    });
}

fn begin_add_projector(mut state: Signal<LibraryUiState>) {
    if state.read().activity.is_busy() {
        return;
    }
    let Some(path) = FileDialog::new()
        .set_title("Add a multimodal projector GGUF")
        .add_filter("GGUF projector", &["gguf"])
        .pick_file()
    else {
        return;
    };
    let Some(db_path) = state.read().db_path.clone() else {
        return;
    };
    state.write().activity = LibraryActivity::AddingProjector;
    state.write().notice = None;

    thread::spawn(move || {
        let result = ModelStore::open(db_path.clone())
            .map_err(|error| error.to_string())
            .and_then(|store| {
                manual_add_projector(&store, &path)
                    .map_err(|error| error.to_string())
                    .map(|projector| projector.name.unwrap_or_else(|| projector.id))
            })
            .and_then(|name| {
                recompute_if_runtime(&db_path)?;
                Ok(format!("Projector added to the library: {name}"))
            });
        finish_operation(state, result, None);
    });
}

fn begin_relink_model(mut state: Signal<LibraryUiState>, model_id: String) {
    if state.read().activity.is_busy() {
        return;
    }
    let Some(path) = FileDialog::new()
        .set_title("Relink missing model to matching GGUF")
        .add_filter("GGUF model", &["gguf"])
        .pick_file()
    else {
        return;
    };
    let Some(db_path) = state.read().db_path.clone() else {
        return;
    };
    state.write().activity = LibraryActivity::Relinking;
    state.write().notice = None;

    thread::spawn(move || {
        let result = ModelStore::open(db_path.clone())
            .map_err(|error| error.to_string())
            .and_then(|store| {
                relink_model(&store, &model_id, &path).map_err(|error| error.to_string())
            })
            .and_then(|_| {
                recompute_if_runtime(&db_path)?;
                Ok("Model relinked by matching content identity.".into())
            });
        finish_operation(state, result, None);
    });
}

fn begin_relink_projector(mut state: Signal<LibraryUiState>, projector_id: String) {
    if state.read().activity.is_busy() {
        return;
    }
    let Some(path) = FileDialog::new()
        .set_title("Relink missing projector to matching GGUF")
        .add_filter("GGUF projector", &["gguf"])
        .pick_file()
    else {
        return;
    };
    let Some(db_path) = state.read().db_path.clone() else {
        return;
    };
    state.write().activity = LibraryActivity::Relinking;
    state.write().notice = None;

    thread::spawn(move || {
        let result = ModelStore::open(db_path.clone())
            .map_err(|error| error.to_string())
            .and_then(|store| {
                relink_projector(&store, &projector_id, &path).map_err(|error| error.to_string())
            })
            .and_then(|_| {
                recompute_if_runtime(&db_path)?;
                Ok("Projector relinked by matching content identity.".into())
            });
        finish_operation(state, result, None);
    });
}

fn begin_associate(mut state: Signal<LibraryUiState>, model_id: String, projector_id: String) {
    if state.read().activity.is_busy() {
        return;
    }
    let Some(db_path) = state.read().db_path.clone() else {
        return;
    };
    state.write().activity = LibraryActivity::Associating;
    state.write().notice = None;

    thread::spawn(move || {
        let result = ModelStore::open(db_path.clone())
            .map_err(|error| error.to_string())
            .and_then(|store| {
                let model = store
                    .get_model(&model_id)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("Model {model_id} is not persisted."))?;
                store
                    .associate_projector(&model, &projector_id)
                    .map_err(|error| error.to_string())?;
                Ok(())
            })
            .and_then(|_| {
                recompute_if_runtime(&db_path)?;
                Ok("Projector association saved and compatibility recomputed.".into())
            });
        finish_operation(state, result, None);
    });
}

fn begin_remove_model(mut state: Signal<LibraryUiState>, model_id: String) {
    if state.read().activity.is_busy() {
        return;
    }
    let Some(db_path) = state.read().db_path.clone() else {
        return;
    };
    state.write().activity = LibraryActivity::Refreshing;
    state.write().notice = None;

    thread::spawn(move || {
        let result = ModelStore::open(db_path)
            .map_err(|error| error.to_string())
            .and_then(|store| {
                remove_model_from_library(&store, &model_id).map_err(|error| error.to_string())
            })
            .map(|_| {
                "Model removed from the library. The GGUF file and benchmark history were not deleted."
                    .into()
            });
        finish_operation(state, result, None);
    });
}

fn begin_remove_projector(mut state: Signal<LibraryUiState>, projector_id: String) {
    if state.read().activity.is_busy() {
        return;
    }
    let Some(db_path) = state.read().db_path.clone() else {
        return;
    };
    state.write().activity = LibraryActivity::Refreshing;
    state.write().notice = None;

    thread::spawn(move || {
        let result = ModelStore::open(db_path)
            .map_err(|error| error.to_string())
            .and_then(|store| {
                remove_projector_from_library(&store, &projector_id)
                    .map_err(|error| error.to_string())
            })
            .map(|_| "Projector removed from the library; the source file was not deleted.".into());
        finish_operation(state, result, None);
    });
}

fn begin_refresh(mut state: Signal<LibraryUiState>) {
    if state.read().activity.is_busy() {
        return;
    }
    let Some(db_path) = state.read().db_path.clone() else {
        return;
    };
    state.write().activity = LibraryActivity::Refreshing;
    state.write().notice = None;

    thread::spawn(move || {
        let result = ModelStore::open(db_path.clone())
            .map_err(|error| error.to_string())
            .and_then(|store| {
                store
                    .refresh_location_existence()
                    .map_err(|error| error.to_string())
            })
            .and_then(|_| {
                recompute_if_runtime(&db_path)?;
                Ok("Path state refreshed and available compatibility evidence recomputed.".into())
            });
        finish_operation(state, result, None);
    });
}

fn begin_recheck(mut state: Signal<LibraryUiState>) {
    if state.read().activity.is_busy() {
        return;
    }
    let Some(db_path) = state.read().db_path.clone() else {
        return;
    };
    state.write().activity = LibraryActivity::Refreshing;
    state.write().notice = None;

    thread::spawn(move || {
        let result = recompute_compatibility(&db_path).map(|count| {
            format!("Compatibility recomputed from current runtime evidence for {count} model(s).")
        });
        finish_operation(state, result, None);
    });
}

fn model_location_state(record: &ModelRecord) -> (&'static str, &'static str) {
    if record
        .locations
        .iter()
        .any(|location| location.state == LocationState::Present)
    {
        ("PRESENT", "present")
    } else if record
        .locations
        .iter()
        .any(|location| location.state == LocationState::Unreadable)
    {
        ("UNREADABLE", "unreadable")
    } else {
        ("MISSING", "missing")
    }
}

fn compatibility_label(item: &ModelItem) -> (String, String) {
    let Some(result) = item.compatibility.as_ref() else {
        return ("NOT CHECKED".into(), "unknown".into());
    };
    let status = result.status.as_str().to_ascii_uppercase();
    if item.compatibility_stale {
        (format!("{status} / STALE"), "stale".into())
    } else {
        (status, result.status.as_str().into())
    }
}

fn short_sha(value: &str) -> String {
    value.chars().take(12).collect()
}

#[allow(non_snake_case)]
pub fn ModelLibraryView() -> Element {
    let state = use_signal_sync(LibraryUiState::initialize);
    let snapshot = state.read().clone();
    let busy = snapshot.activity.is_busy();
    let runtime_label = snapshot
        .runtime
        .as_ref()
        .map(|runtime| runtime.name.clone())
        .unwrap_or_else(|| "NO RUNTIME SELECTED".into());

    rsx! {
        style { dangerous_inner_html: MODEL_LIBRARY_CSS }
        div { class: "ml-crt" }
        div { class: "ml-shell",
            header { class: "ml-topbar",
                div {
                    div { class: "ml-kicker", "> LLAMAWAVE / MODEL LIBRARY" }
                    h1 { "MODEL LIBRARY" }
                    p {
                        "Content-derived model identity, repairable paths, evidence-backed compatibility, and multimodal projector association."
                    }
                }
                div { class: "ml-topbar-state",
                    span { class: "ml-runtime-label", "RUNTIME" }
                    strong { "{runtime_label}" }
                    span {
                        class: if busy { "ml-activity busy" } else { "ml-activity" },
                        "{snapshot.activity.label()}"
                    }
                }
            }

            main { class: "ml-main",
                if let Some((success, message)) = snapshot.notice.as_ref() {
                    div {
                        class: if *success { "ml-notice ok" } else { "ml-notice error" },
                        strong { if *success { "OK" } else { "ERR" } }
                        span { "{message}" }
                    }
                }

                section { class: "ml-command-panel",
                    div { class: "ml-command-copy",
                        div { class: "ml-eyebrow", "> LIBRARY CONTROL" }
                        h2 { "DISCOVER. VERIFY. REPAIR." }
                        p {
                            "Scans never follow reparse-point directory trees. Source files are never deleted by library removal."
                        }
                    }
                    div { class: "ml-actions",
                        button {
                            class: "ml-button primary",
                            disabled: busy,
                            onclick: move |_| begin_scan(state),
                            "SCAN FOLDER"
                        }
                        button {
                            class: "ml-button",
                            disabled: busy,
                            onclick: move |_| begin_add_model(state),
                            "ADD MODEL"
                        }
                        button {
                            class: "ml-button",
                            disabled: busy,
                            onclick: move |_| begin_add_projector(state),
                            "ADD PROJECTOR"
                        }
                        button {
                            class: "ml-button",
                            disabled: busy,
                            onclick: move |_| begin_refresh(state),
                            "REFRESH PATHS"
                        }
                        button {
                            class: "ml-button",
                            disabled: busy || snapshot.runtime.is_none(),
                            onclick: move |_| begin_recheck(state),
                            "RECHECK COMPATIBILITY"
                        }
                    }
                }

                if snapshot.activity == LibraryActivity::Scanning {
                    if let Some(progress) = snapshot.scan_progress.as_ref() {
                        section { class: "ml-scan-progress",
                            div {
                                strong { "SCAN ACTIVE" }
                                span {
                                    "{progress.gguf_candidates} GGUF / {progress.models_saved} models / {progress.projectors_saved} projectors / {progress.errors} errors"
                                }
                            }
                            if let Some(path) = progress.current_path.as_ref() {
                                code { "{path.display()}" }
                            }
                            button {
                                class: "ml-button danger",
                                onclick: move |_| cancel_scan(state),
                                "CANCEL SAFELY"
                            }
                        }
                    }
                }

                section { class: "ml-stats",
                    article {
                        span { "MODELS" }
                        strong { "{snapshot.models.len()}" }
                        small { "content identities" }
                    }
                    article {
                        span { "PROJECTORS" }
                        strong { "{snapshot.projectors.len()}" }
                        small { "mmproj evidence" }
                    }
                    article {
                        span { "SCAN ROOTS" }
                        strong { "{snapshot.scan_roots.len()}" }
                        small { "user selected" }
                    }
                    article {
                        span { "RUNTIME EVIDENCE" }
                        strong { if snapshot.runtime.is_some() { "READY" } else { "MISSING" } }
                        small { "compatibility authority" }
                    }
                }

                section { class: "ml-section",
                    div { class: "ml-section-heading",
                        div {
                            div { class: "ml-eyebrow", "> MODELS / GGUF" }
                            h2 { "LOCAL MODELS" }
                        }
                        span { "{snapshot.models.len()} RECORDS" }
                    }

                    if snapshot.models.is_empty() {
                        div { class: "ml-empty",
                            strong { "NO MODEL LIBRARY ENTRIES" }
                            p { "Scan a folder or add a GGUF manually. Benchmark-only model history is not fabricated into the library." }
                        }
                    } else {
                        div { class: "ml-model-grid",
                            for item in snapshot.models.clone() {
                                {model_card(item, state, busy)}
                            }
                        }
                    }
                }

                section { class: "ml-section",
                    div { class: "ml-section-heading",
                        div {
                            div { class: "ml-eyebrow", "> MULTIMODAL / MMPROJ" }
                            h2 { "PROJECTORS" }
                        }
                        span { "{snapshot.projectors.len()} RECORDS" }
                    }
                    if snapshot.projectors.is_empty() {
                        div { class: "ml-empty compact",
                            strong { "NO PROJECTORS DISCOVERED" }
                            p { "This is valid for text-only models. Add or scan an mmproj GGUF when a model requires one." }
                        }
                    } else {
                        div { class: "ml-projector-list",
                            for projector in snapshot.projectors.clone() {
                                {projector_row(projector, state, busy)}
                            }
                        }
                    }
                }

                details { class: "ml-evidence",
                    summary { "SCAN ROOTS + LAST RUNTIME EVIDENCE" }
                    div { class: "ml-evidence-body",
                        div {
                            h3 { "SCAN ROOTS" }
                            if snapshot.scan_roots.is_empty() {
                                p { class: "ml-muted", "No scan roots persisted yet." }
                            } else {
                                for root in snapshot.scan_roots.iter() {
                                    code { class: "ml-block-code", "{root.display()}" }
                                }
                            }
                        }
                        div {
                            h3 { "LAST SCAN" }
                            if let Some(report) = snapshot.last_scan.as_ref() {
                                p { "{report.summary_line()}" }
                                if report.issues.is_empty() {
                                    p { class: "ml-muted", "No scan issues recorded." }
                                } else {
                                    for issue in report.issues.iter().take(12) {
                                        div { class: "ml-issue",
                                            strong { "{issue.kind:?}" }
                                            span { "{issue.message}" }
                                        }
                                    }
                                }
                            } else {
                                p { class: "ml-muted", "No scan has run in this UI session." }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn model_card(item: ModelItem, state: Signal<LibraryUiState>, busy: bool) -> Element {
    let (location_label, location_class) = model_location_state(&item.record);
    let (compat_label, compat_class) = compatibility_label(&item);
    let model = item.record.model.clone();
    let model_id = model.id.clone();
    let name = model.name.clone().unwrap_or_else(|| model.id.clone());
    let architecture = model
        .architecture
        .clone()
        .unwrap_or_else(|| "UNKNOWN".into());
    let path = item
        .record
        .present_paths()
        .first()
        .map(|path| path.to_path_buf())
        .or_else(|| {
            item.record
                .locations
                .first()
                .map(|location| location.path.clone())
        })
        .unwrap_or_else(|| model.path.clone());
    let reasons = item
        .compatibility
        .as_ref()
        .map(|result| result.reasons.clone())
        .unwrap_or_default();

    rsx! {
        article { class: "ml-model-card",
            div { class: "ml-card-head",
                div {
                    div { class: "ml-card-kicker", "{architecture}" }
                    h3 { "{name}" }
                }
                div { class: "ml-badges",
                    span { class: "ml-badge {location_class}", "{location_label}" }
                    span { class: "ml-badge {compat_class}", "{compat_label}" }
                }
            }

            div { class: "ml-model-meta",
                div {
                    span { "SHA-256" }
                    code { "{short_sha(&model.sha256)}…" }
                }
                div {
                    span { "GGUF" }
                    strong { "v{model.gguf_version}" }
                }
                div {
                    span { "CONTEXT" }
                    strong {
                        {model.context_length.map(|value| value.to_string()).unwrap_or_else(|| "UNKNOWN".into())}
                    }
                }
                div {
                    span { "FILE TYPE" }
                    strong {
                        {model.file_type.map(|value| value.to_string()).unwrap_or_else(|| "UNKNOWN".into())}
                    }
                }
            }

            code { class: "ml-path", "{path.display()}" }

            if !reasons.is_empty() {
                div { class: "ml-reasons",
                    for reason in reasons.iter().take(4) {
                        p {
                            strong { "{reason.code}" }
                            span { "{reason.message}" }
                        }
                    }
                }
            } else if item.compatibility.is_none() {
                p { class: "ml-muted", "Compatibility has not been evaluated for the current runtime." }
            }

            if !item.record.locations.is_empty() {
                details { class: "ml-location-details",
                    summary { "{item.record.locations.len()} LOCATION(S)" }
                    for location in item.record.locations.clone() {
                        div { class: "ml-location-row",
                            span { class: "ml-location-state {location.state.as_str()}", "{location.state.as_str().to_ascii_uppercase()}" }
                            code { "{location.path.display()}" }
                            if let Some(error) = location.last_error.as_ref() {
                                small { "{error}" }
                            }
                        }
                    }
                }
            }

            if !item.projector_choices.is_empty() {
                details { class: "ml-projector-choices",
                    summary {
                        if let Some(projector_id) = item.associated_projector_id.as_ref() {
                            "PROJECTOR ASSOCIATED: {projector_id}"
                        } else {
                            "PROJECTOR CANDIDATES: {item.projector_choices.len()}"
                        }
                    }
                    for choice in item.projector_choices.clone() {
                        div { class: "ml-choice-row",
                            div {
                                strong { "{choice.name}" }
                                code { "{choice.path.display()}" }
                                small { "{choice.status}: {choice.reasons}" }
                            }
                            button {
                                class: "ml-mini-button",
                                disabled: busy || choice.status == "INCOMPATIBLE",
                                onclick: {
                                    let model_id = model_id.clone();
                                    let projector_id = choice.id.clone();
                                    move |_| begin_associate(state, model_id.clone(), projector_id.clone())
                                },
                                if item.associated_projector_id.as_deref() == Some(choice.id.as_str()) {
                                    "SELECTED"
                                } else {
                                    "ASSOCIATE"
                                }
                            }
                        }
                    }
                }
            }

            div { class: "ml-card-actions",
                if location_label != "PRESENT" {
                    button {
                        class: "ml-mini-button",
                        disabled: busy,
                        onclick: {
                            let model_id = model_id.clone();
                            move |_| begin_relink_model(state, model_id.clone())
                        },
                        "RELINK"
                    }
                }
                button {
                    class: "ml-mini-button danger",
                    disabled: busy,
                    title: "Removes the library entry only; never deletes the GGUF file.",
                    onclick: {
                        let model_id = model_id.clone();
                        move |_| begin_remove_model(state, model_id.clone())
                    },
                    "REMOVE ENTRY"
                }
            }
        }
    }
}

fn projector_row(projector: StoredProjector, state: Signal<LibraryUiState>, busy: bool) -> Element {
    let id = projector.projector.id.clone();
    let name = projector
        .projector
        .name
        .clone()
        .unwrap_or_else(|| id.clone());
    let modalities = if projector.projector.modalities.is_empty() {
        "UNKNOWN".into()
    } else {
        projector
            .projector
            .modalities
            .iter()
            .map(|value| format!("{value:?}").to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join(" + ")
    };
    let state_label = projector.state.as_str().to_ascii_uppercase();

    rsx! {
        article { class: "ml-projector-row",
            div {
                span { class: "ml-badge {projector.state.as_str()}", "{state_label}" }
                strong { "{name}" }
                small { "{modalities}" }
            }
            code { "{projector.projector.path.display()}" }
            div { class: "ml-row-actions",
                if projector.state != LocationState::Present {
                    button {
                        class: "ml-mini-button",
                        disabled: busy,
                        onclick: {
                            let id = id.clone();
                            move |_| begin_relink_projector(state, id.clone())
                        },
                        "RELINK"
                    }
                }
                button {
                    class: "ml-mini-button danger",
                    disabled: busy,
                    title: "Removes the projector record only; never deletes the GGUF file.",
                    onclick: {
                        let id = id.clone();
                        move |_| begin_remove_projector(state, id.clone())
                    },
                    "REMOVE ENTRY"
                }
            }
        }
    }
}
