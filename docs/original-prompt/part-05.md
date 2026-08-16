- model load
- CUDA OOM
- prompt processing
- slot selection
- LCP similarity
- cache hits
- MTP acceptance
- model unload
- router proxying


# 44. ALERTS

Examples:

VRAM HEADROOM LOW

MTP ACCEPTANCE REGRESSION

PROMPT CACHE MISS

MODEL LOAD REGRESSION

CUDA OOM

MODEL CONFIG STALE

Every alert must explain the evidence.


# 45. LLAMA.CPP UPDATE REGRESSION TESTING

When binary hash/version changes:

mark tuned profiles:

STALE — llama.cpp changed

Offer:

- Quick Regression Test
- Full Retune
- Keep Existing

Compare old/new benchmark results.


# 46. STORAGE

Detect model storage where possible:

- NVMe
- SATA SSD
- HDD
- network share

Differentiate:

- cold-ish load
- filesystem-cached load
- process reload
- router switch


# 47. DATABASE

Use embedded SQLite.

Store:

- llama installations
- models
- metadata
- profiles
- config snapshots
- benchmark runs
- tuning state
- hardware snapshots
- runtime sessions
- alerts

Use migrations.


# 48. SECURITY

Requirements:

- never log API keys
- redact secrets
- loopback host by default
- canonicalize paths
- explicit process arguments
- no arbitrary shell concatenation
- display executable hashes
- distinguish known vs unknown executables


# 49. FIRST RUN

Wizard:

1. Detect/select llama.cpp
2. Scan hardware
3. Add model folders
4. Inspect GGUF files
5. Import or generate models.ini
6. Validate configuration
7. Configure router/server
8. Optional quick tune
9. Start

Ideal user experience:

Download ZIP
→ extract
→ launch
→ select llama.cpp
→ select models
→ Start


# 50. MANAGED LLAMA.CPP

Eventually support:

EXTERNAL
- arbitrary user-supplied builds

MANAGED
- app downloads and manages releases

Managed runtimes may live under:

runtimes\
├── llama-cpp-cuda-...
├── llama-cpp-vulkan-...
└── llama-cpp-cpu-...

Never remove support for custom builds.


# 51. EXPORT / IMPORT

Allow exporting:

- model profiles
- models.ini
- router setup
- benchmark evidence
- tuning result
- diagnostic bundle

Imported tuning profiles must be revalidated when:

- hardware differs
- llama.cpp differs
- model differs
- driver differs


# 52. DIAGNOSTIC EXPORT

Create a redacted ZIP containing:

- app version
- hardware
- llama.cpp hashes
- supported flags
- effective config
- model metadata
- logs
- benchmark summary

Exclude secrets and private prompts by default.


# 53. TESTING

Unit tests:

- INI parser
- inheritance
- option registry
- command builder
- search algorithm
- scoring
- statistics
- GGUF metadata
- path handling

Integration tests:

Use mock llama-server / llama-bench / llama-fit-params.

Simulate:

- success
- slow load
- CUDA OOM
- malformed output
- router APIs
- MTP
- multimodal
- switching
- unknown flags

Test paths with:

- spaces
- Unicode
- UNC
- different drives


# 54. REQUIRED TEST MATRIX

Cover at least:

A. CUDA + MoE + MTP + vision
B. CUDA + dense model
C. CPU-only
D. full GPU residency
E. model larger than VRAM
F. multiple models/router
G. no models.ini
H. heavily-commented existing models.ini
I. paths with spaces
J. Unicode paths
K. no llama-bench
L. no router capability
M. unknown future options
N. incompatible model
O. missing mmproj
P. moved model file
Q. moved portable app directory


# 55. UI QUALITY

Use reusable components:

- AppShell
- Sidebar
- MetricCard
- StatusBadge
- Panel
- Tabs
- DataTable
- Sparkline
- TimeSeriesChart
- Gauge
- BenchmarkComparison
- DiffView
- PropertyGrid
- CommandPreview
- LogViewer
- Dialog
- Toast
- Tooltip
- SearchField
- SplitPane

Support:

1280×720
1440×900
1920×1080
2560×1440

Respect reduced motion.


# 56. BEGINNER → EXPERT UX

Beginner sees:

Model
Optimized
45.4 tok/s

[Start]

Expert can drill into:

- tensor placement
- CPU masks
- KV types
- MTP acceptance
- checkpoints
- raw CLI arguments
- router residency
- benchmark distributions

Use progressive disclosure.


# 57. NO FAKE FUNCTIONALITY

Do not ship:
