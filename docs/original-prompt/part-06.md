
- dead buttons
- fake graphs
- placeholder metrics presented as real
- imaginary llama.cpp flags
- hardcoded benchmark numbers

If incomplete, mark it honestly.


# 58. RESEARCH RULE

llama.cpp changes rapidly.

Before implementing llama.cpp-specific behavior:

1. inspect selected local binary
2. inspect current upstream source/docs where needed
3. confirm semantics
4. implement capability-driven support
5. test against actual binary

Do not rely on stale assumptions.


# 59. AUTONOMOUS DEVELOPMENT LOOP

Do not stop at a plan.

Continuously:

inspect
→ design
→ implement
→ compile
→ test
→ run
→ inspect UI/runtime
→ identify gaps
→ fix
→ repeat

Do not leave core functionality as permanent TODOs.


# 60. IMPLEMENTATION ORDER

Phase A
- Rust/Dioxus foundation
- portable storage
- database
- design system
- process abstraction

Phase B
- llama.cpp installation discovery
- capability registry
- GGUF/model library
- models.ini parser/generator

Phase C
- server lifecycle
- command builder
- logs
- telemetry
- API client

Phase D
- router
- model load/unload/switching

Phase E
- benchmark engine
- statistics
- comparison/history

Phase F
- staged autotuner

Phase G
- experimental feature framework
- fitter/tensor-placement integration

Phase H
- UX polish
- portable release
- docs
- regression testing


# 61. DEFINITION OF DONE

A completely unrelated Windows user should be able to download the application and have:

E:\CustomLlama\
F:\Models\UnknownModel.gguf

with hardware/build/model combinations the developer has never seen.

The application must be able to:

1. inspect hardware
2. inspect llama.cpp binaries
3. discover capabilities
4. inspect GGUF
5. determine compatibility
6. generate a safe model profile
7. generate models.ini
8. configure llama.cpp to use it
9. start router/server
10. verify health
11. run inference
12. benchmark
13. autotune
14. produce model-specific optimized profiles
15. safely apply/rollback
16. start/stop models
17. switch models
18. retain history
19. survive restart
20. remain functional if the portable app is moved

No terminal should be required for normal operation.


# FINAL PRODUCT PRINCIPLE

The user supplies:

LLAMA.CPP
+
MODELS
+
HARDWARE

The application derives:

CAPABILITIES
+
COMPATIBILITY
+
MODELS.INI
+
ROUTER CONFIG
+
BENCHMARKS
+
AUTOTUNING
+
PROFILES
+
RUNTIME MANAGEMENT

The final result should make llama.cpp feel like a polished desktop inference platform rather than a collection of binaries and command-line flags.

Build this as a serious, maintainable, open-source-grade Rust application.

Do not settle for a proof of concept.
