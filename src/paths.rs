use std::{
    env, fs,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::error::{LlamaManagerError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageMode {
    Portable,
    UserData,
}

impl std::fmt::Display for StorageMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Portable => f.write_str("Portable"),
            Self::UserData => f.write_str("User data"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppPaths {
    pub mode: StorageMode,
    pub root: PathBuf,
    pub data: PathBuf,
    pub config: PathBuf,
    pub logs: PathBuf,
    pub exports: PathBuf,
    pub database: PathBuf,
}

impl AppPaths {
    pub fn detect() -> Result<Self> {
        let exe = env::current_exe()?;
        let exe_dir = exe
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| LlamaManagerError::InvalidPath(exe.clone()))?;

        let env_portable = env::var("LLAMAMANAGER_PORTABLE")
            .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);
        let marker_portable = exe_dir.join("portable.flag").is_file();

        let (mode, root) = if env_portable || marker_portable {
            (StorageMode::Portable, exe_dir)
        } else {
            let dirs = ProjectDirs::from("dev", "xdCloudy", "LlamaManager").ok_or_else(|| {
                LlamaManagerError::State("could not resolve the user data directory".into())
            })?;
            (StorageMode::UserData, dirs.data_local_dir().to_path_buf())
        };

        Self::from_root(mode, root)
    }

    pub fn from_root(mode: StorageMode, root: PathBuf) -> Result<Self> {
        let data = root.join("data");
        let config = root.join("config");
        let logs = root.join("logs");
        let exports = root.join("exports");

        for path in [&data, &config, &logs, &exports] {
            fs::create_dir_all(path)?;
        }

        let database = data.join("llamamanager.db");
        Ok(Self {
            mode,
            root,
            data,
            config,
            logs,
            exports,
            database,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_layout_is_relocatable() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_root(
            StorageMode::Portable,
            temp.path().join("Llama Manager 测试"),
        )
        .unwrap();

        assert_eq!(paths.data, paths.root.join("data"));
        assert_eq!(paths.config, paths.root.join("config"));
        assert_eq!(paths.logs, paths.root.join("logs"));
        assert_eq!(
            paths.database,
            paths.root.join("data").join("llamamanager.db")
        );
        assert!(paths.data.is_dir());
    }
}
