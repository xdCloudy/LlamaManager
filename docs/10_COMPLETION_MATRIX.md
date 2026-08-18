# Milestone Completion Matrix

This document is the authoritative completion contract for LlamaManager milestones.

The roadmap defines **what** to build. This matrix defines **how much evidence is required before a milestone can be called complete**.

A GitHub milestone issue must not be closed merely because code exists or CI compiles. Closure requires the milestone to reach **C5** and satisfy every applicable gate below.

## Maturity levels

| Level | Name | Required evidence |
|---|---|---|
| **C0** | Backlog | Idea exists, but scope/acceptance criteria are not yet complete. |
| **C1** | Scoped | Deliverables, constraints, acceptance criteria, dependencies, and test strategy are written. |
| **C2** | Implemented | The intended end-to-end feature path exists in source with no fake-success path or knowingly dead control. |
| **C3** | Automated green | Formatting, compilation, tests, strict Clippy, and relevant automated/integration checks are green. |
| **C4** | Runtime verified | The feature has been exercised against the real Windows/runtime/hardware/llama.cpp conditions it claims to support. |
| **C5** | Complete | All applicable completion gates are satisfied, evidence is recorded, docs are current, regressions are checked, and the milestone issue is closure-ready. |

### Promotion rules

1. **CI alone can never promote a runtime feature beyond C3.**
2. C4 requires real execution, not mocks, compile evidence, or a process-start-only smoke test unless process start is the entire claimed behaviour.
3. C5 requires every applicable global gate plus the milestone-specific criteria in this document and its GitHub issue.
4. A gate may be marked `N/A` only with a written reason in the issue.
5. Partial work for a future milestone does not promote that milestone unless its complete vertical path and evidence exist.
6. No fake success, placeholder metric, filename-based capability guess, swallowed failure, or silent fallback may be used to satisfy a gate.
7. Evidence must be reproducible enough that another developer can determine why a gate was marked complete.

## Global completion gates

| Gate | Requirement | Completion evidence |
|---|---|---|
| **G1 Scope** | Scope and acceptance contract are explicit. | Roadmap + issue + relevant design/architecture docs agree. |
| **G2 Implementation** | The end-to-end path exists. | Real source path is reachable; no disconnected scaffold or dead control. |
| **G3 Automated verification** | Behaviour has appropriate automated coverage. | Unit/integration/fixture tests cover happy path and important failure paths. |
| **G4 CI** | Repository quality gates are green. | `fmt --check`, `check --all-targets`, tests, strict Clippy, release build, plus milestone-specific automation. |
| **G5 Real runtime** | Claimed behaviour works in its real environment. | Windows/runtime/hardware/llama.cpp execution evidence, not inference from source alone. |
| **G6 Failure & recovery** | Expected failures are explicit and safe. | Non-zero exits/errors remain errors; cancellation/rollback/retry/recovery are tested where applicable. |
| **G7 Persistence & reproducibility** | Durable state and evidence survive as claimed. | Restart/relocation/resume/history/raw evidence verified where applicable. |
| **G8 UX truthfulness** | UI state matches reality. | No fake metrics, stale state is marked, exact commands/reasons/errors are visible where relevant, no dead controls. |
| **G9 Docs & evidence** | Documentation reflects implementation. | Docs, work log, compatibility notes, limitations, and verification evidence updated. |
| **G10 Regression & cleanup** | The tranche leaves the repository healthier. | Relevant prior paths still pass; no temporary workflow/hack/stub remains; new warnings/debt are addressed or explicitly tracked. |

## Current matrix

Legend: `✅` satisfied, `🟡` implemented/partially evidenced but not closure-grade, `⬜` not yet satisfied.

| Milestone | Issue | Maturity | G1 | G2 | G3 | G4 | G5 | G6 | G7 | G8 | G9 | G10 |
|---|---:|---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| **M0 Clean baseline** | — | **C5** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **M1 Real installation → benchmark** | #1 | **C5** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **M2 Model library + compatibility** | #2 | **C5** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **M3 `models.ini`** | #3 | **C5** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **M4 Server lifecycle** | #4 | **C5** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **M5 Router + switching** | #5 | **C5** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **M6 Live telemetry** | #6 | **C1** | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ |
| **M7 Benchmark laboratory** | #7 | **C1** | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ |
| **M8 Autotuner v1** | #8 | **C1** | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ |
| **M9 Advanced tuner** | #9 | **C1** | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ |
| **M10 Product polish** | #10 | **C1** | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ |

