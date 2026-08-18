use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use thiserror::Error;

use crate::{models_ini_validation::ValidationReport, paths::AppPaths};

/// Maximum number of LlamaManager-created backups retained beside a config by default.
///
/// Retention is applied only to files matching the exact target-specific backup prefix.
/// At least one recoverable backup is always retained for an existing file mutation.
pub const DEFAULT_BACKUP_RETENTION: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigWriteMode {
    Managed,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigWriteReceipt {
    pub mode: ConfigWriteMode,
    pub target: PathBuf,
    pub backup: Option<PathBuf>,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRestoreReceipt {
    pub target: PathBuf,
    pub restored_from: PathBuf,
    pub pre_restore_backup: Option<PathBuf>,
    pub bytes_written: u64,
}

#[derive(Debug, Error)]
pub enum ConfigWriteError {
    #[error("configuration validation blocked apply with {error_count} error(s)")]
    ValidationBlocked { error_count: usize },

    #[error("configuration target is not a regular file: {0}")]
    InvalidTarget(PathBuf),

    #[error("configuration target has no usable parent directory: {0}")]
    InvalidParent(PathBuf),

    #[error("configuration backup does not exist or is not a regular file: {0}")]
    InvalidBackup(PathBuf),

    #[error("{action} failed for {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub type ConfigWriteResult<T> = std::result::Result<T, ConfigWriteError>;

/// Returns the deterministic application-owned `models.ini` location.
pub fn managed_models_ini_path(paths: &AppPaths) -> PathBuf {
    paths.config.join("models.ini")
}

/// Writes the managed `models.ini` only after semantic validation allows apply.
///
/// Existing managed files are backed up as well, even though issue #27 only requires
/// mandatory backups for user-owned external files. This makes managed edits equally
/// recoverable without changing the deterministic destination.
pub fn write_managed_models_ini(
    paths: &AppPaths,
    contents: &str,
    validation: &ValidationReport,
) -> ConfigWriteResult<ConfigWriteReceipt> {
    write_validated(
        &managed_models_ini_path(paths),
        contents,
        validation,
        ConfigWriteMode::Managed,
        DEFAULT_BACKUP_RETENTION,
    )
}

/// Writes a user-owned config only after validation, creating a durable backup before
/// replacing any existing file.
pub fn write_external_models_ini(
    target: &Path,
    contents: &str,
    validation: &ValidationReport,
    backup_retention: usize,
) -> ConfigWriteResult<ConfigWriteReceipt> {
    write_validated(
        target,
        contents,
        validation,
        ConfigWriteMode::External,
        backup_retention,
    )
}

/// Restores a previously created backup without requiring the current target to parse.
///
/// Restore is deliberately a recovery path: the current target is backed up first when
/// present, then the selected backup is copied into a same-directory temporary file and
/// atomically replaces the target where the platform supports it.
pub fn restore_backup(
    backup: &Path,
    target: &Path,
    backup_retention: usize,
) -> ConfigWriteResult<ConfigRestoreReceipt> {
    if !backup.is_file() {
        return Err(ConfigWriteError::InvalidBackup(backup.to_path_buf()));
    }
    validate_target_shape(target)?;
    let parent = target_parent(target)?;
    fs::create_dir_all(parent).map_err(|source| io_error("create target directory", parent, source))?;

    let pre_restore_backup = if target.is_file() {
        Some(create_backup(target)?)
    } else {
        None
    };

    let mut source = File::open(backup).map_err(|error| io_error("open restore backup", backup, error))?;
    let temp = unique_sibling_path(target, "restore-tmp")?;
    let mut temp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| io_error("create restore temporary file", &temp, error))?;

    let copied = match io::copy(&mut source, &mut temp_file) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = fs::remove_file(&temp);
            return Err(io_error("copy restore backup", backup, error));
        }
    };
    if let Err(error) = temp_file.flush().and_then(|_| temp_file.sync_all()) {
        drop(temp_file);
        let _ = fs::remove_file(&temp);
        return Err(io_error("flush restore temporary file", &temp, error));
    }
    drop(temp_file);

    if let Err(error) = replace_from_temp(&temp, target) {
        let _ = fs::remove_file(&temp);
        return Err(io_error("replace target during restore", target, error));
    }

    if target.is_file() {
        prune_backups(target, backup_retention.max(1))?;
    }

    Ok(ConfigRestoreReceipt {
        target: target.to_path_buf(),
        restored_from: backup.to_path_buf(),
        pre_restore_backup,
        bytes_written: copied,
    })
}

