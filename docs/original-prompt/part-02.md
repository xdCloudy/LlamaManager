
# 8. DYNAMIC CAPABILITY DISCOVERY

Never assume a llama.cpp option exists.

Inspect the selected binaries:

llama-server --help
llama-bench --help
llama-fit-params --help

and other tools where relevant.

Build a capability registry dynamically.

Example:

✓ Router mode
✓ models.ini
✓ MTP
✓ n-cpu-moe
✓ unified KV
✓ context checkpoints
✓ multimodal
✓ tensor overrides
✓ DirectIO
✗ unsupported-new-feature

Unknown newly discovered options should appear under:

Experimental / Unclassified

Do not silently tune unknown options.


# 9. OPTION REGISTRY

Use structured metadata.

Conceptually:

struct LlamaOption {
    canonical_name,
    cli_aliases,
    domain,
    risk,
    value_type,
    experimental,
    tunable,
    affects_quality,
    affects_memory,
    affects_loading,
    affects_inference,
    dependencies,
    conflicts,
}

Domains:

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


# 10. MODEL LIBRARY

Allow arbitrary model folders and individual GGUF files.

Support:

- recursive scanning
- drag/drop
- manual file selection
- favourites
- tags
- aliases
- missing-file detection
- duplicate detection
- moved-file repair

Do not infer architecture from filenames.

Read GGUF metadata.

Determine where possible:

- architecture
- quantization
- parameters
- active parameters
- layer count
- context
- tokenizer
- chat template
- expert count
- routed experts
- MoE structure
- recurrent/hybrid structure
- MTP capability
- multimodal metadata


# 11. MMPROJ / MULTIMODAL

Support:

- automatic projector discovery
- manual projector association
- multiple projector candidates
- validation
- CPU/GPU projector placement

Never remove vision merely because text-only inference is faster.

Capability requirements are hard constraints.


# 12. MODEL COMPATIBILITY

Before activation, validate model + selected llama.cpp installation.

Display:

Model architecture       Supported
GGUF version             Supported
Chat template            Found
Vision projector         Found
MTP                      Supported
Backend                  CUDA
Status                   READY

If incompatible, explain why and allow switching llama.cpp builds.


# 13. MODELS.INI

models.ini must be a first-class feature.

Support:

- creating from scratch
- importing existing files
- global [*] defaults
- per-model sections
- inheritance
- comments
- unknown settings
- structured editing
- raw editing
- diff view
- validation

The user should be able to start with:

llama.cpp
+
Model-A.gguf
+
Model-B.gguf

and generate a valid models.ini without manual editing.


# 14. MODELS.INI GENERATOR

Provide a wizard:

1. Select models
2. Select workload
3. Select capabilities
4. Generate safe hardware-aware profile
5. Validate
6. Optional benchmark/autotune
7. Generate models.ini
8. Configure llama.cpp to use it

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


# 15. CONFIGURE LLAMA.CPP TO USE MODELS.INI

Do NOT merely generate models.ini.

The application must also configure the selected llama.cpp build correctly.

Determine dynamically how that build expects:

- models.ini
- router presets
- model configuration

to be supplied.

Then:

1. generate config
2. build correct invocation
3. validate invocation
4. start router/server
5. verify expected models were loaded
6. run health check
7. run minimal inference

No manual CLI should be required.


# 16. MANAGED VS EXTERNAL CONFIG

Support:

MANAGED
- app owns generated models.ini

EXTERNAL
- app edits an existing user file

External mode must:

- preserve comments
- preserve unknown options
- create backups
- show diffs
- avoid unnecessary rewrites


# 17. PROCESS MANAGEMENT

Implement robust native process supervision.

Support:

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

On Windows use Job Objects where appropriate.

Do not construct shell command strings.

Always use explicit executable + argument arrays.

Paths containing spaces and Unicode must work correctly.


# 18. ROUTER MODE

Router mode is first-class.

Expose:

- router status
- available models
- loaded models
- loading models
- process ports
- active requests
- LRU state
- residency state

Actions:

- Load
- Unload
- Reload
- Switch
- Preload
