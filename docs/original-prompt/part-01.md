You are the principal engineer, systems architect, performance engineer, UX designer, QA owner, and release engineer for a new production-grade Rust desktop application.

Build a fully portable, universal llama.cpp control plane for arbitrary llama.cpp installations and arbitrary GGUF models.

The application should take a user from:

llama.cpp binaries + GGUF models + hardware

to:

discovered capabilities
→ validated model profiles
→ generated models.ini
→ correctly configured router/server
→ benchmarking
→ autotuning
→ safe configuration apply/rollback
→ runtime monitoring
→ fast model loading/switching

without requiring command-line knowledge.

Do not stop at planning. Implement, compile, test, run, inspect, fix, and iterate until the application is genuinely usable.


# 1. PRODUCT GOAL

This is NOT just a llama-server launcher.

It should combine:

- llama.cpp installation manager
- model library
- GGUF metadata inspector
- multimodal/mmproj manager
- models.ini generator/editor
- router manager
- process supervisor
- live hardware/inference monitor
- benchmark suite
- automatic performance tuner
- experimental-feature lab
- configuration/profile manager
- model switching optimizer
- regression tester

The app must own llama.cpp complexity while still exposing full technical detail to power users.


# 2. PORTABILITY IS NON-NEGOTIABLE

The finished application must be downloadable as a portable Windows build, e.g.:

LlamaManager\
├── LlamaManager.exe
├── data\
├── config\
├── logs\
└── exports\

A user should be able to extract it anywhere and run it.

Normal use must NOT require:

- Python
- Node.js
- npm
- Rust
- Visual Studio
- PowerShell modules
- globally installed runtimes

Core logic must be compiled Rust.

Support both:

PORTABLE MODE
- application state stored relative to executable

USER DATA MODE
- normal per-user application-data directory

Do not depend on registry state.

Do not require administrator rights for normal operation.


# 3. NEVER HARD-CODE THE DEVELOPMENT MACHINE

The development environment may contain things like:

D:\llamacpp
D:\Stored Models
RTX 3060
i5-14600K
64 GB RAM
Agents-A1
262K context
CUDA

These are test cases only.

The application must support arbitrary:

- drive letters
- directories
- llama.cpp builds
- llama.cpp versions
- CUDA builds
- Vulkan builds
- CPU-only builds
- future backends
- GPUs
- CPUs
- RAM sizes
- GGUF architectures
- quantizations
- dense models
- MoE models
- multimodal models
- MTP/speculative models
- context lengths
- mmproj files
- router setups

Any assumption tied to one machine should be treated as an architectural bug.


# 4. TECHNOLOGY

Use Rust.

Preferred UI framework:

Dioxus Desktop

Avoid Electron/Node unless there is an overwhelming technical reason.

Use async Rust and structured concurrency.

Suggested modules:

src/
├── app/
├── ui/
├── llama/
├── models/
├── tuning/
├── benchmark/
├── telemetry/
├── persistence/
└── platform/

Keep:

- process management
- llama.cpp capability discovery
- configuration
- benchmarking
- tuning
- telemetry
- UI

cleanly separated.


# 5. DESIGN

Visual direction:

professional dark desktop system utility
+
restrained vaporwave identity
+
information architecture inspired by usekudu.com

Research the current Kudu UI before implementation.

Borrow principles such as:

- persistent left navigation
- dense but readable dashboards
- strong hierarchy
- compact metric cards
- clear grouping
- progressive disclosure
- system-utility feel

Do NOT clone Kudu.

Vaporwave styling should use:

- near-black / charcoal surfaces
- deep purple
- magenta
- cyan
- restrained orange
- subtle gradients
- occasional glow
- very subtle grid/CRT motifs
- clean modern typography

Avoid:

- excessive neon
- giant retro sunsets
- unreadable glow
- novelty VHS effects
- decorative noise

The result should feel like a premium futuristic AI workstation.


# 6. APPLICATION NAVIGATION

Suggested sidebar:

OVERVIEW
- Dashboard

MODELS
- Library
- Active Models
- Profiles

RUNTIME
- Server
- Router
- Sessions
- Context Cache

PERFORMANCE
- Monitor
- Benchmarks
- Autotune
- Experiments
- Compare

SYSTEM
- Hardware
- llama.cpp
- Storage
- Logs

CONFIGURATION
- models.ini
- Presets
- Snapshots

APP
- Settings

Bottom sidebar status:

llama.cpp    RUNNING
Backend      CUDA
Models       1 ACTIVE
VRAM         11.6 / 12 GB


# 7. LLAMA.CPP INSTALLATIONS

Create a first-class LlamaInstallation model.

Conceptually:

struct LlamaInstallation {
    id,
    name,
    root_path,
    server_binary,
    bench_binary,
    fit_params_binary,
    other_tools,
    version,
    build_hash,
    backend,
    capabilities,
    models_ini_support,
    router_support,
    last_verified,
}

Support multiple installations simultaneously:

- stable
- nightly
- custom
- CUDA
- Vulkan
- CPU

Allow:

- automatic discovery
- selecting a folder
- selecting llama-server directly
- manual correction

Do not assume binaries are under bin\.
