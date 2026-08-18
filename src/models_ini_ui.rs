use std::{fs, path::PathBuf};

use dioxus::prelude::*;
use rfd::FileDialog;

use crate::{
    config_write::{
        ConfigWriteMode, DEFAULT_BACKUP_RETENTION, managed_models_ini_path, restore_backup,
        write_external_models_ini, write_managed_models_ini,
    },
    llama::LlamaInstallation,
    models_ini_editor::{EditorMode, ModelsIniEditorSession},
    models_ini_effective::EffectiveValueSource,
    models_ini_validation::{ConfigDiff, ValidationReport, ValidationSeverity},
    paths::AppPaths,
    persistence::Database,
};

const MODELS_INI_CSS: &str = r#"
.mi-page{min-height:100vh;padding:30px 34px 90px;color:#f6eaff;background:radial-gradient(circle at 74% 6%,rgba(255,0,180,.15),transparent 34%),radial-gradient(circle at 12% 70%,rgba(0,255,255,.08),transparent 35%),#07000e;font-family:"Cascadia Mono","Cascadia Code",Consolas,monospace;box-sizing:border-box}.mi-page *{box-sizing:border-box}.mi-header{display:flex;align-items:flex-start;justify-content:space-between;gap:28px;padding-bottom:20px;border-bottom:1px solid rgba(0,255,255,.44)}.mi-kicker,.mi-section-kicker{color:#00ffff;font-size:9px;font-weight:900;letter-spacing:.15em;text-transform:uppercase}.mi-header h1{margin:7px 0 8px;font-size:clamp(25px,3vw,39px);letter-spacing:.02em}.mi-header p,.mi-muted{color:#a996bb;font-size:11px;line-height:1.6}.mi-runtime{min-width:220px;text-align:right}.mi-runtime strong{display:block;margin-top:6px;font-size:12px}.mi-badge{display:inline-flex;align-items:center;min-height:23px;padding:0 8px;border:1px solid rgba(0,255,255,.52);color:#75ffe2;font-size:8px;font-weight:900;letter-spacing:.08em;text-transform:uppercase}.mi-badge.warn{color:#ffd36b;border-color:rgba(255,211,107,.58)}.mi-badge.error{color:#ff6b9f;border-color:rgba(255,40,120,.62)}.mi-badge.dirty{color:#ff45ff;border-color:rgba(255,0,255,.58)}.mi-notice{margin:16px 0 0;padding:11px 13px;border:1px solid rgba(117,255,226,.5);background:rgba(0,20,18,.62);color:#baffed;font-size:10px}.mi-notice.error{border-color:rgba(255,40,120,.6);background:rgba(35,0,15,.6);color:#ff8db4}.mi-toolbar{display:flex;flex-wrap:wrap;align-items:center;gap:8px;margin:16px 0;padding:12px;border:1px solid rgba(0,255,255,.32);background:rgba(15,2,28,.86)}.mi-button{min-height:32px;padding:0 12px;border:1px solid #00dbe7;border-radius:0;background:transparent;color:#00f5ff;font:inherit;font-size:8px;font-weight:900;letter-spacing:.08em;cursor:pointer;text-transform:uppercase}.mi-button:hover:not(:disabled),.mi-button.active{color:#050009;background:#00ffff}.mi-button.magenta{border-color:#ff00d4;color:#ff45e1}.mi-button.magenta:hover:not(:disabled),.mi-button.magenta.active{color:#08000a;background:#ff00d4}.mi-button.danger{border-color:#ff356f;color:#ff6b95}.mi-button:disabled{opacity:.35;cursor:not-allowed}.mi-button:focus-visible,.mi-input:focus-visible,.mi-raw:focus-visible{outline:2px solid #ff00ff;outline-offset:2px}.mi-target{flex:1 1 320px;min-width:0;padding:8px 10px;border-left:2px solid #ff00d4;background:rgba(0,0,0,.35);color:#d7c7e2;font-size:9px;overflow-wrap:anywhere}.mi-grid{display:grid;grid-template-columns:minmax(0,1.55fr) minmax(320px,.75fr);gap:12px}.mi-panel{border:1px solid rgba(0,255,255,.34);background:linear-gradient(180deg,rgba(29,5,47,.82),rgba(7,0,15,.9));min-width:0}.mi-panel-head{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:12px 14px;border-bottom:1px solid rgba(0,255,255,.28)}.mi-panel-head h2{margin:4px 0 0;font-size:16px}.mi-panel-body{padding:13px}.mi-sections{display:flex;flex-wrap:wrap;gap:6px;margin-bottom:12px}.mi-section-button{padding:7px 9px;border:1px solid rgba(255,0,212,.36);background:rgba(18,0,28,.7);color:#bca6c9;font:inherit;font-size:8px;cursor:pointer}.mi-section-button.active{border-color:#ff00d4;color:#ff76ec}.mi-values{display:grid;gap:7px}.mi-row{display:grid;grid-template-columns:minmax(120px,.45fr) minmax(160px,1fr) auto;align-items:center;gap:8px;padding:8px;border:1px solid rgba(104,66,126,.52);background:rgba(0,0,0,.28)}.mi-key{color:#f2b4ff;font-size:9px;overflow-wrap:anywhere}.mi-input,.mi-raw{width:100%;border:1px solid rgba(0,255,255,.3);border-radius:0;background:#030008;color:#f5eaff;font:inherit;font-size:10px}.mi-input{min-height:30px;padding:6px 8px}.mi-raw{min-height:480px;resize:vertical;padding:12px;line-height:1.55;tab-size:2;white-space:pre}.mi-source{display:flex;gap:6px;align-items:center;justify-content:flex-end}.mi-provenance{color:#877495;font-size:7px;white-space:nowrap}.mi-new-row{display:grid;grid-template-columns:minmax(120px,.5fr) minmax(160px,1fr) auto;gap:8px;margin-top:10px;padding-top:10px;border-top:1px dashed rgba(0,255,255,.3)}.mi-diagnostics,.mi-diff{display:grid;gap:7px}.mi-diagnostic,.mi-diff-entry{padding:9px;border-left:2px solid #9a78ae;background:rgba(0,0,0,.28);color:#c8b6d4;font-size:9px;line-height:1.5}.mi-diagnostic.error{border-left-color:#ff356f}.mi-diagnostic.warning{border-left-color:#ffd36b}.mi-diagnostic strong,.mi-diff-entry strong{color:#f7eaff}.mi-diff-pair{display:grid;grid-template-columns:1fr 1fr;gap:6px;margin-top:6px}.mi-diff-pair code{padding:6px;background:#030008;overflow-wrap:anywhere}.mi-error-box{margin:10px 0;padding:10px;border:1px solid rgba(255,53,111,.6);background:rgba(45,0,18,.58);color:#ff91b2;font-size:9px;line-height:1.55}.mi-empty{padding:70px 20px;text-align:center;border:1px dashed rgba(0,255,255,.3);color:#9d89aa}.mi-empty strong{display:block;margin-bottom:9px;color:#00ffff;font-size:13px}.mi-footer-state{display:flex;flex-wrap:wrap;gap:7px;margin-top:12px}@media(max-width:920px){.mi-page{padding:22px 22px 90px}.mi-header{flex-direction:column}.mi-runtime{text-align:left;min-width:0}.mi-grid{grid-template-columns:1fr}.mi-row,.mi-new-row{grid-template-columns:1fr}.mi-source{justify-content:flex-start}.mi-diff-pair{grid-template-columns:1fr}}@media(prefers-reduced-motion:reduce){.mi-page *,.mi-page *::before,.mi-page *::after{transition:none!important;animation:none!important}}
"#;

#[derive(Debug, Clone)]
struct ConfigUiState {
    session: Option<ModelsIniEditorSession>,
    target: Option<PathBuf>,
    write_mode: Option<ConfigWriteMode>,
    selected_section: String,
    new_key: String,
    new_value: String,
    latest_backup: Option<PathBuf>,
    notice: Option<(bool, String)>,
}

impl Default for ConfigUiState {
    fn default() -> Self {
        Self {
            session: None,
            target: None,
            write_mode: None,
            selected_section: "*".into(),
            new_key: String::new(),
            new_value: String::new(),
            latest_backup: None,
            notice: None,
        }
    }
}

fn load_installation() -> Option<LlamaInstallation> {
    let paths = AppPaths::detect().ok()?;
    let database = Database::open(paths.database).ok()?;
    database.latest_installation().ok().flatten()
}

fn load_document(path: PathBuf, mode: ConfigWriteMode) -> Result<ConfigUiState, String> {
    let source = if path.is_file() {
        fs::read_to_string(&path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?
    } else if mode == ConfigWriteMode::Managed {
        "[*]\r\n".to_string()
    } else {
        return Err(format!(
            "Configuration file does not exist: {}",
            path.display()
        ));
    };

    let session = ModelsIniEditorSession::load(source).map_err(|error| error.to_string())?;
    let selected_section = session
        .document()
        .section_names()
        .into_iter()
        .find(|section| *section != "*")
        .unwrap_or("*")
        .to_string();

    Ok(ConfigUiState {
        session: Some(session),
        target: Some(path),
        write_mode: Some(mode),
        selected_section,
        notice: Some((
            true,
            "Configuration loaded into one canonical editor session.".into(),
        )),
        ..ConfigUiState::default()
    })
}

fn open_external(mut state: Signal<ConfigUiState>) {
    let Some(path) = FileDialog::new()
        .set_title("Open an existing models.ini")
        .add_filter("models.ini", &["ini"])
        .pick_file()
    else {
        return;
    };

    match load_document(path, ConfigWriteMode::External) {
        Ok(next) => state.set(next),
        Err(error) => state.write().notice = Some((false, error)),
    }
}

fn new_external(mut state: Signal<ConfigUiState>) {
    let Some(path) = FileDialog::new()
        .set_title("Choose external models.ini destination")
        .set_file_name("models.ini")
        .save_file()
    else {
        return;
    };

    if path.exists() {
        match load_document(path, ConfigWriteMode::External) {
            Ok(next) => state.set(next),
            Err(error) => state.write().notice = Some((false, error)),
        }
        return;
    }

    match ModelsIniEditorSession::load("[*]\r\n") {
        Ok(session) => {
            state.set(ConfigUiState {
                session: Some(session),
                target: Some(path),
                write_mode: Some(ConfigWriteMode::External),
                selected_section: "*".into(),
                notice: Some((
                    true,
                    "New external configuration prepared; no file has been written yet.".into(),
                )),
                ..ConfigUiState::default()
            });
        }
        Err(error) => state.write().notice = Some((false, error.to_string())),
    }
}

fn open_managed(mut state: Signal<ConfigUiState>) {
    let result = AppPaths::detect()
        .map_err(|error| error.to_string())
        .and_then(|paths| load_document(managed_models_ini_path(&paths), ConfigWriteMode::Managed));

    match result {
        Ok(next) => state.set(next),
        Err(error) => state.write().notice = Some((false, error)),
    }
}

fn set_mode(mut state: Signal<ConfigUiState>, mode: EditorMode) {
    let mut current = state.write();
    let Some(session) = current.session.as_mut() else {
        return;
    };
    match session.switch_mode(mode) {
        Ok(()) => current.notice = None,
        Err(error) => current.notice = Some((false, error.to_string())),
    }
}

fn apply_raw(mut state: Signal<ConfigUiState>, source: String) {
    let mut current = state.write();
    let Some(session) = current.session.as_mut() else {
        return;
    };
    match session.apply_raw_edit(source) {
        Ok(()) => current.notice = None,
        Err(error) => {
            current.notice = Some((
                false,
                format!("Raw draft retained with parse error: {error}"),
            ))
        }
    }
}

fn set_structured_value(mut state: Signal<ConfigUiState>, key: String, value: String) {
    let mut current = state.write();
    let section = current.selected_section.clone();
    let Some(session) = current.session.as_mut() else {
        return;
    };
    match session.set_value(&section, &key, &value) {
        Ok(()) => current.notice = None,
        Err(error) => current.notice = Some((false, error.to_string())),
    }
}

fn reset_structured_value(mut state: Signal<ConfigUiState>, key: String) {
    let mut current = state.write();
    let section = current.selected_section.clone();
    let Some(session) = current.session.as_mut() else {
        return;
    };
    match session.reset_to_inherited(&section, &key) {
        Ok(()) => current.notice = Some((true, format!("{key} reset to inherited/unset state."))),
        Err(error) => current.notice = Some((false, error.to_string())),
    }
}

fn add_structured_value(mut state: Signal<ConfigUiState>) {
    let mut current = state.write();
    let key = current.new_key.trim().to_string();
    let value = current.new_value.clone();
    if key.is_empty() {
        current.notice = Some((false, "A configuration key is required.".into()));
        return;
    }
    let section = current.selected_section.clone();
    let Some(session) = current.session.as_mut() else {
        return;
    };
    match session.set_value(&section, &key, &value) {
        Ok(()) => {
            current.new_key.clear();
            current.new_value.clear();
            current.notice = Some((true, format!("Set {key} in [{section}].")));
        }
        Err(error) => current.notice = Some((false, error.to_string())),
    }
}

fn revert_loaded(mut state: Signal<ConfigUiState>) {
    let mut current = state.write();
    if let Some(session) = current.session.as_mut() {
        session.revert_to_loaded();
        current.notice = Some((
            true,
            "Reverted to the exact source loaded from disk.".into(),
        ));
    }
}

fn save_config(mut state: Signal<ConfigUiState>) {
    let snapshot = state.read().clone();
    let Some(session) = snapshot.session.as_ref() else {
        return;
    };
    let Some(target) = snapshot.target.as_ref() else {
        return;
    };
    let Some(mode) = snapshot.write_mode else {
        return;
    };

    let installation = load_installation();
    let validation = match session.validation(&snapshot.selected_section, installation.as_ref()) {
        Ok(report) => report,
        Err(error) => {
            state.write().notice = Some((false, error.to_string()));
            return;
        }
    };
    if !validation.can_apply() {
        state.write().notice = Some((
            false,
            format!(
                "Apply blocked: {} semantic error(s). Fix the validation errors before writing.",
                validation.errors().count()
            ),
        ));
        return;
    }

    let paths = match AppPaths::detect() {
        Ok(paths) => paths,
        Err(error) => {
            state.write().notice = Some((false, error.to_string()));
            return;
        }
    };
    let source = session.canonical_source().to_string();
    let editor_mode = session.mode();
    let result = match mode {
        ConfigWriteMode::Managed => write_managed_models_ini(&paths, &source, &validation),
        ConfigWriteMode::External => {
            write_external_models_ini(target, &source, &validation, DEFAULT_BACKUP_RETENTION)
        }
    };

    match result {
        Ok(receipt) => {
            let mut reloaded = match ModelsIniEditorSession::load(source) {
                Ok(session) => session,
                Err(error) => {
                    state.write().notice = Some((false, error.to_string()));
                    return;
                }
            };
            let _ = reloaded.switch_mode(editor_mode);
            let mut current = state.write();
            current.session = Some(reloaded);
            current.target = Some(receipt.target.clone());
            current.latest_backup = receipt.backup.clone();
            current.notice = Some((
                true,
                format!(
                    "Saved {} bytes safely to {}{}",
                    receipt.bytes_written,
                    receipt.target.display(),
                    receipt
                        .backup
                        .as_ref()
                        .map(|path| format!("; backup: {}", path.display()))
                        .unwrap_or_default()
                ),
            ));
        }
        Err(error) => state.write().notice = Some((false, error.to_string())),
    }
}

fn restore_latest(mut state: Signal<ConfigUiState>) {
    let snapshot = state.read().clone();
    let (Some(backup), Some(target)) = (snapshot.latest_backup.as_ref(), snapshot.target.as_ref())
    else {
        return;
    };

    match restore_backup(backup, target, DEFAULT_BACKUP_RETENTION) {
        Ok(receipt) => match load_document(
            receipt.target.clone(),
            snapshot.write_mode.unwrap_or(ConfigWriteMode::External),
        ) {
            Ok(mut next) => {
                next.latest_backup = receipt.pre_restore_backup;
                next.notice = Some((
                    true,
                    format!(
                        "Restored {} bytes from {}.",
                        receipt.bytes_written,
                        receipt.restored_from.display()
                    ),
                ));
                state.set(next);
            }
            Err(error) => state.write().notice = Some((false, error)),
        },
        Err(error) => state.write().notice = Some((false, error.to_string())),
    }
}

fn display_effective(value: &Option<crate::models_ini_validation::ConfigValueSnapshot>) -> String {
    value
        .as_ref()
        .map(|value| format!("{}  ← {}", value.value, value.source))
        .unwrap_or_else(|| "<unset>".into())
}

fn validation_for(snapshot: &ConfigUiState) -> Option<ValidationReport> {
    let session = snapshot.session.as_ref()?;
    let installation = load_installation();
    session
        .validation(&snapshot.selected_section, installation.as_ref())
        .ok()
}

fn diff_for(snapshot: &ConfigUiState) -> Option<ConfigDiff> {
    snapshot
        .session
        .as_ref()?
        .diff_from_loaded(&snapshot.selected_section)
        .ok()
}

#[allow(non_snake_case)]
pub fn ModelsIniView() -> Element {
    let mut state = use_signal(ConfigUiState::default);
    let snapshot = state.read().clone();
    let installation = load_installation();
    let validation = validation_for(&snapshot);
    let diff = diff_for(&snapshot);
    let can_apply = snapshot.session.is_some()
        && validation.as_ref().is_some_and(ValidationReport::can_apply)
        && snapshot
            .session
            .as_ref()
            .is_some_and(|session| session.raw_error().is_none());

    let sections = snapshot
        .session
        .as_ref()
        .map(|session| {
            let mut sections = session
                .document()
                .section_names()
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>();
            if !sections.iter().any(|section| section == "*") {
                sections.insert(0, "*".into());
            }
            sections
        })
        .unwrap_or_default();

    let effective = snapshot
        .session
        .as_ref()
        .and_then(|session| session.effective_config(&snapshot.selected_section).ok());
    let target_label = snapshot.target.as_ref().map(|target| {
        format!(
            "{:?} · {}",
            snapshot.write_mode.unwrap_or(ConfigWriteMode::External),
            target.display()
        )
    });

    rsx! {
        style { dangerous_inner_html: MODELS_INI_CSS }
        main { class: "mi-page",
            header { class: "mi-header",
                div {
                    div { class: "mi-kicker", "> LLAMAWAVE / CONFIG LAB" }
                    h1 { "MODELS.INI LAB" }
                    p { "One canonical document. Structured + raw editing, evidence-backed validation, diff-before-write, durable backup and restore." }
                }
                div { class: "mi-runtime",
                    div { class: "mi-section-kicker", "RUNTIME EVIDENCE" }
                    strong {
                        if let Some(installation) = installation.as_ref() {
                            "{installation.name}"
                        } else {
                            "NO RUNTIME SELECTED"
                        }
                    }
                    span { class: if installation.is_some() { "mi-badge" } else { "mi-badge warn" },
                        if installation.is_some() { "CAPABILITY CHECKED" } else { "UNKNOWN CAPABILITY" }
                    }
                }
            }

            if let Some((success, message)) = snapshot.notice.as_ref() {
                div { class: if *success { "mi-notice" } else { "mi-notice error" }, "{message}" }
            }

            div { class: "mi-toolbar",
                button { class: "mi-button", onclick: move |_| open_external(state), "OPEN EXTERNAL" }
                button { class: "mi-button", onclick: move |_| new_external(state), "NEW EXTERNAL" }
                button { class: "mi-button", onclick: move |_| open_managed(state), "OPEN MANAGED" }
                div { class: "mi-target",
                    if let Some(label) = target_label.as_ref() {
                        "{label}"
                    } else {
                        "No configuration loaded. Choose an existing external file, a new external destination, or managed config."
                    }
                }
                button { class: "mi-button magenta", disabled: !can_apply, onclick: move |_| save_config(state), "VALIDATE + SAVE" }
                button { class: "mi-button", disabled: snapshot.session.as_ref().is_none_or(|session| !session.is_dirty()), onclick: move |_| revert_loaded(state), "REVERT" }
                button { class: "mi-button danger", disabled: snapshot.latest_backup.is_none(), onclick: move |_| restore_latest(state), "RESTORE BACKUP" }
            }

            if let Some(session) = snapshot.session.as_ref() {
                div { class: "mi-grid",
                    section { class: "mi-panel",
                        div { class: "mi-panel-head",
                            div {
                                div { class: "mi-section-kicker", "EDITOR / CANONICAL DOCUMENT" }
                                h2 { "{snapshot.selected_section}" }
                            }
                            div {
                                button {
                                    class: if session.mode() == EditorMode::Structured { "mi-button active" } else { "mi-button" },
                                    onclick: move |_| set_mode(state, EditorMode::Structured),
                                    "STRUCTURED"
                                }
                                button {
                                    class: if session.mode() == EditorMode::Raw { "mi-button magenta active" } else { "mi-button magenta" },
                                    onclick: move |_| set_mode(state, EditorMode::Raw),
                                    "RAW"
                                }
                            }
                        }
                        div { class: "mi-panel-body",
                            div { class: "mi-sections",
                                for section in sections.clone() {
                                    button {
                                        class: if section == snapshot.selected_section { "mi-section-button active" } else { "mi-section-button" },
                                        onclick: {
                                            let section = section.clone();
                                            move |_| state.write().selected_section = section.clone()
                                        },
                                        "[{section}]"
                                    }
                                }
                            }

                            if session.mode() == EditorMode::Raw {
                                textarea {
                                    class: "mi-raw",
                                    value: "{session.raw_draft()}",
                                    oninput: move |event| apply_raw(state, event.value()),
                                }
                                if let Some(error) = session.raw_error() {
                                    div { class: "mi-error-box",
                                        strong { "RAW PARSE ERROR" }
                                        div { "line {error.line}, column {error.column}: {error.message}" }
                                        code { "{error.context}" }
                                        div { "Draft is retained verbatim. Structured mode and save remain blocked until this parses." }
                                    }
                                }
                            } else {
                                if let Some(effective) = effective.as_ref() {
                                    div { class: "mi-values",
                                        for (key, item) in effective.values.clone() {
                                            div { class: "mi-row",
                                                div { class: "mi-key", "{key}" }
                                                input {
                                                    class: "mi-input",
                                                    value: "{item.value}",
                                                    oninput: {
                                                        let key = key.clone();
                                                        move |event| set_structured_value(state, key.clone(), event.value())
                                                    }
                                                }
                                                div { class: "mi-source",
                                                    span { class: if item.is_inherited() { "mi-badge warn" } else { "mi-badge" },
                                                        if item.is_inherited() { "INHERITED" } else { "OVERRIDE" }
                                                    }
                                                    span { class: "mi-provenance",
                                                        {match item.source {
                                                            EffectiveValueSource::GlobalDefault { line } => format!("global line {line}"),
                                                            EffectiveValueSource::ModelOverride { line, .. } => format!("model line {line}"),
                                                        }}
                                                    }
                                                    if item.is_override() && snapshot.selected_section != "*" {
                                                        button {
                                                            class: "mi-button",
                                                            onclick: {
                                                                let key = key.clone();
                                                                move |_| reset_structured_value(state, key.clone())
                                                            },
                                                            "RESET"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                div { class: "mi-new-row",
                                    input {
                                        class: "mi-input",
                                        placeholder: "new key / llama.cpp option",
                                        value: "{snapshot.new_key}",
                                        oninput: move |event| state.write().new_key = event.value(),
                                    }
                                    input {
                                        class: "mi-input",
                                        placeholder: "value",
                                        value: "{snapshot.new_value}",
                                        oninput: move |event| state.write().new_value = event.value(),
                                    }
                                    button { class: "mi-button magenta", onclick: move |_| add_structured_value(state), "SET VALUE" }
                                }
                            }
                        }
                    }

                    aside { class: "mi-panel",
                        div { class: "mi-panel-head",
                            div {
                                div { class: "mi-section-kicker", "PRE-APPLY EVIDENCE" }
                                h2 { "VALIDATION + DIFF" }
                            }
                        }
                        div { class: "mi-panel-body",
                            if let Some(validation) = validation.as_ref() {
                                div { class: "mi-footer-state",
                                    span { class: if validation.can_apply() { "mi-badge" } else { "mi-badge error" },
                                        if validation.can_apply() { "APPLY ALLOWED" } else { "APPLY BLOCKED" }
                                    }
                                    span { class: "mi-badge error", "{validation.errors().count()} ERRORS" }
                                    span { class: "mi-badge warn", "{validation.warnings().count()} WARNINGS" }
                                    if session.is_dirty() { span { class: "mi-badge dirty", "UNSAVED CHANGES" } }
                                }
                                div { class: "mi-diagnostics",
                                    for issue in validation.issues.clone() {
                                        div { class: if issue.severity == ValidationSeverity::Error { "mi-diagnostic error" } else { "mi-diagnostic warning" },
                                            strong { "{issue.code}" }
                                            if let Some(key) = issue.key.as_ref() { span { " · {key}" } }
                                            div { "{issue.message}" }
                                            if !issue.evidence.is_empty() {
                                                code { "{issue.evidence.join(\" · \")}" }
                                            }
                                        }
                                    }
                                    if validation.issues.is_empty() {
                                        div { class: "mi-diagnostic", "No semantic warnings or errors for the selected section/runtime evidence." }
                                    }
                                }
                            } else {
                                div { class: "mi-error-box", "Validation is unavailable while the raw draft is invalid." }
                            }

                            div { class: "mi-section-kicker", style: "margin-top:16px", "DIFF FROM LOADED" }
                            if let Some(diff) = diff.as_ref() {
                                div { class: "mi-diff",
                                    for entry in diff.entries.clone() {
                                        div { class: "mi-diff-entry",
                                            strong { "{entry.key}" }
                                            div { class: "mi-diff-pair",
                                                code { "BEFORE\n{display_effective(&entry.effective_before)}" }
                                                code { "AFTER\n{display_effective(&entry.effective_after)}" }
                                            }
                                        }
                                    }
                                    if diff.is_empty() {
                                        div { class: "mi-diagnostic", "No source/effective changes from the loaded document." }
                                    }
                                }
                            } else {
                                div { class: "mi-muted", "Diff unavailable until the current draft parses." }
                            }
                        }
                    }
                }
            } else {
                div { class: "mi-empty",
                    strong { "LOAD A REAL MODELS.INI" }
                    div { "Nothing is fabricated before a source or destination is selected." }
                }
            }
        }
    }
}
