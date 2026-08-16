# LlamaManager Agent Guide

## Mission

Build LlamaManager as a serious, maintainable, open-source-grade Rust/Dioxus desktop control plane for arbitrary `llama.cpp` installations and arbitrary GGUF models.

The repository is specification-driven, but **runtime evidence, tests, compiler output, and the actual source tree are the source of truth for implementation state**.

## Read order

Start with only the documents needed for the current task. The canonical sequence is:

1. `docs/00_START_HERE.md`
2. `docs/01_PRODUCT_SPEC.md`
3. `docs/02_ARCHITECTURE.md`
4. `docs/03_DESIGN_SYSTEM_VAPORWAVE.md`
5. `docs/04_LLAMA_CPP_INTEGRATION.md`
6. `docs/05_BENCHMARKING_AUTOTUNER.md`
7. `docs/06_DATA_AND_STORAGE.md`
8. `docs/07_TEST_STRATEGY.md`
9. `docs/08_IMPLEMENTATION_ROADMAP.md`
10. `docs/09_AGENT_DEVELOPMENT_RULES.md`

`docs/ORIGINAL_PROMPT.md` preserves the original complete product brief.

## Engineering rules

- Work in small, complete vertical slices.
- Do not build a giant speculative scaffold and compile later.
- Keep Rust domain/process/persistence logic independent from Dioxus presentation code where practical.
- Never fabricate llama.cpp capabilities, model metadata, benchmark data, telemetry, or successful runtime state.
- Discover llama.cpp behavior from the selected local binaries and current upstream evidence where needed.
- Never concatenate managed-process shell strings. Use executable + argv + explicit cwd/environment.
- Preserve typed failures. A failed process, parser, database operation, migration, or compatibility check remains a failure unless the product spec explicitly defines a fallback.
- Support spaces, Unicode, arbitrary drive letters, moved portable roots, and custom llama.cpp builds.
- Treat multimodal/mmproj, MTP/speculative, MoE, hybrid/recurrent, and other model capabilities as constraints, not optional assumptions.
- Keep one canonical SQLite migration source under `migrations/`.
- Persist raw evidence alongside interpreted results wherever reproducibility matters.

## Verification loop

For Rust changes, keep the repository green as tightly as possible:

```text
inspect current state
→ make one coherent change
→ cargo check / targeted test
→ fix the current root cause if red
→ verify again
→ continue
```

Before claiming a tranche complete, run the project-appropriate gates:

```text
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

Visible UI work additionally requires launching the real Dioxus application on Windows and visually inspecting the rendered result. Source review alone is not visual verification.

## Current project state

Read `docs/BUILD_STATUS.md` and `docs/WORKLOG.md` before continuing. Do not infer completion from the roadmap.
