use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use thiserror::Error;

use crate::{models_ini_validation::ValidationReport, paths::AppPaths};

pub const DEFAULT_BACKUP_RETENTION: usize = 5;

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

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

pub fn managed_models_ini_path(paths: &AppPaths) -> PathBuf {
    paths.config.join("models.ini")
}

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

pub fn restore_backup(
    backup: &Path,
    target: &Path,
    backup_retention: usize,
) -> ConfigWriteResult<ConfigRestoreReceipt> {
    if !backup.is_file() {
        return Err(ConfigWriteError::InvalidBackup(backup.to_path_buf()));
    }
    validate_target_shape(target)?;
    ensure_parent(target)?;

    let pre_restore_backup = if target.is_file() {
        let backup = create_backup(target)?;
        prune_backups(target, backup_retention.max(1))?;
        Some(backup)
    } else {
        None
    };

    let (temp, mut temp_file) = create_unique_sibling(target, "restore-tmp", ".tmp")?;
    let mut source = File::open(backup)
        .map_err(|error| io_error("open restore backup", backup, error))?;
    let copied = match io::copy(&mut source, &mut temp_file) {
        Ok(bytes) => bytes,
        Err(error) => {
            drop(temp_file);
            let _ = fs::remove_file(&temp);
            return Err(io_error("copy restore backup", backup, error));
        }
    };
    sync_file(&mut temp_file, &temp, "flush restore temporary file")?;
    drop(temp_file);

    if let Err(error) = replace_from_temp(&temp, target) {
        let _ = fs::remove_file(&temp);
        return Err(io_error("replace target during restore", target, error));
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
    ensure_parent(target)?;

    let backup = if target.is_file() {
        let backup = create_backup(target)?;
        prune_backups(target, backup_retention.max(1))?;
        Some(backup)
    } else {
        None
    };

    let (temp, mut temp_file) = create_unique_sibling(target, "write-tmp", ".tmp")?;
    if let Err(error) = temp_file.write_all(contents.as_bytes()) {
        drop(temp_file);
        let _ = fs::remove_file(&temp);
        return Err(io_error("write temporary configuration", &temp, error));
    }
    sync_file(&mut temp_file, &temp, "flush temporary configuration")?;
    drop(temp_file);

    if let Err(error) = replace_from_temp(&temp, target) {
        let _ = fs::remove_file(&temp);
        return Err(io_error("replace configuration target", target, error));
    }

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

fn ensure_parent(target: &Path) -> ConfigWriteResult<()> {
    let parent = target_parent(target)?;
    fs::create_dir_all(parent)
        .map_err(|source| io_error("create target directory", parent, source))
}

fn target_parent(target: &Path) -> ConfigWriteResult<&Path> {
    target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| ConfigWriteError::InvalidParent(target.to_path_buf()))
}

fn sync_file(file: &mut File, path: &Path, action: &'static str) -> ConfigWriteResult<()> {
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|error| io_error(action, path, error))
}

fn create_backup(target: &Path) -> ConfigWriteResult<PathBuf> {
    let (backup, mut destination) = create_unique_sibling(target, "backup", ".bak")?;
    let mut source = File::open(target)
        .map_err(|error| io_error("open configuration for backup", target, error))?;

    if let Err(error) = io::copy(&mut source, &mut destination) {
        drop(destination);
        let _ = fs::remove_file(&backup);
        return Err(io_error("copy configuration backup", target, error));
    }
    if let Err(error) = sync_file(
        &mut destination,
        &backup,
        "flush configuration backup",
    ) {
        drop(destination);
        let _ = fs::remove_file(&backup);
        return Err(error);
    }
    Ok(backup)
}

fn create_unique_sibling(
    target: &Path,
    kind: &str,
    suffix: &str,
) -> ConfigWriteResult<(PathBuf, File)> {
    let parent = target_parent(target)?;
    let file_name = target
        .file_name()
        .ok_or_else(|| ConfigWriteError::InvalidTarget(target.to_path_buf()))?
        .to_string_lossy();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for _ in 0..128 {
        let sequence = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let hidden_prefix = if kind == "backup" { "" } else { "." };
        let candidate = parent.join(format!(
            "{hidden_prefix}{file_name}.llamamanager-{kind}-{stamp:039}-{sequence:020}{suffix}"
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(io_error("create configuration sidecar", &candidate, error));
            }
        }
    }

    Err(io_error(
        "allocate configuration sidecar name",
        target,
        io::Error::new(io::ErrorKind::AlreadyExists, "sidecar name space exhausted"),
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

    let entries = fs::read_dir(parent)
        .map_err(|error| io_error("enumerate configuration backups", parent, error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| io_error("read configuration backup entry", parent, error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) && name.ends_with(".bak") && entry.path().is_file() {
            backups.push((name, entry.path()));
        }
    }

    backups.sort_by(|left, right| left.0.cmp(&right.0));
    let remove_count = backups.len().saturating_sub(retention.max(1));
    for (_, path) in backups.into_iter().take(remove_count) {
        fs::remove_file(&path)
            .map_err(|error| io_error("prune configuration backup", &path, error))?;
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