The matrix is evidence-based. A future milestone can contain incidental code without advancing beyond C1 if its own complete path has not been implemented and verified.

---

# Milestone-specific completion contracts

## M1 — Real installation → real benchmark result

**Current state:** C5 — Complete. Final interactive evidence: `docs/evidence/M1_GUI_BENCHMARK_2026-08-18.md`.

C5 requires all of the following:

- [x] Release build launches in an interactive Windows desktop session.
- [x] Dioxus UI is visually inspected at 1280×720 and a normal desktop resolution.
- [x] An arbitrary real `llama.cpp` installation outside the repository is selected successfully.
- [x] `llama-server` / `llama-bench` discovery works recursively and executable identity/hash evidence is correct.
- [x] Help/version/backend capability evidence comes from the selected binaries.
- [x] A real arbitrary GGUF is selected and metadata is read from the file rather than inferred from its filename.
- [x] A real `llama-bench` run completes through the GUI path.
- [x] Exact executable + argv invocation is visible/recoverable.
- [x] Non-zero benchmark exit remains a failed run and cannot become fake success.
- [x] Raw stdout/stderr/exit status are retained.
- [x] Parsed metrics correspond to retained raw evidence.
- [x] Restarting the app preserves benchmark history.
- [x] Paths containing spaces are exercised; Unicode paths are exercised or a tracked limitation is recorded.
- [x] Relevant regression tests and strict CI remain green after runtime fixes.
- [x] `BUILD_STATUS.md` and `WORKLOG.md` record the runtime evidence and remaining limitations.

## M2 — Model library + compatibility

**Current state:** C5 — Complete. Final interactive evidence: `docs/evidence/M2_MODEL_LIBRARY_2026-08-18.md`.

C5 requires:

- [x] Recursive scan discovers real GGUF files from arbitrary user-selected roots.
- [x] Manual GGUF add works without requiring a scan root.
- [x] Duplicate models resolve deterministically using stable file identity/evidence.
- [x] Missing and moved files are detected and repair/relink flow is verified.
- [x] GGUF inspector exposes required architecture/context/quantization/tensor metadata from file contents.
- [x] Installation ↔ model compatibility report explains *why* a pairing is accepted, limited, or rejected.
- [x] Unsupported architecture is never silently accepted.
- [x] Capability decisions do not depend on model filenames.
- [x] Multimodal projector discovery/association is evidence-backed and ambiguity is surfaced.
- [x] Paths with spaces and Unicode are covered by automated fixtures and real Windows validation.
- [x] Library state survives restart and stale entries are represented truthfully.
- [x] Failure paths for corrupt/truncated/unreadable GGUFs are tested.
- [x] Docs and matrix are updated with observed compatibility limitations.

The interactive closure used a text-only model, so mmproj behaviour is closure-grade automated evidence rather than an interactive multimodal claim. No numeric large-library throughput bound is claimed. These scope limits are recorded in the M2 evidence file.

## M3 — `models.ini` parser, editor and generator

**Current state:** C5 — Complete. Final interactive/runtime evidence: `docs/evidence/M3_MODELS_INI_2026-08-18.md`.

C5 requires:

- [x] Parser handles global `[*]` defaults and per-model sections.
- [x] Comments, ordering where required, blank lines, and unknown keys survive round-trip edits.
- [x] Inheritance/effective-value calculation has automated fixtures.
- [x] Structured editor and raw editor operate on one canonical model and cannot silently diverge.
- [x] Validation blocks invalid configuration from being applied.
- [x] Pre-apply diff clearly shows effective changes.
- [x] Managed config mode and external-file mode are both verified.
- [x] External-file writes create recoverable backups before mutation.
- [x] Restore path is tested against a deliberately bad edit.
- [x] Profile generator emits only capabilities supported by the selected runtime evidence.
- [x] Heavily commented, unknown-key, CRLF, spaces, and Unicode fixtures round-trip successfully.
- [x] No full-file destructive rewrite occurs when a minimal safe edit is possible.
- [x] Restart preserves the selected/managed configuration state.

