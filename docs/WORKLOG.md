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

---

## 2026-08-16 — Full production issue/dependency plan

### Goal

Turn the milestone roadmap into an executable GitHub backlog with explicit prerequisites, evidence gates, and a hard path from the completed baseline to a production-ready v1.0 release.

### Changed

- Added completed M0 issue #11 so the dependency chain has an explicit starting node.
- Expanded milestone epics #1–#10 into implementation-sized feature, verification, and C5-promotion issues.
- Added detailed acceptance criteria covering automated tests, real Windows/runtime evidence, failure/recovery, persistence/reproducibility, UI truthfulness, accessibility, portability, diagnostics, updates, rollback, and production acceptance.
- Added explicit `Blocked by:` prerequisites to child issues and milestone epics.
- Added per-milestone C5 promotion gates: #16, #22, #30, #37, #43, #50, #57, #65, #73, and #84.
- Added final production-readiness/release issue #85.
- Added `docs/11_ISSUE_DEPENDENCY_GRAPH.md` with the critical path and permitted intra-milestone parallelism.
- Marked M1 as the currently active epic and later milestone epics as blocked.

### Current execution state

```text
#11 M0 C5 / CLOSED
  ↓
#1 M1 C3 / ACTIVE
  ↓ via #16
#2 M2 BLOCKED
  ↓ via #22
#3 M3 BLOCKED
  ↓ via #30
#4 M4 BLOCKED
  ↓ via #37
#5 M5 BLOCKED
  ↓ via #43
#6 M6 BLOCKED
  ↓ via #50
#7 M7 BLOCKED
  ↓ via #57
#8 M8 BLOCKED
  ↓ via #65
#9 M9 BLOCKED
  ↓ via #73
#10 M10 BLOCKED
  ↓ via #84
#85 v1.0 production audit/release BLOCKED
```

### Evidence discipline

Issue creation and dependency planning do **not** advance implementation maturity by themselves. M1 remains C3 until real runtime/UI/benchmark evidence is completed. M2–M10 remain C1 until their prerequisites close and their own end-to-end evidence is produced.

### Next

Execute only the unblocked M1 verification work (#12–#15), then run #16 as the C5 promotion gate. Do not begin M2 implementation until that gate has passed and #1 is closure-ready.
