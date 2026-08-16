# LlamaManager

LlamaManager is a native Rust/Dioxus desktop control plane for local `llama.cpp` installations and GGUF models on Windows.

The project is being built as verified vertical slices. The current source tree implements the first usable slice rather than presenting future roadmap items as finished features:

- portable or per-user application state
- embedded SQLite persistence with migrations
- recursive discovery of real `llama-server`, `llama-bench`, and `llama-fit-params` binaries
- SHA-256 identity for selected binaries and GGUF models
- dynamic CLI-capability discovery from the selected binaries' own `--help` output
- real GGUF v2/v3 metadata inspection without filename-based architecture guessing
- exact `llama-bench` invocation preview
- real benchmark execution with stdout/stderr and exit status retained
- current `llama-bench` JSON parsing plus a markdown fallback
- persisted benchmark history across restarts
- a native Dioxus workflow for selecting an installation, selecting a model, running a benchmark, and inspecting evidence
- portable data layout selected by `portable.flag`

The larger product roadmap remains in [`docs/`](docs/). Future stages cover the model library, `models.ini`, server/router lifecycle, live telemetry, benchmark laboratory, staged autotuning, rollback, diagnostics, and release polish.

## Build

The normal development target is Windows 10/11 x64 with the stable MSVC Rust toolchain:

```powershell
cargo fmt --check
cargo check --all-targets
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

The executable is written to `target\release\llamamanager.exe`. Keep `portable.flag` next to the executable for fully portable state under the application directory. Remove it to use the normal per-user application-data directory.

## First usable workflow

1. Launch LlamaManager.
2. Open **Benchmark**.
3. Select an arbitrary `llama.cpp` installation directory.
4. LlamaManager recursively finds supported tools and inspects their real help/version output.
5. Select a GGUF model.
6. LlamaManager reads metadata from the GGUF header and hashes the model.
7. Review the exact `llama-bench` command.
8. Run the benchmark.
9. Inspect parsed samples and the retained raw output; the run remains available in **History** after restart.

Non-zero external-process exits remain errors. LlamaManager does not silently replace failures with dummy or in-memory success paths.

## Portability and privacy

Normal use is designed not to require Python, Node.js, npm, a Rust toolchain, PowerShell modules, administrator rights, or global runtimes. The application does not upload models or prompts. External executable paths are passed directly to `std::process::Command` with argument arrays instead of concatenated shell commands.

## Status

This repository is an active implementation of the specification rather than a finished 1.0 release. The source of truth for what is actually implemented is the code and CI; roadmap features not present in the UI are not claimed as complete.

## License

MIT. See [`LICENSE`](LICENSE).