## M4 — Managed `llama-server` lifecycle

**Current state:** C5 — Complete. Final interactive/runtime evidence: `docs/evidence/M4_SERVER_LIFECYCLE_2026-08-18.md`.

C5 requires:

- [x] Command builder uses executable + argv, never shell-concatenated command strings.
- [x] Start, stop, restart, readiness, health, and minimal inference verification work against a real server.
- [x] Exact command is shown to the user and retained for diagnostics.
- [x] Unsupported flags are blocked from launch based on discovered capability evidence.
- [x] Port collisions are detected before/at launch with an actionable error.
- [x] Stdout/stderr are streamed and retained without hiding fatal output.
- [x] Normal stop, timeout, force-kill, crash, and failed-start states are distinguishable.
- [x] Windows Job Object/process-tree supervision is verified so managed children do not leak.
- [x] Paths containing spaces and Unicode work.
- [x] App restart/recovery correctly represents an already-running or previously-crashed process.
- [x] Minimal inference proves readiness is not merely "process exists".
- [x] CI plus real Windows lifecycle regression checks are green.

## M5 — Router + model switching

**Current state:** C5 — Complete. Final automated/runtime/interactive evidence: `docs/evidence/M5_ROUTER_SWITCHING_2026-08-18.md`.

C5 requires:

- [x] Router availability and supported operations are discovered from real evidence.
- [x] Model registry reflects actual loaded/resident state.
- [x] Load, unload, reload, preload, and switch paths are verified where supported.
- [x] Startup model behaviour is verified after restart; dynamic default-model mutation is unsupported/N/A on pinned b10472, and preferred-target readiness is re-derived from live post-restart evidence.
- [x] Residency/LRU state is visible and not guessed; unavailable evidence remains explicit.
- [x] Active-request state is shown where available and unknown is explicit where unavailable.
- [x] A → B → A switching is verified with real compatible models.
- [x] Active-request eviction failure is handled without corrupting router state where evidence is available; pinned b10472 does not expose active-request evidence, so no unsupported active-request claim is invented.
- [x] Stop timeout and force-kill behaviour remain visible through the verified managed-server lifecycle used by the router path.
- [x] Aliases and routing targets are observable from the UI/diagnostics.
- [x] Switching benchmark records unload/load/readiness/first-token timings with reproducibility evidence.
- [x] Failure/recovery after a bad model load is verified.

## M6 — Live hardware and inference telemetry

C5 requires:

- [ ] GPU utilization, VRAM, temperature, clocks, and power are sourced from real supported APIs where available.
- [ ] CPU total/per-core/topology data is sourced from real system evidence.
- [ ] RAM and managed-process memory are measured rather than estimated.
- [ ] Prompt TPS, decode TPS, TTFT, request latency, context, and batch state have explicit provenance.
- [ ] MTP generated/accepted/acceptance-rate/mean-run metrics are shown only when supported and observed.
- [ ] Unsupported metrics render as unavailable, never zero/fake values.
- [ ] Stale/disconnected telemetry is clearly distinguishable from live data.
- [ ] Sampling frequency and telemetry overhead are measured and bounded.
- [ ] Charts handle gaps/reconnects without fabricating interpolation presented as measured truth.
- [ ] Alerts cite the actual evidence/threshold that triggered them.
- [ ] Telemetry survives server restart/reconnect correctly.
- [ ] Unit conversion/range/overflow edge cases have tests.

## M7 — Benchmark laboratory and comparisons

C5 requires:

