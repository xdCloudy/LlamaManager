# Work Log

This log records verified implementation state, not roadmap aspiration.

## 2026-08-18 — M5 C5 router + model switching closure

### Goal

Promote Milestone 5 only after router discovery, real mutation paths, truthful observability, switching benchmarks, restart reconciliation, failure recovery, strict automation, real Windows runtime evidence, and the final rendered Router Control visual gate were all complete.

### Changed

- Closed #38 after evidence-backed router discovery, capability probing and canonical model-registry reconciliation were verified.
- Closed #39 after serialized real load/unload/reload/preload/switch operations and post-operation reconciliation were verified.
- Closed #40 after residency/LRU, active-request and alias visibility were made evidence-backed, with unavailable state kept explicit rather than guessed.
- Closed #41 after the real A → B → A switch benchmark, reproducibility envelope and failure/recovery behaviour were verified.
- Merged PR #134 as `7626ca44d16f2b3b2b51e575fc2f09db19099b65`, integrating the Router Control UI and restart/reconnect reconciliation.
- Source-review hardening on the final PR head invalidates stale endpoint/runtime authorization, serializes runtime refresh and live reconciliation, prevents duplicate reconciliation races, reports post-operation reconciliation failure truthfully, exposes Reload capability reasons visibly, and converts unexpected worker panic into recoverable retained failure evidence.

### Automated and real-runtime verification

Final implementation head `a79e6445c0d520a7f6517555860f1b13f73ae31a` passed normal CI #255 (`32143778017`) end-to-end:

```text
PowerShell syntax                                      PASS
cargo fmt --all -- --check                            PASS
cargo check --all-targets                             PASS
cargo test --all-targets                              PASS
cargo clippy --all-targets --all-features -D warnings PASS
cargo build --release                                 PASS
desktop process smoke                                 PASS
portable bundle assembly/upload                       PASS
```

Real Windows Runtime Validation #43 (`32143777993`) also passed against pinned llama.cpp b10472 and published GGUFs, including real router discovery, load/unload/preload/switch operations, A → B → A switching, restart/reconnect reconciliation and evidence upload.

The restart evidence records the preferred target as `Verified` before restart, `NeedsLiveReconciliation` after disconnect, and `NotReady / Unloaded` after the restarted router is actually rediscovered. No stale ready state survives reconnect.

Pinned b10472 exposes no supported dynamic default-model mutation route, so LlamaManager records that capability as unsupported/N/A and never substitutes a fake startup-control path.

### Interactive Windows verification

The repository owner built the exact final PR head from source and inspected Router Control at normal and narrow desktop sizes on Windows on 2026-08-18.

The rendered UI was reported good with no blocking overlap, clipping, unusable control, or other layout/UX defect. That completed #42's final human visual gate.

### Result

Issues #38–#42 are complete with implementation, automated, runtime, recovery and rendered-UI evidence. `docs/evidence/M5_ROUTER_SWITCHING_2026-08-18.md` records the C5 closure evidence. M5 satisfies G1–G10 and is ready for #43 promotion; once #43 and #5 close, M6/#44 is formally unblocked.

---

## 2026-08-18 — M4 C5 managed llama-server lifecycle closure

### Goal

Promote Milestone 4 only after the complete managed `llama-server` path had implementation, strict automation, real Windows runtime evidence, rendered lifecycle/error-state validation, and deterministic timeout/force-kill recovery evidence.

### Changed

- Closed #31 after capability-backed executable+argv command construction was implemented and verified.
- Closed #32 after Windows process supervision and Job Object cleanup were implemented and verified.
- Closed #33 after real health + minimal `/completion` inference readiness and actionable port checks were implemented and verified.
- Closed #34 after PR #123 (`a1700e89f913102f1fb513905cb0a5034e77c945`) added bounded stdout/stderr capture, retained lifecycle evidence, redacted export, high-volume fixtures and real Windows runtime evidence.
- Closed #35 after PR #124 (`6797875ea3805b4001bf7c378d0907c005daa9c9`) added the rendered SERVER LAB workflow and interactive Windows lifecycle validation completed.
- Merged PR #125 (`2f6d822e948b025e673f59775541b9ca5961716d`) so normal llama.cpp stderr diagnostics are classified by content rather than treated as errors.
- Merged PR #126 (`c25cda85c95fc96e897ecfa42997345199943d6e`) so visible log labels render INFO/WARN/ERR/FATAL from presentation severity rather than stderr stream identity.
- Closed #36 after the full real lifecycle/failure/recovery matrix was exercised, including timeout, crash, bad-GGUF, force-kill and no-leak verification.

### Interactive Windows verification

The repository owner exercised the release SERVER LAB against the real `D:\llamacpp\bin\llama-server.exe` and a real GGUF model on `127.0.0.1:8080`.

Verified:

