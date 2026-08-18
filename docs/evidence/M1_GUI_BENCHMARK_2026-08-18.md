# M1 C5 Interactive GUI Benchmark Evidence — 2026-08-18

This record captures the final human-operated Milestone 1 acceptance step required by `docs/10_COMPLETION_MATRIX.md`.

## Source and prepared stack

- Source commit exercised: `5a5ebb3fad92f7a25ad4f1f38822d03a48214e30`
- Preparation helper: `scripts/prepare-m1-gui-benchmark.ps1`
- Pinned llama.cpp release: `b10472`
- `llama-bench.exe` SHA-256: `97495a77c5f6d528f9eeff0a43a692574951a6b98e673e637366dd7dfce07d4f`
- GGUF SHA-256: `6151b1929d7f5aa3385d9ddef3393e55587c0a55de661562322bc51dfda93a04`

The operator launched the real release application on an interactive Windows desktop and selected the prepared paths containing spaces:

```text
Runtime root:
C:\LlamaManager\artifacts\m1-gui-benchmark\llama cpp runtime with spaces

Model:
C:\LlamaManager\artifacts\m1-gui-benchmark\Model Files with spaces\stories 15M benchmark.gguf
```

## GUI-triggered benchmark

The benchmark was started from the LlamaWave/LlamaManager Benchmark UI. The UI displayed the real invocation before execution:

```text
C:\LlamaManager\artifacts\m1-gui-benchmark\llama cpp runtime with spaces\llama-bench.exe -m "C:\LlamaManager\artifacts\m1-gui-benchmark\Model Files with spaces\stories 15M benchmark.gguf" -r 3 -o json
```

The GUI reported `Benchmark complete. Raw output and parsed evidence were retained.` and displayed a measured result rather than placeholder state.

Observed latest-result metrics:

```text
pp512  41297.19 tok/s
 tg128   3504.32 tok/s
```

The History view showed one persisted SQLite benchmark record for `stories 15M benchmark.gguf`, backend `RPC`, with the same prompt/decode values. The Overview view showed:

- runtime: detected
- backend: RPC
- model: ready
- capabilities: 433, discovered from `--help`
- runs: 1 persisted evidence record
- benchmark stage: measured

This closes the previously outstanding combined GUI + real-runtime interaction gap: the rendered application selected the real prepared runtime/model, showed the exact real `llama-bench` invocation, executed it from the GUI, presented measured metrics, and exposed the persisted history record.

## Operator screenshot integrity

Three full-window PNG captures were supplied during the verification session. Their SHA-256 digests are recorded so the originally supplied evidence can be integrity-checked:

```text
Benchmark result: b67e47f565780a2bcbbd622e871a18ef3fff728277a84665e4701a7e0de37361
History view:     d158baaad97f769c78424d5fb3ce01ef71327f9db08d2016497f698c3c036d2e
Overview view:    d52c0928402988b8ad4c146855845b73af5e78577c32de36ae57de35955d6165
```

Each capture is 1920×1032.

## Gate result

The final M1 C5 interactive requirement is **PASS**. Combined with the previously recorded native UI, strict CI, real llama.cpp/GGUF/benchmark, failure/recovery, persistence, Unicode-limitation, and cleanup evidence, M1 is closure-ready at **C5 — Complete**.
