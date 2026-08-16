# Benchmarking and Autotuning

The benchmark subsystem is an evidence engine, not a collection of heuristic claims. The complete product requirements are preserved in `ORIGINAL_PROMPT.md`.

## Reproducibility

Benchmark evidence must be associated with the model identity, `llama.cpp` executable identity, effective arguments, backend evidence, workload, raw samples, parser version or schema expectations, and timestamps. Failed runs remain failed and must be retained/diagnosable rather than rewritten as zero-valued success.

## Statistics

Later benchmark-laboratory work must retain raw repetitions and report at least mean, median, standard deviation, coefficient of variation, and min/max where the underlying tool provides enough samples. Tiny numerical differences must not automatically be treated as meaningful improvements.

## Search policy

The autotuner must use staged adaptive search instead of a giant Cartesian brute-force sweep. Planned stages are:

1. discovery and safe baseline
2. mostly independent CPU/offload settings
3. placement and memory interactions
4. loading/lifecycle behavior
5. MTP/speculative settings for compatible models
6. append-heavy agent/cache workloads
7. multimodal behavior when a projector is present
8. whole-profile validation

Candidate generation should use coarse-to-fine refinement, early stopping, dominance pruning, successive halving where suitable, and finalist re-measurement.

## Objectives and Pareto frontier

There is no universal single winner. Profiles may optimize agent use, prompt/decode throughput, startup, switching, memory headroom, power, or a custom weighted objective. LlamaManager should retain Pareto-optimal profiles and the measured evidence behind them.

## Capability constraints

The tuner may only change options that both exist in the selected local build and have semantics LlamaManager understands sufficiently to tune safely. Model capabilities such as multimodal support and MTP are hard constraints unless the user explicitly chooses to disable them.

## Resume and invalidation

Long tuning runs must eventually persist current stage, candidates, completed/pending experiments, results, and current frontier so they can survive cancellation/restart. Evidence becomes stale when relevant model, llama.cpp binary, driver, hardware, configuration, workload, or tuner-version identity changes.
