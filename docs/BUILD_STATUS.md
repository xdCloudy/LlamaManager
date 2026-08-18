# Build Status

## Current tranche

Milestone 1 now has a verified real installation → model → benchmark backend path on Windows:

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

The repository also contains implementation work for later milestones, but milestone promotion remains evidence-gated by `docs/10_COMPLETION_MATRIX.md` and the GitHub dependency graph.

## Strict Windows CI

The normal Windows CI workflow enforces:

```text
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

It also performs a desktop process smoke test and assembles/uploads the portable Windows bundle. The final M1 runtime PR #97 passed the complete normal CI pipeline before merge.

## M1 real-runtime evidence — 2026-08-18

PR #97 (`ca0a22e6083619d7f01fa9204c1e4e2d706994b5`) added and passed a permanent `Real Windows Runtime Validation` workflow on `windows-2025`.

Evidence run: `32086521199`
Artifact: `real-windows-runtime-evidence`
Artifact id: `9306990681`
Artifact digest: `sha256:8997f168b9658a0383fcd5b24f092ad93e482f2f19faf5ac2800c13ec5f91324`

### llama.cpp identity

Pinned upstream release: `b10472`
Archive SHA-256:

```text
ef495329c85c171991972fd3226a179c1900368cab66e2ebba8b21a7471a74e5
```

Validated from an arbitrary Windows path containing spaces and Unicode:

```text
D:\a\_temp\external llama cpp 外部 build
```

Executable identities:

```text
llama-server.exe  76b0a5f72243ccb99079ca71ebd0332f123c52668d815c9d6716a89d46415668
llama-bench.exe   97495a77c5f6d528f9eeff0a43a692574951a6b98e673e637366dd7dfce07d4f
```

The selected binaries produced real version/help/backend/device evidence and 433 discovered capability tokens. Missing installations and fake/non-executable binaries were rejected rather than converted into fallback success.

### Real GGUF evidence

Two independently published, hash-pinned GGUFs were parsed from Windows paths containing spaces and Unicode.

GGUF v3:

```text
SHA-256:       6151b1929d7f5aa3385d9ddef3393e55587c0a55de661562322bc51dfda93a04
version:       3
architecture:  llama
tensors:       57
metadata:      20
```

GGUF v2:

```text
SHA-256:       18e8af33a3f4a4ce87314adbdb16757160c6ad93aee2a8ee7aae2a6350d1cfd7
version:       2
architecture:  llama
tensors:       201
metadata:      19
```

Corrupt/truncated, missing, and non-file GGUF inputs were rejected truthfully. Model identity is content-derived rather than inferred from filenames.

### Real llama-bench evidence

A byte-identical copy of the v3 model was benchmarked from a Windows path containing spaces:

```text
D:\a\_temp\Benchmark Models with spaces\stories 15M benchmark.gguf
```

Exact argument vector:

```text
-m "D:\a\_temp\Benchmark Models with spaces\stories 15M benchmark.gguf" -r 3 -o json
```

The real process exited `0`. Raw stdout/stderr were retained and parsed into two benchmark samples, including:

```text
pp512  7952.984108 tok/s
tg128  1467.728985 tok/s
```

The run retained both binary and model SHA-256 identities. A real missing-model invocation remained a typed non-zero failure. Malformed/partial benchmark JSON remains a parse failure. The cancellable execution path owns the child process, kills/waits on interruption, drains stdout/stderr concurrently, and records a distinct `BenchmarkInterrupted` result. Installation/model/benchmark state was persisted to SQLite and verified after reopen.

### Recorded limitation

The pinned upstream `llama-bench` b10472 build cannot load the same model when its Windows path contains Unicode; its own CLI/runtime boundary renders `模型` as `??` and exits non-zero. LlamaManager's GGUF parser successfully reads both real models from Unicode paths, and the upstream benchmark limitation is retained explicitly in the evidence artifact rather than hidden by fallback behavior.

## Interactive UI evidence

Issue #12 has completed native Windows visual verification: the release UI was captured and manually inspected at 1280×720 and 1600×900 and reported clean.

The remaining M1 C5 gate is narrower: `docs/10_COMPLETION_MATRIX.md` requires one real benchmark to be launched through the interactive GUI path. The backend/runtime path and the rendered UI have each been verified independently, but those two facts are not being combined into an unsupported claim that the GUI-triggered benchmark interaction itself was exercised.

Until that final interaction is recorded, M1 should be treated as **C4 — runtime verified**, not C5.

## Workflow cleanup

The active workflow set is intentionally limited to:

- `ci.yml`
- `release.yml`
- `real-runtime-validation.yml`

The real-runtime workflow is a permanent reproducible regression/evidence gate, not a temporary self-modifying validation workflow.