- exact managed command preview and API-key redaction;
- start → real health + HTTP 200 `/completion` → `Ready`;
- graceful stop and restart → `Ready` without stale ownership;
- occupied-port state represented as unknown/unowned rather than fake-managed;
- external process termination detected as `Crashed` with retained exit/log evidence;
- deliberately invalid GGUF produced a real failed-start/crash with retained load errors;
- normal llama.cpp stderr startup/inference/timing output displays as `INFO`, while real failures display as `ERR`;
- force-kill requires a second confirmation step;
- narrow and normal/wide layouts remained usable through lifecycle and error states.

### Timeout / force-kill matrix

A real managed llama-server was suspended before readiness. LlamaWave remained truthful: after about 31.34 seconds it reported a readiness timeout, lifecycle `Failed`, ownership `MANAGED`, and retained logs instead of claiming success or stopped state.

The real server still handled Ctrl+C cooperatively, so a deterministic Windows wrapper fixture enabled inherited ignore-Ctrl+C semantics while launching the real llama-server child inside the managed Job Object. The child reached real readiness and minimal inference. `STOP GRACEFULLY` then exercised the five-second grace-period-expiry branch: the UI remained `Ready` + `MANAGED`, stated that the process was still owned and NOT stopped, and armed `CONFIRM FORCE KILL`.

After confirmation, Job Object termination removed the managed tree and the final listener check returned `Port8080Listening=False`.

### CI

PR #126 CI run `32111119407` passed:

```text
PowerShell syntax                                      PASS
cargo fmt --all -- --check                            PASS
cargo check --all-targets                             PASS
cargo test --all-targets                              PASS
cargo clippy --all-targets --all-features -D warnings PASS
cargo build --release                                 PASS
desktop process smoke                                 PASS
portable bundle assembly/upload                       PASS
```

Permanent Real Windows Runtime Validation run `32103995058` also passed on the M4 implementation path with pinned llama.cpp `b10472`, published GGUFs, spaces/Unicode paths, real server readiness and minimal inference evidence.

### Result

Issues #31–#36 are closed with implementation/runtime evidence. `docs/evidence/M4_SERVER_LIFECYCLE_2026-08-18.md` records the closure evidence. M4 satisfies the applicable G1–G10 gates and is ready for #37 C5 promotion.

The recurring managed-console auto-hide `ERROR_INVALID_HANDLE` warning observed during interactive runs is retained as non-blocking presentation polish: it did not affect process ownership, readiness, logging, shutdown or cleanup truth.

---

## 2026-08-18 — M3 C5 models.ini runtime closure

### Goal

Promote Milestone 3 only after the parser/editor/write/generator stack had both closure-grade automation and a real rendered Windows acceptance pass.

### Changed

