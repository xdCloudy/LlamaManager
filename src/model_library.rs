use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::{
    error::{LlamaManagerError, Result},
    gguf::{ModelInfo, inspect_gguf},
    llama::now_ms,
    model_store::{FileFingerprint, ModelStore},
    multimodal::{ProjectorInfo, is_projector_gguf},
};

const EDGE_SAMPLE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanIssueKind {
    Walk,
    Fingerprint,
    Inspect,
    Persistence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanIssue {
    pub path: Option<PathBuf>,
    pub kind: ScanIssueKind,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanProgress {
    pub visited_entries: u64,
    pub gguf_candidates: u64,
    pub full_inspections: u64,
    pub reused_unchanged: u64,
    pub models_saved: u64,
    pub projectors_saved: u64,
    pub errors: u64,
    pub current_path: Option<PathBuf>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub root: PathBuf,
    pub progress: ScanProgress,
    pub issues: Vec<ScanIssue>,
    pub started_at_unix_ms: u128,
    pub finished_at_unix_ms: u128,
}

impl ScanReport {
    pub fn summary_line(&self) -> String {
        let state = if self.progress.cancelled {
            "cancelled"
        } else {
            "complete"
        };
        format!(
            "{state}: {} GGUF candidates, {} models, {} projectors, {} reused, {} errors",
            self.progress.gguf_candidates,
            self.progress.models_saved,
            self.progress.projectors_saved,
            self.progress.reused_unchanged,
            self.progress.errors
        )
    }
}

pub fn fingerprint_file(path: &Path) -> Result<FileFingerprint> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(LlamaManagerError::InvalidPath(path.to_path_buf()));
    }

    let file_size = metadata.len();
    let modified_at_unix_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis());

    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    hasher.update(b"llamamanager:edge-fingerprint:v1\n");
    hasher.update(file_size.to_le_bytes());
    if let Some(modified) = modified_at_unix_ms {
        hasher.update(modified.to_le_bytes());
    }

    let head_len =
        usize::try_from(file_size.min(EDGE_SAMPLE_BYTES as u64)).unwrap_or(EDGE_SAMPLE_BYTES);
    let mut head = vec![0_u8; head_len];
    file.read_exact(&mut head)?;
    hasher.update(&head);

    if file_size > EDGE_SAMPLE_BYTES as u64 {
        let tail_len =
            usize::try_from(file_size.min(EDGE_SAMPLE_BYTES as u64)).unwrap_or(EDGE_SAMPLE_BYTES);
        file.seek(SeekFrom::End(-(tail_len as i64)))?;
        let mut tail = vec![0_u8; tail_len];
        file.read_exact(&mut tail)?;
        hasher.update(&tail);
    }

    Ok(FileFingerprint {
        file_size,
        modified_at_unix_ms,
        edge_sha256: hex::encode(hasher.finalize()),
    })
}

