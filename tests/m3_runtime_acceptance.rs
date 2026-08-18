use std::{collections::{BTreeMap, BTreeSet}, fs, path::PathBuf};

use llamamanager::{
    compatibility::{ARCHITECTURE_REGISTRY_REVISION, CompatibilityResult, CompatibilityStatus, installation_fingerprint},
    config_write::{ConfigWriteError, managed_models_ini_path, restore_backup, write_external_models_ini, write_managed_models_ini},
    gguf::{MetadataValue, ModelInfo},
    llama::{LlamaInstallation, ToolEvidence},
    models_ini::ModelsIniDocument,
    models_ini_editor::{EditorMode, ModelsIniEditorSession},
    paths::{AppPaths, StorageMode},
    profile_generator::{ProfileDestination, ProfileGenerationRequest, RecommendationBasis, RecommendedSetting, apply_generated_profile, generate_profile},
};

fn installation() -> LlamaInstallation {
    let root = PathBuf::from(r"C:\runtime evidence 外部 with spaces");
    LlamaInstallation {
        id: "runtime-m3".into(),
        name: "runtime".into(),
        root_path: root.clone(),
        server: Some(ToolEvidence {
            path: root.join("llama-server.exe"),
            sha256: "a".repeat(64),
            version_output: "b10472".into(),
            help_output: "--model FILE --threads N --ctx-size N --batch-size N --ubatch-size N".into(),
            device_output: "CPU".into(),
        }),
        bench: None,
        fit_params: None,
        backend: Some("CPU".into()),
        capabilities: BTreeSet::new(),
        discovered_at_unix_ms: 1,
    }
}

fn model() -> ModelInfo {
    ModelInfo {
        id: "model-m3".into(),
        path: PathBuf::from(r"D:\Models 外部\evidence model.gguf"),
        file_size: 123,
        sha256: "b".repeat(64),
        gguf_version: 3,
        tensor_count: 1,
        metadata_count: 3,
        name: Some("M3 Evidence Model".into()),
        architecture: Some("qwen35".into()),
        context_length: Some(32768),
        quantization_version: Some(2),
        general_type: Some("model".into()),
        file_type: Some(7),
        parameter_count: Some(1_000_000),
        tensor_type_counts: BTreeMap::from([(2, 1)]),
        metadata: BTreeMap::from([
            (
                "general.architecture".into(),
                MetadataValue::String("qwen35".into()),
            ),
            ("general.file_type".into(), MetadataValue::UInt(7)),
            (
                "general.quantization_version".into(),
                MetadataValue::UInt(2),
            ),
        ]),
        inspected_at_unix_ms: 1,
    }
}

fn compatibility(model: &ModelInfo, installation: &LlamaInstallation) -> CompatibilityResult {
    CompatibilityResult {
        model_id: model.id.clone(),
        installation_id: installation.id.clone(),
        model_sha256: model.sha256.clone(),
        installation_fingerprint: installation_fingerprint(installation),
        registry_revision: ARCHITECTURE_REGISTRY_REVISION.into(),
        status: CompatibilityStatus::Compatible,
        reasons: Vec::new(),
        computed_at_unix_ms: 1,
    }
}

