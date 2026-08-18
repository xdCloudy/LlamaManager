#![allow(clippy::unnecessary_lazy_evaluations)]

// The model-library workspace performs blocking GGUF/database work on worker
// threads. Specialize Dioxus Signal to SyncStorage for this module so the UI
// state can be updated safely from those workers while preserving live scan
// progress and cancellation.
//
// The included implementation intentionally uses `unwrap_or_else` while moving
// fallback identity fields out of owned records; the strict Clippy lint is
// scoped to this included module rather than weakened repository-wide.
type Signal<T> = dioxus::prelude::Signal<T, dioxus::prelude::SyncStorage>;

include!("model_library_ui.rs");