pub fn scan_root(
    store: &ModelStore,
    root: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(&ScanProgress),
) -> Result<ScanReport> {
    if !root.is_dir() {
        return Err(LlamaManagerError::InvalidPath(root.to_path_buf()));
    }

    store.upsert_scan_root(root)?;
    let started_at_unix_ms = now_ms();
    let mut progress = ScanProgress::default();
    let mut issues = Vec::new();
    let mut seen_model_paths = BTreeSet::new();
    let mut seen_projector_paths = BTreeSet::new();
    on_progress(&progress);

    // Reparse points/symlinks are deliberately not followed. There is no arbitrary
    // depth limit: a user-selected real directory tree is scanned recursively.
    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        if cancel.load(Ordering::SeqCst) {
            progress.cancelled = true;
            break;
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                progress.errors += 1;
                issues.push(ScanIssue {
                    path: error.path().map(Path::to_path_buf),
                    kind: ScanIssueKind::Walk,
                    message: error.to_string(),
                });
                on_progress(&progress);
                continue;
            }
        };

        progress.visited_entries += 1;
        if entry.file_type().is_symlink() || !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let is_gguf = path
            .extension()
            .map(|extension| extension.to_string_lossy().eq_ignore_ascii_case("gguf"))
            .unwrap_or(false);
        if !is_gguf {
            continue;
        }

        progress.gguf_candidates += 1;
        progress.current_path = Some(path.to_path_buf());
        on_progress(&progress);

        let fingerprint = match fingerprint_file(path) {
            Ok(value) => value,
            Err(error) => {
                progress.errors += 1;
                let message = error.to_string();
                if let Err(mark_error) = store.mark_known_path_unreadable(path, &message) {
                    progress.errors += 1;
                    issues.push(ScanIssue {
                        path: Some(path.to_path_buf()),
                        kind: ScanIssueKind::Persistence,
                        message: format!("failed to persist unreadable state: {mark_error}"),
                    });
                }
                issues.push(ScanIssue {
                    path: Some(path.to_path_buf()),
                    kind: ScanIssueKind::Fingerprint,
                    message,
                });
                on_progress(&progress);
                continue;
            }
        };

        if let Some(location) = store.model_location_by_path(path)?
            && location.fingerprint == fingerprint
        {
            if let Err(error) = store.touch_model_location(path) {
                progress.errors += 1;
                issues.push(ScanIssue {
                    path: Some(path.to_path_buf()),
                    kind: ScanIssueKind::Persistence,
                    message: error.to_string(),
                });
            } else {
                progress.reused_unchanged += 1;
                seen_model_paths.insert(path.to_path_buf());
            }
            on_progress(&progress);
            continue;
        }

        if let Some(projector) = store.projector_by_path(path)?
            && projector.fingerprint == fingerprint
        {
            if let Err(error) = store.touch_projector(path) {
                progress.errors += 1;
                issues.push(ScanIssue {
                    path: Some(path.to_path_buf()),
                    kind: ScanIssueKind::Persistence,
                    message: error.to_string(),
                });
            } else {
                progress.reused_unchanged += 1;
                seen_projector_paths.insert(path.to_path_buf());
            }
            on_progress(&progress);
            continue;
        }

        progress.full_inspections += 1;
        let inspected = match inspect_gguf(path) {
            Ok(value) => value,
            Err(error) => {
                progress.errors += 1;
                let message = error.to_string();
                if let Err(mark_error) = store.mark_known_path_unreadable(path, &message) {
                    progress.errors += 1;
                    issues.push(ScanIssue {
                        path: Some(path.to_path_buf()),
                        kind: ScanIssueKind::Persistence,
                        message: format!("failed to persist unreadable state: {mark_error}"),
                    });
                }
                issues.push(ScanIssue {
                    path: Some(path.to_path_buf()),
                    kind: ScanIssueKind::Inspect,
                    message,
                });
                on_progress(&progress);
                continue;
            }
        };

        if is_projector_gguf(&inspected) {
            let Some(projector) = ProjectorInfo::from_gguf(&inspected) else {
                progress.errors += 1;
                issues.push(ScanIssue {
                    path: Some(path.to_path_buf()),
                    kind: ScanIssueKind::Inspect,
                    message: "GGUF exposed projector markers but projector evidence could not be constructed".into(),
                });
                on_progress(&progress);
                continue;
            };
            match store.save_projector(&projector, &fingerprint) {
                Ok(()) => {
                    progress.projectors_saved += 1;
                    seen_projector_paths.insert(path.to_path_buf());
                }
                Err(error) => {
                    progress.errors += 1;
                    issues.push(ScanIssue {
                        path: Some(path.to_path_buf()),
                        kind: ScanIssueKind::Persistence,
                        message: error.to_string(),
                    });
                }
            }
        } else {
            match store.save_model_with_location(&inspected, &fingerprint) {
                Ok(()) => {
                    progress.models_saved += 1;
                    seen_model_paths.insert(path.to_path_buf());
                }
                Err(error) => {
                    progress.errors += 1;
                    issues.push(ScanIssue {
                        path: Some(path.to_path_buf()),
                        kind: ScanIssueKind::Persistence,
                        message: error.to_string(),
                    });
                }
            }
        }
        on_progress(&progress);
    }

    progress.current_path = None;
    if !progress.cancelled {
        if let Err(error) = store.reconcile_model_locations(root, &seen_model_paths) {
            progress.errors += 1;
            issues.push(ScanIssue {
                path: Some(root.to_path_buf()),
                kind: ScanIssueKind::Persistence,
                message: error.to_string(),
            });
        }
        if let Err(error) = store.reconcile_projectors(root, &seen_projector_paths) {
            progress.errors += 1;
            issues.push(ScanIssue {
                path: Some(root.to_path_buf()),
                kind: ScanIssueKind::Persistence,
                message: error.to_string(),
            });
        }
    }

    let report = ScanReport {
        root: root.to_path_buf(),
        progress,
        issues,
        started_at_unix_ms,
        finished_at_unix_ms: now_ms(),
    };
    store.save_scan_summary(&report)?;
    on_progress(&report.progress);
    Ok(report)
}