- Merged the lossless parser (#95), inheritance/provenance engine (#98), capability-aware validation/diff (#100), canonical structured/raw editor session (#108), safe managed/external write+backup+restore path (#105), evidence-backed generator (#110), and automated Windows runtime/recovery matrix (#111).
- Merged PR #119 at `0059744ba69a49dfd892b8b72e1601053b6b02e4`, wiring the real CONFIG LAB workflow into LlamaWave.
- Merged PR #120 at `fa63989656a364e8ea48c4c31e380bbf4bf1f90d`, adding a deterministic 4,000-comment Unicode/CRLF rendered stress fixture and explicit A–E acceptance checklist.
- Merged PR #121 at `35dd7e1ad27263fc6d00d7b61c90fc31801f3ac2` after interactive QA found long PRE-APPLY EVIDENCE text could escape its panel; the fix constrains/wraps diagnostic/diff content and clarifies fixture names.

### Interactive Windows verification

The repository owner completed sections A–E of `scripts/prepare-m3-config-validation.ps1` in the real release UI.

Verified:

- the normal external fixture opened successfully;
- `[agent 模型]` and `[*]` values showed inherited/override provenance;
- a structured `threads` change produced a real BEFORE/AFTER diff;
- malformed raw input showed a contextual RAW PARSE ERROR and disabled VALIDATE + SAVE;
- fixing the raw error restored validation/diff;
- external save created a backup and RESTORE BACKUP recovered the prior value;
- managed config survived a full LlamaWave restart;
- Unicode/space paths and CRLF/comment-heavy source remained usable;
- the 4,000-comment long-file fixture remained responsive through RAW ↔ STRUCTURED switching and narrow layout;
- no fake save/success state was observed.

The user accepted the post-fix workflow as usable and explicitly chose not to block milestone completion on further cosmetic UI polish.

### CI

PR #121 CI run `32102022938` passed:

```text
PowerShell syntax                                      PASS
cargo fmt --all -- --check                            PASS
cargo check --all-targets                             PASS
cargo test --all-targets                              PASS
cargo clippy --all-targets --all-features -D warnings PASS
cargo build --release                                 PASS
desktop process smoke                                 PASS
portable bundle assembly/upload                       PASS
```

### Result

Issues #26 and #29 are closed with rendered/runtime evidence. `docs/evidence/M3_MODELS_INI_2026-08-18.md` records the closure evidence. M3 satisfies the applicable G1–G10 gates and is ready for #30 C5 promotion; residual cosmetic UI polish is deferred rather than treated as a correctness/runtime blocker.

---

## 2026-08-18 — M2 C5 model-library runtime closure

### Goal

Complete the Milestone 2 evidence pass only after the real user-facing Model Library existed and its scan, failure, repair, compatibility, responsive-layout and restart paths had been exercised on interactive Windows.

### Changed

- Merged PR #117 at `a504eb6a1181dbcccf7b0a4191d5a0200607a463`, wiring the Model Library into LlamaWave.
- Added recursive scan, manual add, content-derived identities, multiple known locations, explicit present/missing/unreadable states, relink, non-destructive library removal, compatibility evidence/reasons, projector association UI, persisted scan roots, and responsive/reduced-motion presentation.
- Hardened the Dioxus model-library state for worker-thread GGUF/database operations and resolved strict Clippy findings before merge.
- Retained automated acceptance for cancellation, reparse-point safety, locked/corrupt input isolation, dedupe, move/relink identity, compatibility staleness, projector behaviour, Unicode/spaces and persistence.

### CI

Final M2 implementation CI run `32098153891` passed:

```text
PowerShell syntax                                  PASS
cargo fmt --all -- --check                        PASS
cargo check --all-targets                         PASS
cargo test --all-targets                          PASS
cargo clippy --all-targets --all-features -D warnings PASS
cargo build --release                             PASS
desktop process smoke                             PASS
portable bundle assembly/upload                   PASS
```

### Interactive Windows verification

The repository owner exercised the real release UI with the selected llama.cpp runtime and a real hash-pinned GGUF.

Verified:

- manual model add works independently of a scan root;
- recursive scan from `C:\LlamaManager\artifacts\M2 模型 library with spaces` works with spaces and Unicode;
- the scan isolated one corrupt/error candidate without poisoning valid discovery;
- byte-identical copies resolved to one content identity with multiple known locations;
- the text model displayed `PRESENT` / `COMPATIBLE` with evidence-backed compatibility reasons;
- after all present copies were moved/removed and paths refreshed, the identity changed to `MISSING` and `RELINK` surfaced;
- relink to `C:\LlamaManager\artifacts\M2 moved 模型\stories moved.gguf` succeeded by matching content identity;
- the UI returned to `PRESENT` while retaining old missing locations as evidence;
- after closing and reopening LlamaWave, the relinked location/library/compatibility state persisted;
- normal and narrow desktop layouts remained usable and readable.

The complete evidence record and limitations are in `docs/evidence/M2_MODEL_LIBRARY_2026-08-18.md` and issue #21.

### Limitations retained

The interactive model was text-only, so no interactive mmproj claim is made; multimodal projector logic is covered by deterministic automated tests. No separate numeric large-library throughput bound is claimed. The prior upstream llama-bench Unicode-path limitation remains documented separately and does not prevent M2 scanning/inspection from handling Unicode paths.

### Result

M2 has closure-grade implementation, automation, real Windows runtime/UI evidence, persistence, failure handling, and truthful compatibility presentation. The remaining #22 work is the final documentation/regression/promotion gate before #2 closes and M3 is formally released.

---

## 2026-08-18 — M1 C5 interactive GUI benchmark closure

### Goal

Exercise the one remaining human-operated M1 acceptance path: launch a real benchmark from the native GUI against the pinned prepared runtime/model and verify the displayed invocation, measured result, and persisted history.

### Verified

The repository owner ran the release application on an interactive Windows desktop from source commit `5a5ebb3fad92f7a25ad4f1f38822d03a48214e30` after preparing the stack with `scripts/prepare-m1-gui-benchmark.ps1`.

The GUI selected:

```text
runtime: C:\LlamaManager\artifacts\m1-gui-benchmark\llama cpp runtime with spaces
model:   C:\LlamaManager\artifacts\m1-gui-benchmark\Model Files with spaces\stories 15M benchmark.gguf
```

The Benchmark view exposed the real `llama-bench.exe` command with `-m <model> -r 3 -o json`, executed it from the GUI, reported completion with raw/parsed evidence retained, and displayed:

```text
pp512  41297.19 tok/s
tg128   3504.32 tok/s
```

The History view showed one persisted SQLite record with matching prompt/decode metrics. The Overview showed runtime detected, RPC backend, model ready, 433 discovered capabilities, one persisted run, and measured benchmark state.

Three 1920×1032 operator screenshots were captured; their SHA-256 digests and the full verification record are stored in `docs/evidence/M1_GUI_BENCHMARK_2026-08-18.md`.

The source commit exercised by the operator had already passed standard Windows CI run `32094641702` and permanent real Windows Runtime Validation run `32094641715`.

### Result

The final G8 GUI-interaction evidence gap is closed. Together with the previously recorded G1–G7 and G9–G10 evidence, M1 reaches **C5 — Complete** and #16/#1 are closure-ready. The M2 dependency gate may now be released.

---

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

M1 was truthfully held at **C4 — Runtime verified** at this checkpoint because a real benchmark had not yet been launched through the interactive GUI path. That final interaction was subsequently completed and is recorded in the C5 entry above.

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