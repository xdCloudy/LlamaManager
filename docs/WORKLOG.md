# Work Log

This log records verified implementation state, not roadmap aspiration.

## 2026-08-18 — M1 real Windows runtime validation and C4 evidence

### Goal

Close the remaining automatable Milestone 1 evidence gaps without fabricating the final GUI-interaction claim required for C5.

### Changed

- Added a permanent Windows real-runtime validation workflow in PR #97.
- Added a cancellable `llama-bench` execution path that owns the child process, drains stdout/stderr concurrently, kills/waits on cancellation, and returns typed `BenchmarkInterrupted` evidence.
- Added real published GGUF v2/v3 validation under Windows paths containing spaces and Unicode.
- Added real installation discovery/error validation against a pinned upstream llama.cpp build outside the repository.
- Added real benchmark success, non-zero failure, cancellation and SQLite restart-persistence evidence.
- Retained the pinned upstream Unicode-model-path benchmark failure as an explicit limitation rather than hiding it.
- Closed #13, #14 and #15 after their runtime evidence was recorded.
- Added Windows M2 model-library acceptance coverage in PR #101 while respecting the formal #16 dependency gate.
- Added M3 lossless `models.ini` parser (#95) and inheritance/provenance engine (#98) as implementation work-ahead slices; milestone promotion remains dependency-gated.

### Verified

Normal Windows CI on PR #97 passed:

```text
cargo fmt --all -- --check                              PASS
cargo check --all-targets                               PASS
cargo test --all-targets                                PASS
cargo clippy --all-targets --all-features -- -D warnings PASS
cargo build --release                                   PASS
desktop process smoke                                   PASS
portable bundle assembly/upload                         PASS
```

Real runtime validation run `32086521199` also passed on `windows-2025`.

Pinned runtime identity:

```text
llama.cpp release: b10472
archive SHA-256:   ef495329c85c171991972fd3226a179c1900368cab66e2ebba8b21a7471a74e5
llama-server SHA:  76b0a5f72243ccb99079ca71ebd0332f123c52668d815c9d6716a89d46415668
llama-bench SHA:   97495a77c5f6d528f9eeff0a43a692574951a6b98e673e637366dd7dfce07d4f
```

Real GGUF identities:

```text
v3 SHA-256: 6151b1929d7f5aa3385d9ddef3393e55587c0a55de661562322bc51dfda93a04
v2 SHA-256: 18e8af33a3f4a4ce87314adbdb16757160c6ad93aee2a8ee7aae2a6350d1cfd7
```

Successful real benchmark evidence used the v3 model from a path containing spaces and retained exact argv/raw output/exit/model+binary identities. The captured samples included pp512 `7952.984108 tok/s` and tg128 `1467.728985 tok/s`. SQLite state was reopened and verified after the run. A real missing-model invocation remained failed, and a real cancellation produced typed interruption evidence.

The evidence artifact for run `32086521199` is `real-windows-runtime-evidence`, artifact id `9306990681`, digest `sha256:8997f168b9658a0383fcd5b24f092ad93e482f2f19faf5ac2800c13ec5f91324`.

### Recorded limitation

The pinned upstream `llama-bench` b10472 Windows build fails when the model path contains Unicode because its own runtime output converts the Unicode path to `??`. The same real GGUF was parsed successfully by LlamaManager from the Unicode path. Benchmark success is separately verified using the identical model bytes from a path containing spaces. The limitation is retained in `unicode-benchmark-evidence.json`.

### M1 maturity

Issue #12 already recorded a successful manual visual inspection of the native release UI at 1280×720 and 1600×900.

M1 is now truthfully at **C4 — Runtime verified**. The one remaining C5 requirement is a real benchmark launched through the interactive GUI path. Backend real-runtime evidence and standalone UI visual evidence do not prove that interaction, so #16 and #1 remain open until it is exercised and recorded.

### Workflow cleanup

Only `ci.yml`, `release.yml`, and the permanent `real-runtime-validation.yml` remain under `.github/workflows`. Earlier temporary/self-modifying validation workflows are not present.

---

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

### Remaining at that checkpoint

- Confirm the latest strict current-head CI run, including `cargo fmt --check` and artifact assembly.
- Inspect the desktop smoke-test result.
- Launch the real Dioxus application on an interactive Windows desktop and visually verify rendered UI.
- Exercise Milestone 1 against a real arbitrary llama.cpp installation and GGUF model.
- Confirm benchmark persistence across restart using real runtime evidence.

### Risks recorded at that checkpoint

- Real llama-bench output variants may require more parser fixtures despite current upstream JSON-field alignment.
- A headless/hosted CI desktop process can reveal early crashes but cannot replace interactive visual QA.

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

### Initial execution state

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

Issue creation and dependency planning do **not** advance implementation maturity by themselves. Runtime/UI/benchmark evidence is required before C5 promotion, and later milestones remain dependency-gated until their prerequisites close.
