use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, LlamaManagerError>;

#[derive(Debug, Error)]
pub enum LlamaManagerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("path is not valid for this operation: {0}")]
    InvalidPath(PathBuf),

    #[error("no usable llama.cpp binaries were found under {0}")]
    NoLlamaBinaries(PathBuf),

    #[error("required llama.cpp tool is missing: {0}")]
    MissingTool(&'static str),

    #[error("process failed ({program}) with exit code {code:?}: {stderr}")]
    ProcessFailed {
        program: String,
        code: Option<i32>,
        stderr: String,
    },

    #[error("benchmark was interrupted ({program}) with exit code {code:?}")]
    BenchmarkInterrupted {
        program: String,
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },

    #[error("GGUF parse error: {0}")]
    Gguf(String),

    #[error("benchmark output could not be parsed: {0}")]
    BenchmarkParse(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),

    #[error("application state error: {0}")]
    State(String),
}
