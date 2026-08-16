# Build Status

## Current tranche

The repository contains the Milestone 0 foundation plus the first Milestone 1 implementation path:

```text
select llama.cpp installation
→ discover/hash relevant binaries
→ capture capability/version evidence
→ select GGUF
→ inspect real GGUF metadata
→ preview exact llama-bench invocation
→ execute llama-bench
→ retain raw stdout/stderr/exit status
→ parse benchmark samples
→ persist run/history in SQLite
→ present the workflow in Dioxus
```

The source intentionally does not claim later roadmap features as implemented.

## Verification state

The source-authoring environment used for this initial GitHub tranche does not contain a Rust/Cargo toolchain or Windows desktop session. Therefore **the current commit is not considered green until GitHub Actions and native Windows verification say so**.

Required CI gates:

```text
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

Required native verification:

- Dioxus app launches on Windows.
- Rendered UI is visually inspected at supported desktop sizes.
- Real arbitrary llama.cpp installation discovery succeeds.
- Real arbitrary GGUF metadata inspection succeeds.
- Real llama-bench execution succeeds and non-zero exit remains an error.
- Persisted benchmark history survives restart.

Any failure in those gates is a blocking defect to repair before moving deeper into the roadmap.
