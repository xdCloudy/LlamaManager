use std::{path::PathBuf, thread};

use dioxus::prelude::*;
use rfd::FileDialog;

use crate::{
    benchmark::{BenchmarkRun, default_benchmark_arguments, format_command, run_default_benchmark},
    error::Result,
    gguf::{ModelInfo, inspect_gguf},
    llama::{LlamaInstallation, inspect_installation},
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
            Self::Idle => "READY",
            Self::InspectingInstallation => "SCANNING RUNTIME",
            Self::InspectingModel => "READING GGUF",
            Self::Benchmarking => "BENCHMARKING",
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

static BOOTSTRAP: std::sync::OnceLock<std::result::Result<Bootstrap, String>> =
    std::sync::OnceLock::new();

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
        return fatal_screen(
            bootstrap
                .as_ref()
                .err()
                .map(String::as_str)
                .unwrap_or("Unknown bootstrap error"),
        );
    };

    let initial = bootstrap.initial.clone();
    let mut state = use_signal_sync(|| initial);
    let snapshot = state.read().clone();

    let select_installation = move |_| {
        if state.read().activity.is_busy() {
            return;
        }
        let Some(folder) = FileDialog::new()
            .set_title("Select llama.cpp installation")
            .pick_folder()
        else {
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
                    current.notice = Some((
                        true,
                        "Runtime inspected from the selected llama.cpp binaries.".into(),
                    ));
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
                state.write().notice =
                    Some((false, "Select a llama.cpp installation first.".into()));
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
            state.write().notice = Some((
                false,
                "The selected installation does not contain llama-bench.".into(),
            ));
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
                    current.notice = Some((
                        true,
                        "Benchmark complete. Raw output and parsed evidence were retained.".into(),
                    ));
                }
                Err(error) => current.notice = Some((false, error.to_string())),
            }
        });
    };

    let can_benchmark = snapshot
        .installation
        .as_ref()
        .and_then(|item| item.bench.as_ref())
        .is_some()
        && snapshot.model.is_some()
        && !snapshot.activity.is_busy();

    let blocker = benchmark_blocker(&snapshot);

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
        Section::Overview => overview(
            &snapshot,
            move |_| state.write().section = Section::Benchmark,
            move |_| state.write().section = Section::History,
        ),
        Section::Benchmark => benchmark_view(
            &snapshot,
            command_preview,
            can_benchmark,
            blocker,
            select_installation,
            select_model,
            run_benchmark,
        ),
        Section::History => history_view(&snapshot.history),
        Section::System => system_view(&snapshot),
    };

    rsx! {
        style { dangerous_inner_html: APP_CSS }
        div { class: "crt-overlay" }
        div { class: "app-shell",
            aside { class: "sidebar",
                div { class: "brand",
                    div { class: "brand-kicker", "LOCAL INFERENCE LAB" }
                    div { class: "brand-title", "LLAMAWAVE" }
                    div { class: "brand-line" }
                }

                nav { class: "nav",
                    div { class: "nav-label", "WORKSPACE" }
                    button {
                        class: nav_class(snapshot.section, Section::Overview),
                        onclick: move |_| state.write().section = Section::Overview,
                        span { class: "nav-index", "01" }
                        span { "Overview" }
                    }
                    button {
                        class: nav_class(snapshot.section, Section::Benchmark),
                        onclick: move |_| state.write().section = Section::Benchmark,
                        span { class: "nav-index", "02" }
                        span { "Benchmark" }
                    }
                    button {
                        class: nav_class(snapshot.section, Section::History),
                        onclick: move |_| state.write().section = Section::History,
                        span { class: "nav-index", "03" }
                        span { "History" }
                    }
                    div { class: "nav-label", "SYSTEM" }
                    button {
                        class: nav_class(snapshot.section, Section::System),
                        onclick: move |_| state.write().section = Section::System,
                        span { class: "nav-index", "04" }
                        span { "Storage & state" }
                    }
                }

                div { class: "sidebar-status",
                    div { class: "sidebar-status-title", "> SYSTEM STATUS" }
                    {status_row("RUNTIME", snapshot.installation.as_ref().map(|_| "DETECTED").unwrap_or("NOT SET"))}
                    {status_row("BACKEND", snapshot.installation.as_ref().and_then(|item| item.backend.as_deref()).unwrap_or("UNKNOWN"))}
                    {status_row("MODEL", snapshot.model.as_ref().map(|_| "READY").unwrap_or("NOT SET"))}
                }
            }

            main { class: "main",
                header { class: "topbar",
                    div { class: "page-heading",
                        div { class: "page-kicker", "> LLAMAWAVE / {section_slug(snapshot.section)}" }
                        h1 { {section_title(snapshot.section)} }
                    }
                    div { class: "topbar-actions",
                        if snapshot.section == Section::Overview {
                            button {
                                class: "topbar-link",
                                onclick: move |_| state.write().section = Section::Benchmark,
                                "OPEN BENCHMARK"
                            }
                        }
                        div {
                            class: if snapshot.activity.is_busy() { "activity-badge busy" } else { "activity-badge" },
                            {snapshot.activity.label()}
                        }
                    }
                }

                if let Some((success, message)) = snapshot.notice.as_ref() {
                    div {
                        class: if *success { "notice success" } else { "notice error" },
                        span { class: "notice-prefix", if *success { "OK" } else { "ERR" } }
                        span { "{message}" }
                    }
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
        div { class: "crt-overlay" }
        div { class: "fatal",
            div { class: "fatal-card",
                div { class: "fatal-code", "> BOOT FAILURE" }
                h1 { "LLAMAWAVE COULD NOT INITIALIZE" }
                p { "{message}" }
                p { class: "muted", "No fallback database or fabricated success state was created." }
            }
        }
    }
}

fn nav_class(current: Section, item: Section) -> &'static str {
    if current == item {
        "nav-item active"
    } else {
        "nav-item"
    }
}

fn section_title(section: Section) -> &'static str {
    match section {
        Section::Overview => "OVERVIEW",
        Section::Benchmark => "BENCHMARK LAB",
        Section::History => "BENCHMARK HISTORY",
        Section::System => "STORAGE & STATE",
    }
}

