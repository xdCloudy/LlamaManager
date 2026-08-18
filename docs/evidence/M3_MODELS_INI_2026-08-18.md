# Milestone 3 — models.ini C5 evidence

Date: 2026-08-18

## Scope

Milestone 3 covers the `models.ini` parser, inheritance/effective-value engine, canonical structured/raw editor session, validation and pre-apply diff, safe managed/external writes with backup/restore, evidence-backed profile generation, and rendered Config Lab workflows.

## Implementation evidence

Merged implementation/runtime PRs include:

- #95 — lossless `models.ini` parser
- #98 — inheritance/effective-value provenance
- #100 — capability-aware validation and pre-apply diff
- #108 — canonical structured/raw editor session
- #105 — safe managed/external writes, backup, atomic replacement and restore
- #110 — evidence-backed profile generator
- #111 — automated Windows runtime/recovery acceptance
- #119 — rendered CONFIG LAB workflow
- #120 — deterministic long-file/rendered acceptance fixture
- #121 — PRE-APPLY EVIDENCE wrapping/containment fix after interactive QA

## Automated evidence

The merged test matrix covers:

- exact no-op round-trip preservation including comments, blank lines, unknown keys, Unicode, spaces and line endings;
- deterministic `[*]` inheritance and per-model override provenance;
- malformed syntax and semantic-validation rejection;
- source/effective pre-apply diff;
- canonical RAW/STRUCTURED state with invalid raw draft retention;
- 10k-key canonical-session regression;
- safe external and managed writes, same-directory backup, atomic replacement and restore;
- invalid/locked write preservation;
- external structured and raw save/reopen round trips;
- backup/restore recovery after deliberate bad write;
- managed restart-equivalent persistence;
- generated-profile write/reopen without semantic/document drift.

## Interactive Windows verification

The repository owner ran `scripts/prepare-m3-config-validation.ps1` and completed sections A–E of the rendered acceptance checklist in the real LlamaWave release UI.

Verified:

- normal external fixture opened in CONFIG LAB;
- `[agent 模型]` and global `[*]` values rendered with inherited/override provenance;
- structured `threads` change produced a non-empty BEFORE/AFTER diff;
- malformed raw input produced a contextual RAW PARSE ERROR and disabled VALIDATE + SAVE;
- fixing the malformed raw line recovered validation/diff;
- external save reported backup creation and RESTORE BACKUP returned the prior value;
- managed configuration survived a complete application restart;
- Unicode/space paths and CRLF/comment-heavy source remained usable;
- a 4,000-comment long-file fixture remained responsive through RAW ↔ STRUCTURED switching and narrow-window use;
- no fake save/success state was observed.

Interactive QA found one cosmetic-but-real defect: long diagnostic/diff text escaped the PRE-APPLY EVIDENCE panel. PR #121 fixed containment/wrapping and strict Windows CI run `32102022938` passed. The owner accepted the corrected workflow as usable and explicitly chose not to block milestone completion on further cosmetic UI polish.

## Final CI

PR #121 CI run `32102022938` passed:

```text
PowerShell syntax                                      PASS
cargo fmt --all -- --check                            PASS
cargo check --all-targets                             PASS
cargo test --all-targets                              PASS
cargo clippy --all-targets --all-features -D warnings PASS
cargo build --release                                 PASS
desktop process smoke                                 PASS
portable bundle assembly/upload                       PASS
```

## Limitations / deferred polish

Residual visual polish that does not affect truthfulness, safety or usability is deferred to later product-polish work. No known parser/editor/write fake-success, destructive-write or data-loss defect remains open for M3 acceptance.

## Result

M3 satisfies the applicable G1–G10 completion gates and is closure-ready at **C5 — Complete**.
