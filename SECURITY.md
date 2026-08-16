# Security Policy

## Scope

LlamaManager launches user-selected local executables, reads local GGUF/configuration files, stores benchmark/runtime evidence, and may eventually manage local inference servers. Security-sensitive behavior therefore includes process execution, path handling, configuration mutation, secret handling, diagnostics, and loopback/network exposure.

## Baseline requirements

- Never concatenate arbitrary shell command strings for managed llama.cpp processes.
- Pass an explicit executable path and argument array.
- Treat selected executables as external/untrusted inputs until identified and inspected.
- Do not log API keys or secret-bearing configuration values.
- Redact diagnostic exports by default.
- Prefer loopback binding for managed local services unless the user explicitly configures otherwise.
- Preserve exact executable hashes and configuration evidence where practical.
- Validate/canonicalize paths without breaking legitimate spaces, Unicode, custom drives, or portable relocation.

## Reporting

For now, report security issues privately to the repository owner rather than opening a public issue containing exploit details or secrets.