fn section_slug(section: Section) -> &'static str {
    match section {
        Section::Overview => "overview",
        Section::Benchmark => "benchmark",
        Section::History => "history",
        Section::System => "system",
    }
}

fn status_row(label: &str, value: &str) -> Element {
    let class = if matches!(value, "DETECTED" | "READY") {
        "status-value good"
    } else {
        "status-value"
    };
    rsx! {
        div { class: "status-row",
            span { "{label}" }
            strong { class: "{class}", "{value}" }
        }
    }
}

fn benchmark_blocker(snapshot: &UiState) -> &'static str {
    if snapshot.installation.is_none() {
        "Select a llama.cpp installation to continue."
    } else if snapshot
        .installation
        .as_ref()
        .and_then(|item| item.bench.as_ref())
        .is_none()
    {
        "The selected runtime does not contain llama-bench."
    } else if snapshot.model.is_none() {
        "Select a GGUF model to continue."
    } else if snapshot.activity.is_busy() {
        "Wait for the current operation to finish."
    } else {
        "Runtime and model are ready."
    }
}

fn overview(
    snapshot: &UiState,
    open_benchmark: impl FnMut(MouseEvent) + 'static,
    open_history: impl FnMut(MouseEvent) + 'static,
) -> Element {
    let prompt = snapshot
        .latest_run
        .as_ref()
        .and_then(BenchmarkRun::prompt_tps);
    let decode = snapshot
        .latest_run
        .as_ref()
        .and_then(BenchmarkRun::decode_tps);

    let runtime_ready = snapshot
        .installation
        .as_ref()
        .and_then(|item| item.bench.as_ref())
        .is_some();
    let model_ready = snapshot.model.is_some();
    let run_ready = runtime_ready && model_ready;
    let runtime_state = if runtime_ready { "READY" } else { "REQUIRED" };
    let model_state = if model_ready { "READY" } else { "REQUIRED" };
    let run_state = if snapshot.latest_run.is_some() {
        "MEASURED"
    } else if run_ready {
        "READY"
    } else {
        "LOCKED"
    };

    rsx! {
        section { class: "hero",
            div { class: "hero-copy",
                div { class: "eyebrow", "> LOCAL PERFORMANCE WORKSTATION" }
                h2 {
                    span { "MEASURE" }
                    span { class: "gradient-text", "THE MACHINE." }
                }
                p {
                    "Inspect the binaries you actually run, read the GGUF you actually load, and benchmark the exact local stack without guessed metadata."
                }
                div { class: "hero-actions",
                    button { class: "button primary", onclick: open_benchmark, span { "START BENCHMARK" } }
                    button { class: "button ghost", onclick: open_history, "VIEW HISTORY" }
                }
                div { class: "hero-proof",
                    span { "> LOCAL ONLY" }
                    span { "> RAW OUTPUT RETAINED" }
                    span { "> NO FAKE GREEN" }
                }
            }
            div { class: "hero-art",
                div { class: "sun" }
                div { class: "horizon" }
                div { class: "mountain mountain-a" }
                div { class: "mountain mountain-b" }
                div { class: "hero-orbit orbit-a" }
                div { class: "hero-orbit orbit-b" }
            }
        }

        section { class: "readiness",
            {readiness_step("01", "RUNTIME", "Select llama.cpp", runtime_state, runtime_ready)}
            div { class: "readiness-link" }
            {readiness_step("02", "MODEL", "Inspect GGUF", model_state, model_ready)}
            div { class: "readiness-link" }
            {readiness_step("03", "MEASURE", "Run benchmark", run_state, snapshot.latest_run.is_some())}
        }

        div { class: "metric-grid",
            {metric_card("PROMPT", format_metric(prompt, "tok/s"), "Latest pp throughput", "cyan")}
            {metric_card("DECODE", format_metric(decode, "tok/s"), "Latest tg throughput", "magenta")}
            {metric_card(
                "CAPABILITIES",
                snapshot
                    .installation
                    .as_ref()
                    .map(|item| item.capabilities.len().to_string())
                    .unwrap_or_else(|| "—".into()),
                "Discovered from --help",
                "orange",
            )}
            {metric_card("RUNS", snapshot.history.len().to_string(), "Persisted evidence", "purple")}
        }

        div { class: "overview-grid",
            section { class: "panel data-panel",
                {panel_heading("RUNTIME", "LLAMA.CPP")}
                if let Some(installation) = snapshot.installation.as_ref() {
                    {property("ROOT", &installation.root_path.to_string_lossy())}
                    {property("BACKEND", installation.backend.as_deref().unwrap_or("Unknown"))}
                    {property(
                        "LLAMA-BENCH",
                        installation
                            .bench
                            .as_ref()
                            .map(|tool| tool.path.to_string_lossy())
                            .as_deref()
                            .unwrap_or("Not found"),
                    )}
                } else {
                    {empty_state("RUNTIME NOT CONFIGURED", "Choose the llama.cpp build you actually use. LlamaWave will inspect its binaries instead of assuming capabilities.")}
                }
            }

            section { class: "panel data-panel",
                {panel_heading("MODEL", "GGUF")}
                if let Some(model) = snapshot.model.as_ref() {
                    {property("NAME", model.name.as_deref().unwrap_or("Not declared"))}
                    {property("ARCH", model.architecture.as_deref().unwrap_or("Not declared"))}
                    {property(
                        "CONTEXT",
                        &model
                            .context_length
                            .map(format_number)
                            .unwrap_or_else(|| "Not declared".into()),
                    )}
                    {property("SHA-256", &short_hash(&model.sha256))}
                } else {
                    {empty_state("MODEL NOT CONFIGURED", "Choose a GGUF file and LlamaWave will read the file header instead of guessing from its filename.")}
                }
            }

            section { class: "terminal-panel overview-terminal",
                div { class: "terminal-titlebar",
                    span { "> SESSION / READINESS" }
                    span { class: "terminal-state", if run_ready { "ARMED" } else { "WAITING" } }
                }
                div { class: "terminal-body",
                    {terminal_line("runtime", runtime_state)}
                    {terminal_line("model", model_state)}
                    {terminal_line("benchmark", run_state)}
                    div { class: "terminal-cursor-line",
                        span { class: "prompt-symbol", ">" }
                        span {
                            if run_ready {
                                "Stack ready. Open Benchmark Lab to measure."
                            } else {
                                "Complete the required inputs to unlock measurement."
                            }
                        }
                        span { class: "cursor", "_" }
                    }
                }
            }
        }
    }
}

