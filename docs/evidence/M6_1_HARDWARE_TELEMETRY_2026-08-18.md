# M6.1 Hardware Telemetry — Windows Runtime Evidence

Date: 2026-08-18
Issue: #44
PR: #136
Validated source/workflow head: `c041251d02a0d47c9dabf60b5572fd8a379b35eb`

## Scope

M6.1 establishes the typed hardware-provider foundation for live telemetry and measures CPU, system RAM, and managed-process memory from real Windows APIs. GPU telemetry, inference telemetry, time-series storage/charts, alerts, and the rendered telemetry UI remain scoped to #45–#49.

## Provider contract

`src/hardware_telemetry.rs` provides:

- `TelemetryState<T>` with distinct `Live`, `Unavailable`, `Error`, and `Stale` states;
- explicit units, source provider/API, and sample timestamps on every reading;
- bounded configurable cadence: 250 ms minimum, 60 s maximum, 1 s default;
- total and per-logical-processor CPU utilization from `NtQuerySystemInformation(SystemProcessorPerformanceInformation)` counter deltas;
- logical processor count from `GetActiveProcessorCount(ALL_PROCESSOR_GROUPS)`;
- physical core count from `GetLogicalProcessorInformationEx(RelationProcessorCore)`;
- total/available/used physical RAM from `GlobalMemoryStatusEx`;
- PID-scoped working set, peak working set, and private bytes from `OpenProcess` + `K32GetProcessMemoryInfo`.

The first CPU sample is explicitly unavailable because utilization requires a counter delta. A provider failure after a live reading becomes stale with the last observed value/time retained; a failure without prior live evidence remains an error. Missing/dead process identity is unavailable rather than a zero-memory reading.

## Automated verification

Normal CI #264 (`32149362109`) passed on `c041251d02a0d47c9dabf60b5572fd8a379b35eb`:

- PowerShell syntax
- `cargo fmt --all -- --check`
- `cargo check --all-targets`
- `cargo test --all-targets`
- strict Clippy with warnings denied
- `cargo build --release`
- desktop process smoke
- portable bundle assembly/upload

Unit coverage includes cadence bounding, CPU counter-delta arithmetic, counter rollback, checked RAM arithmetic, stale carry-forward, error-without-prior evidence, Windows topology/RAM sanity, current-process memory, and variable-size/truncated topology records.

## Real Windows runtime verification

Real Windows Runtime Validation #45 (`32149362116`) passed end-to-end on `windows-2025`.

The dedicated hardware validation:

1. sampled the native provider with the test process PID;
2. verified the first CPU sample was unavailable rather than fabricated;
3. sampled again after the configured 300 ms interval;
4. verified live total/per-logical CPU values were within 0–100%;
5. verified logical/physical topology and RAM invariants;
6. verified live PID-scoped working-set/peak/private memory;
7. wrote `hardware-telemetry.json` with units, source APIs, timestamps, and state envelopes;
8. independently cross-checked topology and total RAM through Windows CIM (`Win32_Processor` / `Win32_OperatingSystem`).

Observed provider sample:

```text
CPU total:                  5.263157894736842 %
CPU logical processor 0:    0.0 %
CPU logical processor 1:    5.263157894736842 %
CPU logical processor 2:    0.0 %
CPU logical processor 3:   15.789473684210526 %
Logical processors:         4
Physical cores:             2
Total physical RAM:         17,174,360,064 bytes
Available physical RAM:     13,730,189,312 bytes
Used physical RAM:           3,444,170,752 bytes
Measured PID:               3708
Working set:                 4,968,448 bytes
Peak working set:            4,968,448 bytes
Private bytes:                 946,176 bytes
```

Independent CIM evidence:

```text
Provider logical processors: 4
CIM logical processors:      4
Provider physical cores:     2
CIM physical cores:          2
Provider total RAM:          17,174,360,064 bytes
CIM total visible RAM:       17,174,360,064 bytes
Total-RAM delta:             0 bytes
Allowed total-RAM tolerance: 858,718,003 bytes
```

The runtime job then continued through the existing pinned llama.cpp/GGUF/server/router suite; all source gates, runtime/inference, router discovery/operations, A → B → A switching, restart reconciliation, evidence summary, and artifact upload passed.

Evidence artifact:

```text
name:   real-windows-runtime-evidence
id:     9329363691
digest: sha256:d203ba02439abde09dd6a2ba7b10e9182078a93037dee7a1881ebfa1fa34c2ee
```

The first version of the CIM check incorrectly read the internally tagged `TelemetryState` JSON one level too shallow. The native provider test itself passed. Commit `c041251d02a0d47c9dabf60b5572fd8a379b35eb` corrected the evidence reader to use `.state.state` / `.state.value`; the rerun then passed both the provider and independent cross-check. No provider value was changed to make the check pass.

## Result

M6.1 has closure-grade source, automated, Windows runtime, independent-system-view, failure-state, unit/provenance, and regression evidence for the CPU/RAM/process provider layer.

No claim is made here for GPU telemetry, inference metrics, long-term time-series persistence, alerts, or rendered telemetry UI. Those remain later M6 work.