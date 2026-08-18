#![cfg(windows)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
};

use llamamanager::{
    compatibility::{CompatibilityStatus, evaluate_compatibility},
    gguf::{MetadataValue, ModelInfo},
    llama::{LlamaInstallation, ToolEvidence},
    model_library::{fingerprint_file, manual_add_model, relink_model, scan_root},
    model_store::ModelStore,
    multimodal::{Modality, ProjectorInfo},
    persistence::Database,
};
use std::os::windows::fs::OpenOptionsExt;
use tempfile::tempdir;

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

    fs::write(path, data).unwrap();
}

fn open_store(path: &Path) -> ModelStore {
    Database::open(path).unwrap();
    ModelStore::open(path).unwrap()
}

fn text_model(id: &str, path: PathBuf, architecture: &str) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        path,
        file_size: 8,
        sha256: "a".repeat(64),
        gguf_version: 3,
        tensor_count: 1,
        metadata_count: 5,
        name: Some("acceptance-model".into()),
        architecture: Some(architecture.into()),
        context_length: Some(4096),
        quantization_version: Some(2),
        general_type: Some("model".into()),
        file_type: Some(15),
        parameter_count: Some(16),
        tensor_type_counts: BTreeMap::from([(12, 1)]),
        metadata: BTreeMap::from([(
            "general.architecture".into(),
            MetadataValue::String(architecture.into()),
        )]),
        inspected_at_unix_ms: 1,
    }
}

fn installation(id: &str, help: &str) -> LlamaInstallation {
    let tool = ToolEvidence {
        path: PathBuf::from(r"C:\runtime with spaces 外部\llama-server.exe"),
        sha256: "b".repeat(64),
        version_output: "version evidence".into(),
        help_output: help.into(),
        device_output: "CPU".into(),
    };
    LlamaInstallation {
        id: id.into(),
        name: "runtime".into(),
        root_path: PathBuf::from(r"C:\runtime with spaces 外部"),
        server: Some(tool),
        bench: None,
        fit_params: None,
        backend: Some("CPU".into()),
        capabilities: BTreeSet::from(["--model".into(), "--mmproj".into()]),
        discovered_at_unix_ms: 1,
    }
}

fn projector(id: &str, path: PathBuf, sha: char, modalities: BTreeSet<Modality>) -> ProjectorInfo {
    ProjectorInfo {
        id: id.into(),
        path,
        file_size: 8,
        sha256: sha.to_string().repeat(64),
        name: Some(id.into()),
        general_type: Some("mmproj".into()),
        architecture: Some("clip".into()),
        projector_type: Some("fixture".into()),
        modalities,
        source_model_hint: Some("acceptance-model".into()),
        inspected_at_unix_ms: 1,
    }
}

#[test]
fn scan_is_recursive_idempotent_dedupes_content_and_preserves_restart_state() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("Model Library with spaces 模型");
    let deep = root.join("one").join("two").join("three");
    fs::create_dir_all(&deep).unwrap();

    let first = deep.join("first 模型.gguf");
    let duplicate = root.join("duplicate same content.gguf");
    write_minimal_model(&first, "qwen35", "Evidence model");
    fs::copy(&first, &duplicate).unwrap();
    fs::write(root.join("corrupt but isolated.gguf"), b"not a GGUF").unwrap();

    let db = temp.path().join("library.sqlite");
    let store = open_store(&db);
    let cancel = AtomicBool::new(false);
    let first_report = scan_root(&store, &root, &cancel, |_| {}).unwrap();

    assert_eq!(first_report.progress.models_saved, 2);
    assert_eq!(first_report.progress.errors, 1);
    let records = store.list_model_records().unwrap();
    assert_eq!(
        records.len(),
        1,
        "duplicate content must be one model identity"
    );
    assert_eq!(records[0].locations.len(), 2);

    let second_report = scan_root(&store, &root, &cancel, |_| {}).unwrap();
    assert_eq!(second_report.progress.reused_unchanged, 2);
    assert_eq!(store.list_model_records().unwrap().len(), 1);
    drop(store);

    let reopened = open_store(&db);
    let restarted_records = reopened.list_model_records().unwrap();
    assert_eq!(restarted_records.len(), 1);
    assert_eq!(restarted_records[0].locations.len(), 2);
    assert_eq!(reopened.list_scan_roots().unwrap(), vec![root]);
}

