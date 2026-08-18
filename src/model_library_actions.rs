use crate::{
    error::{LlamaManagerError, Result},
    model_store::ModelStore,
};

/// Remove a model from the user-facing library without deleting the GGUF file or
/// the canonical `models` row used by benchmark history.
///
/// The canonical row is intentionally retained because `benchmark_runs` references
/// it. A later scan/manual add of the same content can therefore restore the
/// library location without breaking historical evidence.
pub fn remove_model_from_library(store: &ModelStore, model_id: &str) -> Result<()> {
    let mut connection = rusqlite::Connection::open(store.path())?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    let transaction = connection.transaction()?;

    transaction.execute(
        "DELETE FROM projector_associations WHERE model_id = ?1",
        [model_id],
    )?;
    transaction.execute(
        "DELETE FROM compatibility_results WHERE model_id = ?1",
        [model_id],
    )?;
    let removed = transaction.execute(
        "DELETE FROM model_locations WHERE model_id = ?1",
        [model_id],
    )?;

    if removed == 0 {
        return Err(LlamaManagerError::State(format!(
            "model {model_id} has no library locations to remove"
        )));
    }

    transaction.commit()?;
    Ok(())
}

/// Remove a projector library record and its associations without deleting the
/// projector file from disk.
pub fn remove_projector_from_library(store: &ModelStore, projector_id: &str) -> Result<()> {
    let mut connection = rusqlite::Connection::open(store.path())?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    let transaction = connection.transaction()?;

    transaction.execute(
        "DELETE FROM projector_associations WHERE projector_id = ?1",
        [projector_id],
    )?;
    let removed = transaction.execute("DELETE FROM projectors WHERE id = ?1", [projector_id])?;

    if removed == 0 {
        return Err(LlamaManagerError::State(format!(
            "projector {projector_id} is not in the library"
        )));
    }

    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::Path};

    use crate::{
        gguf::ModelInfo,
        model_store::{FileFingerprint, ModelStore},
        multimodal::ProjectorInfo,
        persistence::Database,
    };

    use super::*;

    fn fingerprint() -> FileFingerprint {
        FileFingerprint {
            file_size: 8,
            modified_at_unix_ms: Some(1),
            edge_sha256: "edge".into(),
        }
    }

    fn model(path: &Path) -> ModelInfo {
        ModelInfo {
            id: "model-remove-test".into(),
            path: path.to_path_buf(),
            file_size: 8,
            sha256: "a".repeat(64),
            gguf_version: 3,
            tensor_count: 0,
            metadata_count: 0,
            name: Some("removal fixture".into()),
            architecture: Some("llama".into()),
            context_length: Some(4096),
            quantization_version: Some(2),
            general_type: Some("model".into()),
            file_type: Some(1),
            parameter_count: Some(0),
            tensor_type_counts: BTreeMap::new(),
            metadata: BTreeMap::new(),
            inspected_at_unix_ms: 1,
        }
    }

    #[test]
    fn removing_library_model_keeps_canonical_identity_for_history() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("library.sqlite");
        Database::open(&db).unwrap();
        let store = ModelStore::open(&db).unwrap();
        let path = temp.path().join("model.gguf");
        let model = model(&path);

        store
            .save_model_with_location(&model, &fingerprint())
            .unwrap();
        assert_eq!(store.list_model_records().unwrap()[0].locations.len(), 1);

        remove_model_from_library(&store, &model.id).unwrap();

        let records = store.list_model_records().unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].locations.is_empty());
        assert!(store.get_model(&model.id).unwrap().is_some());
    }

    #[test]
    fn removing_projector_does_not_touch_source_file() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("library.sqlite");
        Database::open(&db).unwrap();
        let store = ModelStore::open(&db).unwrap();
        let path = temp.path().join("mmproj.gguf");
        std::fs::write(&path, b"fixture").unwrap();

        let projector = ProjectorInfo {
            id: "projector-remove-test".into(),
            path: path.clone(),
            file_size: 7,
            sha256: "b".repeat(64),
            name: Some("projector fixture".into()),
            general_type: Some("mmproj".into()),
            architecture: Some("clip".into()),
            projector_type: Some("fixture".into()),
            modalities: Default::default(),
            source_model_hint: None,
            inspected_at_unix_ms: 1,
        };
        store.save_projector(&projector, &fingerprint()).unwrap();

        remove_projector_from_library(&store, &projector.id).unwrap();

        assert!(store.get_projector(&projector.id).unwrap().is_none());
        assert!(path.is_file());
    }
}
