- Pin resident
- Unpin
- Set startup

Router-level configuration must remain separate from model-specific configuration.


# 19. MODEL SWITCHING

Benchmark actual router switching.

Measure:

- cold load
- warm load
- first request
- unload
- reload
- A → B
- B → A
- A → B → A

Track:

- load time
- first-token latency
- VRAM release
- RAM usage
- storage throughput
- process startup
- first inference latency

Tune:

- models-max
- autoload
- startup models
- stop-timeout
- load mode
- warmup
- repack
- residency strategy


# 20. DASHBOARD

Immediately show:

- active model
- llama.cpp status
- backend
- prompt TPS
- decode TPS
- TTFT
- VRAM
- RAM
- GPU utilization
- context usage
- cache reuse
- MTP acceptance
- model health
- tuning status

Use live graphs and compact metric cards.


# 21. LIVE MONITOR

GPU:

- utilization
- VRAM
- temperature
- clocks
- power
- PCIe metrics where available

CPU:

- total
- per-core
- P/E core usage
- frequency
- process usage

RAM:

- resident memory
- commit
- page faults where available

Inference:

- prompt TPS
- decode TPS
- TTFT
- request latency
- context
- batch
- ubatch
- current slot

MTP:

- generated drafts
- accepted drafts
- acceptance %
- mean accepted run
- rejected drafts
- effective speedup


# 22. BENCHMARK LAB

Support:

- Quick Benchmark
- Full Benchmark
- Prompt
- Decode
- Context Scaling
- MTP
- MoE Placement
- Batch Sweep
- Thread Sweep
- KV Sweep
- Load Benchmark
- Router Switch Benchmark
- Vision Benchmark
- Custom Experiment

Every result stores:

- model
- llama.cpp build hash
- hardware
- config
- workload
- repetitions
- mean
- median
- standard deviation
- min/max
- VRAM
- RAM
- GPU utilization
- CPU utilization
- power
- temperature


# 23. AUTOTUNER

Build a staged, adaptive optimizer.

Never use a giant Cartesian brute-force search.

Search order matters.


# 24. STAGE 0 — DISCOVERY / BASELINE

Discover:

- hardware
- llama.cpp build
- supported flags
- model metadata
- architecture
- MoE
- MTP
- vision
- RAM
- VRAM
- storage

Generate a safe initial profile if none exists.

Establish baseline performance.


# 25. STAGE 1 — MOSTLY INDEPENDENT SETTINGS

Test first:

- threads
- polling
- CPU affinity
- Flash Attention
- KV offload
- op offload

Generate topology-aware candidates.

For hybrid CPUs, understand:

- P-core count
- E-core count
- physical cores
- logical processors

Do not assume logical CPU numbering.


# 26. STAGE 2 — PLACEMENT / MEMORY

Tune interactions:

- n-gpu-layers
- n-cpu-moe
- fit
- fit-target
- batch-size
- ubatch-size
- KV types
- no-host
- tensor overrides

For MoE:

coarse n-cpu-moe sweep
→ detect performance cliff
→ fine sweep around transition
→ compare finalists

Example behavior:

20 → terrible
21 → high decode but broken prefill
22 → strong decode + strong prefill
23 → nearly tied
24 → slower

The tuner should recognize Pareto-optimal candidates rather than blindly picking the largest number.


# 27. LLAMA-FIT-PARAMS

If available, integrate llama-fit-params.

Capture:

- proposed GPU layers
- memory estimates
- tensor placement
- override-tensor suggestions

Treat fitter output as another candidate.

Compare:

- auto fitter
- manual n-cpu-moe
- fitter tensor placement

Only use tensor overrides when supported and benchmark-proven.


# 28. STAGE 3 — LOADING / LIFECYCLE

Loading is not an inference-only setting.

Benchmark where supported:

- mmap
- mlock
- mmap+mlock
- dio
- warmup on/off
- repack on/off

Measure separately:

- process spawn → healthy
- healthy → first response
- true-ish cold load
- filesystem-cached load
- steady-state inference

Do not let warmup=false win simply because it hides work in the first request.

Score real time-to-useful-response.


# 29. STAGE 4 — MTP / SPECULATIVE

For compatible models tune:

- spec-draft-n-max
- spec-draft-n-min
- spec-draft-p-min
- spec-draft-p-split
- draft threads
- draft batch threads
- draft CPU affinity
- draft GPU layers
- draft n-cpu-moe
- draft KV type
- draft backend sampling