fn write_validated(
    target: &Path,
    contents: &str,
    validation: &ValidationReport,
    mode: ConfigWriteMode,
    backup_retention: usize,
) -> ConfigWriteResult<ConfigWriteReceipt> {
    let error_count = validation.errors().count();
    if error_count > 0 {
        return Err(ConfigWriteError::ValidationBlocked { error_count });
    }

    validate_target_shape(target)?;
    let parent = target_parent(target)?;
    fs::create_dir_all(parent).map_err(|source| io_error("create target directory", parent, source))?;

    let backup = if target.is_file() {
        Some(create_backup(target)?)
    } else {
        None
    };

    let temp = unique_sibling_path(target, "write-tmp")?;
    let write_result = write_temp_file(&temp, contents.as_bytes());
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }

    if let Err(error) = replace_from_temp(&temp, target) {
        let _ = fs::remove_file(&temp);
        return Err(io_error("replace configuration target", target, error));
    }

    prune_backups(target, backup_retention.max(1))?;

    Ok(ConfigWriteReceipt {
        mode,
        target: target.to_path_buf(),
        backup,
        bytes_written: contents.len() as u64,
    })
}

fn validate_target_shape(target: &Path) -> ConfigWriteResult<()> {
    if target.exists() && !target.is_file() {
        return Err(ConfigWriteError::InvalidTarget(target.to_path_buf()));
    }
    Ok(())
}

fn target_parent(target: &Path) -> ConfigWriteResult<&Path> {
    target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| ConfigWriteError::InvalidParent(target.to_path_buf()))
}

fn write_temp_file(path: &Path, bytes: &[u8]) -> ConfigWriteResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error("create temporary configuration", path, error))?;
    file.write_all(bytes)
        .map_err(|error| io_error("write temporary configuration", path, error))?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|error| io_error("flush temporary configuration", path, error))
}

fn create_backup(target: &Path) -> ConfigWriteResult<PathBuf> {
    let backup = unique_backup_path(target)?;
    let mut source = File::open(target).map_err(|error| io_error("open configuration for backup", target, error))?;
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
        .map_err(|error| io_error("create configuration backup", &backup, error))?;

    if let Err(error) = io::copy(&mut source, &mut destination) {
        drop(destination);
        let _ = fs::remove_file(&backup);
        return Err(io_error("copy configuration backup", target, error));
    }
    if let Err(error) = destination.flush().and_then(|_| destination.sync_all()) {
        drop(destination);
        let _ = fs::remove_file(&backup);
        return Err(io_error("flush configuration backup", &backup, error));
    }
    Ok(backup)
}

