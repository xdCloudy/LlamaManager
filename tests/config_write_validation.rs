use std::{
    fs,
    path::{Path, PathBuf},
};

use llamamanager::{
    config_write::{
        ConfigWriteError, ConfigWriteMode, managed_models_ini_path, restore_backup,
        write_external_models_ini, write_managed_models_ini,
    },
    models_ini_validation::{ValidationIssue, ValidationReport, ValidationSeverity},
    paths::{AppPaths, StorageMode},
};

fn valid_report() -> ValidationReport {
    ValidationReport { issues: Vec::new() }
}

fn invalid_report() -> ValidationReport {
    ValidationReport {
        issues: vec![ValidationIssue {
            severity: ValidationSeverity::Error,
            code: "test_invalid".into(),
            key: Some("threads".into()),
            message: "invalid for test".into(),
            evidence: vec!["threads=0".into()],
        }],
    }
}

#[test]
fn managed_path_is_deterministic_relocatable_and_writable() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::from_root(
        StorageMode::Portable,
        temp.path().join("portable config 根 with spaces"),
    )
    .unwrap();

    let target = managed_models_ini_path(&paths);
    assert_eq!(target, paths.config.join("models.ini"));

    let receipt =
        write_managed_models_ini(&paths, "[*]\r\nthreads=8\r\n", &valid_report()).unwrap();
    assert_eq!(receipt.mode, ConfigWriteMode::Managed);
    assert_eq!(receipt.target, target);
    assert_eq!(fs::read(&target).unwrap(), b"[*]\r\nthreads=8\r\n");
}

#[test]
fn validation_error_blocks_before_external_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("external models.ini");
    let original = b"[*]\r\nthreads=8\r\n";
    fs::write(&target, original).unwrap();

    let error =
        write_external_models_ini(&target, "[*]\nthreads=0\n", &invalid_report(), 3).unwrap_err();

    assert!(matches!(
        error,
        ConfigWriteError::ValidationBlocked { error_count: 1 }
    ));
    assert_eq!(fs::read(&target).unwrap(), original);
    assert!(list_backups(&target).is_empty());
}

#[test]
fn external_write_backs_up_and_preserves_utf8_and_line_endings_exactly() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("用户 configs with spaces");
    fs::create_dir_all(&directory).unwrap();
    let target = directory.join("models.ini");
    let old = "[*]\r\n# keep CRLF\r\nmodel=C:\\模型\\old.gguf\r\n";
    let new = "[*]\n# intentionally LF\nmodel=C:\\模型\\new.gguf\n";
    fs::write(&target, old.as_bytes()).unwrap();

    let receipt = write_external_models_ini(&target, new, &valid_report(), 5).unwrap();
    let backup = receipt.backup.unwrap();

    assert_eq!(receipt.mode, ConfigWriteMode::External);
    assert_eq!(receipt.bytes_written, new.len() as u64);
    assert_eq!(fs::read(&target).unwrap(), new.as_bytes());
    assert_eq!(fs::read(backup).unwrap(), old.as_bytes());
}

#[test]
fn restore_recovers_known_good_bytes_and_preserves_bad_pre_restore_state() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("models.ini");
    let original = "[*]\nthreads=8\n";
    fs::write(&target, original).unwrap();

    let written =
        write_external_models_ini(&target, "[*]\nthreads=12\n", &valid_report(), 5).unwrap();
    let original_backup = written.backup.unwrap();

    fs::write(&target, "this is a deliberately bad edit\n").unwrap();
    let restored = restore_backup(&original_backup, &target, 5).unwrap();
    let rescue = restored.pre_restore_backup.unwrap();

    assert_eq!(fs::read_to_string(&target).unwrap(), original);
    assert_eq!(
        fs::read_to_string(rescue).unwrap(),
        "this is a deliberately bad edit\n"
    );
}

#[test]
fn backup_retention_is_target_scoped_bounded_and_never_zero() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("models.ini");
    let unrelated = temp.path().join("unrelated.bak");
    fs::write(&target, "[*]\nthreads=1\n").unwrap();
    fs::write(&unrelated, "do not touch").unwrap();

    for threads in 2..8 {
        write_external_models_ini(
            &target,
            &format!("[*]\nthreads={threads}\n"),
            &valid_report(),
            2,
        )
        .unwrap();
    }
    assert_eq!(list_backups(&target).len(), 2);
    assert_eq!(fs::read_to_string(&unrelated).unwrap(), "do not touch");

    write_external_models_ini(&target, "[*]\nthreads=9\n", &valid_report(), 0).unwrap();
    assert_eq!(list_backups(&target).len(), 1);
}

#[test]
fn non_file_target_is_rejected_without_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("models.ini");
    fs::create_dir(&target).unwrap();

    assert!(matches!(
        write_external_models_ini(&target, "[*]\n", &valid_report(), 5),
        Err(ConfigWriteError::InvalidTarget(path)) if path == target
    ));
}

#[cfg(windows)]
#[test]
fn windows_exclusive_lock_is_actionable_and_non_destructive() {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("locked models.ini");
    let original = "[*]\r\nthreads=8\r\n";
    fs::write(&target, original).unwrap();

    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&target)
        .unwrap();
    let error = write_external_models_ini(&target, "[*]\r\nthreads=12\r\n", &valid_report(), 5)
        .unwrap_err();
    drop(lock);

    assert!(matches!(error, ConfigWriteError::Io { .. }));
    assert_eq!(fs::read_to_string(&target).unwrap(), original);
}

#[cfg(windows)]
#[test]
fn windows_read_only_target_is_an_actionable_failure() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("read only models.ini");
    let original = "[*]\r\nthreads=8\r\n";
    fs::write(&target, original).unwrap();

    let mut permissions = fs::metadata(&target).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&target, permissions).unwrap();

    let result = write_external_models_ini(&target, "[*]\r\nthreads=12\r\n", &valid_report(), 5);

    let mut permissions = fs::metadata(&target).unwrap().permissions();
    permissions.set_readonly(false);
    fs::set_permissions(&target, permissions).unwrap();

    assert!(matches!(result, Err(ConfigWriteError::Io { .. })));
    assert_eq!(fs::read_to_string(&target).unwrap(), original);
}

fn list_backups(target: &Path) -> Vec<PathBuf> {
    let prefix = format!(
        "{}.llamamanager-backup-",
        target.file_name().unwrap().to_string_lossy()
    );
    let mut backups: Vec<_> = fs::read_dir(target.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .map(|name| {
                    let name = name.to_string_lossy();
                    name.starts_with(&prefix) && name.ends_with(".bak")
                })
                .unwrap_or(false)
        })
        .collect();
    backups.sort();
    backups
}
