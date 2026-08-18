# M2 Model Library Runtime Evidence — 2026-08-18

## Source and CI

The final user-facing Model Library workflow was merged in PR #117 at commit `a504eb6a1181dbcccf7b0a4191d5a0200607a463`.

Strict Windows CI run `32098153891` passed the complete repository gate:

```text
PowerShell syntax             PASS
cargo fmt --all -- --check   PASS
cargo check --all-targets    PASS
cargo test --all-targets     PASS
strict Clippy                PASS
cargo build --release        PASS
desktop process smoke        PASS
portable bundle assembly     PASS
portable bundle upload       PASS
```

## Automated acceptance

The M2 automated suite covers recursive and idempotent scanning, spaces and Unicode paths, directory-junction recursion safety, cancellation without false missing-state reconciliation, locked/unreadable file isolation, content-derived deduplication, move/relink identity preservation, compatibility staleness, projector association/capability behaviour, and restart persistence.

The merged UI exposes real scan/add/relink/remove actions, present/missing/unreadable location evidence, compatibility status and reasons, projector association state, scan evidence, and responsive/reduced-motion layouts. Source GGUF files are never deleted by library-entry removal.

## Interactive Windows acceptance

The repository owner exercised the release application on a real interactive Windows desktop using the same real, hash-pinned GGUF identity used by the prior runtime-validation work.

The exercised workflow included:

1. A manual model add independent of any scan root.
2. Recursive scan of a user-selected path containing both spaces and Unicode:

   ```text
   C:\LlamaManager\artifacts\M2 模型 library with spaces
   ```

3. The completed scan reported three GGUF candidates, two discovered model files, no projectors, and one isolated error. The corrupt/non-GGUF input did not poison discovery of the valid model.
4. Byte-identical duplicate copies resolved to one content identity with multiple locations instead of creating conflicting model identities.
5. The selected text model displayed `PRESENT` and `COMPATIBLE` with evidence-backed reasons, including architecture recognition and the explicit absence of a required projector.
6. All currently present copies were moved or removed, followed by `REFRESH PATHS`. The single model identity changed truthfully to `MISSING`, all known locations were shown as missing, and the `RELINK` control became available.
7. The operator relinked the same content identity to:

   ```text
   C:\LlamaManager\artifacts\M2 moved 模型\stories moved.gguf
   ```

8. The UI reported `Model relinked by matching content identity.` and returned the model to `PRESENT` while retaining the old missing locations as evidence.
9. The application was closed and reopened. The relinked location, library identity, and compatibility state persisted across restart.
10. The Model Library was visually inspected at both normal desktop width and a narrow desktop window; the exercised workflow remained usable and its states remained readable.

Operator screenshots were reviewed during the #21 acceptance session. Issue #21 records the human/runtime acceptance result; this document records the reproducible facts without embedding private conversation attachments.

## Truthfulness and limitations

- The interactive model was text-only and did not require an mmproj. Projector discovery, association, ambiguity, and compatibility remain covered by deterministic automated acceptance tests; no interactive multimodal claim is made here.
- No separate synthetic large-library throughput benchmark was run. Blocking GGUF/database work is dispatched off the UI thread, scan cancellation is implemented, and the exercised normal workflow remained responsive. This evidence does not claim a numeric large-library performance bound.
- Model-library scanning and GGUF inspection accepted Unicode paths. The separately documented upstream `llama-bench` b10472 Unicode-model-path limitation from M1 remains an upstream benchmark boundary and is not hidden by M2.
- Compatibility is derived from inspected GGUF metadata plus the selected runtime's discovered capability evidence. Filenames are not used to fabricate architecture or capability support.

## Result

M2's implementation, automated verification, real Windows Model Library workflow, failure handling, restart persistence, and UI truthfulness have all been exercised at closure quality. Together with the final documentation/regression pass in #22, this evidence supports promotion of Milestone 2 to **C5 — Complete**.