#[test]
fn locked_file_isolated_without_aborting_other_windows_paths() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("locked path test 模型");
    fs::create_dir_all(&root).unwrap();
    let good = root.join("good.gguf");
    let locked = root.join("locked unreadable.gguf");
    write_minimal_model(&good, "qwen35", "Good");
    write_minimal_model(&locked, "qwen35", "Locked");

    let exclusive = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked)
        .unwrap();

    let db = temp.path().join("library.sqlite");
    let store = open_store(&db);
    let report = scan_root(&store, &root, &AtomicBool::new(false), |_| {}).unwrap();
    drop(exclusive);

    assert_eq!(store.list_model_records().unwrap().len(), 1);
    assert!(report.progress.errors >= 1);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.path.as_deref() == Some(locked.as_path()))
    );
}

#[test]
fn directory_junction_is_not_followed_during_recursive_scan() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("scan root 模型");
    let outside = temp.path().join("outside target");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    write_minimal_model(
        &outside.join("must-not-be-followed.gguf"),
        "qwen35",
        "Outside",
    );

    let junction = root.join("junction 外部");
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&outside)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "Windows directory junction creation failed"
    );

    let db = temp.path().join("library.sqlite");
    let store = open_store(&db);
    let report = scan_root(&store, &root, &AtomicBool::new(false), |_| {}).unwrap();
    assert_eq!(report.progress.gguf_candidates, 0);
    assert!(store.list_model_records().unwrap().is_empty());
}

#[test]
fn cancelled_scan_does_not_mark_unseen_model_missing() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("existing 模型.gguf");
    write_minimal_model(&path, "qwen35", "Existing");
    let db = temp.path().join("library.sqlite");
    let store = open_store(&db);
    manual_add_model(&store, &path).unwrap();

    let cancel = AtomicBool::new(true);
    let report = scan_root(&store, temp.path(), &cancel, |_| {}).unwrap();
    assert!(report.progress.cancelled);
    assert!(!store.list_model_records().unwrap()[0].is_missing());
}

#[test]
fn move_and_relink_preserves_evidence_identity_and_rejects_collision() {
    let temp = tempdir().unwrap();
    let first = temp.path().join("original model.gguf");
    let moved = temp.path().join("moved 模型.gguf");
    let collision = temp.path().join("same name different content.gguf");
    write_minimal_model(&first, "qwen35", "Original");
    fs::copy(&first, &moved).unwrap();
    write_minimal_model(&collision, "qwen35", "Different");

    let db = temp.path().join("library.sqlite");
    let store = open_store(&db);
    let added = manual_add_model(&store, &first).unwrap();
    fs::remove_file(&first).unwrap();
    store.refresh_location_existence().unwrap();
    assert!(store.list_model_records().unwrap()[0].is_missing());

    let relinked = relink_model(&store, &added.id, &moved).unwrap();
    assert_eq!(relinked.id, added.id);
    assert!(relink_model(&store, &added.id, &collision).is_err());

    drop(store);
    let reopened = open_store(&db);
    let record = reopened.list_model_records().unwrap().remove(0);
    assert_eq!(record.model.id, added.id);
    assert!(record.present_paths().contains(&moved.as_path()));
}

