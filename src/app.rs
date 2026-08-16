use std::{path::PathBuf, thread};

use dioxus::prelude::*;
use rfd::FileDialog;

use crate::{
    benchmark::{default_benchmark_arguments, format_command, run_default_benchmark, BenchmarkRun},
    error::Result,
    gguf::{inspect_gguf, ModelInfo},
    llama::{inspect_installation, LlamaInstallation},
    paths::AppPaths,
    persistence::{BenchmarkHistoryItem, Database},
};

const APP_CSS: &str = include_str!("../assets/app.css");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Overview,
    Benchmark,
    History,
    System,
}

#[derive(Debug, Clone)]
enum Activity {
    Idle,
    InspectingInstallation,
    InspectingModel,
    Benchmarking,
}

impl Activity {
    fn label(&self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::InspectingInstallation => "Inspecting llama.cpp",
            Self::InspectingModel => "Inspecting GGUF",
            Self::Benchmarking => "Benchmark running",
        }
    }

    fn is_busy(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}

#[derive(Debug, Clone)]
struct UiState {
    paths: AppPaths,
    db: Database,
    section: Section,
    installation: Option<LlamaInstallation>,
    model: Option<ModelInfo>,
    latest_run: Option<BenchmarkRun>,
    history: Vec<BenchmarkHistoryItem>,
    activity: Activity,
    notice: Option<(bool, String)>,
}

