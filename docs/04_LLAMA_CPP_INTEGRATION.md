# llama.cpp Integration

This document records the integration rules implemented by LlamaManager. The original full product requirements are preserved in `ORIGINAL_PROMPT.md`.

## Source of truth

A selected local `llama.cpp` installation is authoritative for the capabilities that LlamaManager may expose. LlamaManager must not assume that an option exists merely because a current upstream build has it.

For each discovered tool LlamaManager records the executable path, SHA-256, version output where available, and complete help output. `llama-bench --list-devices` is captured when the selected build advertises that capability. This raw evidence is retained alongside interpreted capability data.

## Installation discovery

An installation may be anywhere on disk. Tool discovery searches the selected root recursively and currently recognizes:

- `llama-server`
- `llama-bench`
- `llama-fit-params`

No `bin\\` layout is assumed. A usable first-slice installation must expose at least `llama-server` or `llama-bench`.

## Capability discovery

CLI option tokens are extracted from the actual selected binaries' help text. The registry is deliberately evidence-based: option presence does not mean LlamaManager understands or is allowed to tune that option. Later milestones will promote known options into richer structured metadata while unknown options remain visible and untuned.

## Process execution

Managed tools are always launched as an executable path plus an argument vector. LlamaManager does not build a shell command and execute it through PowerShell or `cmd.exe`. This is required for correctness with spaces/Unicode and reduces injection risk.

A non-zero managed-process exit is a failure. stdout, stderr, and exit status are evidence and must not be converted into a fake successful result.

## GGUF

Model architecture and relevant metadata are read from the GGUF itself rather than guessed from filenames. The current parser supports GGUF metadata versions 2 and 3 and preserves unknown key/value metadata in a generic representation. The complete selected model is SHA-256 hashed to provide a stable evidence identity.

## Benchmarking

The first vertical slice executes the selected installation's real `llama-bench` binary. The current baseline uses three repetitions and requests JSON output when the selected binary advertises `--output` or `-o`; otherwise a markdown parser is used.

Every stored run retains:

- installation identity
- model identity
- exact executable path and argv
- start/end timestamps
- exit code
- raw stdout/stderr
- parsed benchmark samples

The exact effective invocation is shown before execution and after completion.