#[test]
fn external_structured_edit_diff_save_reopen_preserves_crlf_comments_unknown_and_unicode() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("用户 configs with spaces");
    fs::create_dir_all(&directory).unwrap();
    let target = directory.join("models.ini");
    let source = "[*]\r\n# keep this comment\r\nfuture-option=preserve-me\r\nthreads=4\r\n[模型 profile]\r\nmodel=D:\\Models 外部\\evidence model.gguf\r\nthreads=8\r\n";
    fs::write(&target, source.as_bytes()).unwrap();

    let runtime = installation();
    let mut editor = ModelsIniEditorSession::load(fs::read_to_string(&target).unwrap()).unwrap();
    editor.set_value("模型 profile", "threads", "12").unwrap();

    let diff = editor.diff_from_loaded("模型 profile").unwrap();
    assert!(!diff.is_empty());
    let validation = editor.validation("模型 profile", Some(&runtime)).unwrap();
    assert!(validation.can_apply(), "{:?}", validation.issues);

    let receipt = write_external_models_ini(
        &target,
        editor.canonical_source(),
        &validation,
        5,
    )
    .unwrap();
    assert!(receipt.backup.as_ref().unwrap().is_file());

    let saved = fs::read(&target).unwrap();
    assert!(saved.windows(2).any(|pair| pair == b"\r\n"));
    let reopened = ModelsIniEditorSession::load(String::from_utf8(saved).unwrap()).unwrap();
    assert_eq!(
        reopened.document().last_value("模型 profile", "threads"),
        Some("12")
    );
    assert_eq!(
        reopened.document().last_value("*", "future-option"),
        Some("preserve-me")
    );
    assert!(reopened.canonical_source().contains("# keep this comment\r\n"));
    assert!(reopened.canonical_source().contains("D:\\Models 外部\\evidence model.gguf"));
}

#[test]
fn external_raw_edit_validation_save_and_reopen_use_same_canonical_document() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("raw config 外部.ini");
    let original = "[*]\nthreads=4\n[model]\nmodel=D:\\Models\\a.gguf\n";
    fs::write(&target, original).unwrap();

    let runtime = installation();
    let mut editor = ModelsIniEditorSession::load(original).unwrap();
    editor.switch_mode(EditorMode::Raw).unwrap();
    editor
        .apply_raw_edit("[*]\nthreads=6\n[model]\nmodel=D:\\Models\\a.gguf\nctx-size=8192\n")
        .unwrap();
    let validation = editor.validation("model", Some(&runtime)).unwrap();
    assert!(validation.can_apply(), "{:?}", validation.issues);
    write_external_models_ini(&target, editor.raw_draft(), &validation, 5).unwrap();

    let reopened = ModelsIniEditorSession::load(fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(reopened.document().last_value("*", "threads"), Some("6"));
    assert_eq!(reopened.document().last_value("model", "ctx-size"), Some("8192"));
    assert_eq!(reopened.raw_draft(), reopened.canonical_source());
}

#[test]
fn invalid_raw_or_semantic_edit_cannot_damage_original_external_file() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("protected models.ini");
    let original = "[*]\nthreads=4\n[model]\nmodel=model.gguf\n";
    fs::write(&target, original).unwrap();

    let mut raw_editor = ModelsIniEditorSession::load(original).unwrap();
    raw_editor.switch_mode(EditorMode::Raw).unwrap();
    assert!(raw_editor.apply_raw_edit("[*]\nthis is malformed\n").is_err());
    assert!(raw_editor.validation("model", Some(&installation())).is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), original);

    let semantic = ModelsIniEditorSession::load(
        "[*]\nthreads=0\n[model]\nmodel=model.gguf\n",
    )
    .unwrap();
    let validation = semantic.validation("model", Some(&installation())).unwrap();
    assert!(!validation.can_apply());
    let error = write_external_models_ini(
        &target,
        semantic.canonical_source(),
        &validation,
        5,
    )
    .unwrap_err();
    assert!(matches!(error, ConfigWriteError::ValidationBlocked { .. }));
    assert_eq!(fs::read_to_string(&target).unwrap(), original);
}

