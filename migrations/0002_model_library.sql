CREATE TABLE IF NOT EXISTS model_scan_roots (
    root_path TEXT PRIMARY KEY,
    added_at_unix_ms TEXT NOT NULL,
    last_scanned_at_unix_ms TEXT,
    last_scan_summary_json TEXT
);

CREATE TABLE IF NOT EXISTS model_locations (
    model_id TEXT NOT NULL,
    path TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    modified_at_unix_ms TEXT,
    quick_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('present', 'missing', 'unreadable')),
    first_seen_at_unix_ms TEXT NOT NULL,
    last_seen_at_unix_ms TEXT NOT NULL,
    last_error TEXT,
    PRIMARY KEY(model_id, path),
    FOREIGN KEY(model_id) REFERENCES models(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_model_locations_path
    ON model_locations(path);
CREATE INDEX IF NOT EXISTS idx_model_locations_model
    ON model_locations(model_id);
CREATE INDEX IF NOT EXISTS idx_models_sha256
    ON models(sha256);
CREATE UNIQUE INDEX IF NOT EXISTS idx_llama_installations_id
    ON llama_installations(id);

CREATE TABLE IF NOT EXISTS projectors (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    modified_at_unix_ms TEXT,
    quick_fingerprint TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('present', 'missing', 'unreadable')),
    updated_at_unix_ms TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_projectors_path
    ON projectors(path);
CREATE INDEX IF NOT EXISTS idx_projectors_sha256
    ON projectors(sha256);

CREATE TABLE IF NOT EXISTS projector_associations (
    model_id TEXT NOT NULL,
    projector_id TEXT NOT NULL,
    status TEXT NOT NULL,
    user_selected INTEGER NOT NULL DEFAULT 0,
    evidence_json TEXT NOT NULL,
    updated_at_unix_ms TEXT NOT NULL,
    PRIMARY KEY(model_id, projector_id),
    FOREIGN KEY(model_id) REFERENCES models(id) ON DELETE CASCADE,
    FOREIGN KEY(projector_id) REFERENCES projectors(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS compatibility_results (
    model_id TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    model_sha256 TEXT NOT NULL,
    installation_fingerprint TEXT NOT NULL,
    registry_revision TEXT NOT NULL,
    status TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    computed_at_unix_ms TEXT NOT NULL,
    PRIMARY KEY(model_id, installation_id),
    FOREIGN KEY(model_id) REFERENCES models(id) ON DELETE CASCADE,
    FOREIGN KEY(installation_id) REFERENCES llama_installations(id) ON DELETE CASCADE
);