fn readiness_step(number: &str, label: &str, detail: &str, state: &str, complete: bool) -> Element {
    rsx! {
        div { class: if complete { "readiness-step complete" } else { "readiness-step" },
            span { class: "readiness-number", "{number}" }
            div {
                strong { "{label}" }
                span { "{detail}" }
            }
            span { class: "readiness-state", "{state}" }
        }
    }
}

fn terminal_line(label: &str, value: &str) -> Element {
    rsx! {
        div { class: "terminal-line",
            span { class: "prompt-symbol", ">" }
            span { class: "terminal-key", "{label}" }
            span { class: "terminal-dots" }
            strong { "{value}" }
        }
    }
}

fn benchmark_view(
    snapshot: &UiState,
    command_preview: String,
    can_benchmark: bool,
    blocker: &'static str,
    select_installation: impl FnMut(MouseEvent) + 'static,
    select_model: impl FnMut(MouseEvent) + 'static,
    run_benchmark: impl FnMut(MouseEvent) + 'static,
) -> Element {
    let runtime_ready = snapshot
        .installation
        .as_ref()
        .and_then(|item| item.bench.as_ref())
        .is_some();
    let model_ready = snapshot.model.is_some();

    rsx! {
        section { class: "lab-header",
            div {
                div { class: "eyebrow", "> CONTROLLED EXPERIMENT" }
                h2 { "BENCHMARK YOUR REAL STACK" }
                p { "Three steps. Each input is inspected before execution, and the exact command remains visible before you run it." }
            }
            div { class: "lab-state",
                span { "STAGE" }
                strong { if can_benchmark { "03 / 03" } else if runtime_ready { "02 / 03" } else { "01 / 03" } }
            }
        }

        div { class: "workflow-grid",
            section { class: if runtime_ready { "panel step-panel complete" } else { "panel step-panel active" },
                div { class: "step-head",
                    span { class: "step-number", "01" }
                    div {
                        div { class: "eyebrow", "RUNTIME" }
                        h2 { "SELECT LLAMA.CPP" }
                    }
                    span { class: "step-status", if runtime_ready { "READY" } else { "REQUIRED" } }
                }
                if let Some(installation) = snapshot.installation.as_ref() {
                    div { class: "selected-path", title: installation.root_path.display().to_string(), {installation.root_path.display().to_string()} }
                    div { class: "chip-row",
                        if installation.server.is_some() { span { class: "chip ok", "LLAMA-SERVER" } }
                        if installation.bench.is_some() { span { class: "chip ok", "LLAMA-BENCH" } }
                        if installation.fit_params.is_some() { span { class: "chip", "FIT-PARAMS" } }
                        if let Some(backend) = installation.backend.as_deref() { span { class: "chip accent", "{backend}" } }
                    }
                } else {
                    p { class: "step-copy", "Choose the directory containing the llama.cpp build you actually run. Tool locations are discovered recursively." }
                }
                button {
                    class: "button outline",
                    disabled: snapshot.activity.is_busy(),
                    onclick: select_installation,
                    span { if runtime_ready { "CHANGE RUNTIME" } else { "SELECT RUNTIME" } }
                }
            }

            section { class: if model_ready { "panel step-panel complete" } else if runtime_ready { "panel step-panel active" } else { "panel step-panel" },
                div { class: "step-head",
                    span { class: "step-number", "02" }
                    div {
                        div { class: "eyebrow", "MODEL" }
                        h2 { "SELECT GGUF" }
                    }
                    span { class: "step-status", if model_ready { "READY" } else { "REQUIRED" } }
                }
                if let Some(model) = snapshot.model.as_ref() {
                    div { class: "selected-path", title: model.path.display().to_string(), {model.path.display().to_string()} }
                    div { class: "chip-row",
                        if let Some(architecture) = model.architecture.as_deref() { span { class: "chip accent", "{architecture}" } }
                        span { class: "chip", "GGUF V{model.gguf_version}" }
                        span { class: "chip", {human_bytes(model.file_size)} }
                    }
                } else {
                    p { class: "step-copy", "Choose the GGUF you want to measure. Architecture and context come from its metadata, not its filename." }
                }
                button {
                    class: "button outline",
                    disabled: snapshot.activity.is_busy(),
                    onclick: select_model,
                    span { if model_ready { "CHANGE MODEL" } else { "SELECT GGUF" } }
                }
            }
        }

        section { class: if can_benchmark { "terminal-panel command-panel armed" } else { "terminal-panel command-panel" },
            div { class: "terminal-titlebar",
                span { "> EXECUTION PREVIEW" }
                span { class: "terminal-state", if can_benchmark { "ARMED" } else { "BLOCKED" } }
            }
            div { class: "command-wrap",
                div { class: "command-prefix", "$" }
                code { class: "command", "{command_preview}" }
            }
            div { class: "command-actions",
                button {
                    class: "button primary",
                    disabled: !can_benchmark,
                    onclick: run_benchmark,
                    span {
                        if matches!(snapshot.activity, Activity::Benchmarking) {
                            "BENCHMARKING..."
                        } else {
                            "RUN BENCHMARK"
                        }
                    }
                }
                div { class: if can_benchmark { "run-helper ready" } else { "run-helper" },
                    span { class: "helper-dot" }
                    span { "{blocker}" }
                }
                span { class: "run-meta", "3 REPS / RAW EVIDENCE RETAINED" }
            }
        }

        if let Some(run) = snapshot.latest_run.as_ref() {
            section { class: "panel results-panel",
                {panel_heading("LATEST RESULT", "MEASURED")}
                div { class: "result-grid",
                    for sample in &run.samples {
                        div { class: "result-card",
                            div { class: "result-test", "> {sample.test}" }
                            div { class: "result-value", {format!("{:.2}", sample.avg_tokens_per_second)} }
                            div { class: "result-unit", "TOKENS / SECOND" }
                            if let Some(stddev) = sample.stddev_tokens_per_second {
                                div { class: "result-stddev", {format!("± {:.2}", stddev)} }
                            }
                        }
                    }
                }
                details { class: "evidence",
                    summary { "SHOW EXACT INVOCATION + RAW OUTPUT" }
                    code { class: "command evidence-command", {run.command_preview()} }
                    pre { "{run.stdout}" }
                    if !run.stderr.trim().is_empty() {
                        pre { class: "stderr", "{run.stderr}" }
                    }
                }
            }
        }
    }
}

