# Build Status

## Current tranche

Milestone 2 is **C5 — Complete** with a verified real Windows model-library and compatibility workflow:

```text
select real llama.cpp installation
→ manually add / recursively scan GGUFs
→ derive stable model identity from file contents
→ retain duplicate locations without duplicate identities
→ inspect GGUF metadata
→ evaluate installation ↔ model compatibility from evidence
→ isolate corrupt/unreadable inputs
→ detect moved/missing files
→ relink by matching content identity
→ persist library + compatibility state across restart
→ present truthful states/reasons in Dioxus
```

Final user-facing M2 implementation: PR #117, merge commit `a504eb6a1181dbcccf7b0a4191d5a0200607a463`.

Final source CI for the M2 implementation: run `32098153891`, which passed PowerShell syntax, formatting, all-target check, tests, strict Clippy, release build, desktop process smoke, portable bundle assembly, and artifact upload.

The interactive Windows acceptance session then verified scan/add/dedupe/corrupt-input isolation/missing/relink/restart behaviour using paths containing spaces and Unicode. The full closure record is `docs/evidence/M2_MODEL_LIBRARY_2026-08-18.md` and issue #21.

### M2 limitations retained explicitly

- The interactive model was text-only and did not require an mmproj. Multimodal projector discovery/association/compatibility is covered by deterministic automated tests; no interactive multimodal claim is made.
- No separate numeric large-library throughput bound is claimed. Blocking GGUF/database work is worker-threaded, scan cancellation is covered, and the exercised desktop workflow remained responsive.
- The upstream b10472 `llama-bench` Unicode-model-path limitation documented under M1 does not affect M2 GGUF inspection/scanning; M2 exercised Unicode model-library paths successfully.

## Strict Windows CI

The normal Windows CI workflow enforces:

```text
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

It also performs a desktop process smoke test and assembles/uploads the portable Windows bundle. M2's final implementation commit `a504eb6a1181dbcccf7b0a4191d5a0200607a463` passed the complete pipeline in run `32098153891`.

For the prior M1 runtime tranche, source commit `5a5ebb3fad92f7a25ad4f1f38822d03a48214e30` passed standard CI run `32094641702`, and permanent real Windows runtime run `32094641715` passed the pinned llama.cpp/GGUF/benchmark checks plus real `llama-server` readiness and minimal inference validation.

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

The pinned upstream `llama-bench` b10472 build cannot load the same model when its Windows path contains Unicode; its own CLI/runtime boundary renders `模型` as `??` and exits non-zero. LlamaManager's GGUF parser successfully reads both real models from the Unicode path, and the upstream benchmark limitation is retained explicitly in the evidence artifact rather than hidden by fallback behavior.

## M1 interactive UI evidence — C5 closure

Issue #12 completed native Windows visual verification: the release UI was captured and manually inspected at 1280×720 and 1600×900 and reported clean.

On 2026-08-18 the repository owner then completed the remaining combined interaction against source commit `5a5ebb3fad92f7a25ad4f1f38822d03a48214e30` using `scripts/prepare-m1-gui-benchmark.ps1`. In the real release window, the prepared b10472 runtime and hash-pinned real GGUF were selected from paths containing spaces and the benchmark was started from the GUI.

The Benchmark view showed the exact real `llama-bench.exe` invocation, reported completion with raw and parsed evidence retained, and displayed measured `pp512 41297.19 tok/s` and `tg128 3504.32 tok/s` results. The History view showed one persisted SQLite record with those values, and the Overview showed runtime detected, RPC backend, model ready, 433 discovered capabilities, and one persisted run.

The operator-captured evidence and integrity hashes are recorded in `docs/evidence/M1_GUI_BENCHMARK_2026-08-18.md`.

This satisfies the previously outstanding M1 GUI-triggered benchmark requirement. M1 is **C5 — Complete**.

## Workflow cleanup

The active workflow set is intentionally limited to:

- `ci.yml`
- `release.yml`
- `real-runtime-validation.yml`

The real-runtime workflow is a permanent reproducible regression/evidence gate, not a temporary self-modifying validation workflow.