#[test]
fn compatibility_reasons_persist_and_installation_drift_marks_them_stale() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("library.sqlite");
    let database = Database::open(&db).unwrap();
    let store = ModelStore::open(&db).unwrap();

    let model_path = temp.path().join("compat model.gguf");
    fs::write(&model_path, b"fixture!!").unwrap();
    let model = text_model("model-compat", model_path, "qwen35");
    store
        .save_model_with_location(&model, &fingerprint_file(&model.path).unwrap())
        .unwrap();

    let install = installation("installation-compat", "--model FILE --mmproj FILE");
    database.save_installation(&install).unwrap();
    let result = evaluate_compatibility(&model, &install, None);
    assert_eq!(result.status, CompatibilityStatus::Compatible);
    assert!(!result.reasons.is_empty());
    store.save_compatibility(&result).unwrap();
    drop(store);
    drop(database);

    let reopened = open_store(&db);
    let persisted = reopened
        .load_compatibility(&model.id, &install.id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.status, result.status);
    assert_eq!(persisted.reasons[0].code, result.reasons[0].code);

    let mut changed = install.clone();
    changed.server.as_mut().unwrap().sha256 = "c".repeat(64);
    assert!(persisted.is_stale(&model, &changed));

    let unknown = text_model(
        "model-unknown",
        PathBuf::from("unknown.gguf"),
        "future-arch",
    );
    assert_eq!(
        evaluate_compatibility(&unknown, &install, None).status,
        CompatibilityStatus::Unknown
    );

    let mut partial = model.clone();
    partial.quantization_version = None;
    assert_eq!(
        evaluate_compatibility(&partial, &install, None).status,
        CompatibilityStatus::Limited
    );
}

#[test]
fn projector_choice_association_relink_and_runtime_capability_are_explicit() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("library.sqlite");
    let database = Database::open(&db).unwrap();
    let store = ModelStore::open(&db).unwrap();

    let model_path = temp.path().join("vision model 模型.gguf");
    fs::write(&model_path, b"model-vl").unwrap();
    let model = text_model("model-vl", model_path, "qwen3vl");
    store
        .save_model_with_location(&model, &fingerprint_file(&model.path).unwrap())
        .unwrap();

    let first_path = temp.path().join("mmproj one 模型.gguf");
    let second_path = temp.path().join("mmproj two 模型.gguf");
    let audio_path = temp.path().join("wrong audio projector.gguf");
    fs::write(&first_path, b"project1").unwrap();
    fs::write(&second_path, b"project2").unwrap();
    fs::write(&audio_path, b"audio-pr").unwrap();

    let first = projector(
        "projector-one",
        first_path.clone(),
        '1',
        BTreeSet::from([Modality::Vision]),
    );
    let second = projector(
        "projector-two",
        second_path.clone(),
        '2',
        BTreeSet::from([Modality::Vision]),
    );
    let audio = projector(
        "projector-audio",
        audio_path.clone(),
        '3',
        BTreeSet::from([Modality::Audio]),
    );

    store
        .save_projector(&first, &fingerprint_file(&first_path).unwrap())
        .unwrap();
    store
        .save_projector(&second, &fingerprint_file(&second_path).unwrap())
        .unwrap();
    store
        .save_projector(&audio, &fingerprint_file(&audio_path).unwrap())
        .unwrap();

    let candidates = store.projector_candidates(&model).unwrap();
    assert_eq!(candidates.len(), 3);
    assert!(store.associated_projector(&model.id).unwrap().is_none());
    assert!(store.associate_projector(&model, &audio.id).is_err());
    store.associate_projector(&model, &first.id).unwrap();
    assert_eq!(
        store.associated_projector(&model.id).unwrap().unwrap().id,
        first.id
    );

    let with_mmproj = installation("installation-mmproj", "--model FILE --mmproj FILE");
    database.save_installation(&with_mmproj).unwrap();
    assert_eq!(
        evaluate_compatibility(&model, &with_mmproj, Some(&first)).status,
        CompatibilityStatus::Compatible
    );
    let without_mmproj = installation("installation-no-mmproj", "--model FILE");
    assert_eq!(
        evaluate_compatibility(&model, &without_mmproj, Some(&first)).status,
        CompatibilityStatus::Incompatible
    );
    assert_eq!(
        evaluate_compatibility(&model, &with_mmproj, None).status,
        CompatibilityStatus::Limited
    );

    let moved_path = temp.path().join("moved projector 外部.gguf");
    fs::copy(&first_path, &moved_path).unwrap();
    let mut moved = first.clone();
    moved.path = moved_path.clone();
    store
        .relink_projector(&first.id, &moved, &fingerprint_file(&moved_path).unwrap())
        .unwrap();
    drop(store);
    drop(database);

    let reopened = open_store(&db);
    let associated = reopened.associated_projector(&model.id).unwrap().unwrap();
    assert_eq!(associated.id, first.id);
    assert_eq!(associated.path, moved_path);
}
