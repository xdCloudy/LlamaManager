# M3 configuration write, backup and restore policy

This document records the safety contract implemented by `src/config_write.rs`.

## Encoding and line endings

LlamaManager writes the exact UTF-8 bytes represented by the validated in-memory document. It does not inject a BOM and does not normalize CRLF/LF line endings during the write layer. Preservation/serialization decisions belong to the lossless `models.ini` document model, not to the filesystem writer.

## Managed configuration

The managed configuration destination is deterministic and application-owned:

```text
<AppPaths.config>/models.ini
```

The location therefore follows the existing portable/user-data root policy and remains relocatable. Existing managed files are backed up before replacement even though external/user-owned files are the mandatory backup case.

## External configuration

An existing external target is never mutated until:

1. semantic validation reports zero errors;
2. the target is confirmed to be a regular file;
3. a recoverable same-directory backup has been created and flushed;
4. the replacement contents have been fully written, flushed and synced to a same-directory temporary file.

Only then is the target replaced.

## Replacement semantics

On Windows, existing targets are replaced with `ReplaceFileW`, using the prepared same-directory temporary file. This avoids shell command construction and uses the platform file-replacement primitive while preserving the original file identity/metadata semantics Windows provides.

For a new file, normal same-directory rename is used. On non-Windows test/development hosts, `rename` is used for replacement.

If backup creation, temporary-file write/flush, or replacement fails, the operation returns a typed `ConfigWriteError` with the failed action and path. It does not report success and best-effort cleans up the temporary file.

## Restore

Restore deliberately does not require the current target to parse or validate: it is a recovery path for a bad edit. If a current target exists, LlamaManager first creates a new backup of that pre-restore state, then copies the selected backup to a synced same-directory temporary file and performs the same replacement operation.

This means a restore can recover the known-good backup without silently discarding the state being replaced.

## Backup naming and retention

Backups are stored beside the target using:

```text
<filename>.llamamanager-backup-<unix-ms>-<counter>.bak
```

The default retention is **5** backups per exact target filename. A caller may request another bound, but retention is clamped to at least **1** so an existing-file mutation never intentionally removes every recovery point.

Cleanup only considers files matching the exact LlamaManager backup prefix for that target. Unrelated `.bak` files are never pruned.

## Failure coverage

Automated tests cover:

- validation blocking before any mutation;
- deterministic managed path resolution;
- spaces and Unicode paths;
- exact UTF-8 and CRLF/LF byte preservation;
- external backup creation;
- recovery from a deliberately bad edit;
- bounded backup retention;
- directory/non-file rejection;
- Windows exclusive-lock failure without destructive mutation.

Permission, read-only and sharing violations surface through the same actionable typed I/O error contract and remain failures rather than fallback success.
