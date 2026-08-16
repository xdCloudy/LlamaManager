# Product Specification

## 1. Product goal

LlamaManager is a universal native control plane for arbitrary `llama.cpp` installations and arbitrary GGUF models.

The user supplies:

```text
LLAMA.CPP
+
MODELS
+
HARDWARE
```

The application derives:

```text
CAPABILITIES
+
COMPATIBILITY
+
MODELS.INI
+
ROUTER CONFIGURATION
+
BENCHMARKS
+
AUTOTUNING
+
PROFILES
+
RUNTIME MANAGEMENT
```

The target experience is a polished desktop inference platform rather than a collection of binaries and flags.

## 2. Target platform

Primary:

- Windows 10/11
- x64
- portable distribution

Architecture must not unnecessarily prevent later support for other operating systems.

## 3. Distribution modes

### Portable mode

Example:

```text
LlamaManager\
├── LlamaManager.exe
├── data\
├── config\
├── logs\
└── exports\
```

State is stored relative to the executable.

### User data mode

State is stored in the normal per-user application data location.

Normal use must not require:

- administrator rights
- registry configuration
- Python
- Node.js
- npm
- Rust
- Visual Studio
- globally installed runtimes

## 4. Major product areas

### Overview

- Dashboard

### Models

- Library
- Active Models
- Profiles

### Runtime

- Server
- Router
- Sessions
- Context Cache

### Performance

- Monitor
- Benchmarks
- Autotune
- Experiments
- Compare

### System

- Hardware
- llama.cpp
- Storage
- Logs

### Configuration

- models.ini
- Presets
- Snapshots

### App

- Settings

Bottom sidebar status should summarize the live runtime:

```text
llama.cpp    RUNNING
Backend      CUDA
Models       1 ACTIVE
VRAM         11.6 / 12 GB
```

## 5. First run

Wizard:

1. detect or select llama.cpp
2. scan hardware
3. add model folders/files
4. inspect GGUF metadata
5. import or generate `models.ini`
6. validate configuration
7. configure router/server
8. optionally quick-tune
9. start

Ideal path:

```text
Download ZIP
→ extract
→ launch
→ select llama.cpp
→ select models
→ Start
```

## 6. Model library

Support:

- recursive folder scanning
- individual GGUF selection
- drag/drop
- favourites
- tags
- aliases
- missing-file detection
- duplicate detection
- moved-file repair
- metadata refresh

Do not infer architecture from filenames.

Inspect GGUF metadata to determine where possible:

- architecture
- quantization
- parameter count
- active parameter count
- layer count
- context length
- tokenizer
- chat template
- expert count
- routed experts
- MoE structure
- recurrent/hybrid structure
- MTP capability
- multimodal metadata

## 7. Multimodal / mmproj

Support:

- automatic projector discovery
- manual association
- multiple candidate projectors
- validation
- CPU/GPU projector placement
- compatibility checks

Vision capability is a hard constraint.

The performance tuner must not silently disable vision merely because text-only inference would be faster.

## 8. Compatibility

Before activation, show a compatibility report such as:

```text
Model architecture       Supported
GGUF version             Supported
Chat template            Found
Vision projector         Found
MTP                      Supported
Backend                  CUDA
Status                   READY
```

If incompatible, explain the exact reason and suggest compatible llama.cpp installations/builds where possible.

## 9. models.ini

`models.ini` is a first-class feature.

Support:

- creating from scratch
- importing existing files
- global `[*]` defaults
- per-model sections
- inheritance
- comments
- unknown options
- structured editing
- raw editing
- validation
- diff view
- backup and restore

### Managed mode

The app owns the generated file.

### External mode

The app edits a user-owned file while preserving:

- comments
- formatting where practical
- unknown keys
- untouched sections

External edits require:

- backup
- diff
- validation
- minimum unnecessary rewrite

## 10. Profile generator

Wizard:

1. select models
2. select workload
3. select required capabilities
4. generate safe hardware-aware profile
5. validate
6. optional benchmark/autotune
7. generate `models.ini`
8. configure llama.cpp to use it

Workload presets:

- Balanced
- Coding Agent
- Max Performance
- Long Context
- Vision
- Low VRAM
- Fast Loading
- Fast Switching
- Custom

## 11. Runtime control

Provide:

- start
- stop
- restart
- graceful termination
- force termination
- crash detection
- child tracking
- port collision detection
- stdout/stderr streaming
- log rotation
- orphan cleanup

On Windows, use Job Objects where appropriate.

## 12. Router mode

Router mode is first-class.

Display:

- router status
- available models
- loaded models
- loading models
- child ports
- active requests
- LRU state
- residency state

Actions:

- Load
- Unload
- Reload
- Switch
- Preload
- Pin resident
- Unpin
- Set startup

Router-level configuration must remain separate from model-specific configuration.

## 13. Model switching

Measure:

- cold load
- warm load
- first request
- unload
- reload
- A → B
- B → A
- A → B → A

Track:

- load time
- first-token latency
- VRAM release
- RAM
- storage throughput
- process startup
- first inference latency

Tune:

- models-max
- autoload
- startup models
- stop timeout
- load mode
- warmup
- repack
- residency strategy

## 14. Dashboard

Immediately show:

- active model
- llama.cpp status
- backend
- prompt TPS
- decode TPS
- TTFT
- VRAM
- RAM
- GPU utilization
- context usage
- cache reuse
- MTP acceptance
- model health
- tuning status

## 15. Monitoring

### GPU

- utilization
- VRAM
- temperature
- clocks
- power
- PCIe metrics where available

### CPU

- total usage
- per-core usage
- P/E-core usage where identifiable
- frequency
- llama.cpp process usage

### RAM

- resident memory
- commit
- page faults where available

### Inference

- prompt TPS
- decode TPS
- TTFT
- request latency
- context usage
- batch
- ubatch
- active slot

### MTP

- generated drafts
- accepted drafts
- acceptance ratio
- mean accepted run
- rejected drafts
- effective speedup

## 16. Logs and alerts

Log viewer:

- severity filter
- regex search
- source filter
- pause/autoscroll
- export
- structured parsing

Parse events such as:

- model load/unload
- CUDA OOM
- prompt processing
- slot selection
- cache hits
- MTP acceptance
- router proxying

Alerts should be evidence-backed, for example:

- VRAM HEADROOM LOW
- MTP ACCEPTANCE REGRESSION
- PROMPT CACHE MISS
- MODEL LOAD REGRESSION
- CUDA OOM
- MODEL CONFIG STALE

## 17. Export / import

Allow exporting:

- model profiles
- `models.ini`
- router setup
- benchmark evidence
- tuning results
- diagnostic bundles

Imported tuning data must be revalidated when hardware, driver, model, or llama.cpp build differs.

## 18. Diagnostic bundle

Create a redacted ZIP containing:

- app version
- hardware
- llama.cpp executable hashes
- discovered capabilities
- effective configuration
- model metadata
- logs
- benchmark summary

Exclude secrets and private prompts by default.

## 19. Managed llama.cpp

Long-term modes:

### External

Arbitrary user-supplied installations remain fully supported.

### Managed

LlamaManager may download and manage releases under a local runtime directory.

Managed runtime support must never replace support for custom builds.
