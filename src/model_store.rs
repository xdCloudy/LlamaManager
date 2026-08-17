use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::{
    compatibility::CompatibilityResult,
    error::{LlamaManagerError, Result},
    gguf::ModelInfo,
    llama::now_ms,
    multimodal::{ProjectorInfo, ProjectorMatch, ProjectorMatchStatus, evaluate_projector_pair},
};

const MIGRATION_2: &str = include_str!("../migrations/0002_model_library.sql");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFingerprint {
    pub file_size: u64,
    pub modified_at_unix_ms: Option<u128>,
    pub edge_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationState {
    Present,
    Missing,
    Unreadable,
}

impl LocationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Missing => "missing",
            Self::Unreadable => "unreadable",
        }
    }

    fn from_db(value: &str) -> Self {
        match value {
            "present" => Self::Present,
            "unreadable" => Self::Unreadable,
            _ => Self::Missing,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLocation {
    pub model_id: String,
    pub path: PathBuf,
    pub fingerprint: FileFingerprint,
    pub state: LocationState,
    pub first_seen_at_unix_ms: u128,
    pub last_seen_at_unix_ms: u128,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRecord {
    pub model: ModelInfo,
    pub locations: Vec<ModelLocation>,
}

impl ModelRecord {
    pub fn is_missing(&self) -> bool {
        self.locations.is_empty()
            || self
                .locations
                .iter()
                .all(|location| location.state != LocationState::Present)
    }

    pub fn present_paths(&self) -> Vec<&Path> {
        self.locations
            .iter()
            .filter(|location| location.state == LocationState::Present)
            .map(|location| location.path.as_path())
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredProjector {
    pub projector: ProjectorInfo,
    pub fingerprint: FileFingerprint,
    pub state: LocationState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectorCandidate {
    pub projector: ProjectorInfo,
    pub pairing: ProjectorMatch,
}

#[derive(Debug, Clone)]
pub struct ModelStore {
    path: PathBuf,
}

impl ModelStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let store = Self { path: path.into() };
        store.initialize()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connection(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(connection)
    }

    fn initialize(&self) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute_batch(MIGRATION_2)?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_unix_ms) VALUES (2, ?1)",
            [now_ms().to_string()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_model_with_location(
        &self,
        model: &ModelInfo,
        fingerprint: &FileFingerprint,
    ) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let existing_path: Option<String> = transaction
            .query_row(
                "SELECT path FROM models WHERE id = ?1",
                [&model.id],
                |row| row.get(0),
            )
            .optional()?;
        let canonical_path = existing_path
            .map(PathBuf::from)
            .filter(|path| path.exists())
            .unwrap_or_else(|| model.path.clone());
        let mut canonical = model.clone();
        canonical.path = canonical_path.clone();
        let payload = serde_json::to_string(&canonical)?;
        transaction.execute(
            "INSERT INTO models(id, path, sha256, architecture, name, payload_json, updated_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                path = excluded.path,
                sha256 = excluded.sha256,
                architecture = excluded.architecture,
                name = excluded.name,
                payload_json = excluded.payload_json,
                updated_at_unix_ms = excluded.updated_at_unix_ms",
            params![
                model.id,
                canonical_path.to_string_lossy(),
                model.sha256,
                model.architecture,
                model.name,
                payload,
                now_ms().to_string(),
            ],
        )?;
        Self::upsert_location(&transaction, &model.id, &model.path, fingerprint, None)?;
        transaction.commit()?;
        Ok(())
    }

    fn upsert_location(
        transaction: &Transaction<'_>,
        model_id: &str,
        path: &Path,
        fingerprint: &FileFingerprint,
        last_error: Option<&str>,
    ) -> Result<()> {
        let now = now_ms().to_string();
        transaction.execute(
            "INSERT INTO model_locations(
                model_id, path, file_size, modified_at_unix_ms, quick_fingerprint,
                state, first_seen_at_unix_ms, last_seen_at_unix_ms, last_error
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'present', ?6, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET
                model_id = excluded.model_id,
                file_size = excluded.file_size,
                modified_at_unix_ms = excluded.modified_at_unix_ms,
                quick_fingerprint = excluded.quick_fingerprint,
                state = 'present',
                last_seen_at_unix_ms = excluded.last_seen_at_unix_ms,
                last_error = excluded.last_error",
            params![
                model_id,
                path.to_string_lossy(),
                sql_i64(fingerprint.file_size, "model file size")?,
                fingerprint
                    .modified_at_unix_ms
                    .map(|value| value.to_string()),
                fingerprint.edge_sha256,
                now,
                last_error,
            ],
        )?;
        Ok(())
    }

    pub fn get_model(&self, model_id: &str) -> Result<Option<ModelInfo>> {
        let connection = self.connection()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload_json FROM models WHERE id = ?1",
                [model_id],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }

    pub fn model_ids_by_sha(&self, sha256: &str) -> Result<Vec<String>> {
        let connection = self.connection()?;
        let mut statement =
            connection.prepare("SELECT id FROM models WHERE sha256 = ?1 ORDER BY id")?;
        let rows = statement.query_map([sha256], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn model_location_by_path(&self, path: &Path) -> Result<Option<ModelLocation>> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT model_id, path, file_size, modified_at_unix_ms, quick_fingerprint,
                        state, first_seen_at_unix_ms, last_seen_at_unix_ms, last_error
                 FROM model_locations WHERE path = ?1",
                [path.to_string_lossy().as_ref()],
                location_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn touch_model_location(&self, path: &Path) -> Result<()> {
        self.connection()?.execute(
            "UPDATE model_locations
             SET state = 'present', last_seen_at_unix_ms = ?2, last_error = NULL
             WHERE path = ?1",
            params![path.to_string_lossy(), now_ms().to_string()],
        )?;
        Ok(())
    }

    pub fn mark_known_path_unreadable(&self, path: &Path, message: &str) -> Result<()> {
        let connection = self.connection()?;
        connection.execute(
            "UPDATE model_locations
             SET state = 'unreadable', last_error = ?2, last_seen_at_unix_ms = ?3
             WHERE path = ?1",
            params![path.to_string_lossy(), message, now_ms().to_string()],
        )?;
        connection.execute(
            "UPDATE projectors SET state = 'unreadable', updated_at_unix_ms = ?2 WHERE path = ?1",
            params![path.to_string_lossy(), now_ms().to_string()],
        )?;
        Ok(())
    }

    pub fn reconcile_model_locations(&self, root: &Path, seen: &BTreeSet<PathBuf>) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let paths = {
            let mut statement = transaction.prepare("SELECT path FROM model_locations")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for stored in paths {
            let path = PathBuf::from(&stored);
            if path.starts_with(root) && !seen.contains(&path) {
                transaction.execute(
                    "UPDATE model_locations SET state = 'missing', last_error = NULL WHERE path = ?1",
                    [&stored],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn refresh_location_existence(&self) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let paths = {
            let mut statement = transaction.prepare("SELECT path FROM model_locations")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for stored in paths {
            let state = if Path::new(&stored).is_file() {
                "present"
            } else {
                "missing"
            };
            transaction.execute(
                "UPDATE model_locations SET state = ?2 WHERE path = ?1",
                params![stored, state],
            )?;
        }
        let projector_paths = {
            let mut statement = transaction.prepare("SELECT path FROM projectors")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for stored in projector_paths {
            let state = if Path::new(&stored).is_file() {
                "present"
            } else {
                "missing"
            };
            transaction.execute(
                "UPDATE projectors SET state = ?2 WHERE path = ?1",
                params![stored, state],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list_model_records(&self) -> Result<Vec<ModelRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT payload_json FROM models ORDER BY CAST(updated_at_unix_ms AS INTEGER) DESC, id",
        )?;
        let payloads = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut records = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let model: ModelInfo = serde_json::from_str(&payload)?;
            let mut location_statement = connection.prepare(
                "SELECT model_id, path, file_size, modified_at_unix_ms, quick_fingerprint,
                        state, first_seen_at_unix_ms, last_seen_at_unix_ms, last_error
                 FROM model_locations WHERE model_id = ?1
                 ORDER BY CASE state WHEN 'present' THEN 0 WHEN 'unreadable' THEN 1 ELSE 2 END, path",
            )?;
            let locations = location_statement
                .query_map([&model.id], location_from_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            records.push(ModelRecord { model, locations });
        }
        Ok(records)
    }

    pub fn relink_model(
        &self,
        model_id: &str,
        inspected: &ModelInfo,
        fingerprint: &FileFingerprint,
    ) -> Result<()> {
        let expected = self
            .get_model(model_id)?
            .ok_or_else(|| LlamaManagerError::State(format!("model {model_id} not found")))?;
        if expected.sha256 != inspected.sha256 {
            return Err(LlamaManagerError::State("relink SHA-256 mismatch".into()));
        }
        let mut updated = inspected.clone();
        updated.id = model_id.to_string();
        self.save_model_with_location(&updated, fingerprint)
    }

    pub fn upsert_scan_root(&self, root: &Path) -> Result<()> {
        self.connection()?.execute(
            "INSERT INTO model_scan_roots(root_path, added_at_unix_ms)
             VALUES (?1, ?2) ON CONFLICT(root_path) DO NOTHING",
            params![root.to_string_lossy(), now_ms().to_string()],
        )?;
        Ok(())
    }

    pub fn save_scan_summary<T: Serialize>(&self, root: &Path, report: &T) -> Result<()> {
        let payload = serde_json::to_string(report)?;
        self.connection()?.execute(
            "UPDATE model_scan_roots
             SET last_scanned_at_unix_ms = ?2, last_scan_summary_json = ?3 WHERE root_path = ?1",
            params![root.to_string_lossy(), now_ms().to_string(), payload],
        )?;
        Ok(())
    }

    pub fn list_scan_roots(&self) -> Result<Vec<PathBuf>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT root_path FROM model_scan_roots ORDER BY CAST(added_at_unix_ms AS INTEGER), root_path",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .map(PathBuf::from)
            .collect())
    }

    pub fn save_projector(
        &self,
        projector: &ProjectorInfo,
        fingerprint: &FileFingerprint,
    ) -> Result<()> {
        let payload = serde_json::to_string(projector)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM projectors WHERE path = ?1 AND id <> ?2",
            params![projector.path.to_string_lossy(), projector.id],
        )?;
        transaction.execute(
            "INSERT INTO projectors(
                id, path, sha256, file_size, modified_at_unix_ms, quick_fingerprint,
                payload_json, state, updated_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'present', ?8)
             ON CONFLICT(id) DO UPDATE SET
                path = excluded.path,
                sha256 = excluded.sha256,
                file_size = excluded.file_size,
                modified_at_unix_ms = excluded.modified_at_unix_ms,
                quick_fingerprint = excluded.quick_fingerprint,
                payload_json = excluded.payload_json,
                state = 'present',
                updated_at_unix_ms = excluded.updated_at_unix_ms",
            params![
                projector.id,
                projector.path.to_string_lossy(),
                projector.sha256,
                sql_i64(projector.file_size, "projector file size")?,
                fingerprint
                    .modified_at_unix_ms
                    .map(|value| value.to_string()),
                fingerprint.edge_sha256,
                payload,
                now_ms().to_string(),
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn projector_by_path(&self, path: &Path) -> Result<Option<StoredProjector>> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT payload_json, file_size, modified_at_unix_ms, quick_fingerprint, state
                 FROM projectors WHERE path = ?1",
                [path.to_string_lossy().as_ref()],
                projector_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_projector(&self, projector_id: &str) -> Result<Option<StoredProjector>> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT payload_json, file_size, modified_at_unix_ms, quick_fingerprint, state
                 FROM projectors WHERE id = ?1",
                [projector_id],
                projector_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn touch_projector(&self, path: &Path) -> Result<()> {
        self.connection()?.execute(
            "UPDATE projectors SET state = 'present', updated_at_unix_ms = ?2 WHERE path = ?1",
            params![path.to_string_lossy(), now_ms().to_string()],
        )?;
        Ok(())
    }

    pub fn reconcile_projectors(&self, root: &Path, seen: &BTreeSet<PathBuf>) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let paths = {
            let mut statement = transaction.prepare("SELECT path FROM projectors")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for stored in paths {
            let path = PathBuf::from(&stored);
            if path.starts_with(root) && !seen.contains(&path) {
                transaction.execute(
                    "UPDATE projectors SET state = 'missing' WHERE path = ?1",
                    [&stored],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list_projectors(&self) -> Result<Vec<StoredProjector>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT payload_json, file_size, modified_at_unix_ms, quick_fingerprint, state
             FROM projectors
             ORDER BY CASE state WHEN 'present' THEN 0 WHEN 'unreadable' THEN 1 ELSE 2 END, path",
        )?;
        let rows = statement.query_map([], projector_from_row)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn relink_projector(
        &self,
        projector_id: &str,
        projector: &ProjectorInfo,
        fingerprint: &FileFingerprint,
    ) -> Result<()> {
        let expected = self.get_projector(projector_id)?.ok_or_else(|| {
            LlamaManagerError::State(format!("projector {projector_id} not found"))
        })?;
        if expected.projector.sha256 != projector.sha256 {
            return Err(LlamaManagerError::State(
                "projector relink SHA-256 mismatch".into(),
            ));
        }
        let payload = serde_json::to_string(projector)?;
        let changed = self.connection()?.execute(
            "UPDATE projectors
             SET path = ?2, file_size = ?3, modified_at_unix_ms = ?4, quick_fingerprint = ?5,
                 payload_json = ?6, state = 'present', updated_at_unix_ms = ?7
             WHERE id = ?1",
            params![
                projector_id,
                projector.path.to_string_lossy(),
                sql_i64(projector.file_size, "projector file size")?,
                fingerprint
                    .modified_at_unix_ms
                    .map(|value| value.to_string()),
                fingerprint.edge_sha256,
                payload,
                now_ms().to_string(),
            ],
        )?;
        if changed == 0 {
            return Err(LlamaManagerError::State(format!(
                "projector {projector_id} not found"
            )));
        }
        Ok(())
    }

    pub fn projector_candidates(&self, model: &ModelInfo) -> Result<Vec<ProjectorCandidate>> {
        Ok(self
            .list_projectors()?
            .into_iter()
            .filter(|stored| stored.state == LocationState::Present)
            .map(|stored| ProjectorCandidate {
                pairing: evaluate_projector_pair(model, &stored.projector),
                projector: stored.projector,
            })
            .collect())
    }

    pub fn associate_projector(
        &self,
        model: &ModelInfo,
        projector_id: &str,
    ) -> Result<ProjectorMatch> {
        let stored = self.get_projector(projector_id)?.ok_or_else(|| {
            LlamaManagerError::State(format!("projector {projector_id} not found"))
        })?;
        if stored.state != LocationState::Present {
            return Err(LlamaManagerError::State(
                "projector is not currently present".into(),
            ));
        }
        let pairing = evaluate_projector_pair(model, &stored.projector);
        if pairing.status == ProjectorMatchStatus::Incompatible {
            return Err(LlamaManagerError::State(format!(
                "projector association rejected: {}",
                pairing.reasons.join("; ")
            )));
        }
        let evidence = serde_json::to_string(&pairing)?;
        self.connection()?.execute(
            "INSERT INTO projector_associations(model_id, projector_id, status, user_selected, evidence_json, updated_at_unix_ms)
             VALUES (?1, ?2, ?3, 1, ?4, ?5)
             ON CONFLICT(model_id, projector_id) DO UPDATE SET
                status = excluded.status, user_selected = 1,
                evidence_json = excluded.evidence_json, updated_at_unix_ms = excluded.updated_at_unix_ms",
            params![
                model.id,
                projector_id,
                format!("{:?}", pairing.status).to_ascii_lowercase(),
                evidence,
                now_ms().to_string(),
            ],
        )?;
        Ok(pairing)
    }

    pub fn associated_projector(&self, model_id: &str) -> Result<Option<ProjectorInfo>> {
        let connection = self.connection()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT p.payload_json
                 FROM projector_associations a
                 JOIN projectors p ON p.id = a.projector_id
                 WHERE a.model_id = ?1 AND a.user_selected = 1 AND p.state = 'present'
                 ORDER BY CAST(a.updated_at_unix_ms AS INTEGER) DESC LIMIT 1",
                [model_id],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }

    pub fn save_compatibility(&self, result: &CompatibilityResult) -> Result<()> {
        let payload = serde_json::to_string(result)?;
        self.connection()?.execute(
            "INSERT INTO compatibility_results(
                model_id, installation_id, model_sha256, installation_fingerprint,
                registry_revision, status, payload_json, computed_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(model_id, installation_id) DO UPDATE SET
                model_sha256 = excluded.model_sha256,
                installation_fingerprint = excluded.installation_fingerprint,
                registry_revision = excluded.registry_revision,
                status = excluded.status,
                payload_json = excluded.payload_json,
                computed_at_unix_ms = excluded.computed_at_unix_ms",
            params![
                result.model_id,
                result.installation_id,
                result.model_sha256,
                result.installation_fingerprint,
                result.registry_revision,
                result.status.as_str(),
                payload,
                result.computed_at_unix_ms.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn load_compatibility(
        &self,
        model_id: &str,
        installation_id: &str,
    ) -> Result<Option<CompatibilityResult>> {
        let connection = self.connection()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload_json FROM compatibility_results WHERE model_id = ?1 AND installation_id = ?2",
                params![model_id, installation_id],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }
}

fn location_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelLocation> {
    Ok(ModelLocation {
        model_id: row.get(0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        fingerprint: FileFingerprint {
            file_size: row.get::<_, i64>(2)?.max(0) as u64,
            modified_at_unix_ms: parse_optional_u128(row.get(3)?),
            edge_sha256: row.get(4)?,
        },
        state: LocationState::from_db(&row.get::<_, String>(5)?),
        first_seen_at_unix_ms: parse_u128(row.get::<_, String>(6)?),
        last_seen_at_unix_ms: parse_u128(row.get::<_, String>(7)?),
        last_error: row.get(8)?,
    })
}

fn projector_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredProjector> {
    let payload: String = row.get(0)?;
    let projector = serde_json::from_str(&payload).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(StoredProjector {
        projector,
        fingerprint: FileFingerprint {
            file_size: row.get::<_, i64>(1)?.max(0) as u64,
            modified_at_unix_ms: parse_optional_u128(row.get(2)?),
            edge_sha256: row.get(3)?,
        },
        state: LocationState::from_db(&row.get::<_, String>(4)?),
    })
}

fn parse_u128(value: String) -> u128 {
    value.parse().unwrap_or_default()
}

fn parse_optional_u128(value: Option<String>) -> Option<u128> {
    value.and_then(|value| value.parse().ok())
}

fn sql_i64(value: u64, label: &str) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| LlamaManagerError::State(format!("{label} exceeds SQLite INTEGER range")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn model(id: &str, sha: &str, path: &Path) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            path: path.to_path_buf(),
            file_size: 8,
            sha256: sha.into(),
            gguf_version: 3,
            tensor_count: 0,
            metadata_count: 0,
            name: Some("same-name".into()),
            architecture: Some("qwen35".into()),
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

    fn fingerprint() -> FileFingerprint {
        FileFingerprint {
            file_size: 8,
            modified_at_unix_ms: Some(1),
            edge_sha256: "edge".into(),
        }
    }

    #[test]
    fn duplicate_content_has_one_model_with_multiple_locations() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("db.sqlite");
        crate::persistence::Database::open(&db_path).unwrap();
        let store = ModelStore::open(&db_path).unwrap();
        let first = model("model-same", "a", &temp.path().join("one.gguf"));
        let second = model("model-same", "a", &temp.path().join("two.gguf"));
        store
            .save_model_with_location(&first, &fingerprint())
            .unwrap();
        store
            .save_model_with_location(&second, &fingerprint())
            .unwrap();
        let records = store.list_model_records().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].locations.len(), 2);
    }

    #[test]
    fn same_name_different_content_remains_distinct() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("db.sqlite");
        crate::persistence::Database::open(&db_path).unwrap();
        let store = ModelStore::open(&db_path).unwrap();
        let first = model("model-a", "a", &temp.path().join("one.gguf"));
        let second = model("model-b", "b", &temp.path().join("two.gguf"));
        store
            .save_model_with_location(&first, &fingerprint())
            .unwrap();
        store
            .save_model_with_location(&second, &fingerprint())
            .unwrap();
        assert_eq!(store.list_model_records().unwrap().len(), 2);
    }
}