#[test]
fn backup_restore_recovers_deliberately_bad_external_write_and_preserves_bad_state() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("restore path 外部.ini");
    let original = "[*]\nthreads=4\n[model]\nmodel=model.gguf\n";
    fs::write(&target, original).unwrap();

    let editor = ModelsIniEditorSession::load(
        "[*]\nthreads=8\n[model]\nmodel=model.gguf\n",
    )
    .unwrap();
    let validation = editor.validation("model", Some(&installation())).unwrap();
    let receipt = write_external_models_ini(
        &target,
        editor.canonical_source(),
        &validation,
        5,
    )
    .unwrap();
    let original_backup = receipt.backup.unwrap();

    let deliberately_bad = "not valid models.ini\n";
    fs::write(&target, deliberately_bad).unwrap();
    let restored = restore_backup(&original_backup, &target, 5).unwrap();

    assert_eq!(fs::read_to_string(&target).unwrap(), original);
    let bad_state_backup = restored.pre_restore_backup.unwrap();
    assert_eq!(
        fs::read_to_string(bad_state_backup).unwrap(),
        deliberately_bad
    );
}

#[test]
fn managed_config_survives_reconstructed_app_paths_restart() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("portable state 外部 with spaces");
    let first_paths = AppPaths::from_root(StorageMode::Portable, root.clone()).unwrap();
    let editor = ModelsIniEditorSession::load(
        "[*]\nthreads=4\n[model]\nmodel=model.gguf\nctx-size=8192\n",
    )
    .unwrap();
    let validation = editor.validation("model", Some(&installation())).unwrap();
    assert!(validation.can_apply());
    write_managed_models_ini(&first_paths, editor.canonical_source(), &validation).unwrap();

    drop(first_paths);
    let restarted_paths = AppPaths::from_root(StorageMode::Portable, root).unwrap();
    let managed = managed_models_ini_path(&restarted_paths);
    let reopened = ModelsIniEditorSession::load(fs::read_to_string(managed).unwrap()).unwrap();
    assert_eq!(reopened.document().last_value("model", "ctx-size"), Some("8192"));
    assert_eq!(reopened.document().last_value("*", "threads"), Some("4"));
}

#[test]
fn generated_profile_write_and_reopen_has_no_semantic_drift() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("generated state 外部");
    let paths = AppPaths::from_root(StorageMode::Portable, root).unwrap();
    let target = temp.path().join("external generated 外部.ini");
    let baseline = "[*]\r\n# baseline comment\r\nthreads=4\r\n";
    fs::write(&target, baseline).unwrap();

    let runtime = installation();
    let selected_model = model();
    let compatible = compatibility(&selected_model, &runtime);
    let recommendations = vec![RecommendedSetting {
        key: "ctx-size".into(),
        value: "16384".into(),
        capability_aliases: vec!["--ctx-size".into(), "-c".into()],
        basis: RecommendationBasis::Derived,
        evidence: vec!["requested=16384; model_metadata_max=32768".into()],
    }];
    let disabled = BTreeSet::new();
    let request = ProfileGenerationRequest {
        section: "Generated 外部",
        baseline_source: baseline,
        installation: &runtime,
        model: &selected_model,
        compatibility: &compatible,
        projector: None,
        recommendations: &recommendations,
        required_settings: &[],
        explicitly_disabled_features: &disabled,
        destination: ProfileDestination::External(target.clone()),
    };

    let generated = generate_profile(&request).unwrap();
    assert!(generated.can_apply(), "{:?}", generated.validation.issues);
    assert!(!generated.diff.is_empty());
    apply_generated_profile(&paths, &generated).unwrap();

    let reopened_source = fs::read_to_string(&target).unwrap();
    let reopened = ModelsIniEditorSession::load(reopened_source.clone()).unwrap();
    let generated_document = ModelsIniDocument::parse(&generated.source).unwrap();
    let reopened_document = ModelsIniDocument::parse(&reopened_source).unwrap();
    assert_eq!(generated_document.serialize(), reopened_document.serialize());
    assert_eq!(
        reopened.document().last_value("Generated 外部", "ctx-size"),
        Some("16384")
    );
    assert_eq!(
        reopened.document().last_value("Generated 外部", "model"),
        Some(r"D:\Models 外部\evidence model.gguf")
    );
    assert!(reopened.canonical_source().contains("# baseline comment\r\n"));
}