pub fn manual_add_model(store: &ModelStore, path: &Path) -> Result<ModelInfo> {
    let fingerprint = fingerprint_file(path)?;
    let model = inspect_gguf(path)?;
    if is_projector_gguf(&model) {
        return Err(LlamaManagerError::Unsupported(
            "selected GGUF is a multimodal projector; add it as a projector instead".into(),
        ));
    }
    store.save_model_with_location(&model, &fingerprint)?;
    Ok(model)
}

pub fn manual_add_projector(store: &ModelStore, path: &Path) -> Result<ProjectorInfo> {
    let fingerprint = fingerprint_file(path)?;
    let inspected = inspect_gguf(path)?;
    let projector = ProjectorInfo::from_gguf(&inspected).ok_or_else(|| {
        LlamaManagerError::Unsupported(
            "selected GGUF does not contain recognized projector/CLIP metadata evidence".into(),
        )
    })?;
    store.save_projector(&projector, &fingerprint)?;
    Ok(projector)
}

pub fn relink_model(store: &ModelStore, model_id: &str, new_path: &Path) -> Result<ModelInfo> {
    let expected = store
        .get_model(model_id)?
        .ok_or_else(|| LlamaManagerError::State(format!("model {model_id} is not persisted")))?;
    let fingerprint = fingerprint_file(new_path)?;
    let inspected = inspect_gguf(new_path)?;
    if is_projector_gguf(&inspected) {
        return Err(LlamaManagerError::State(
            "relink target is a projector rather than the requested model".into(),
        ));
    }
    if inspected.sha256 != expected.sha256 {
        return Err(LlamaManagerError::State(format!(
            "relink rejected: selected file SHA-256 {} does not match model SHA-256 {}",
            inspected.sha256, expected.sha256
        )));
    }
    store.relink_model(model_id, &inspected, &fingerprint)?;
    Ok(inspected)
}

