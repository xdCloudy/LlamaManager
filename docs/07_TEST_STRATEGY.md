# Test Strategy

## 1. Quality gates

Every feature tranche must finish with:

```text
cargo fmt --check
cargo check
cargo test
cargo clippy
```

plus targeted runtime/UI verification.

Do not knowingly carry a broken repository into the next tranche.

## 2. Unit tests

At minimum cover:

- INI parsing
- inheritance
- comment preservation
- unknown option preservation
- option registry
- command builder
- capability parsing
- GGUF metadata parsing
- path handling
- statistics
- scoring
- Pareto dominance
- search algorithms
- cache-key generation
- stale-profile invalidation
- hardware fingerprint normalization

## 3. Integration tests

Use mock executables for:

- llama-server
- llama-bench
- llama-fit-params

Mocks should simulate:

- successful startup
- slow load
- malformed output
- non-zero exit
- CUDA OOM
- unsupported flags
- router APIs
- MTP metrics
- multimodal loading
- model switching
- stop timeout
- forced termination
- invalid API key
- child connection failure

A failed process must never be represented as a successful empty result.

## 4. File/path tests

Test:

- spaces
- Unicode
- different drives
- UNC paths where applicable
- moved model file
- moved portable root
- read-only external config
- missing files
- duplicate files

## 5. Required scenario matrix

At least:

### A
CUDA + MoE + MTP + vision

### B
CUDA + dense model

### C
CPU-only

### D
full GPU residency

### E
model larger than VRAM

### F
multiple models/router

### G
no `models.ini`

### H
heavily commented existing `models.ini`

### I
paths with spaces

### J
Unicode paths

### K
no `llama-bench`

### L
no router capability

### M
unknown future options

### N
incompatible model

### O
missing mmproj

### P
moved model file

### Q
moved portable application directory

## 6. Real-system validation

Mocks are necessary but insufficient.

Before claiming a vertical slice complete, test against at least one real llama.cpp installation and one real GGUF where the feature requires them.

Later releases should test across multiple architectures/backends.

## 7. Benchmark validation

For benchmark parsing:

- keep representative raw outputs as fixtures
- include multiple llama.cpp versions where formats differ
- verify locale/decimal handling
- verify partial output
- verify cancellation
- verify nonzero exit handling
- verify missing metrics

## 8. Autotuner validation

Test the optimizer using deterministic fake benchmark functions.

Cases:

- obvious single optimum
- Pareto tradeoff
- noisy measurements
- tied candidates
- performance cliff
- outlier sample
- candidate failure
- early stop
- resume after interruption
- invalidated cache

The tuner should be testable without launching a real model.

## 9. UI verification

For major pages verify:

- 1280×720
- 1440×900
- 1920×1080
- 2560×1440

Test:

- long file paths
- long model names
- empty states
- loading states
- error states
- reduced motion
- dense tables
- large logs
- disconnected runtime
- stale profile
- incompatible model

Do not mark a page complete based only on compilation.

## 10. Release readiness

Before a release:

- clean build
- clean tests
- clippy reviewed
- database migration test from previous release
- portable relocation test
- first-run test on a clean user profile
- no required developer runtimes
- diagnostic export test
- secret-redaction test
- rollback test
- router/server lifecycle test
- at least one end-to-end benchmark

