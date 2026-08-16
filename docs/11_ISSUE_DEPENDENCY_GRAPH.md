# Issue Dependency Graph

This document is the execution map from the clean baseline to a production-ready v1.0 release.

The roadmap defines milestone intent. `10_COMPLETION_MATRIX.md` defines maturity and evidence. GitHub issues define executable work. This document defines **ordering and prerequisite relationships** between those issues.

## Execution policy

1. A milestone may be scoped in advance, but its implementation must not begin until the previous milestone's C5 promotion gate is closed.
2. Every issue containing `Blocked by: #...` is blocked until those referenced issues are closed with their acceptance evidence satisfied.
3. Work inside one milestone may run in parallel only where its issue blockers allow it.
4. A promotion gate is not implementation work. It is an independent evidence/regression pass that promotes the parent milestone to C5.
5. The parent milestone epic closes only after its promotion-gate issue passes.
6. Incidental code for a future milestone does not remove its blocker or advance its maturity by itself.
7. A prerequisite may be waived only by updating the affected issue with an explicit technical reason, replacement evidence, and corresponding matrix/docs change. Silent bypass is not allowed.
8. No release may be produced by skipping runtime, failure/recovery, persistence, visual, accessibility, or reproducibility gates that are applicable to the claimed functionality.

## Critical path

```text
#11  M0 Clean baseline — C5 / closed
  │
  ▼
#1   M1 Real installation → real benchmark
#12–#15 implementation/runtime verification
#16  M1 C5 promotion gate
  │
  ▼
#2   M2 Model library + compatibility
#17–#21 implementation/runtime verification
#22  M2 C5 promotion gate
  │
  ▼
#3   M3 models.ini
#23–#29 implementation/runtime verification
#30  M3 C5 promotion gate
  │
  ▼
#4   M4 Managed server lifecycle
#31–#36 implementation/runtime verification
#37  M4 C5 promotion gate
  │
  ▼
#5   M5 Router + model switching
#38–#42 implementation/runtime verification
#43  M5 C5 promotion gate
  │
  ▼
#6   M6 Live telemetry
#44–#49 implementation/runtime verification
#50  M6 C5 promotion gate
  │
  ▼
#7   M7 Benchmark laboratory
#51–#56 implementation/runtime verification
#57  M7 C5 promotion gate
  │
  ▼
#8   M8 Adaptive autotuner v1
#58–#64 implementation/runtime verification
#65  M8 C5 promotion gate
  │
  ▼
#9   M9 Advanced tuning + regression framework
#66–#72 implementation/runtime verification
#73  M9 C5 promotion gate
  │
  ▼
#10  M10 Product polish + release preparation
#74–#83 implementation/runtime/production verification
#84  M10 C5 promotion gate
  │
  ▼
#85  v1.0 production-readiness audit, tag and release
```

## Milestone issue map

| Milestone | Epic | Entry prerequisite | Child issues | Final gate | Unlocks |
|---|---:|---:|---|---:|---|
| M0 Clean baseline | #11 | — | baseline evidence in #11 | #11 closed C5 | M1 |
| M1 Installation → benchmark | #1 | #11 | #12–#15 | #16 | M2 |
| M2 Model library | #2 | #16 | #17–#21 | #22 | M3 |
| M3 `models.ini` | #3 | #22 | #23–#29 | #30 | M4 |
| M4 Server lifecycle | #4 | #30 | #31–#36 | #37 | M5 |
| M5 Router/switching | #5 | #37 | #38–#42 | #43 | M6 |
| M6 Telemetry | #6 | #43 | #44–#49 | #50 | M7 |
| M7 Benchmark laboratory | #7 | #50 | #51–#56 | #57 | M8 |
| M8 Autotuner v1 | #8 | #57 | #58–#64 | #65 | M9 |
| M9 Advanced tuner | #9 | #65 | #66–#72 | #73 | M10 |
| M10 Production polish | #10 | #73 | #74–#83 | #84 | v1.0 |
| v1.0 production release | #85 | #84 + epics #1–#10 C5 | final audit in #85 | #85 | production release |

## Detailed dependency graph

### M1 — Real installation → real benchmark

```text
#11
 ├─► #12 UI visual verification ──────────────┐
 ├─► #13 llama.cpp installation validation ─┐│
 └─► #14 GGUF validation ──────────────────┐││
                                           ▼▼│
                                         #15│ real llama-bench + persistence
                                           └┴─► #16 M1 C5 gate
```

#12, #13 and #14 may proceed independently after M0. #15 requires real installation and GGUF evidence. #16 requires all four validation tracks.

### M2 — Model library + compatibility

