use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{
    benchmark::BenchmarkRun,
    error::Result,
    gguf::ModelInfo,
    llama::{LlamaInstallation, now_ms},
};

const MIGRATION_1: &str = include_str!("../migrations/0001_initial.sql");

#[derive(Debug, Clone)]
pub struct Database {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkHistoryItem {
    pub id: String,
    pub started_at_unix_ms: u128,
    pub model_path: String,
    pub backend: Option<String>,
    pub prompt_tps: Option<f64>,
    pub decode_tps: Option<f64>,
    pub command_preview: String,
}

impl Database {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let database = Self { path: path.into() };
        database.initialize()?;
        Ok(database)
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
        transaction.execute_batch(MIGRATION_1)?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_unix_ms) VALUES (?1, ?2)",
            params![1_i64, now_ms().to_string()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_installation(&self, installation: &LlamaInstallation) -> Result<()> {
        let payload = serde_json::to_string(installation)?;
        self.connection()?.execute(
            "INSERT INTO llama_installations(id, root_path, name, backend, payload_json, updated_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(root_path) DO UPDATE SET
                name = excluded.name,
                backend = excluded.backend,
                payload_json = excluded.payload_json,
                updated_at_unix_ms = excluded.updated_at_unix_ms",
            params![
                installation.id,
                installation.root_path.to_string_lossy(),
                installation.name,
                installation.backend,
                payload,
                now_ms().to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn save_model(&self, model: &ModelInfo) -> Result<()> {
        let payload = serde_json::to_string(model)?;
        self.connection()?.execute(
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
                model.path.to_string_lossy(),
                model.sha256,
                model.architecture,
                model.name,
                payload,
                now_ms().to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn save_benchmark(&self, run: &BenchmarkRun) -> Result<()> {
        let payload = serde_json::to_string(run)?;
        self.connection()?.execute(
            "INSERT INTO benchmark_runs(
                id, installation_id, model_id, started_at_unix_ms,
                finished_at_unix_ms, exit_code, payload_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                run.id,
                run.installation_id,
                run.model_id,
                run.started_at_unix_ms.to_string(),
                run.finished_at_unix_ms.to_string(),
                run.exit_code,
                payload,
            ],
        )?;
        Ok(())
    }

    pub fn recent_benchmarks(&self, limit: usize) -> Result<Vec<BenchmarkHistoryItem>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT b.payload_json, m.path
             FROM benchmark_runs b
             JOIN models m ON m.id = b.model_id
             ORDER BY CAST(b.started_at_unix_ms AS INTEGER) DESC
             LIMIT ?1",
        )?;

        let rows = statement.query_map([limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut history = Vec::new();
        for row in rows {
            let (payload, model_path) = row?;
            let run: BenchmarkRun = serde_json::from_str(&payload)?;
            history.push(BenchmarkHistoryItem {
                id: run.id.clone(),
                started_at_unix_ms: run.started_at_unix_ms,
                model_path,
                backend: run
                    .backend
                    .clone()
                    .or_else(|| run.samples.iter().find_map(|sample| sample.backend.clone())),
                prompt_tps: run.prompt_tps(),
                decode_tps: run.decode_tps(),
                command_preview: run.command_preview(),
            });
        }
        Ok(history)
    }

    pub fn latest_installation(&self) -> Result<Option<LlamaInstallation>> {
        let connection = self.connection()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload_json FROM llama_installations ORDER BY CAST(updated_at_unix_ms AS INTEGER) DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }

    pub fn latest_model(&self) -> Result<Option<ModelInfo>> {
        let connection = self.connection()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload_json FROM models ORDER BY CAST(updated_at_unix_ms AS INTEGER) DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("db.sqlite");
        Database::open(&path).unwrap();
        Database::open(&path).unwrap();

        let connection = Connection::open(path).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