impl UiState {
    fn load(paths: AppPaths, db: Database) -> Result<Self> {
        let installation = db.latest_installation()?;
        let model = db.latest_model()?;
        let history = db.recent_benchmarks(20)?;
        Ok(Self {
            paths,
            db,
            section: Section::Overview,
            installation,
            model,
            latest_run: None,
            history,
            activity: Activity::Idle,
            notice: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Bootstrap {
    initial: UiState,
}

impl Bootstrap {
    pub fn initialize() -> Result<Self> {
        let paths = AppPaths::detect()?;
        let db = Database::open(paths.database.clone())?;
        let initial = UiState::load(paths, db)?;
        Ok(Self { initial })
    }
}

static BOOTSTRAP: std::sync::OnceLock<std::result::Result<Bootstrap, String>> = std::sync::OnceLock::new();

pub fn set_bootstrap(value: std::result::Result<Bootstrap, String>) {
    let _ = BOOTSTRAP.set(value);
}

#[allow(non_snake_case)]
pub fn App() -> Element {
    let bootstrap = BOOTSTRAP.get();
    let Some(bootstrap) = bootstrap else {
        return fatal_screen("Application bootstrap state was not initialized.");
    };
    let Ok(bootstrap) = bootstrap else {
        return fatal_screen(bootstrap.as_ref().err().map(String::as_str).unwrap_or("Unknown bootstrap error"));
    };

    let initial = bootstrap.initial.clone();
    let mut state = use_signal_sync(|| initial);
    let snapshot = state.read().clone();

    let select_installation = move |_| {
        if state.read().activity.is_busy() {
            return;
        }
        let Some(folder) = FileDialog::new().set_title("Select llama.cpp installation").pick_folder() else {
            return;
        };

        state.write().activity = Activity::InspectingInstallation;
        state.write().notice = None;
        let mut worker_state = state;
        thread::spawn(move || {
            let result = inspect_installation(&folder).and_then(|installation| {
                let db = worker_state.read().db.clone();
                db.save_installation(&installation)?;
                Ok(installation)
            });

            let mut current = worker_state.write();
            current.activity = Activity::Idle;
            match result {
                Ok(installation) => {
                    current.installation = Some(installation);
                    current.notice = Some((true, "llama.cpp installation inspected from real binaries.".into()));
                }
                Err(error) => current.notice = Some((false, error.to_string())),
            }
        });
    };

    let select_model = move |_| {
        if state.read().activity.is_busy() {
            return;
        }
        let Some(file) = FileDialog::new()
            .set_title("Select GGUF model")
            .add_filter("GGUF model", &["gguf"])
            .pick_file()
        else {
            return;
        };

        state.write().activity = Activity::InspectingModel;
        state.write().notice = None;
        let mut worker_state = state;
        thread::spawn(move || {
            let result = inspect_gguf(&file).and_then(|model| {
                let db = worker_state.read().db.clone();
                db.save_model(&model)?;
                Ok(model)
            });

            let mut current = worker_state.write();
            current.activity = Activity::Idle;
            match result {
                Ok(model) => {
                    current.model = Some(model);
                    current.notice = Some((true, "GGUF metadata inspected and persisted.".into()));
                }
                Err(error) => current.notice = Some((false, error.to_string())),
            }
        });
    };

    let run_benchmark = move |_| {
        if state.read().activity.is_busy() {
            return;
        }
        let (installation, model, db) = {
            let current = state.read();
            let Some(installation) = current.installation.clone() else {
                drop(current);
                state.write().notice = Some((false, "Select a llama.cpp installation first.".into()));
                return;
            };
            let Some(model) = current.model.clone() else {
                drop(current);
                state.write().notice = Some((false, "Select a GGUF model first.".into()));
                return;
            };
            (installation, model, current.db.clone())
        };

        if installation.bench.is_none() {
            state.write().notice = Some((false, "The selected installation does not contain llama-bench.".into()));
            return;
        }

        state.write().activity = Activity::Benchmarking;
        state.write().notice = None;
        let mut worker_state = state;
        thread::spawn(move || {
            let result = run_default_benchmark(&installation, &model).and_then(|run| {
                db.save_benchmark(&run)?;
                let history = db.recent_benchmarks(20)?;
                Ok((run, history))
            });

            let mut current = worker_state.write();
            current.activity = Activity::Idle;
            match result {
                Ok((run, history)) => {
                    current.history = history;
                    current.latest_run = Some(run);
                    current.notice = Some((true, "Benchmark completed with real llama-bench output.".into()));
                }
                Err(error) => current.notice = Some((false, error.to_string())),
            }
        });
    };

    let can_benchmark = snapshot.installation.as_ref().and_then(|item| item.bench.as_ref()).is_some()
        && snapshot.model.is_some()
        && !snapshot.activity.is_busy();

    let command_preview = match (&snapshot.installation, &snapshot.model) {
        (Some(installation), Some(model)) => {
            if let Some(bench) = installation.bench.as_ref() {
                let args = default_benchmark_arguments(installation, model);
                format_command(&bench.path, &args)
            } else {
                "Selected installation does not contain llama-bench.".into()
            }
        }
        _ => "Select a llama.cpp installation with llama-bench and a GGUF model.".into(),
    };

    let section_content = match snapshot.section {
        Section::Overview => overview(&snapshot),
        Section::Benchmark => benchmark_view(
            &snapshot,
            command_preview,
            can_benchmark,
            select_installation,
            select_model,
            run_benchmark,
        ),
        Section::History => history_view(&snapshot.history),
        Section::System => system_view(&snapshot),
    };

    rsx! {
        style { dangerous_inner_html: APP_CSS }
        div { class: "app-shell",
            aside { class: "sidebar",
                div { class: "brand",
                    div { class: "brand-mark", "LM" }
                    div {
                        div { class: "brand-title", "LlamaManager" }
                        div { class: "brand-subtitle", "LLAMA.CPP CONTROL PLANE" }
                    }
                }

                nav { class: "nav",
                    div { class: "nav-label", "WORKSPACE" }
                    button { class: nav_class(snapshot.section, Section::Overview), onclick: move |_| state.write().section = Section::Overview, "Overview" }
                    button { class: nav_class(snapshot.section, Section::Benchmark), onclick: move |_| state.write().section = Section::Benchmark, "Benchmark" }
                    button { class: nav_class(snapshot.section, Section::History), onclick: move |_| state.write().section = Section::History, "History" }
                    div { class: "nav-label", "SYSTEM" }
                    button { class: nav_class(snapshot.section, Section::System), onclick: move |_| state.write().section = Section::System, "Storage & state" }
                }

                div { class: "sidebar-status",
                    {status_row("llama.cpp", snapshot.installation.as_ref().map(|_| "DETECTED").unwrap_or("NOT SET"))}
                    {status_row("Backend", snapshot.installation.as_ref().and_then(|item| item.backend.as_deref()).unwrap_or("UNKNOWN"))}
                    {status_row("Model", snapshot.model.as_ref().map(|_| "READY").unwrap_or("NOT SET"))}
                    {status_row("Activity", snapshot.activity.label())}
                }
            }

            main { class: "main",
                header { class: "topbar",
                    div {
                        h1 { {section_title(snapshot.section)} }
                        p { "Capability-driven benchmarking with reproducible evidence." }
                    }
                    div { class: if snapshot.activity.is_busy() { "activity-badge busy" } else { "activity-badge" }, {snapshot.activity.label()} }
                }

                if let Some((success, message)) = snapshot.notice.as_ref() {
                    div { class: if *success { "notice success" } else { "notice error" }, "{message}" }
                }

                div { class: "content",
                    {section_content}
                }
            }
        }
    }
}

fn fatal_screen(message: &str) -> Element {
    rsx! {
        style { dangerous_inner_html: APP_CSS }
        div { class: "fatal",
            div { class: "fatal-card",
                div { class: "eyebrow", "STARTUP FAILURE" }
                h1 { "LlamaManager could not initialize" }
                p { "{message}" }
                p { class: "muted", "No fallback database or fake success state was created." }
            }
        }
    }
}

fn nav_class(current: Section, item: Section) -> &'static str {
    if current == item { "nav-item active" } else { "nav-item" }
}

fn section_title(section: Section) -> &'static str {
    match section {
        Section::Overview => "Overview",
        Section::Benchmark => "Real benchmark",
        Section::History => "Benchmark history",
        Section::System => "Storage & state",
    }
}

fn status_row(label: &str, value: &str) -> Element {
    rsx! { div { class: "status-row", span { "{label}" } strong { "{value}" } } }
}

fn overview(snapshot: &UiState) -> Element {
    let prompt = snapshot.latest_run.as_ref().and_then(BenchmarkRun::prompt_tps);
    let decode = snapshot.latest_run.as_ref().and_then(BenchmarkRun::decode_tps);
    rsx! {
        section { class: "hero panel",
            div { class: "eyebrow", "EVIDENCE FIRST" }
            h2 { "Turn a local llama.cpp build and GGUF into measured facts." }
            p { "LlamaManager inspects the binaries you actually selected, reads genuine GGUF metadata, executes llama-bench directly, and stores raw + parsed evidence." }
        }

        div { class: "metric-grid",
            {metric_card("PROMPT", format_metric(prompt, "tok/s"), "Latest real pp result")}
            {metric_card("DECODE", format_metric(decode, "tok/s"), "Latest real tg result")}
            {metric_card("CAPABILITIES", snapshot.installation.as_ref().map(|item| item.capabilities.len().to_string()).unwrap_or_else(|| "—".into()), "Discovered from --help")}
            {metric_card("HISTORY", snapshot.history.len().to_string(), "Persisted benchmark runs")}
        }

        div { class: "two-col",
            section { class: "panel",
                {panel_heading("llama.cpp installation", "Binary evidence")}
                if let Some(installation) = snapshot.installation.as_ref() {
                    {property("Root", &installation.root_path.to_string_lossy())}
                    {property("Backend", installation.backend.as_deref().unwrap_or("Unknown"))}
                    {property("llama-server", installation.server.as_ref().map(|tool| tool.path.to_string_lossy()).as_deref().unwrap_or("Not found"))}
                    {property("llama-bench", installation.bench.as_ref().map(|tool| tool.path.to_string_lossy()).as_deref().unwrap_or("Not found"))}
                } else {
                    {empty_state("No installation selected", "Open Benchmark and choose an arbitrary llama.cpp folder.")}
                }
            }
            section { class: "panel",
                {panel_heading("GGUF model", "Metadata evidence")}
                if let Some(model) = snapshot.model.as_ref() {
                    {property("Name", model.name.as_deref().unwrap_or("Not declared"))}
                    {property("Architecture", model.architecture.as_deref().unwrap_or("Not declared"))}
                    {property("Context", &model.context_length.map(format_number).unwrap_or_else(|| "Not declared".into()))}
                    {property("SHA-256", &short_hash(&model.sha256))}
                } else {
                    {empty_state("No model selected", "Choose a GGUF and LlamaManager will parse its metadata instead of guessing from its filename.")}
                }
            }
        }
    }
}

fn benchmark_view(
    snapshot: &UiState,
    command_preview: String,
    can_benchmark: bool,
    select_installation: impl FnMut(MouseEvent) + 'static,
    select_model: impl FnMut(MouseEvent) + 'static,
    run_benchmark: impl FnMut(MouseEvent) + 'static,
) -> Element {
    rsx! {
        div { class: "workflow-grid",
            section { class: "panel step-panel",
                div { class: "step-number", "01" }
                {panel_heading("llama.cpp", "Inspect actual binaries")}
                if let Some(installation) = snapshot.installation.as_ref() {
                    div { class: "selected-path", {installation.root_path.display().to_string()} }
                    div { class: "chip-row",
                        if installation.server.is_some() { span { class: "chip ok", "llama-server" } }
                        if installation.bench.is_some() { span { class: "chip ok", "llama-bench" } }
                        if installation.fit_params.is_some() { span { class: "chip", "llama-fit-params" } }
                        if let Some(backend) = installation.backend.as_deref() { span { class: "chip accent", "{backend}" } }
                    }
                } else {
                    p { class: "muted", "Select any llama.cpp installation. Tool locations are discovered recursively; bin\\ is not assumed." }
                }
                button { class: "button secondary", disabled: snapshot.activity.is_busy(), onclick: select_installation, "Select installation" }
            }

            section { class: "panel step-panel",
                div { class: "step-number", "02" }
                {panel_heading("GGUF", "Read real metadata")}
                if let Some(model) = snapshot.model.as_ref() {
                    div { class: "selected-path", {model.path.display().to_string()} }
                    div { class: "chip-row",
                        if let Some(architecture) = model.architecture.as_deref() { span { class: "chip accent", "{architecture}" } }
                        span { class: "chip", "GGUF v{model.gguf_version}" }
                        span { class: "chip", {human_bytes(model.file_size)} }
                    }
                } else {
                    p { class: "muted", "Select a local GGUF. Architecture, context and other metadata come from the file header, not its name." }
                }
                button { class: "button secondary", disabled: snapshot.activity.is_busy(), onclick: select_model, "Select GGUF" }
            }
        }

        section { class: "panel command-panel",
            {panel_heading("Command preview", "Exactly what will run")}
            code { class: "command", "{command_preview}" }
            div { class: "command-actions",
                button { class: "button primary", disabled: !can_benchmark, onclick: run_benchmark,
                    if matches!(snapshot.activity, Activity::Benchmarking) { "Benchmarking…" } else { "Run real benchmark" }
                }
                span { class: "muted small", "3 repetitions • llama-bench JSON when supported" }
            }
        }

        if let Some(run) = snapshot.latest_run.as_ref() {
            section { class: "panel",
                {panel_heading("Latest result", "Raw evidence retained")}
                div { class: "result-grid",
                    for sample in &run.samples {
                        div { class: "result-card",
                            div { class: "result-test", "{sample.test}" }
                            div { class: "result-value", {format!("{:.2}", sample.avg_tokens_per_second)} }
                            div { class: "result-unit", "tokens / second" }
                            if let Some(stddev) = sample.stddev_tokens_per_second {
                                div { class: "result-stddev", {format!("± {:.2}", stddev)} }
                            }
                        }
                    }
                }
                details { class: "evidence",
                    summary { "Show exact invocation and raw output" }
                    code { class: "command", {run.command_preview()} }
                    pre { "{run.stdout}" }
                    if !run.stderr.trim().is_empty() { pre { class: "stderr", "{run.stderr}" } }
                }
            }
        }
    }
}

fn history_view(history: &[BenchmarkHistoryItem]) -> Element {
    rsx! {
        section { class: "panel",
            {panel_heading("Stored runs", "SQLite-backed evidence")}
            if history.is_empty() {
                {empty_state("No benchmark history yet", "Complete a real llama-bench run and it will appear here after restart.")}
            } else {
                div { class: "table",
                    div { class: "table-row table-head", span { "Model" } span { "Backend" } span { "Prompt" } span { "Decode" } }
                    for item in history {
                        div { class: "table-row",
                            span { class: "truncate", title: "{item.model_path}", {file_name(&item.model_path)} }
                            span { {item.backend.as_deref().unwrap_or("Unknown")} }
                            span { {format_metric(item.prompt_tps, "tok/s")} }
                            span { {format_metric(item.decode_tps, "tok/s")} }
                        }
                    }
                }
            }
        }
    }
}

fn system_view(snapshot: &UiState) -> Element {
    rsx! {
        div { class: "two-col",
            section { class: "panel",
                {panel_heading("Application storage", "Portable or per-user")}
                {property("Mode", &snapshot.paths.mode.to_string())}
                {property("Root", &snapshot.paths.root.to_string_lossy())}
                {property("Database", &snapshot.db.path().to_string_lossy())}
                {property("Logs", &snapshot.paths.logs.to_string_lossy())}
                {property("Exports", &snapshot.paths.exports.to_string_lossy())}
            }
            section { class: "panel",
                {panel_heading("Integrity policy", "No fake green")}
                ul { class: "policy-list",
                    li { "Binary capabilities come from local --help evidence." }
                    li { "GGUF metadata is parsed from the selected file." }
                    li { "Non-zero llama-bench exits remain failures." }
                    li { "Raw benchmark stdout/stderr is retained." }
                    li { "No production in-memory database fallback." }
                }
            }
        }
    }
}

fn panel_heading(title: &str, eyebrow: &str) -> Element {
    rsx! { div { class: "panel-heading", div { class: "eyebrow", "{eyebrow}" } h2 { "{title}" } } }
}

fn metric_card(label: &str, value: String, detail: &str) -> Element {
    rsx! { div { class: "metric-card", div { class: "metric-label", "{label}" } div { class: "metric-value", "{value}" } div { class: "metric-detail", "{detail}" } } }
}

fn property(label: &str, value: &str) -> Element {
    rsx! { div { class: "property", span { "{label}" } strong { title: "{value}", "{value}" } } }
}

fn empty_state(title: &str, detail: &str) -> Element {
    rsx! { div { class: "empty", strong { "{title}" } p { "{detail}" } } }
}

fn format_metric(value: Option<f64>, unit: &str) -> String {
    value.map(|value| format!("{value:.2} {unit}")).unwrap_or_else(|| "—".into())
}

fn human_bytes(value: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    if value as f64 >= GIB {
        format!("{:.2} GiB", value as f64 / GIB)
    } else {
        format!("{:.1} MiB", value as f64 / MIB)
    }
}

fn format_number(value: u64) -> String {
    let text = value.to_string();
    let mut out = String::new();
    for (index, ch) in text.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 { out.push(','); }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn short_hash(hash: &str) -> String {
    if hash.len() > 18 { format!("{}…{}", &hash[..10], &hash[hash.len() - 6..]) } else { hash.into() }
}

fn file_name(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path)
        .to_string()
}
