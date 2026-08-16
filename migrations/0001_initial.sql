CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_unix_ms TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS llama_installations (
    id TEXT PRIMARY KEY,
    root_path TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    backend TEXT,
    payload_json TEXT NOT NULL,
    updated_at_unix_ms TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS models (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    architecture TEXT,
    name TEXT,
    payload_json TEXT NOT NULL,
    updated_at_unix_ms TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS benchmark_runs (
    id TEXT PRIMARY KEY,
    installation_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    started_at_unix_ms TEXT NOT NULL,
    finished_at_unix_ms TEXT NOT NULL,
    exit_code INTEGER,
    payload_json TEXT NOT NULL,
    FOREIGN KEY(installation_id) REFERENCES llama_installations(id),
    FOREIGN KEY(model_id) REFERENCES models(id)
);

CREATE INDEX IF NOT EXISTS idx_benchmark_runs_started
    ON benchmark_runs(started_at_unix_ms DESC);
