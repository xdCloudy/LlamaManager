use std::{fs, path::Path};

use llamamanager::{
    config_write::{restore_backup, write_external_models_ini},
    models_ini_validation::ValidationReport,
};

fn valid_report() -> ValidationReport {
    ValidationReport { issues: Vec::new() }
}

fn backup_count(target: &Path) -> usize {
    let prefix = format!(
        "{}.llamamanager-backup-",
        target.file_name().unwrap().to_string_lossy()
    );
    fs::read_dir(target.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(&prefix) && name.ends_with(".bak") && entry.path().is_file()
        })
        .count()
}

#[test]
fn restore_secures_selected_old_backup_before_low_retention_pruning() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("models.ini");
    let original = "[*]\nthreads=8\n";
    fs::write(&target, original).unwrap();

    let first =
        write_external_models_ini(&target, "[*]\nthreads=10\n", &valid_report(), 5).unwrap();
    let selected_old_backup = first.backup.unwrap();
    write_external_models_ini(&target, "[*]\nthreads=12\n", &valid_report(), 5).unwrap();

    fs::write(&target, "deliberately broken current config\n").unwrap();
    assert!(backup_count(&target) >= 2);

    let restored = restore_backup(&selected_old_backup, &target, 1).unwrap();

    assert_eq!(fs::read_to_string(&target).unwrap(), original);
    assert_eq!(restored.restored_from, selected_old_backup);
    assert_eq!(
        fs::read_to_string(restored.pre_restore_backup.unwrap()).unwrap(),
        "deliberately broken current config\n"
    );
    assert_eq!(backup_count(&target), 1);
}