- [ ] Quick and full presets run end-to-end.
- [ ] Prompt/decode/context-scaling benchmarks are supported with explicit workload definitions.
- [ ] Thread, batch, KV, MTP, loading, and switching sweeps retain exact candidate configurations.
- [ ] Benchmark history survives restart and supports side-by-side comparison.
- [ ] Raw samples, stdout/stderr, exit status, binary/model identity, and environment envelope are retained.
- [ ] Failed runs remain failed and are excluded from "winner" logic unless explicitly inspected.
- [ ] Warmup/repetition policy is documented and encoded.
- [ ] Variance/statistics are shown for repeated samples.
- [ ] Meaningful-change/confidence handling prevents measurement noise being declared a win.
- [ ] Re-running a stored benchmark definition is possible or all differences are disclosed.
- [ ] Context/memory failures are represented explicitly rather than discarded.
- [ ] Comparison UI never mixes incompatible workloads without warning.

## M8 — Adaptive autotuner v1

C5 requires:

- [ ] Stage 0 records a reproducible baseline.
- [ ] Stage 1 explores threads/offload within discovered capability and safety constraints.
- [ ] Stage 2 explores placement/memory parameters without violating hard limits.
- [ ] Search is coarse-to-fine/adaptive rather than an unbounded blind Cartesian sweep.
- [ ] Candidate inputs/results/rejections are persisted.
- [ ] Interrupted tuning resumes without repeating completed work unnecessarily.
- [ ] Pareto frontier is computed from measured objectives and constraints.
- [ ] Deterministic fake-objective tests validate search/convergence/constraint logic.
- [ ] A real tuning run matches or beats baseline according to the declared objective.
- [ ] Quality/capability constraints cannot be traded away silently for speed.
- [ ] Every generated profile links back to measured evidence.
- [ ] Applying a generated profile requires validation and offers safe rollback.

## M9 — Advanced tuning, cache/MTP/multimodal and regression framework

C5 requires:

- [ ] Loading/lifecycle tuning measures real load/readiness/unload/switch behaviour.
- [ ] MTP/speculative stage tunes only flags confirmed by runtime capability evidence.
- [ ] Append-heavy agent/context-cache workload measures the workload it claims to optimize.
- [ ] Multimodal/mmproj tuning preserves functional vision unless the user explicitly disables it.
- [ ] Whole-profile validation reruns representative workloads after local stage optimization.
- [ ] Experimental flags are isolated behind capability/version/evidence tracking.
- [ ] `llama-fit-params` integration is used only where the selected installation actually supports it.
- [ ] Binary hash/version, relevant driver/hardware identity, and model identity invalidate or mark stale old profiles when necessary.
- [ ] Regression detection compares like-for-like benchmark envelopes.
- [ ] Stale profile state is clearly visible and cannot masquerade as current evidence.
- [ ] A regression can be traced to stored before/after evidence.
- [ ] Advanced tuning cannot silently reduce required context, multimodal, MTP, KV-quality, or other declared hard constraints.

## M10 — Product polish, portable release and diagnostics

C5 requires:

- [ ] First-run wizard succeeds for a clean Windows user with no existing LlamaManager state.
- [ ] Beginner and expert modes expose progressive detail without changing underlying truth/behaviour.
- [ ] Final vaporwave design pass matches the documented restrained design system.
- [ ] Keyboard navigation, readable focus state, contrast, and reduced-motion behaviour are verified.
- [ ] Normal use requires no Rust/Python/Node/npm/PowerShell-module developer runtime.
- [ ] Portable bundle works after relocation to a different writable folder.
- [ ] Import/export round-trips supported application state safely.
- [ ] Diagnostic bundle contains useful evidence while redacting secrets/API keys/private values defined by policy.
- [ ] Update/regression workflow preserves rollback/recovery.
- [ ] UI is verified at documented minimum and normal desktop resolutions.
- [ ] No dead controls, placeholder metrics, fake progress, or knowingly misleading success states remain.
- [ ] Release documentation covers installation, first run, portability, backup/restore, diagnostics, and limitations.
- [ ] Release artifact is built from a green tagged commit and passes final smoke/runtime checks.

---

## Issue closure template

Before closing any milestone issue, record:

```text
Maturity: C5
Global gates: G1-G10 PASS or justified N/A
CI evidence: <run/commit>
Runtime evidence: <environment + result>
Regression evidence: <tests/workloads>
Docs updated: <paths>
Known limitations: <none or explicit list>
```

If that evidence cannot be written truthfully, the issue is not complete.