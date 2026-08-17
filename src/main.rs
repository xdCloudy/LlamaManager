#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
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

    let window = WindowBuilder::new()
        .with_title("LlamaWave")
        .with_inner_size(LogicalSize::new(1440.0, 900.0))
        .with_min_inner_size(LogicalSize::new(1100.0, 700.0));

    let config = Config::new()
        .with_window(window)
        .with_menu(None)
        .with_background_color((7, 3, 18, 255))
        .with_disable_context_menu(true);

    dioxus::LaunchBuilder::new().with_cfg(config).launch(App);
}
