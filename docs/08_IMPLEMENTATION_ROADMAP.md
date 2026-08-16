# Implementation Roadmap

This roadmap intentionally prioritizes complete vertical slices over broad scaffolding.

## Milestone 0 — Clean baseline

Deliver:

- Rust project
- Dioxus Desktop launches
- app shell
- typed errors
- tracing/logging
- portable/user-data path resolver
- SQLite connection + migration system
- minimal tests
- git initialized with first green checkpoint

Exit criteria:

```text
fmt PASS
check PASS
test PASS
clippy PASS/understood
app launches
```

No major feature code should be started before this.

---

## Milestone 1 — Real installation → real benchmark result

Deliver the first complete application path:

```text
select llama.cpp folder
→ detect llama-server/llama-bench
→ hash binaries
→ capture version/help
→ capability registry
→ select GGUF
→ inspect metadata
→ run llama-bench
→ parse metrics
→ save benchmark
→ show result in Performance UI
```

Exit criteria:

- no stub metadata
- no filename-based architecture guessing
- nonzero benchmark exit is an error
- exact invocation is visible
- result persists across restart

---

## Milestone 2 — Model library + compatibility

Deliver:

- recursive scan
- manual file add
- dedupe
- missing/moved file detection
- GGUF metadata inspector
- installation/model compatibility
- multimodal projector association

Exit criteria:

- arbitrary paths
- spaces/Unicode tested
- compatibility reasons visible
- unsupported architecture is not silently accepted

---

## Milestone 3 — models.ini

Deliver:

- parser
- comments/unknown keys preservation
- inheritance
- structured editor
- raw editor
- validation
- diff
- managed/external modes
- generator wizard

Exit criteria:

- round-trip tests
- heavily commented INI fixture
- backups
- invalid configuration cannot be silently applied

---

## Milestone 4 — Server lifecycle

Deliver:

- command builder
- start/stop/restart
- readiness
- health check
- minimal inference
- logs
- port detection
- Windows Job Object
- crash/exit reporting

Exit criteria:

- real server launches from GUI
- exact command shown
- spaces/Unicode paths work
- no shell concatenation
- failure state is clear

---

## Milestone 5 — Router + model switching

Deliver:

- router discovery
- model registry
- load/unload
- switch
- startup model
- LRU/residency visibility
- active request visibility where available
- switch benchmark

Exit criteria:

- A → B → A verified
- active-request eviction failure is handled/explained
- stop timeout/force-kill visible
- model aliases and routing behavior observable

---

## Milestone 6 — Live telemetry

Deliver:

- GPU
- VRAM
- CPU
- RAM
- inference metrics
- context
- prompt/decode TPS
- MTP metrics
- charts
- evidence-backed alerts

Exit criteria:

- no fake metrics
- stale/disconnected state is explicit
- monitoring overhead measured and bounded

---

## Milestone 7 — Benchmark laboratory

Deliver:

- quick/full
- prompt/decode
- context scaling
- thread sweep
- batch sweep
- KV sweep
- MTP
- load
- switching
- history
- compare

Exit criteria:

- reproducibility envelope complete
- statistics/variance shown
- raw samples retained
- failed runs remain failed

---

## Milestone 8 — Autotuner v1

Deliver:

- Stage 0 baseline
- Stage 1 threads/offload
- Stage 2 placement/memory
- adaptive search
- candidate persistence
- resume
- Pareto frontier
- evidence-backed profile creation

Exit criteria:

- tested against deterministic fake objective functions
- real tuning run beats or matches baseline without violating constraints
- every generated profile has evidence

---

## Milestone 9 — Advanced tuner

Deliver:

- load/lifecycle stage
- MTP stage
- agent/cache workload
- multimodal stage
- whole-profile validation
- regression detection after llama.cpp updates
- experimental feature framework
- llama-fit-params integration

---

## Milestone 10 — Product polish

Deliver:

- complete first-run wizard
- beginner/expert progressive disclosure
- vaporwave design pass
- accessibility/reduced motion
- portable release packaging
- import/export
- diagnostic bundle
- documentation
- update/regression workflows

---

## Feature discipline

Before adding a feature ask:

1. Does it belong to the current milestone?
2. Is there a real end-to-end path to test it?
3. Do we know the local llama.cpp capability?
4. Can we persist enough evidence to debug it later?
5. Can it fail safely?
6. Will the UI tell the truth about its state?

If not, do not build it yet.
