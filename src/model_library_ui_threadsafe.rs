// The model-library workspace performs blocking GGUF/database work on worker
// threads. Specialize Dioxus Signal to SyncStorage for this module so the UI
// state can be updated safely from those workers while preserving live scan
// progress and cancellation.
type Signal<T> = dioxus::prelude::Signal<T, dioxus::prelude::SyncStorage>;

include!("model_library_ui.rs");
