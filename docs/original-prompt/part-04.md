Measure:

- final decode TPS
- draft acceptance
- accepted/generated ratio
- mean accepted length
- wasted draft work

Optimize actual output throughput.


# 30. STAGE 5 — AGENT / CACHE WORKLOAD

Explicitly benchmark append-heavy coding-agent workloads.

Generate realistic prompts with:

- large system prompt
- repository context
- tools
- conversation history
- tool calls
- tool results
- small appended requests

Tune:

- cache-prompt
- cache-ram
- ctx-checkpoints
- checkpoint-min-step
- cache-idle-slots
- kv-unified
- cache-reuse where supported
- SWA-related options where valid

Measure prefix reuse and incremental suffix cost.


# 31. STAGE 6 — MULTIMODAL

If mmproj exists:

compare projector CPU/GPU placement.

Measure:

- image preprocessing
- image request latency
- VRAM
- effect on normal text performance

Vision capability must remain intact.


# 32. STAGE 7 — WHOLE PROFILE

Validate:

- ORIGINAL
- OPTIMIZED
- SAFE OPTIMIZED
- MAX PERFORMANCE
- FAST LOAD
- FAST SWITCHING

Test:

- fresh prompt
- long prompt
- decode
- cached continuation
- MTP
- vision
- configured context
- startup
- first request
- sustained session


# 33. OPTIMIZATION OBJECTIVES

Support:

AGENT
- decode
- cache reuse
- long context
- TTFT
- stability

THROUGHPUT
- prompt + decode TPS

FAST LOAD
- startup + first usable request

FAST SWITCH
- model lifecycle latency

BALANCED

CUSTOM weighted objective

Retain Pareto-frontier profiles rather than one absolute winner.


# 34. PROFILE SYSTEM

Generate multiple profiles per model:

Model / Balanced
Model / Max Performance
Model / Low VRAM
Model / Fast Load
Model / Long Context
Model / Fast Switching

Each profile contains measured evidence.


# 35. EXPERIMENTAL FEATURES

Add:

OFF
SAFE
AGGRESSIVE

Experimental features may include dynamically discovered:

- DirectIO
- no-host
- tensor overrides
- backend sampling
- speculative modes
- SWA options
- new KV mechanisms
- new offload strategies

Never silently enable risky options.

Show:

- potential benefit
- risk
- supported build
- benchmark evidence


# 36. STATISTICS

Do not treat:

45.483 > 45.473

as meaningful.

Support:

- repetitions
- mean
- median
- stddev
- coefficient of variation
- minimum meaningful improvement
- confidence-aware comparison

Tie-break using:

- lower VRAM
- lower RAM
- faster loading
- lower power
- simpler configuration

depending on objective.


# 37. ADAPTIVE SEARCH

Use:

- coarse-to-fine search
- successive halving
- local refinement
- early stopping
- dominance pruning
- Pareto pruning
- finalist verification

Learn from intermediate results.


# 38. BENCHMARK CACHE

Cache keys should include:

- model identity/hash
- model mtime
- llama.cpp binary hash
- backend
- driver
- hardware fingerprint
- configuration
- workload
- autotuner version

Changing relevant components invalidates results.


# 39. RESUMABLE AUTOTUNING

Persist:

- stage
- candidates
- completed experiments
- pending experiments
- results
- current winner

Runs must survive:

- app restart
- crash
- Windows reboot
- user cancellation

Provide:

Pause
Resume
Stop Safely


# 40. CONFIGURATION SNAPSHOTS

Every significant change should be reversible.

Store:

- timestamp
- model
- previous config
- new config
- benchmark evidence
- llama.cpp build
- hardware
- note

Support:

Compare
Restore
Clone Profile


# 41. CONFIG DIFF

Before applying:

[Model]

-n-gpu-layers = auto
+n-gpu-layers = 99

+n-cpu-moe = 22

Show measured evidence beside every change.


# 42. COMMAND PREVIEW

Show the exact effective llama-server invocation.

Allow:

- copy
- export PowerShell
- export CMD
- compare commands

Never hide what the app is executing.


# 43. LOG VIEWER

Provide:

- structured parsing
- severity filters
- regex search
- source filters
- auto-scroll
- pause
- export

Parse events including:
