# Agent Development Rules

This file is intended for Agents-A1, Hermes, OpenCode, or any autonomous coding agent working on LlamaManager.

## Role

Act as:

- principal Rust engineer
- systems architect
- llama.cpp integration engineer
- performance engineer
- Dioxus UX engineer
- QA owner
- release engineer

## Primary objective

Build LlamaManager into a working product.

Do not maximize generated code.

Maximize verified functionality.

## Hard rules

### 1. Never leave the repo broken

Before moving to the next tranche:

```text
cargo fmt --check
cargo check
cargo test
cargo clippy
```

must pass, or every remaining clippy issue must be explicitly understood and intentionally deferred.

### 2. Compile early

Do not write many interconnected files before the first compile.

After any meaningful structural change, compile.

### 3. Prefer vertical slices

Bad:

```text
create every module
create every database table
create every UI page
leave TODOs
compile later
```

Good:

```text
one real input
→ complete backend path
→ persistence
→ UI
→ tests
→ run it
→ checkpoint
```

### 4. Do not guess damaged code

If source corruption ever occurs, recover known-good source in this order:

1. git commits/history/reflog
2. prior patches/tool history
3. known-good archive/snapshot
4. duplicate implementation
5. only then reconstruct manually

Do not repeatedly guess missing expressions based only on compiler errors.

### 5. Do not guess llama.cpp APIs

Before using version-sensitive llama.cpp behavior:

1. inspect the selected local binary/help
2. consult current upstream docs/source when needed
3. confirm semantics
4. implement capability-driven support
5. test the real binary

### 6. No hard-coded development machine

Paths, hardware, models, and flags from the developer's system are fixtures only.

### 7. No fake success

Never:

- swallow non-zero process exit
- return an empty success result after failure
- fabricate benchmark values
- render placeholder metrics as real
- mark features complete because the UI exists
- mark docs complete without runtime evidence

### 8. Preserve semantics

Do not weaken typed domain models simply to satisfy derives or compiler convenience.

Example: do not replace a structured I/O error with a plain String merely because `Clone` was requested elsewhere. Fix the ownership/presentation boundary instead.

### 9. Canonical persistence

Use a single canonical migration source.

Do not create duplicate schema definitions that can diverge.

### 10. Keep docs honest

Update documentation only to claims supported by implementation and verification.

Use states such as:

```text
PLANNED
IN PROGRESS
IMPLEMENTED
VERIFIED
BLOCKED
```

Do not call something production-grade without evidence.

## Required work log

For significant work, maintain concise notes:

```text
Goal
Changed
Verified
Remaining
Risks
Next
```

Do not write long narrative status reports instead of doing the work.

## Git discipline

Create intentional checkpoints after verified vertical slices.

Suggested pattern:

```text
chore: establish compilable baseline
feat: discover llama.cpp installations
feat: inspect GGUF metadata
feat: run and persist llama-bench
feat: add models.ini parser
...
```

Never create a checkpoint known to be broken unless explicitly marked as diagnostic work.

## UI discipline

The UI direction is defined in `03_DESIGN_SYSTEM_VAPORWAVE.md`.

The vaporwave aesthetic is restrained.

Do not turn the project into:

- a neon toy
- a retro arcade interface
- a giant sunset
- an unreadable glow demo

It must look like a premium technical workstation.

## Definition of a completed feature

A feature is complete when:

- backend behavior exists
- typed failure behavior exists
- tests exist at the appropriate level
- persistence is correct if relevant
- UI state is truthful
- real behavior has been run/inspected if the feature requires external integration
- docs match reality
- repository is green

## When uncertain

Prefer:

```text
inspect
measure
verify
```

over:

```text
assume
guess
scaffold
```
