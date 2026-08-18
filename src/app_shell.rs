use dioxus::prelude::*;

use crate::{
    model_library_ui::ModelLibraryView, models_ini_ui::ModelsIniView,
    router_observability::RouterObservabilityView, server_ui::ServerLifecycleView,
};

#[path = "app.rs"]
mod legacy_app;

use legacy_app::App as LegacyApp;
pub use legacy_app::{Bootstrap, set_bootstrap};

const SHELL_CSS: &str = r#"
.surface-switcher {
  position: fixed;
  z-index: 1000;
  right: 18px;
  bottom: 18px;
  display: flex;
  gap: 2px;
  padding: 3px;
  border: 1px solid rgba(0, 255, 255, 0.42);
  background: rgba(5, 0, 12, 0.94);
  box-shadow: 0 0 18px rgba(0, 255, 255, 0.12);
  font-family: "Cascadia Mono", "Cascadia Code", Consolas, monospace;
}
.surface-switcher button {
  min-height: 30px;
  padding: 0 10px;
  border: 1px solid transparent;
  border-radius: 0;
  color: #82758f;
  background: transparent;
  font: inherit;
  font-size: 8px;
  font-weight: 800;
  letter-spacing: 0.10em;
  cursor: pointer;
}
.surface-switcher button:hover {
  color: #00ffff;
  border-color: rgba(0, 255, 255, 0.25);
}
.surface-switcher button.active {
  color: #050009;
  border-color: #00ffff;
  background: #00ffff;
}
.surface-switcher button:focus-visible {
  outline: 2px solid #ff00ff;
  outline-offset: 2px;
}

/* Config Lab evidence can contain long runtime flags, paths and diagnostics.
   Keep every evidence string inside its panel instead of allowing grid overflow. */
.mi-panel-body,
.mi-diagnostics,
.mi-diff,
.mi-diagnostic,
.mi-diff-entry,
.mi-diff-pair,
.mi-diagnostic code,
.mi-diff-entry code {
  min-width: 0;
  max-width: 100%;
}
.mi-diagnostic,
.mi-diff-entry,
.mi-diagnostic code,
.mi-diff-entry code {
  overflow-wrap: anywhere;
  word-break: break-word;
  white-space: normal;
}

@media (max-width: 760px) {
  .surface-switcher {
    left: 10px;
    right: 10px;
    justify-content: stretch;
  }
  .surface-switcher button { flex: 1; padding: 0 5px; font-size: 7px; }
}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    Lab,
    Models,
    Config,
    Router,
    Server,
}

#[allow(non_snake_case)]
pub fn App() -> Element {
    let mut surface = use_signal(|| Surface::Lab);
    let current = surface();

    rsx! {
        style { dangerous_inner_html: SHELL_CSS }

        if current == Surface::Lab {
            LegacyApp {}
        } else if current == Surface::Models {
            ModelLibraryView {}
        } else if current == Surface::Config {
            ModelsIniView {}
        } else if current == Surface::Router {
            RouterObservabilityView {}
        } else {
            ServerLifecycleView {}
        }

        nav { class: "surface-switcher", aria_label: "Workspace switcher",
            button {
                class: if current == Surface::Lab { "active" } else { "" },
                onclick: move |_| surface.set(Surface::Lab),
                "CORE LAB"
            }
            button {
                class: if current == Surface::Models { "active" } else { "" },
                onclick: move |_| surface.set(Surface::Models),
                "MODEL LIBRARY"
            }
            button {
                class: if current == Surface::Config { "active" } else { "" },
                onclick: move |_| surface.set(Surface::Config),
                "CONFIG LAB"
            }
            button {
                class: if current == Surface::Router { "active" } else { "" },
                onclick: move |_| surface.set(Surface::Router),
                "ROUTER"
            }
            button {
                class: if current == Surface::Server { "active" } else { "" },
                onclick: move |_| surface.set(Surface::Server),
                "SERVER LAB"
            }
        }
    }
}
