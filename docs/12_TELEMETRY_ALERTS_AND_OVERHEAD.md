# Telemetry alerts and overhead policy

This document defines the Milestone 6 alert semantics and the engineering budget used to measure the monitor's own cost.

## Alert evidence contract

An alert is a policy evaluation, not a hardware fact. LlamaManager does not ship universal temperature, utilization, memory, or throughput thresholds from this layer. A configured alert rule must name all of the evidence needed to explain the decision:

- rule ID
- metric
- source provider
- source API
- stable series identity
- unit
- comparator
- trigger threshold
- clear threshold
- evaluation window
- minimum live sample count
- debounce interval
- severity
- human-readable reason

The event history retains the live samples that satisfied the trigger or clear window. History is bounded so alert evidence cannot grow memory without limit.

## Truthful data-state rules

Only a finite value whose support state is `supported` and whose telemetry state is `live` may advance a trigger or clear window.

The following states suppress normal live alert evaluation:

- stale
- unavailable
- error
- disconnected
- paused
- reset
- a live sample without a finite value

Suppression is not resolution. If an alert was already active and its source becomes stale or disconnected, the presentation state becomes `suppressed` while the internal active state is retained. A later live sample resumes evaluation from the real state instead of inventing a clear event during the evidence gap.

## Hysteresis and debounce

Above-threshold alerts require:

`clear < trigger`

Below-threshold alerts require:

`clear > trigger`

A threshold must remain satisfied for the configured window and minimum live-sample count before a transition occurs. A debounce interval prevents a new transition immediately after the previous one. Pending windows are discarded when their threshold condition stops being true or when telemetry stops being live.

User-adjusted thresholds are revalidated before they are accepted. Invalid hysteresis, non-finite values, empty evidence fields, zero windows/sample counts, or thresholds outside a configured metric range are rejected. Updating a threshold resets pending per-rule state so evidence gathered under the previous threshold is never reused under the new policy.

## Severity policy

Severity describes product policy and must not be presented as a vendor guarantee:

| Severity | Meaning |
| --- | --- |
| `info` | Advisory or observational condition; no immediate corrective action is implied. |
| `warning` | Sustained configured condition associated with likely pressure, degradation, or a user-defined operating concern. |
| `critical` | Sustained configured condition the policy owner considers likely to cause failure, service loss, or violation of an explicitly defined operating limit. |

Changing severity does not change the metric evidence. A rule's severity, thresholds, source and reason remain visible together.

## Telemetry overhead budget

Milestone 6 uses the following **engineering targets**, not claims about all hardware:

| Metric | Idle target | Active-inference target |
| --- | ---: | ---: |
| LlamaManager monitor CPU, normalized to total host CPU capacity | <= 1% | <= 2% |
| Peak private-memory growth during measurement | <= 64 MiB | <= 64 MiB |
| p95 polling work as fraction of sampling cadence | <= 25% | <= 25% |

CPU is measured from the LlamaManager/test process's Windows `GetProcessTimes` kernel+user counters and divided by elapsed wall time and logical processor count. This intentionally measures the monitor process rather than charging llama.cpp inference CPU to LlamaManager.

Private bytes and working set are sampled with Windows process memory counters. Poll latency measures the actual CPU/RAM + NVIDIA NVML telemetry polling work. The real validation uses a one-second polling cadence.

If a target is exceeded, the evidence remains valid; the result must be disclosed as a budget violation rather than rounded away or converted to a pass. The budget can only be considered satisfied after the real Windows idle and active-inference harness has been run on representative hardware.

## Automated verification

Normal CI covers deterministic behavior:

- threshold and range validation
- above/below hysteresis requirements
- sustained trigger windows
- clear windows
- debounce
- stale/unavailable/error suppression
- active-alert suppression without false resolution
- stable identity isolation
- bounded alert history
- CPU normalization math
- p95 polling math
- named budget violations
- counter/clock regression rejection

The real overhead test is intentionally ignored by normal CI because it requires a real Windows llama.cpp installation and GGUF model.

## Real Windows overhead evidence

Required environment variables:

- `LLAMAMANAGER_REAL_LLAMA_ROOT` — directory containing the real llama.cpp tools, including `llama-bench.exe`
- `LLAMAMANAGER_REAL_BENCH_MODEL` — real GGUF used by `llama-bench`
- `LLAMAMANAGER_REAL_EVIDENCE_DIR` — output directory for retained evidence

Optional:

- `LLAMAMANAGER_TELEMETRY_OVERHEAD_SECONDS` — duration of each phase; default 10 seconds, clamped to 3–120 seconds
- `LLAMAMANAGER_LLAMA_RELEASE_TAG` — release identity recorded in the evidence when available

From Command Prompt:

```cmd
set LLAMAMANAGER_REAL_LLAMA_ROOT=C:\path\to\llama.cpp
set LLAMAMANAGER_REAL_BENCH_MODEL=C:\path\to\model.gguf
set LLAMAMANAGER_REAL_EVIDENCE_DIR=%CD%\artifacts\real-runtime\telemetry-overhead
cargo test --test real_telemetry_overhead -- --ignored --nocapture
```

The harness performs two phases:

1. idle monitoring with the native Windows hardware provider and NVIDIA NVML provider;
2. the same monitoring while real `llama-bench` inference executes repeatedly in a child process.

It writes:

`telemetry-overhead.json`

The JSON retains the exact llama-bench SHA-256, model SHA-256, phase duration, polling cadence, budget policy, idle measurement, active-inference measurement, completed benchmark count, and named budget violations if any.

Issue #48 must remain open until this real evidence exists and the result is either within budget or the measured over-budget behavior is explicitly accepted and disclosed in the product/milestone evidence.
