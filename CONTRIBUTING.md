# Contributing to LlamaManager

LlamaManager is intentionally built as verified vertical slices rather than broad speculative scaffolding.

## Before changing code

1. Read `AGENTS.md`.
2. Read `docs/BUILD_STATUS.md` and `docs/WORKLOG.md`.
3. Identify the current milestone in `docs/08_IMPLEMENTATION_ROADMAP.md`.
4. Read only the domain documentation relevant to the change.

## Development gates

On Windows with the stable MSVC toolchain:

```powershell
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

UI changes should also be exercised in the real desktop application at the supported layout sizes documented in the design/test specifications.

## Pull requests

Keep PRs focused on one coherent vertical slice or defect. Include:

- what changed
- why the change is needed
- exact verification performed
- screenshots for visible UI changes
- any known limitations or unverified external-runtime behavior

Do not present placeholder data, dead controls, fabricated metrics, or unverified llama.cpp capability assumptions as complete functionality.
