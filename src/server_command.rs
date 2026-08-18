use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::llama::LlamaInstallation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEnvironmentValue {
    pub value: OsString,
    pub sensitive: bool,
}

impl ServerEnvironmentValue {
    pub fn plain(value: impl Into<OsString>) -> Self {
        Self {
            value: value.into(),
            sensitive: false,
        }
    }

    pub fn secret(value: impl Into<OsString>) -> Self {
        Self {
            value: value.into(),
            sensitive: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ServerLaunchSettings {
    pub model: PathBuf,
    pub mmproj: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub threads: Option<u32>,
    pub context_size: Option<u64>,
    pub gpu_layers: Option<u32>,
    pub batch_size: Option<u64>,
    pub ubatch_size: Option<u64>,
    pub api_key: Option<String>,
    pub api_key_file: Option<PathBuf>,
    pub environment: BTreeMap<OsString, ServerEnvironmentValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerLaunchSpec {
    pub executable: PathBuf,
    pub argv: Vec<OsString>,
    pub cwd: PathBuf,
    pub environment: BTreeMap<OsString, OsString>,
    sensitive_argv_indexes: BTreeSet<usize>,
    sensitive_environment_keys: BTreeSet<OsString>,
}

impl ServerLaunchSpec {
    pub fn diagnostic_command(&self) -> String {
        let mut rendered = vec![quote_os(self.executable.as_os_str())];
        rendered.extend(self.argv.iter().enumerate().map(|(index, value)| {
            if self.sensitive_argv_indexes.contains(&index) {
                quote_os(OsStr::new("<redacted>"))
            } else {
                quote_os(value)
            }
        }));
        rendered.join(" ")
    }

    pub fn diagnostic_environment(&self) -> BTreeMap<String, String> {
        self.environment
            .iter()
            .map(|(key, value)| {
                let key_text = key.to_string_lossy().into_owned();
                let value_text = if self.sensitive_environment_keys.contains(key) {
                    "<redacted>".into()
                } else {
                    value.to_string_lossy().into_owned()
                };
                (key_text, value_text)
            })
            .collect()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ServerCommandError {
    #[error("selected llama.cpp installation does not contain llama-server")]
    ServerMissing,

    #[error("model path is empty")]
    ModelPathEmpty,

    #[error("runtime capability evidence does not expose required option {option}")]
    UnsupportedOption { option: String },

    #[error("invalid launch setting {field}: {reason}")]
    InvalidValue { field: &'static str, reason: String },
}

pub fn build_server_launch_spec(
    installation: &LlamaInstallation,
    settings: &ServerLaunchSettings,
) -> Result<ServerLaunchSpec, ServerCommandError> {
    let server = installation
        .server
        .as_ref()
        .ok_or(ServerCommandError::ServerMissing)?;
    if settings.model.as_os_str().is_empty() {
        return Err(ServerCommandError::ModelPathEmpty);
    }

    let capabilities = server_options(&server.help_output);
    let mut argv = Vec::new();
    let mut sensitive_argv_indexes = BTreeSet::new();

    push_path_option(
        &mut argv,
        &capabilities,
        &["--model", "-m"],
        &settings.model,
        false,
        &mut sensitive_argv_indexes,
    )?;

    if let Some(path) = &settings.mmproj {
        push_path_option(
            &mut argv,
            &capabilities,
            &["--mmproj", "-mm"],
            path,
            false,
            &mut sensitive_argv_indexes,
        )?;
    }
    if let Some(path) = &settings.config {
        push_path_option(
            &mut argv,
            &capabilities,
            &["--config"],
            path,
            false,
            &mut sensitive_argv_indexes,
        )?;
    }
    if let Some(host) = &settings.host {
        if host.trim().is_empty() {
            return Err(ServerCommandError::InvalidValue {
                field: "host",
                reason: "host cannot be empty".into(),
            });
        }
        push_value_option(
            &mut argv,
            &capabilities,
            &["--host"],
            host,
            false,
            &mut sensitive_argv_indexes,
        )?;
    }
    if let Some(port) = settings.port {
        if port == 0 {
            return Err(ServerCommandError::InvalidValue {
                field: "port",
                reason: "port must be in 1..=65535".into(),
            });
        }
        push_value_option(
            &mut argv,
            &capabilities,
            &["--port"],
            port.to_string(),
            false,
            &mut sensitive_argv_indexes,
        )?;
    }
    if let Some(threads) = settings.threads {
        require_positive("threads", threads as u64)?;
        push_value_option(
            &mut argv,
            &capabilities,
            &["--threads", "-t"],
            threads.to_string(),
            false,
            &mut sensitive_argv_indexes,
        )?;
    }
    if let Some(context_size) = settings.context_size {
        require_positive("context_size", context_size)?;
        push_value_option(
            &mut argv,
            &capabilities,
            &["--ctx-size", "-c"],
            context_size.to_string(),
            false,
            &mut sensitive_argv_indexes,
        )?;
    }
    if let Some(gpu_layers) = settings.gpu_layers {
        push_value_option(
            &mut argv,
            &capabilities,
            &["--n-gpu-layers", "-ngl"],
            gpu_layers.to_string(),
            false,
            &mut sensitive_argv_indexes,
        )?;
    }
    if let Some(batch_size) = settings.batch_size {
        require_positive("batch_size", batch_size)?;
        push_value_option(
            &mut argv,
            &capabilities,
            &["--batch-size", "-b"],
            batch_size.to_string(),
            false,
            &mut sensitive_argv_indexes,
        )?;
    }
    if let Some(ubatch_size) = settings.ubatch_size {
        require_positive("ubatch_size", ubatch_size)?;
        if let Some(batch_size) = settings.batch_size
            && ubatch_size > batch_size
        {
            return Err(ServerCommandError::InvalidValue {
                field: "ubatch_size",
                reason: format!("ubatch-size {ubatch_size} exceeds batch-size {batch_size}"),
            });
        }
        push_value_option(
            &mut argv,
            &capabilities,
            &["--ubatch-size", "-ub"],
            ubatch_size.to_string(),
            false,
            &mut sensitive_argv_indexes,
        )?;
    }
    if let Some(api_key) = &settings.api_key {
        if api_key.is_empty() {
            return Err(ServerCommandError::InvalidValue {
                field: "api_key",
                reason: "API key cannot be empty when configured".into(),
            });
        }
        push_value_option(
            &mut argv,
            &capabilities,
            &["--api-key"],
            api_key,
            true,
            &mut sensitive_argv_indexes,
        )?;
    }
    if let Some(api_key_file) = &settings.api_key_file {
        push_path_option(
            &mut argv,
            &capabilities,
            &["--api-key-file"],
            api_key_file,
            false,
            &mut sensitive_argv_indexes,
        )?;
    }

    let mut environment = BTreeMap::new();
    let mut sensitive_environment_keys = BTreeSet::new();
    for (key, value) in &settings.environment {
        if key.as_os_str().is_empty() {
            return Err(ServerCommandError::InvalidValue {
                field: "environment",
                reason: "environment variable name cannot be empty".into(),
            });
        }
        environment.insert(key.clone(), value.value.clone());
        if value.sensitive {
            sensitive_environment_keys.insert(key.clone());
        }
    }

    Ok(ServerLaunchSpec {
        executable: server.path.clone(),
        argv,
        cwd: installation.root_path.clone(),
        environment,
        sensitive_argv_indexes,
        sensitive_environment_keys,
    })
}

fn require_positive(field: &'static str, value: u64) -> Result<(), ServerCommandError> {
    if value == 0 {
        Err(ServerCommandError::InvalidValue {
            field,
            reason: "value must be greater than zero".into(),
        })
    } else {
        Ok(())
    }
}

fn push_path_option(
    argv: &mut Vec<OsString>,
    capabilities: &BTreeSet<String>,
    aliases: &[&str],
    value: &Path,
    sensitive: bool,
    sensitive_indexes: &mut BTreeSet<usize>,
) -> Result<(), ServerCommandError> {
    if value.as_os_str().is_empty() {
        return Err(ServerCommandError::InvalidValue {
            field: "path",
            reason: "path cannot be empty".into(),
        });
    }
    push_value_option(
        argv,
        capabilities,
        aliases,
        value.as_os_str(),
        sensitive,
        sensitive_indexes,
    )
}

fn push_value_option(
    argv: &mut Vec<OsString>,
    capabilities: &BTreeSet<String>,
    aliases: &[&str],
    value: impl AsRef<OsStr>,
    sensitive: bool,
    sensitive_indexes: &mut BTreeSet<usize>,
) -> Result<(), ServerCommandError> {
    let option = aliases
        .iter()
        .find(|candidate| capabilities.contains(**candidate))
        .copied()
        .ok_or_else(|| ServerCommandError::UnsupportedOption {
            option: aliases.first().copied().unwrap_or("<unknown>").into(),
        })?;
    argv.push(option.into());
    argv.push(value.as_ref().to_os_string());
    if sensitive {
        sensitive_indexes.insert(argv.len() - 1);
    }
    Ok(())
}

fn server_options(help: &str) -> BTreeSet<String> {
    help.split_whitespace()
        .filter_map(|token| {
            let option = token.trim_matches(|character: char| {
                matches!(
                    character,
                    ',' | ';' | ':' | '[' | ']' | '(' | ')' | '`' | '<' | '>'
                )
            });
            option.starts_with('-').then(|| option.to_owned())
        })
        .collect()
}

fn quote_os(value: &OsStr) -> String {
    let value = value.to_string_lossy();
    if !value.is_empty()
        && value
            .chars()
            .all(|character| !character.is_whitespace() && !matches!(character, '"' | '\\'))
    {
        return value.into_owned();
    }

    let mut rendered = String::from("\"");
    let mut backslashes = 0usize;
    for character in value.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                rendered.push_str(&"\\".repeat(backslashes * 2 + 1));
                rendered.push('"');
                backslashes = 0;
            }
            _ => {
                rendered.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                rendered.push(character);
            }
        }
    }
    rendered.push_str(&"\\".repeat(backslashes * 2));
    rendered.push('"');
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llama::ToolEvidence;

    fn installation(help: &str) -> LlamaInstallation {
        let root = PathBuf::from(r"C:\llama cpp 外部 build");
        LlamaInstallation {
            id: "runtime-1".into(),
            name: "runtime".into(),
            root_path: root.clone(),
            server: Some(ToolEvidence {
                path: root.join("bin").join("llama-server.exe"),
                sha256: "a".repeat(64),
                version_output: "version".into(),
                help_output: help.into(),
                device_output: "CPU".into(),
            }),
            bench: None,
            fit_params: None,
            backend: Some("CPU".into()),
            capabilities: BTreeSet::new(),
            discovered_at_unix_ms: 1,
        }
    }

    #[test]
    fn typed_spec_preserves_unicode_space_paths_as_distinct_argv() {
        let selected = installation(
            "--model FILE --mmproj FILE --config FILE --host HOST --port N --threads N --ctx-size N --n-gpu-layers N --batch-size N --ubatch-size N",
        );
        let expected_executable = selected.server.as_ref().unwrap().path.clone();
        let expected_cwd = selected.root_path.clone();
        let settings = ServerLaunchSettings {
            model: PathBuf::from(r"D:\Models 外部\my model.gguf"),
            mmproj: Some(PathBuf::from(r"D:\Models 外部\视觉 projector.gguf")),
            config: Some(PathBuf::from(r"D:\Configs 外部\server config.ini")),
            host: Some("127.0.0.1".into()),
            port: Some(8080),
            threads: Some(8),
            context_size: Some(65536),
            gpu_layers: Some(20),
            batch_size: Some(512),
            ubatch_size: Some(128),
            ..Default::default()
        };
        let model_arg = settings.model.as_os_str().to_os_string();
        let mmproj_arg = settings.mmproj.as_ref().unwrap().as_os_str().to_os_string();
        let config_arg = settings.config.as_ref().unwrap().as_os_str().to_os_string();

        let spec = build_server_launch_spec(&selected, &settings).unwrap();
        assert_eq!(spec.executable, expected_executable);
        assert_eq!(spec.cwd, expected_cwd);
        assert!(spec.argv.contains(&model_arg));
        assert!(spec.argv.contains(&mmproj_arg));
        assert!(spec.argv.contains(&config_arg));
        assert!(spec.diagnostic_command().contains("my model.gguf"));
        assert!(spec.diagnostic_command().contains("视觉 projector.gguf"));
    }

    #[test]
    fn unsupported_option_fails_before_a_launch_spec_exists() {
        let selected = installation("--model FILE --port N");
        let settings = ServerLaunchSettings {
            model: PathBuf::from("model.gguf"),
            context_size: Some(8192),
            ..Default::default()
        };
        assert_eq!(
            build_server_launch_spec(&selected, &settings).unwrap_err(),
            ServerCommandError::UnsupportedOption {
                option: "--ctx-size".into()
            }
        );
    }

    #[test]
    fn cross_field_and_zero_values_fail_deterministically() {
        let selected = installation("--model FILE --batch-size N --ubatch-size N --threads N");
        let bad_batch = ServerLaunchSettings {
            model: PathBuf::from("model.gguf"),
            batch_size: Some(128),
            ubatch_size: Some(256),
            ..Default::default()
        };
        assert!(matches!(
            build_server_launch_spec(&selected, &bad_batch),
            Err(ServerCommandError::InvalidValue {
                field: "ubatch_size",
                ..
            })
        ));

        let zero_threads = ServerLaunchSettings {
            model: PathBuf::from("model.gguf"),
            threads: Some(0),
            ..Default::default()
        };
        assert!(matches!(
            build_server_launch_spec(&selected, &zero_threads),
            Err(ServerCommandError::InvalidValue {
                field: "threads",
                ..
            })
        ));
    }

    #[test]
    fn diagnostics_redact_secret_argument_and_environment_without_mutating_execution_data() {
        let selected = installation("--model FILE --api-key KEY");
        let mut environment = BTreeMap::new();
        environment.insert(
            "PUBLIC_SETTING".into(),
            ServerEnvironmentValue::plain("visible"),
        );
        environment.insert(
            "PRIVATE_TOKEN".into(),
            ServerEnvironmentValue::secret("secret-env"),
        );
        let settings = ServerLaunchSettings {
            model: PathBuf::from("model.gguf"),
            api_key: Some("super-secret".into()),
            environment,
            ..Default::default()
        };

        let spec = build_server_launch_spec(&selected, &settings).unwrap();
        assert!(
            spec.argv
                .iter()
                .any(|item| item.as_os_str() == OsStr::new("super-secret"))
        );
        assert_eq!(
            spec.environment.get(OsStr::new("PRIVATE_TOKEN")).unwrap(),
            "secret-env"
        );

        let diagnostic = spec.diagnostic_command();
        assert!(!diagnostic.contains("super-secret"));
        assert!(diagnostic.contains("<redacted>"));
        let diagnostic_environment = spec.diagnostic_environment();
        assert_eq!(diagnostic_environment["PUBLIC_SETTING"], "visible");
        assert_eq!(diagnostic_environment["PRIVATE_TOKEN"], "<redacted>");
    }

    #[test]
    fn preferred_supported_alias_is_emitted_from_server_help_evidence() {
        let selected = installation("-m FILE -c N -t N");
        let settings = ServerLaunchSettings {
            model: PathBuf::from("model.gguf"),
            context_size: Some(4096),
            threads: Some(4),
            ..Default::default()
        };
        let spec = build_server_launch_spec(&selected, &settings).unwrap();
        assert_eq!(
            spec.argv,
            vec!["-m", "model.gguf", "-t", "4", "-c", "4096"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
        );
    }
}
