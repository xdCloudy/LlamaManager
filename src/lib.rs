#[path = "app_shell.rs"]
pub mod app;
pub mod benchmark;
pub mod compatibility;
pub mod config_write;
pub mod error;
pub mod gguf;
pub mod gpu_telemetry;
pub mod hardware_telemetry;
pub mod inference_telemetry;
#[path = "inference_telemetry_ui_combined.rs"]
pub mod inference_telemetry_ui;
#[path = "inference_telemetry_ui.rs"]
mod inference_telemetry_ui_legacy;
pub mod llama;
pub mod model_library;
pub mod model_library_actions;
#[path = "model_library_ui_threadsafe.rs"]
pub mod model_library_ui;
pub mod model_store;
pub mod models_ini;
pub mod models_ini_editor;
pub mod models_ini_effective;
pub mod models_ini_ui;
pub mod models_ini_validation;
pub mod multimodal;
pub mod passive_inference_telemetry;
pub mod paths;
pub mod persistence;
pub mod profile_generator;
pub mod router;
#[path = "router_management_ui.rs"]
pub mod router_management;
pub mod router_observability;
#[path = "streaming_inference_probe_router.rs"]
pub mod streaming_inference_probe;
#[path = "streaming_inference_probe.rs"]
mod streaming_inference_probe_legacy;
pub mod telemetry_alert_ui;
pub mod telemetry_alerts;
pub mod telemetry_chart_ui;
pub mod telemetry_history;
pub mod telemetry_overhead;
pub mod telemetry_ui;
// Router operations deliberately expose their runtime/evidence dependencies at call sites;
// keep the lint exceptions local to this module rather than weakening crate-wide Clippy.
#[allow(
    clippy::too_many_arguments,
    clippy::derivable_impls,
    clippy::redundant_closure
)]
pub mod router_operations;
pub mod router_switch_benchmark;
pub mod server_command;
pub mod server_console;
pub mod server_logs;
pub mod server_process;
pub mod server_readiness;
#[path = "server_ui_clean.rs"]
pub mod server_ui;
