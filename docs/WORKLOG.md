# Work Log

This log records verified implementation state, not roadmap aspiration.

## 2026-08-16 — Initial GitHub source tranche

### Goal

Publish the clean LlamaManager source baseline and the first real installation → model → benchmark vertical slice to GitHub with the complete specification/roadmap set.

### Changed

- Rust/Dioxus desktop application source added.
- Portable/per-user path resolution added.
- SQLite persistence with a canonical migration source added.
- Real llama.cpp tool discovery and executable hashing added.
- Capability evidence collection from selected binaries added.
- GGUF metadata/header inspection added.
- Real `llama-bench` command preview, execution, raw-output retention, parsing, and persistent benchmark history added.
- Initial restrained-vaporwave desktop styling added.
- Windows CI and release workflows added.
- `Cargo.lock` generated and committed.
- Full product specification, architecture, design, integration, benchmark/autotuner, persistence, test, roadmap, engineering-rule documentation, and preserved original product brief added.

### Verified

GitHub Actions on Windows has produced a green source/build checkpoint with:

```text
cargo check --all-targets                                  PASS
cargo test --all-targets                                   PASS
cargo clippy --all-targets --all-features -- -D warnings  PASS
cargo build --release                                      PASS
```

The first CI cycle found two strict Clippy failures in llama.cpp capability parsing; both were fixed and re-run successfully. Rustfmt was applied to the source tree before the green checkpoint.

Current CI now restores `cargo fmt --all -- --check`, retains the strict compile/test/Clippy/release gates, performs a best-effort native process smoke test, and uploads a portable Windows artifact.

### Remaining

- Confirm the latest strict current-head CI run, including `cargo fmt --check` and artifact assembly.
- Inspect the desktop smoke-test result.
- Launch the real Dioxus application on an interactive Windows desktop and visually verify rendered UI.
- Exercise Milestone 1 against a real arbitrary llama.cpp installation and GGUF model.
- Confirm benchmark persistence across restart using real runtime evidence.

### Risks

- Real llama-bench output variants may require more parser fixtures despite current upstream JSON-field alignment.
- A headless/hosted CI desktop process can reveal early crashes but cannot replace interactive visual QA.

### Next

Finish strict CI/artifact verification, then native Windows visual/runtime validation. Only then promote Milestone 1 to fully verified and begin Milestone 2 model-library/compatibility work.
