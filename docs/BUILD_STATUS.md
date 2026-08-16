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

## Verified Windows CI evidence

GitHub Actions on `windows-latest` has already verified the source through the following gates on the initial green checkpoint:

```text
cargo check --all-targets                                  PASS
cargo test --all-targets                                   PASS
cargo clippy --all-targets --all-features -- -D warnings  PASS
cargo build --release                                      PASS
```

The first CI repair cycle also applied `rustfmt`, and subsequent source fixes were written against the formatted tree. `Cargo.lock` is now committed for reproducible application dependency resolution.

The current CI workflow additionally enforces:

```text
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

and packages a `LlamaManager-windows-x64.zip` workflow artifact. The latest strict run is the authoritative source for whether the current head remains green.

## Native verification still required

Compilation and CI do not prove the external-runtime or rendered-UI claims. Before Milestone 1 is called fully verified, perform native Windows validation with real inputs:

- Dioxus app launches and remains healthy.
- Rendered UI is visually inspected at supported desktop sizes.
- Real arbitrary llama.cpp installation discovery succeeds.
- Real arbitrary GGUF metadata inspection succeeds.
- Real llama-bench execution succeeds and non-zero exit remains an error.
- Persisted benchmark history survives restart.

The CI desktop-process smoke step is useful crash evidence but is not a substitute for rendered visual inspection.
