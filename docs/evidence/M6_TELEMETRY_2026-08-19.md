# M6 Telemetry Runtime Evidence — 2026-08-19

Issue: #49

This record captures verified interactive telemetry behaviour from the owner's real Windows workstation. It supplements the automated and Real Windows Runtime Validation evidence already linked from #49; it does not claim visual/runtime checks that were not actually exercised.

## Live passive llama.cpp telemetry

The owner attached the production TELEMETRY workspace to the stable llama.cpp router endpoint at `127.0.0.1:8080`. LlamaWave automatically resolved the active router child and displayed:

```text
model:                  Qwen3.8-27B
router endpoint:        127.0.0.1:8080
resolved child source:  127.0.0.1:50973
speculative mode:       draft-mtp
prompt rate:            79.90 tok/s
 decode rate:            6.67 tok/s
requests processing:    0
requests deferred:      0
prompt tokens total:    98,505
 decode tokens total:    17,329
busy slots / decode:    1.00
```

The passive monitor banner remained `PASSIVE LIVE`. The owner confirmed the previously observed stale-output problem was resolved after #161: transient child-port races/timeouts recover before stale fallback where possible, while retained stale values remain available if recovery ultimately fails.

## Truthful unsupported state

The same rendered workspace visibly showed `Unavailable` / `UNAVAILABLE` for runtime counters not exported by the selected llama.cpp `/metrics` response:

- cached prompt total;
- MTP drafted total;
- MTP accepted total;
- MTP acceptance;
- speculative drafts total.

Each unavailable card states that the current `/metrics` response did not expose the field and that no zero was synthesized. This is the intended truthfulness contract: absence of evidence is not presented as a measured zero.

Although router args identify the runtime as `draft-mtp`, the selected llama.cpp build did not export the corresponding MTP counters through `/metrics` in this capture. LlamaWave therefore does not infer draft/accepted counts from unrelated logs or process state.

## Request-bound evidence limitation

No successful request-bound 4-token probe was captured in this screenshot. TTFT, request latency, and exact per-request MTP timing evidence therefore remain unclaimed. This is expected while a one-slot model is busy; passive process/runtime metrics may remain live without consuming an inference slot.

The combined #49 acceptance item for real prompt/decode/TTFT/MTP request evidence remains open until the missing request-bound fields are actually exercised on the owner's runtime. Unsupported MTP `/metrics` counters are retained as an explicit runtime limitation rather than converted into fake telemetry.

## Existing automated/runtime evidence

PR #161 (`8a092642bf13ca1a7dfdc67df0bec11a7aed6265`) added resilient passive polling and metric-name compatibility. Final head `b9e3630b9439b5d55d3807d4da427f2d71255b12` passed:

- CI run `32290862239`;
- Real Windows Runtime Validation run `32290862444`.

Those runs cover strict Rust quality gates, release build/smoke/bundle, pinned real llama.cpp/GGUF runtime checks, telemetry overhead, real inference/streaming telemetry, router operations/switching/restart reconciliation, and deterministic recovery from an ephemeral child `/metrics` timeout.

## Still open for #49

The following acceptance remains intentionally open rather than inferred from this single wide-screen capture:

- real request-bound TTFT/MTP evidence on the owner's active runtime;
- deliberate server stop/disconnect rendered-state inspection;
- deliberate reconnect rendered-state inspection;
- telemetry history charts at both narrow and normal desktop sizes;
- interactive alert trigger/clear visual verification.