fn unique_backup_path(target: &Path) -> ConfigWriteResult<PathBuf> {
    let parent = target_parent(target)?;
    let file_name = target
        .file_name()
        .ok_or_else(|| ConfigWriteError::InvalidTarget(target.to_path_buf()))?
        .to_string_lossy();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    for counter in 0..1000_u16 {
        let candidate = parent.join(format!(
            "{file_name}.llamamanager-backup-{stamp:020}-{counter:03}.bak"
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(io_error(
        "allocate configuration backup name",
        target,
        io::Error::new(io::ErrorKind::AlreadyExists, "backup name space exhausted"),
    ))
}

fn unique_sibling_path(target: &Path, kind: &str) -> ConfigWriteResult<PathBuf> {
    let parent = target_parent(target)?;
    let file_name = target
        .file_name()
        .ok_or_else(|| ConfigWriteError::InvalidTarget(target.to_path_buf()))?
        .to_string_lossy();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for counter in 0..1000_u16 {
        let candidate = parent.join(format!(
            ".{file_name}.llamamanager-{kind}-{}-{stamp}-{counter}",
            std::process::id()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(io_error(
        "allocate temporary configuration name",
        target,
        io::Error::new(io::ErrorKind::AlreadyExists, "temporary name space exhausted"),
    ))
}

fn backup_prefix(target: &Path) -> ConfigWriteResult<String> {
    let file_name = target
        .file_name()
        .ok_or_else(|| ConfigWriteError::InvalidTarget(target.to_path_buf()))?
        .to_string_lossy();
    Ok(format!("{file_name}.llamamanager-backup-"))
}

fn prune_backups(target: &Path, retention: usize) -> ConfigWriteResult<()> {
    let parent = target_parent(target)?;
    let prefix = backup_prefix(target)?;
    let mut backups = Vec::new();

    let entries = fs::read_dir(parent).map_err(|error| io_error("enumerate configuration backups", parent, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_error("read configuration backup entry", parent, error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) && name.ends_with(".bak") && entry.path().is_file() {
            backups.push((name, entry.path()));
        }
    }

    backups.sort_by(|left, right| left.0.cmp(&right.0));
    let remove_count = backups.len().saturating_sub(retention.max(1));
    for (_, path) in backups.into_iter().take(remove_count) {
        fs::remove_file(&path).map_err(|error| io_error("prune configuration backup", &path, error))?;
    }
    Ok(())
}

#[cfg(windows)]
fn replace_from_temp(temp: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    if !target.exists() {
        return fs::rename(temp, target);
    }

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *mut std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
        ) -> i32;
    }

    let replaced: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let replacement: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let success = unsafe {
        ReplaceFileW(
            replaced.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if success == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_from_temp(temp: &Path, target: &Path) -> io::Result<()> {
    fs::rename(temp, target)
}

fn io_error(action: &'static str, path: &Path, source: io::Error) -> ConfigWriteError {
    ConfigWriteError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models_ini_validation::{ValidationIssue, ValidationSeverity},
        paths::StorageMode,
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
    fn managed_path_is_deterministic_and_relocatable() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_root(
            StorageMode::Portable,
            temp.path().join("portable config 根 with spaces"),
        )
        .unwrap();
        assert_eq!(managed_models_ini_path(&paths), paths.config.join("models.ini"));
    }

    #[test]
    fn validation_error_blocks_before_touching_external_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("external models.ini");
        fs::write(&target, b"[*]\r\nthreads=8\r\n").unwrap();

        let error = write_external_models_ini(&target, "[*]\nthreads=0\n", &invalid_report(), 3)
            .unwrap_err();
        assert!(matches!(
            error,
            ConfigWriteError::ValidationBlocked { error_count: 1 }
        ));
        assert_eq!(fs::read(&target).unwrap(), b"[*]\r\nthreads=8\r\n");
        assert!(list_backups(&target).is_empty());
    }

    #[test]
    fn external_write_backs_up_and_preserves_utf8_and_line_endings_exactly() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("用户 configs with spaces");
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("models.ini");
        let old = "[*]\r\n# keep CRLF\r\nmodel=C:\\模型\\old.gguf\r\n";
        let new = "[*]\n# intentionally LF\nmodel=C:\\模型\\new.gguf\n";
        fs::write(&target, old.as_bytes()).unwrap();

        let receipt = write_external_models_ini(&target, new, &valid_report(), 5).unwrap();
        let backup = receipt.backup.unwrap();
        assert_eq!(receipt.mode, ConfigWriteMode::External);
        assert_eq!(fs::read(&target).unwrap(), new.as_bytes());
        assert_eq!(fs::read(backup).unwrap(), old.as_bytes());
        assert_eq!(receipt.bytes_written, new.len() as u64);
    }

    #[test]
    fn restore_recovers_backup_and_preserves_pre_restore_state() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("models.ini");
        let original = "[*]\nthreads=8\n";
        fs::write(&target, original).unwrap();
        let written = write_external_models_ini(
            &target,
            "[*]\nthreads=12\n",
            &valid_report(),
            5,
        )
        .unwrap();
        let original_backup = written.backup.unwrap();

        fs::write(&target, "this is a deliberately bad edit\n").unwrap();
        let restored = restore_backup(&original_backup, &target, 5).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), original);
        let rescue = restored.pre_restore_backup.unwrap();
        assert_eq!(
            fs::read_to_string(rescue).unwrap(),
            "this is a deliberately bad edit\n"
        );
    }

    #[test]
    fn backup_retention_is_bounded_but_never_zero() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("models.ini");
        fs::write(&target, "[*]\nthreads=1\n").unwrap();

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

        write_external_models_ini(&target, "[*]\nthreads=9\n", &valid_report(), 0).unwrap();
        assert_eq!(list_backups(&target).len(), 1);
    }

    #[test]
    fn directory_target_is_rejected_without_mutation() {
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
    fn windows_locked_file_failure_is_actionable_and_non_destructive() {
        use std::os::windows::fs::OpenOptionsExt;

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
        let error = write_external_models_ini(
            &target,
            "[*]\r\nthreads=12\r\n",
            &valid_report(),
            5,
        )
        .unwrap_err();
        drop(lock);

        assert!(matches!(error, ConfigWriteError::Io { .. }));
        assert_eq!(fs::read_to_string(&target).unwrap(), original);
    }

    fn list_backups(target: &Path) -> Vec<PathBuf> {
        let parent = target.parent().unwrap();
        let prefix = backup_prefix(target).unwrap();
        let mut backups: Vec<_> = fs::read_dir(parent)
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
}
