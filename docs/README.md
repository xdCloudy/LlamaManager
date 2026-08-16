# LlamaManager

A portable, production-grade Rust desktop control plane for `llama.cpp`.

LlamaManager turns:

```text
llama.cpp binaries + GGUF models + hardware
```

into:

```text
capability discovery
→ model compatibility
→ validated profiles
→ models.ini
→ router/server runtime
→ benchmarking
→ evidence-backed autotuning
→ safe apply / rollback
→ monitoring
→ fast model switching
```

without requiring normal users to understand `llama.cpp` command-line flags.

## Product position

LlamaManager is **not** merely a launcher.

It is intended to become a native Windows inference workstation that combines:

- llama.cpp installation management
- GGUF model library and metadata inspection
- multimodal / mmproj management
- `models.ini` generation and structured editing
- router/server lifecycle management
- process supervision
- hardware and inference telemetry
- benchmark laboratory
- adaptive performance autotuning
- experimental feature validation
- profile and snapshot management
- model switching optimization
- regression testing across llama.cpp updates

The application should hide unnecessary complexity from beginners while retaining complete technical transparency for power users.

## Non-negotiables

1. **Rust-first, native desktop application.**
2. **Dioxus Desktop** is the preferred UI framework.
3. **Portable Windows distribution** with no Python, Node.js, npm, Rust toolchain, PowerShell modules, or globally installed runtime required for normal use.
4. **No machine-specific assumptions.**
5. **No fake functionality.**
6. **No hard-coded llama.cpp flags without capability verification.**
7. **No giant unverified scaffold.**
8. **Every feature tranche must end in a green repository.**
9. **Every performance recommendation must be backed by measured evidence.**
10. **The UI must preserve the restrained vaporwave identity described in `03_DESIGN_SYSTEM_VAPORWAVE.md`.**

## Start here

Read in this order:

1. `00_START_HERE.md`
2. `01_PRODUCT_SPEC.md`
3. `02_ARCHITECTURE.md`
4. `03_DESIGN_SYSTEM_VAPORWAVE.md`
5. `04_LLAMA_CPP_INTEGRATION.md`
6. `05_BENCHMARKING_AUTOTUNER.md`
7. `06_DATA_AND_STORAGE.md`
8. `07_TEST_STRATEGY.md`
9. `08_IMPLEMENTATION_ROADMAP.md`
10. `09_AGENT_DEVELOPMENT_RULES.md`
11. `10_COMPLETION_MATRIX.md`

`08_IMPLEMENTATION_ROADMAP.md` defines what each milestone should deliver. `10_COMPLETION_MATRIX.md` is the authoritative evidence/closure contract: milestones progress from C0 through C5 and their GitHub issues must not close before every applicable completion gate is satisfied.

`ORIGINAL_PROMPT.md` preserves the original complete project brief verbatim for reference.

## Definition of done

A completely unrelated Windows user should be able to extract LlamaManager anywhere, point it at an arbitrary compatible `llama.cpp` installation and arbitrary GGUF models, then use the GUI to:

- inspect hardware
- inspect llama.cpp binaries
- discover supported capabilities
- inspect GGUF metadata
- determine model/runtime compatibility
- generate a safe hardware-aware profile
- generate or import `models.ini`
- configure the runtime correctly
- start and verify the router/server
- run inference
- benchmark
- autotune
- produce model-specific optimized profiles
- safely apply and roll back changes
- load, unload, start, stop, and switch models
- preserve history and tuning evidence
- survive application restart and Windows reboot
- continue working after the portable folder is moved

Normal operation must not require a terminal.

## Current implementation state

See `BUILD_STATUS.md`, `WORKLOG.md`, and `10_COMPLETION_MATRIX.md` for evidence-based implementation and verification status. The roadmap describes intended work; source, CI, runtime evidence, and the completion gates determine what is actually complete.
