# Architecture

## 1. Architectural goals

The architecture must optimize for:

- correctness
- recoverability
- capability discovery
- reproducible benchmarking
- separation between domain logic and UI
- portability
- future llama.cpp changes
- testability
- explicit process execution
- safe persistence

## 2. Suggested crate/module layout

Start with a single application crate unless real pressure justifies a workspace.

```text
src/
├── main.rs
├── app/
│   ├── mod.rs
│   ├── state.rs
│   └── commands.rs
├── ui/
│   ├── mod.rs
│   ├── shell/
│   ├── components/
│   └── pages/
├── llama/
│   ├── mod.rs
│   ├── installation.rs
│   ├── capabilities.rs
│   ├── options.rs
│   ├── process.rs
│   ├── command.rs
│   ├── router.rs
│   ├── server_api.rs
│   └── models_ini/
├── models/
│   ├── mod.rs
│   ├── library.rs
│   ├── gguf.rs
│   ├── compatibility.rs
│   └── multimodal.rs
├── benchmark/
│   ├── mod.rs
│   ├── runner.rs
│   ├── parser.rs
│   ├── workloads.rs
│   └── stats.rs
├── tuning/
│   ├── mod.rs
│   ├── candidate.rs
│   ├── objectives.rs
│   ├── stages/
│   ├── pareto.rs
│   └── cache.rs
├── telemetry/
│   ├── mod.rs
│   ├── hardware.rs
│   ├── runtime.rs
│   └── metrics.rs
├── persistence/
│   ├── mod.rs
│   ├── database.rs
│   ├── migrations.rs
│   └── repositories/
├── platform/
│   ├── mod.rs
│   ├── paths.rs
│   ├── windows.rs
│   └── process.rs
└── error.rs
```

Do not create empty modules solely because this document lists them. Add modules as vertical slices require them.

## 3. Dependency direction

Prefer:

```text
UI
↓
application commands/state
↓
domain services
↓
llama/model/benchmark/tuning abstractions
↓
platform + persistence
```

The UI should not construct llama.cpp command lines itself.

The benchmark tuner should not directly manipulate Dioxus state.

The process supervisor should not know presentation concerns.

## 4. Core domain models

### LlamaInstallation

Conceptually:

```rust
struct LlamaInstallation {
    id: InstallationId,
    name: String,
    root_path: PathBuf,
    server_binary: PathBuf,
    bench_binary: Option<PathBuf>,
    fit_params_binary: Option<PathBuf>,
    other_tools: Vec<PathBuf>,
    version: Option<String>,
    build_hash: Option<String>,
    backend: BackendKind,
    capabilities: CapabilityRegistry,
    models_ini_support: bool,
    router_support: bool,
    last_verified: DateTime<Utc>,
}
```

### ModelRecord

```rust
struct ModelRecord {
    id: ModelId,
    path: PathBuf,
    file_hash: Option<String>,
    size_bytes: u64,
    modified_at: SystemTime,
    metadata: GgufMetadata,
    mmproj_candidates: Vec<ProjectorCandidate>,
    tags: Vec<String>,
    favourite: bool,
}
```

### ModelProfile

A profile should contain:

- selected model
- selected llama installation
- required capabilities
- effective option set
- workload objective
- evidence status
- benchmark evidence references
- created/updated timestamps
- stale reason
- user notes

### BenchmarkRun

A benchmark run must be reproducible.

Store:

- run ID
- model ID/hash
- installation ID/hash
- backend
- hardware snapshot
- driver/runtime data where available
- exact effective arguments
- environment overrides relevant to execution
- workload
- repetitions
- raw result samples
- summary statistics
- memory/telemetry snapshots
- start/end timestamps
- exit status
- stderr/stdout references
- parser version

## 5. Error handling

Use typed errors.

Prefer `thiserror` or equivalent typed error enums.

Do not convert structured source errors into arbitrary strings merely to make a type `Clone`.

If UI state needs clonable errors, create a separate presentation-safe error record:

```rust
struct ErrorView {
    category: ErrorCategory,
    message: String,
    details: Option<String>,
}
```

while retaining the original typed error internally.

## 6. Async model

Use async Rust for:

- process output streaming
- router/server API operations
- benchmarks
- telemetry polling
- file scanning where appropriate
- long-running tuning sessions

Use structured cancellation.

A benchmark/tuning run should have:

- explicit owner
- cancellation token
- lifecycle state
- persisted progress
- bounded child tasks

Avoid detached background tasks that can outlive their state owner.

## 7. Process execution

Never use concatenated shell command strings.

Always use:

```text
explicit executable
+
argument vector
+
explicit working directory
+
explicit environment changes
```

Requirements:

- paths with spaces
- Unicode paths
- different drives
- clear stdout/stderr capture
- process exit codes
- graceful stop
- forced stop fallback
- orphan prevention

On Windows, use Job Objects where appropriate.

## 8. Capability model

A capability registry should capture more than booleans.

Example:

```rust
struct CapabilityRegistry {
    executable_hash: String,
    discovered_at: DateTime<Utc>,
    options: BTreeMap<String, DiscoveredOption>,
    features: FeatureSet,
    raw_help_hashes: BTreeMap<ToolKind, String>,
}
```

A discovered option may include:

```rust
struct DiscoveredOption {
    canonical_name: String,
    aliases: Vec<String>,
    value_hint: Option<String>,
    source_tool: ToolKind,
    raw_help_excerpt: String,
    classification: OptionDomain,
    known_semantics: bool,
}
```

Unknown options should remain visible rather than discarded.

## 9. Option registry

Known option semantics should be described structurally.

```rust
struct LlamaOptionDefinition {
    canonical_name: String,
    cli_aliases: Vec<String>,
    domain: OptionDomain,
    risk: RiskLevel,
    value_type: OptionValueType,
    experimental: bool,
    tunable: bool,
    affects_quality: bool,
    affects_memory: bool,
    affects_loading: bool,
    affects_inference: bool,
    dependencies: Vec<OptionConstraint>,
    conflicts: Vec<OptionConstraint>,
}
```

Domains include:

- Inference
- Prefill
- Decode
- Scheduling
- GPUPlacement
- CPUPlacement
- Memory
- KVCache
- Speculative
- MTP
- Multimodal
- Loading
- Router
- ContextCache
- Sampling
- Security
- ModelSemantics
- Experimental

The registry should distinguish:

1. option exists locally
2. application understands its semantics
3. option is safe to tune automatically

These are not equivalent.

## 10. UI state

Avoid a giant mutable global state object.

Use domain-specific services/state slices and compose them into the application model.

The UI should render observable state and invoke commands.

Long-running operations should expose:

- state
- progress
- current stage
- current action
- cancellation capability
- last error
- evidence/results

## 11. Persistence boundary

Persistence repositories should expose typed domain operations.

Do not scatter raw SQL across UI or service modules.

Migrations must be canonical and singular.

There should not be multiple competing copies of the same schema embedded in Rust and `.sql` files.

## 12. Security boundary

Security requirements:

- never log API keys
- redact tokens/secrets
- loopback bind by default
- canonicalize paths
- explicit process arguments
- show executable hashes
- distinguish known vs unknown executables
- do not execute arbitrary discovered binaries without user intent
- diagnostic exports redact secrets by default

