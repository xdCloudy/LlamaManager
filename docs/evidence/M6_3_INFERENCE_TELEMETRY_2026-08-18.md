# M6.3 Inference Telemetry — Real Runtime Evidence

Date: 2026-08-18
Issue: #46
PR: #137
Validated source/workflow head: `61195b15b756043b73714e57c677ad1a5ab78fed`

## Scope

M6.3 adds typed ingestion for real per-request inference telemetry and client-observed streaming latency evidence. It does not claim GPU telemetry, long-term time-series storage/charts, alerts, or the final rendered telemetry UI; those remain separate M6 work.

## Truthfulness model

`src/inference_telemetry.rs` retains metric state, unit, source field/provider, observation timestamp, request identity, endpoint, server PID, requested model, and reported model.

Metrics distinguish live, unavailable, error, and stale evidence. Missing or JSON-null fields remain unavailable. Malformed/version-shifted fields become errors rather than zero values. Disconnect/restart marks prior live request metrics stale while retaining their last observed timestamp/value.

The parser treats llama.cpp `timings.draft_*` fields as **generic speculative-decoding evidence**. They are not labeled MTP merely because draft counters exist. MTP-specific generated/accepted/acceptance/mean-run fields become eligible only when `generation_settings["speculative.types"]` explicitly identifies an MTP mode. A non-MTP mode such as `none` or `draft` leaves the MTP projection unavailable.

Per-request prompt/decode rates are consumed directly from the same response rather than being derived from cross-request cumulative counter deltas, so a new request or server identity cannot inherit a previous rate baseline.

## Automated verification

Normal CI #273 (`32152424078`) passed on `61195b15b756043b73714e57c677ad1a5ab78fed`:

- PowerShell syntax
- `cargo fmt --all -- --check`
- `cargo check --all-targets`
- `cargo test --all-targets`
- strict Clippy with warnings denied
- `cargo build --release`
- desktop process smoke
- portable bundle assembly/upload

Fixture coverage includes:

- native prompt/decode timing fields and request/model/server identity;
- generic speculative counters without MTP mislabeling;
- explicit MTP-mode projection;
- invalid speculative acceptance ratio;
- missing/null metrics;
- malformed metric type with unaffected sibling metrics preserved;
- version-shifted `timings` payload shape;
- invalid JSON/non-object root;
- accepted speculative tokens exceeding generated tokens;
- disconnect-to-stale transition;
- new request/server identity without counter-delta carryover.

## Real streaming llama.cpp verification

Real Windows Runtime Validation #49 (`32152424178`) exercised pinned llama.cpp b10472 and the published hash-pinned `stories 15M benchmark.gguf` through a real managed `llama-server`.

The test launched the server under the existing Windows Job Object supervisor, waited for health + minimal inference readiness, then issued a real streaming `/completion` request and measured first-token arrival from the client transport. The final SSE event supplied llama.cpp timing evidence for the same request.

Observed request evidence:

```text
HTTP status:             200
SSE events:              5
server PID:              8756
TTFT:                    3.0915 ms
request latency:         6.6042 ms
prompt tokens:           5
cached prompt tokens:    1
decode tokens:           4
context usage:           10 tokens
prompt throughput:       2378.686964795433 tok/s
decode throughput:       908.5402786190187 tok/s
reported speculative:    none
```

The final b10472 response did not expose context-capacity, batch-size, or KV-cache occupancy fields, so those metrics remained explicitly unavailable. It reported `generation_settings["speculative.types"] = "none"`; therefore generic speculative counters and all MTP-specific counters/rates remained unavailable rather than being emitted as zeros.

The request identity retained the endpoint, server PID, requested model path, and llama.cpp-reported model path for association with the measured TTFT/latency and server timings.

After the inference test, the permanent runtime workflow continued through the existing router discovery, router operations, A → B → A switching, restart reconciliation, evidence summary, and artifact upload regression path successfully.

Evidence artifact:

```text
name:   real-windows-runtime-evidence
id:     9330579811
digest: sha256:c6773411281439771965f78d9d992f45d8f45a0153a88549350d432e1bc32c96
```

## Result

M6.3 has closure-grade parser/state semantics, request/server/model association, real prompt/decode throughput, real client-observed TTFT/request latency, explicit partial/unavailable state, speculative-vs-MTP separation, disconnect staleness handling, strict CI, and real Windows llama.cpp evidence.

Context capacity, batch state and KV occupancy were truthfully unavailable in the pinned response and are not inferred. MTP runtime counters were not exercised because the pinned validation server reported speculative mode `none`; the MTP parser/projection path is fixture-covered and gated on explicit MTP mode evidence rather than generic draft counters.