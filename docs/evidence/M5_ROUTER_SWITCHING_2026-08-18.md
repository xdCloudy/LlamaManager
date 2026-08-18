# M5 Router + Model Switching — C5 Evidence

Date: 2026-08-18
Milestone: #5
Promotion gate: #43
Implementation/UX closure: #42 / PR #134
Merged implementation: `7626ca44d16f2b3b2b51e575fc2f09db19099b65`
Validated PR head: `a79e6445c0d520a7f6517555860f1b13f73ae31a`

## Scope

Milestone 5 delivers evidence-backed llama.cpp router discovery, real model load/unload/preload/switch operations, truthful residency/active-request/alias observability, A → B → A switching benchmarks, failure/recovery evidence, and a desktop Router Control surface with restart/reconnect reconciliation.

## Automated verification

Normal CI #255 (`32143778017`) passed on the final implementation head:

- PowerShell syntax
- `cargo fmt --all -- --check`
- `cargo check --all-targets`
- `cargo test --all-targets`
- strict Clippy with warnings denied
- `cargo build --release`
- desktop process smoke
- portable bundle assembly/upload

Real Windows Runtime Validation #43 (`32143777993`) also passed against pinned llama.cpp b10472 and published GGUFs, including:

- selected runtime/source gates
- real llama-server readiness and inference
- router discovery and canonical registry evidence
- real load/unload/preload/switch operations
- A → B → A switch benchmark
- restart/reconnect reconciliation
- evidence artifact generation

## Truthfulness and recovery

The final source-review hardening verifies that:

- endpoint/runtime/auth/LAN identity changes invalidate retained live authorization before mutation;
- runtime-evidence refresh and live reconciliation are serialized with router work;
- duplicate reconciliation dispatches cannot race the canonical snapshot;
- successful mutation plus failed reconciliation is reported as stale/unreconciled rather than false success;
- disabled router actions expose visible support/capability reasons;
- an unexpected operation-worker panic becomes retained recoverable failure evidence instead of permanently wedging the UI busy state.

The restart evidence records the preferred target as `Verified` before restart, `NeedsLiveReconciliation` immediately after disconnect, and `NotReady / Unloaded` after the restarted router is actually rediscovered. No stale ready state survives reconnect.

Pinned llama.cpp b10472 does not expose a supported dynamic default-model mutation route. LlamaManager therefore persists only the local preferred target and never presents persistence alone as proof of router readiness.

## Interactive Windows verification

The repository owner built the exact final PR head from source and inspected Router Control at normal and narrow desktop sizes on Windows on 2026-08-18.

The rendered layout was reported good with no blocking overlap, clipping, unusable controls, or other layout/UX issue. This closes the human visual gate in #42.

## C5 result

M5 satisfies G1–G10:

- G1 Scope — milestone/child acceptance is explicit.
- G2 Implementation — router discovery, operations, observability, switching benchmark, and management UI are integrated.
- G3 Automated verification — unit/integration/real-router regression coverage is green.
- G4 CI — CI #255 is green end-to-end.
- G5 Real runtime — Real Windows Runtime Validation #43 is green end-to-end.
- G6 Failure & recovery — failed operation, cancellation, reconciliation failure, and worker-panic recovery remain explicit.
- G7 Persistence & reproducibility — preferred target and benchmark/evidence envelopes persist without claiming stale readiness.
- G8 UX truthfulness — stale/unknown/unsupported state remains explicit and owner visual verification passed.
- G9 Docs & evidence — this evidence record, `BUILD_STATUS.md`, `WORKLOG.md`, and `10_COMPLETION_MATRIX.md` are updated for promotion.
- G10 Regression & cleanup — final strict CI/runtime suites are green and no temporary promotion workflow is introduced.

Known limitation: dynamic default-model mutation is unsupported by the pinned b10472 router API and is recorded N/A rather than simulated.

With #38–#42 complete and this promotion evidence recorded, #43 may close #5 at C5 and unlock M6/#44.