fn history_view(history: &[BenchmarkHistoryItem]) -> Element {
    rsx! {
        section { class: "history-header",
            div {
                div { class: "eyebrow", "> PERSISTED EVIDENCE" }
                h2 { "MEASUREMENTS, NOT MEMORIES." }
                p { "Every completed run remains available after restart so you can compare real local results over time." }
            }
            div { class: "history-count",
                strong { "{history.len()}" }
                span { "RUNS" }
            }
        }

        section { class: "terminal-panel history-panel",
            div { class: "terminal-titlebar",
                span { "> BENCHMARK HISTORY / SQLITE" }
                span { class: "terminal-state", "{history.len()} RECORDS" }
            }
            if history.is_empty() {
                {empty_state("NO RUNS RECORDED", "Complete a benchmark in the lab and its measured evidence will appear here after restart.")}
            } else {
                div { class: "table",
                    div { class: "table-row table-head",
                        span { "MODEL" }
                        span { "BACKEND" }
                        span { "PROMPT" }
                        span { "DECODE" }
                    }
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
        section { class: "history-header",
            div {
                div { class: "eyebrow", "> LOCAL STATE" }
                h2 { "KNOW WHERE EVERYTHING LIVES." }
                p { "Portable and per-user paths are shown explicitly. Nothing important should be hidden behind application magic." }
            }
        }

        div { class: "two-col",
            section { class: "panel data-panel",
                {panel_heading("APPLICATION STORAGE", "PATHS")}
                {property("MODE", &snapshot.paths.mode.to_string())}
                {property("ROOT", &snapshot.paths.root.to_string_lossy())}
                {property("DATABASE", &snapshot.db.path().to_string_lossy())}
                {property("LOGS", &snapshot.paths.logs.to_string_lossy())}
                {property("EXPORTS", &snapshot.paths.exports.to_string_lossy())}
            }
            section { class: "terminal-panel policy-panel",
                div { class: "terminal-titlebar",
                    span { "> INTEGRITY POLICY" }
                    span { class: "terminal-state", "ENFORCED" }
                }
                ul { class: "policy-list",
                    li { span { "01" } "Binary capabilities come from local --help evidence." }
                    li { span { "02" } "GGUF metadata is parsed from the selected file." }
                    li { span { "03" } "Non-zero llama-bench exits remain failures." }
                    li { span { "04" } "Raw benchmark stdout/stderr is retained." }
                    li { span { "05" } "No production in-memory database fallback." }
                }
            }
        }
    }
}

fn panel_heading(title: &str, eyebrow: &str) -> Element {
    rsx! {
        div { class: "panel-heading",
            div { class: "eyebrow", "> {eyebrow}" }
            h2 { "{title}" }
        }
    }
}

fn metric_card(label: &str, value: String, detail: &str, accent: &str) -> Element {
    rsx! {
        div { class: "metric-card {accent}",
            div { class: "metric-label", "{label}" }
            div { class: "metric-value", "{value}" }
            div { class: "metric-detail", "{detail}" }
        }
    }
}

fn property(label: &str, value: &str) -> Element {
    rsx! {
        div { class: "property",
            span { "{label}" }
            strong { title: "{value}", "{value}" }
        }
    }
}

fn empty_state(title: &str, detail: &str) -> Element {
    rsx! {
        div { class: "empty",
            div { class: "empty-mark", "◇" }
            div {
                strong { "{title}" }
                p { "{detail}" }
            }
        }
    }
}

fn format_metric(value: Option<f64>, unit: &str) -> String {
    value
        .map(|value| format!("{value:.2} {unit}"))
        .unwrap_or_else(|| "—".into())
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
        if index > 0 && index % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn short_hash(hash: &str) -> String {
    if hash.len() > 18 {
        format!("{}…{}", &hash[..10], &hash[hash.len() - 6..])
    } else {
        hash.into()
    }
}

fn file_name(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path)
        .to_string()
}