pub fn relink_projector(
    store: &ModelStore,
    projector_id: &str,
    new_path: &Path,
) -> Result<ProjectorInfo> {
    let expected = store.get_projector(projector_id)?.ok_or_else(|| {
        LlamaManagerError::State(format!("projector {projector_id} is not persisted"))
    })?;
    let fingerprint = fingerprint_file(new_path)?;
    let inspected = inspect_gguf(new_path)?;
    let projector = ProjectorInfo::from_gguf(&inspected).ok_or_else(|| {
        LlamaManagerError::State("relink target is not a recognized projector GGUF".into())
    })?;
    if projector.sha256 != expected.projector.sha256 {
        return Err(LlamaManagerError::State(format!(
            "projector relink rejected: selected file SHA-256 {} does not match stored SHA-256 {}",
            projector.sha256, expected.projector.sha256
        )));
    }
    store.relink_projector(projector_id, &projector, &fingerprint)?;
    Ok(projector)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelinkResolution {
    NoMatch,
    Unique(String),
    Ambiguous(Vec<String>),
}

pub fn identify_relink_target(store: &ModelStore, path: &Path) -> Result<RelinkResolution> {
    let inspected = inspect_gguf(path)?;
    if is_projector_gguf(&inspected) {
        return Ok(RelinkResolution::NoMatch);
    }
    let matches = store.model_ids_by_sha(&inspected.sha256)?;
    Ok(classify_relink_matches(matches))
}

fn classify_relink_matches(mut matches: Vec<String>) -> RelinkResolution {
    matches.sort();
    matches.dedup();
    match matches.len() {
        0 => RelinkResolution::NoMatch,
        1 => RelinkResolution::Unique(matches.remove(0)),
        _ => RelinkResolution::Ambiguous(matches),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn open_store(path: &Path) -> ModelStore {
        crate::persistence::Database::open(path).unwrap();
        ModelStore::open(path).unwrap()
    }

    fn push_string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }

    fn write_minimal_model(path: &Path, architecture: &str, name: &str) {
        let mut data = Vec::new();
        data.extend_from_slice(b"GGUF");
        data.extend_from_slice(&3_u32.to_le_bytes());
        data.extend_from_slice(&0_u64.to_le_bytes());
        data.extend_from_slice(&5_u64.to_le_bytes());

        for (key, value) in [
            ("general.type", "model"),
            ("general.name", name),
            ("general.architecture", architecture),
        ] {
            push_string(&mut data, key);
            data.extend_from_slice(&8_u32.to_le_bytes());
            push_string(&mut data, value);
        }

        push_string(&mut data, "general.quantization_version");
        data.extend_from_slice(&10_u32.to_le_bytes());
        data.extend_from_slice(&2_u64.to_le_bytes());

        push_string(&mut data, "general.file_type");
        data.extend_from_slice(&10_u32.to_le_bytes());
        data.extend_from_slice(&15_u64.to_le_bytes());

        std::fs::write(path, data).unwrap();
    }

    #[test]
    fn manual_add_accepts_spaces_and_unicode_and_survives_restart() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("library.sqlite");
        let folder = temp.path().join("Models with spaces 模型");
        std::fs::create_dir(&folder).unwrap();
        let model_path = folder.join("Qwen 模型.gguf");
        write_minimal_model(&model_path, "qwen35", "Unicode model");

        let store = open_store(&db_path);
        let added = manual_add_model(&store, &model_path).unwrap();
        drop(store);

        let reopened = open_store(&db_path);
        let records = reopened.list_model_records().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model.id, added.id);
        assert_eq!(records[0].present_paths(), vec![model_path.as_path()]);
    }

    #[test]
    fn scan_reports_bad_file_without_aborting_good_discovery_and_reuses_unchanged() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("scan root 模型");
        std::fs::create_dir(&root).unwrap();
        write_minimal_model(&root.join("good.gguf"), "qwen35", "Good");
        std::fs::write(root.join("bad.gguf"), b"not a gguf").unwrap();
        let db_path = temp.path().join("store.sqlite");
        let store = open_store(&db_path);
        let cancel = AtomicBool::new(false);

        let first = scan_root(&store, &root, &cancel, |_| {}).unwrap();
        assert_eq!(first.progress.models_saved, 1);
        assert_eq!(first.progress.errors, 1);
        assert_eq!(store.list_model_records().unwrap().len(), 1);

        let second = scan_root(&store, &root, &cancel, |_| {}).unwrap();
        assert_eq!(second.progress.reused_unchanged, 1);
        assert_eq!(second.progress.full_inspections, 1); // malformed file is retried; valid file is not re-hashed
        assert_eq!(store.list_model_records().unwrap().len(), 1);
    }

    #[test]
    fn cancellation_does_not_reconcile_unseen_records_as_missing() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("models");
        std::fs::create_dir(&root).unwrap();
        let model_path = root.join("model.gguf");
        write_minimal_model(&model_path, "qwen35", "Model");
        let db_path = temp.path().join("store.sqlite");
        let store = open_store(&db_path);
        manual_add_model(&store, &model_path).unwrap();

        let cancel = AtomicBool::new(true);
        let report = scan_root(&store, &root, &cancel, |_| {}).unwrap();
        assert!(report.progress.cancelled);
        assert!(!store.list_model_records().unwrap()[0].is_missing());
    }

    #[test]
    fn relink_preserves_model_identity_and_rejects_different_content() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first.gguf");
        let moved = temp.path().join("renamed 模型.gguf");
        let other = temp.path().join("other.gguf");
        write_minimal_model(&first, "qwen35", "Same");
        std::fs::copy(&first, &moved).unwrap();
        write_minimal_model(&other, "qwen35", "Different");

        let db_path = temp.path().join("store.sqlite");
        let store = open_store(&db_path);
        let model = manual_add_model(&store, &first).unwrap();
        std::fs::remove_file(&first).unwrap();
        store.refresh_location_existence().unwrap();
        assert!(store.list_model_records().unwrap()[0].is_missing());

        let relinked = relink_model(&store, &model.id, &moved).unwrap();
        assert_eq!(relinked.id, model.id);
        assert!(relink_model(&store, &model.id, &other).is_err());
    }

    #[test]
    fn ambiguous_relink_matches_are_explicit_and_deterministic() {
        assert_eq!(
            classify_relink_matches(vec!["b".into(), "a".into(), "b".into()]),
            RelinkResolution::Ambiguous(vec!["a".into(), "b".into()])
        );
    }
}
