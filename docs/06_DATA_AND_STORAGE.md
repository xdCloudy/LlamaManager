# Data and Storage

## 1. Storage modes

### Portable

Store application-owned state relative to the executable.

Recommended:

```text
LlamaManager\
├── LlamaManager.exe
├── data\
│   └── llamamanager.db
├── config\
├── logs\
└── exports\
```

### User data

Use the platform-appropriate per-user application data directory.

The path resolver should be centralized and testable.

## 2. SQLite

Use embedded SQLite.

Use schema migrations.

Maintain one canonical source for each migration.

Do not duplicate the schema in both Rust string constants and `.sql` files.

## 3. Suggested entities

### llama_installations

- id
- name
- root_path
- server_binary
- bench_binary
- fit_params_binary
- version
- build_hash
- backend
- last_verified

### capability_snapshots

- id
- installation_id
- executable_hash
- captured_at
- serialized capability registry
- raw help hashes

### models

- id
- canonical_path
- file_hash
- size
- mtime
- favourite
- first_seen
- last_seen

### model_metadata

- model_id
- captured_at
- architecture
- quantization
- parameters
- active_parameters
- layer_count
- context
- tokenizer metadata
- chat template
- expert metadata
- MTP metadata
- multimodal metadata
- raw metadata payload

### projector_associations

- id
- model_id
- projector_path
- projector_hash
- validation state
- user selected
- notes

### profiles

- id
- model_id
- installation_id
- name
- objective
- effective config
- evidence state
- stale state
- created_at
- updated_at

### config_snapshots

- id
- model/profile scope
- timestamp
- previous config
- new config
- benchmark evidence
- llama.cpp hash
- hardware fingerprint
- note

### benchmark_runs

- id
- model
- installation
- hardware snapshot
- effective config
- workload
- start/end
- status
- summary metrics
- raw samples
- logs reference

### tuning_sessions

- id
- model/profile
- objective
- stage
- candidates
- completed work
- pending work
- current best
- Pareto frontier
- state
- created/updated

### hardware_snapshots

- id
- captured_at
- CPU/topology
- RAM
- GPU/VRAM
- drivers/runtime
- storage

### runtime_sessions

- id
- installation
- router/server
- started_at
- stopped_at
- process metadata
- effective invocation
- exit state

### alerts

- id
- timestamp
- category
- severity
- evidence
- acknowledged
- resolved

## 4. Paths

Store stable logical references where needed, but preserve actual paths.

Portable mode must survive the entire application folder moving.

When an application-owned path is inside the portable root, prefer a relocatable relative representation.

External paths remain absolute.

## 5. Configuration snapshots

Every significant configuration change should be reversible.

Before apply:

1. capture current configuration
2. validate target configuration
3. create snapshot
4. show diff
5. apply
6. verify runtime health
7. mark snapshot outcome

On failure, offer immediate rollback.

## 6. Diagnostic retention

Logs and raw benchmark output can grow quickly.

Implement retention settings:

- max log age
- max log size
- benchmark raw-output retention
- export-on-demand
- database vacuum/maintenance where appropriate

Do not discard summary evidence needed to understand a stored profile.

## 7. Secrets

Do not store plaintext API keys in logs or benchmark payloads.

If secret storage is required, use an explicit secure strategy.

Diagnostic exports must redact secrets by default.