```text
#16 → #17 scan/add → #18 identity/relink → #19 compatibility → #20 multimodal
                         └──────────────┬───────────────┘
                                        ▼
                                      #21 UX/runtime validation
                                        ▼
                                      #22 M2 C5 gate
```

### M3 — `models.ini`

```text
#22 → #23 parser → #24 inheritance ───────┐
          │             │                 ├─► #26 structured/raw editor ─┐
          │             └─► #25 validation/diff ────────────────┐       │
          └────────────────────────────────► #27 safe writes ───┼─► #29 runtime validation
#19 compatibility ───────────────────────────► #28 generator ────┘       │
                                                                          ▼
                                                                        #30 M3 C5 gate
```

### M4 — Managed server lifecycle

```text
#30 → #31 command builder → #32 process supervisor ─┬─► #33 readiness/inference ─┐
                                                    └─► #34 logs/state ──────────┤
                                                                               ▼
                                                                             #35 lifecycle UI
                                                                               ▼
                                                                             #36 real lifecycle matrix
                                                                               ▼
                                                                             #37 M4 C5 gate
```

### M5 — Router + switching

```text
#37 → #38 discovery/registry → #39 operations → #40 residency/active state
                                      │                │
                                      └──────────────► #41 switching benchmark
                                                       │
                                                       ▼
                                                     #42 UI/reconciliation
                                                       ▼
                                                     #43 M5 C5 gate
```

### M6 — Telemetry

```text
#43 → #44 provider + CPU/RAM ─► #45 GPU ─────┐
        │                                    │
        └───────────────────► #46 inference ─┼─► #47 time-series/charts → #48 alerts/overhead
                                             │                              │
                                             └──────────────────────────────┴─► #49 live validation
                                                                                 ▼
                                                                               #50 M6 C5 gate
```

### M7 — Benchmark laboratory

```text
#50 → #51 schema/envelope → #52 canonical runner ─► #53 sweeps ───┐
          │                   └───────────────────► #54 statistics ├─► #55 history/compare UX
          └────────────────────────────────────────────────────────┘          │
                                                                               ▼
                                                                             #56 real validation
                                                                               ▼
                                                                             #57 M7 C5 gate
```

### M8 — Adaptive autotuner v1

```text
#57 → #58 objective/constraint harness → #59 baseline → #60 Stage 1 → #61 Stage 2
          │                                      │          │          │
          └──────────────────────────────────────┴──────────┴─────────► #62 orchestration/resume
                                                                         ▼
                                                                       #63 Pareto/profile/apply
                                                                         ▼
                                                                       #64 real tuning validation
                                                                         ▼
                                                                       #65 M8 C5 gate
```

### M9 — Advanced tuning

```text
#65 ─┬─► #66 lifecycle tuning ────────────────┐
     ├─► #67 MTP tuning ──────────────────────┤
     ├─► #68 agent/context-cache workload ────┤
     ├─► #69 multimodal tuning ───────────────┤
     └─► #70 experimental/fit-params ─────────┤
                                              ▼
                                            #71 staleness/regression
                                              ▼
                                            #72 whole-profile validation
                                              ▼
                                            #73 M9 C5 gate
```

Some M9 tracks also depend on earlier specialized foundations explicitly recorded in their issue bodies, such as MTP telemetry, multimodal association, and benchmark reproducibility.

### M10 — Production polish

```text
#73 ─┬─► #74 first-run → #75 progressive disclosure → #76 accessibility ─► #81 final visual QA ─┐
     ├─► #77 portable packaging ─► #78 import/export ────────────────────────────────────────────┤
     ├────────────────────────────► #79 diagnostics ──────────────────────────────────────────────┤
     └─► (#71 + #77) ─────────────► #80 update/regression ────────────────────────────────────────┤
                                                                                                  ▼
                                                                                                #82 release docs
                                                                                                  ▼
                                                                                                #83 clean-machine acceptance
                                                                                                  ▼
                                                                                                #84 M10 C5 gate
                                                                                                  ▼
                                                                                                #85 v1.0 release audit
```

## Production-ready definition

The project is **not production ready** because a release build exists or because CI is green.

Production readiness is achieved only when:

- M1–M10 parent epics are closed at C5;
- all promotion gates #16, #22, #30, #37, #43, #50, #57, #65, #73 and #84 have passed;
- #83 has validated the real product on a clean Windows environment;
- #85 has independently audited the same release candidate and its artifact;
- no critical/high-severity release-blocking defect remains open;
- the tested artifact, source commit, docs and release notes all describe the same product state.

Only then should `v1.0.0` be tagged and published.
