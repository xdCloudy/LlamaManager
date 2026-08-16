# Repository Manifest

## Application

- `Cargo.toml` — Rust package/dependencies
- `rust-toolchain.toml` — pinned toolchain channel/profile
- `src/main.rs` — native application entry point
- `src/lib.rs` — library exports
- `src/app.rs` — Dioxus application shell/workflow
- `src/llama.rs` — llama.cpp installation/tool discovery and capability evidence
- `src/gguf.rs` — GGUF metadata inspection
- `src/benchmark.rs` — llama-bench invocation, execution, parsing, statistics-facing result model
- `src/persistence.rs` — SQLite persistence boundary
- `src/paths.rs` — portable/user-data path resolution
- `src/error.rs` — typed application errors
- `assets/app.css` — desktop UI styling
- `migrations/0001_initial.sql` — canonical initial SQLite schema
- `portable.flag` — opt-in portable storage mode marker

## Documentation

- `README.md` — public repository overview and current usable workflow
- `AGENTS.md` — repository-wide engineering/agent rules
- `CHANGELOG.md` — release/change history
- `CONTRIBUTING.md` — contribution workflow
- `SECURITY.md` — security/reporting expectations
- `docs/README.md` — documentation index
- `docs/00_START_HERE.md` — project philosophy and first execution path
- `docs/01_PRODUCT_SPEC.md` — product requirements
- `docs/02_ARCHITECTURE.md` — architectural boundaries
- `docs/03_DESIGN_SYSTEM_VAPORWAVE.md` — UI/visual system
- `docs/04_LLAMA_CPP_INTEGRATION.md` — llama.cpp integration rules
- `docs/05_BENCHMARKING_AUTOTUNER.md` — benchmark/tuner methodology
- `docs/06_DATA_AND_STORAGE.md` — persistence and portability
- `docs/07_TEST_STRATEGY.md` — verification strategy
- `docs/08_IMPLEMENTATION_ROADMAP.md` — milestone roadmap
- `docs/09_AGENT_DEVELOPMENT_RULES.md` — implementation discipline
- `docs/10_COMPLETION_MATRIX.md` — C0–C5 maturity and G1–G10 completion contract
- `docs/11_ISSUE_DEPENDENCY_GRAPH.md` — issue prerequisites, promotion gates, critical path, and v1.0 release dependency chain
- `docs/BUILD_STATUS.md` — evidence-based current verification state
- `docs/WORKLOG.md` — concise implementation/planning tranche log
- `docs/ORIGINAL_PROMPT.md` — preserved original comprehensive brief

## Automation

- `.github/workflows/ci.yml` — Windows Rust quality gates
- `.github/workflows/release.yml` — Windows release build/artifact workflow
- `.github/ISSUE_TEMPLATE/bug_report.yml` — structured defect reports
- `.github/ISSUE_TEMPLATE/feature_request.yml` — roadmap/feature proposals
- `.github/pull_request_template.md` — verification-focused PR checklist
