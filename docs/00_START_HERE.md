# Start Here

## Mission

Build LlamaManager from a clean repository as a serious, maintainable, open-source-grade Rust desktop application.

The project is ambitious. The main engineering risk is not a lack of features; it is allowing architectural scope to outrun verified implementation.

The project must therefore be built as **small, complete vertical slices**.

## Core development rule

Do not write ten interconnected subsystems and compile later.

Use this loop:

```text
inspect requirements
→ design the smallest complete slice
→ implement
→ cargo fmt --check
→ cargo check
→ cargo test
→ cargo clippy
→ run the application
→ inspect actual runtime/UI behavior
→ fix
→ create a git checkpoint
→ continue
```

A tranche is not complete while the repository is red.

## Milestone 0 — source integrity

Before feature work:

```text
cargo fmt --check   PASS
cargo check         PASS
cargo test          PASS
cargo clippy        PASS or all findings explicitly understood
```

Then create the first checkpoint:

```text
chore: establish compilable LlamaManager baseline
```

The baseline should contain only:

- project structure
- Dioxus application shell
- logging
- error model
- portable/user-data path resolution
- SQLite initialization and migrations
- minimal test harness

Do not pre-create large quantities of speculative production code.

## First real vertical slice

The first meaningful end-to-end slice is:

```text
select real llama.cpp installation
→ detect binaries
→ identify version/build/backend
→ discover supported options
→ select real GGUF
→ inspect real metadata
→ run one real llama-bench command
→ parse the result
→ persist the run
→ display the run in Dioxus
```

Once this works, LlamaManager is an application rather than a scaffold.

## Architectural invariants

These should remain true throughout the project:

### 1. Capability-driven behavior

Never assume the local llama.cpp build has a flag merely because upstream currently has it.

Inspect local binaries first.

### 2. Arbitrary paths

All supported paths must tolerate:

- spaces
- Unicode
- different drive letters
- UNC paths where supported
- moved portable directories

Never build shell command strings. Use executable + argument arrays.

### 3. No development-machine coupling

The following may be used as test fixtures only:

- `D:\llamacpp`
- `D:\Stored Models`
- RTX 3060
- i5-14600K
- 64 GB RAM
- Agents-A1
- 262K context
- CUDA

Any assumption that these are universal is a defect.

### 4. Evidence before optimization

No profile is "optimized" because a heuristic says so.

An optimized profile requires benchmark evidence stored with:

- model identity/hash
- llama.cpp executable hash/version/build
- backend
- hardware fingerprint
- driver/runtime information
- full effective configuration
- workload
- repetitions
- measured results
- variance
- timestamp

### 5. No silent capability loss

If a model is multimodal, MTP-capable, MoE, hybrid/recurrent, or otherwise special, the tuner must treat that as a hard capability constraint unless the user explicitly chooses to disable it.

### 6. Transparent execution

The app must be able to show the exact effective `llama-server`, `llama-bench`, or other invocation it will execute.

### 7. Reversible configuration

Every meaningful configuration change should be diffable and restorable.

### 8. UI truthfulness

No fake metrics, placeholder graphs presented as live data, dead buttons, or imaginary llama.cpp flags.

Incomplete features must be visibly marked as incomplete.

## Product philosophy

The beginner should see:

```text
Model
Optimized
45.4 tok/s

[ Start ]
```

The expert should be able to drill down into:

- GPU/CPU placement
- CPU topology
- KV types
- MTP acceptance
- prompt/context cache behavior
- exact CLI arguments
- router residency
- benchmark distributions
- profile history
- rollback evidence

Use progressive disclosure rather than choosing between simplicity and power.
