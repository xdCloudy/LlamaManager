#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use llamamanager::app::{App, Bootstrap, set_bootstrap};
use tracing_subscriber::EnvFilter;

fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();

    let bootstrap = Bootstrap::initialize().map_err(|error| {
        tracing::error!(%error, "application bootstrap failed");
        error.to_string()
    });
    set_bootstrap(bootstrap);

    dioxus::launch(App);
}
