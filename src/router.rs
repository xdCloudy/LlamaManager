use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    llama::{LlamaInstallation, now_ms},
    model_store::ModelStore,
    server_readiness::ServerEndpoint,
};

const MAX_ROUTER_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ERROR_BODY_CHARS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterRole {
    Router,
    SingleModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterFeatureState {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterFeatureEvidence {
    pub state: RouterFeatureState,
    pub reason: String,
}

impl RouterFeatureEvidence {
    fn supported(reason: impl Into<String>) -> Self {
        Self {
            state: RouterFeatureState::Supported,
            reason: reason.into(),
        }
    }

    fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            state: RouterFeatureState::Unsupported,
            reason: reason.into(),
        }
    }

    fn unknown(reason: impl Into<String>) -> Self {
        Self {
            state: RouterFeatureState::Unknown,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterStaticCapabilities {
    pub server_sha256: Option<String>,
    pub server_version: Option<String>,
    pub router_cli_observed: bool,
    pub models_dir: bool,
    pub models_preset: bool,
    pub models_max: bool,
    pub models_autoload: bool,
    pub observed_options: BTreeSet<String>,
}

impl RouterStaticCapabilities {
    pub fn from_installation(installation: &LlamaInstallation) -> Self {
        let Some(server) = installation.server.as_ref() else {
            return Self {
                server_sha256: None,
                server_version: None,
                router_cli_observed: false,
                models_dir: false,
                models_preset: false,
                models_max: false,
                models_autoload: false,
                observed_options: BTreeSet::new(),
            };
        };

        let mut observed_options = BTreeSet::new();
        for option in [
            "--models-dir",
            "--models-preset",
            "--models-max",
            "--models-autoload",
            "--no-models-autoload",
        ] {
            if help_has_option(&server.help_output, option) {
                observed_options.insert(option.to_string());
            }
        }

        let models_dir = observed_options.contains("--models-dir");
        let models_preset = observed_options.contains("--models-preset");
        let models_max = observed_options.contains("--models-max");
        let models_autoload = observed_options.contains("--models-autoload")
            || observed_options.contains("--no-models-autoload");
        let router_cli_observed = models_dir || models_preset || models_max || models_autoload;

        Self {
            server_sha256: Some(server.sha256.clone()),
            server_version: (!server.version_output.trim().is_empty())
                .then(|| server.version_output.trim().to_string()),
            router_cli_observed,
            models_dir,
            models_preset,
            models_max,
            models_autoload,
            observed_options,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterEndpointCapabilities {
    pub props: RouterFeatureEvidence,
    pub list_models: RouterFeatureEvidence,
    pub reload_models: RouterFeatureEvidence,
    pub load_model: RouterFeatureEvidence,
    pub unload_model: RouterFeatureEvidence,
    pub model_events: RouterFeatureEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterModelPhase {
    Unloaded,
    Downloading,
    Loading,
    Loaded,
    Sleeping,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterModelStatus {
    pub phase: RouterModelPhase,
    pub failed: bool,
    pub exit_code: Option<i64>,
    pub args: Vec<String>,
    pub progress: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterLibraryLinkKind {
    ExactPath,
    Sha256,
    AmbiguousSha256,
    Unmatched,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterLibraryLink {
    pub kind: RouterLibraryLinkKind,
    pub model_id: Option<String>,
    pub candidates: Vec<String>,
    pub reason: String,
}

impl RouterLibraryLink {
    fn unmatched(reason: impl Into<String>) -> Self {
        Self {
            kind: RouterLibraryLinkKind::Unmatched,
            model_id: None,
            candidates: Vec::new(),
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterModel {
    pub id: String,
    pub routing_targets: Vec<String>,
    pub path: Option<PathBuf>,
    pub sha256: Option<String>,
    pub status: RouterModelStatus,
    /// `Some` only when the router payload explicitly reports residency.
    /// LlamaWave does not infer residency from filenames or from a merely running process.
    pub resident: Option<bool>,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub library_link: RouterLibraryLink,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterRegistry {
    pub endpoint: String,
    pub role: RouterRole,
    pub static_capabilities: RouterStaticCapabilities,
    pub endpoints: RouterEndpointCapabilities,
    pub models: Vec<RouterModel>,
    pub observed_at_unix_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterDisconnectEvidence {
    pub error: String,
    pub observed_at_unix_ms: u128,
}

/// Tracks the latest live router snapshot without converting a disconnect into stale success.
/// A disconnected tracker can retain its previous snapshot for diagnostics, but callers must
/// inspect `disconnect` before treating that snapshot as current.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RouterRegistryTracker {
    pub current: Option<RouterRegistry>,
    pub disconnect: Option<RouterDisconnectEvidence>,
}

impl RouterRegistryTracker {
    pub fn reconcile(&mut self, result: Result<RouterRegistry, RouterDiscoveryError>) {
        match result {
            Ok(registry) => {
                self.current = Some(registry);
                self.disconnect = None;
            }
            Err(error) => {
                self.disconnect = Some(RouterDisconnectEvidence {
                    error: error.to_string(),
                    observed_at_unix_ms: now_ms(),
                });
            }
        }
    }

    pub fn is_live(&self) -> bool {
        self.current.is_some() && self.disconnect.is_none()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RouterDiscoveryError {
    #[error("selected llama.cpp installation does not contain llama-server evidence")]
    MissingServerEvidence,

    #[error("router target host {host} could not be resolved: {message}")]
    HostResolution { host: String, message: String },

    #[error("router transport failed for {path}: {message}")]
    Transport { path: String, message: String },

    #[error("router authentication failed at {path} with HTTP {status_code}")]
    AuthenticationRejected { path: String, status_code: u16 },

    #[error("router endpoint {path} is unavailable on this runtime (HTTP {status_code})")]
    EndpointUnsupported { path: String, status_code: u16 },

    #[error("router endpoint {path} returned HTTP {status_code}: {body_excerpt}")]
    HttpFailure {
        path: String,
        status_code: u16,
        body_excerpt: String,
    },

    #[error("router protocol drift at {path}: {message}; response={body_excerpt}")]
    ProtocolDrift {
        path: String,
        message: String,
        body_excerpt: String,
    },

    #[error("failed to map router model to the M2 library: {message}")]
    LibraryLookup { message: String },
}

pub fn discover_router_registry(
    installation: &LlamaInstallation,
    endpoint: &ServerEndpoint,
    model_store: Option<&ModelStore>,
    timeout: Duration,
) -> Result<RouterRegistry, RouterDiscoveryError> {
    if installation.server.is_none() {
        return Err(RouterDiscoveryError::MissingServerEvidence);
    }

    let static_capabilities = RouterStaticCapabilities::from_installation(installation);
    let props = request_json(endpoint, "GET", "/props", None, timeout)?;
    ensure_success("/props", &props)?;
    let role = parse_role(&props.body)?;

    let props_evidence = RouterFeatureEvidence::supported(
        "GET /props returned a recognized live llama-server role",
    );

    if role == RouterRole::SingleModel {
        return Ok(RouterRegistry {
            endpoint: endpoint.authority(),
            role,
            static_capabilities,
            endpoints: RouterEndpointCapabilities {
                props: props_evidence,
                list_models: RouterFeatureEvidence::unsupported(
                    "live /props role is a single-model server, not router mode",
                ),
                reload_models: RouterFeatureEvidence::unsupported(
                    "live /props role is a single-model server, not router mode",
                ),
                load_model: RouterFeatureEvidence::unsupported(
                    "live /props role is a single-model server, not router mode",
                ),
                unload_model: RouterFeatureEvidence::unsupported(
                    "live /props role is a single-model server, not router mode",
                ),
                model_events: RouterFeatureEvidence::unsupported(
                    "live /props role is a single-model server, not router mode",
                ),
            },
            models: Vec::new(),
            observed_at_unix_ms: now_ms(),
        });
    }

    let models_response = request_json(endpoint, "GET", "/models", None, timeout)?;
    ensure_success("/models", &models_response)?;
    let mut models = parse_models(&models_response.body)?;

    if let Some(store) = model_store {
        for model in &mut models {
            model.library_link = map_to_library(model, store)?;
        }
    }

    Ok(RouterRegistry {
        endpoint: endpoint.authority(),
        role,
        static_capabilities,
        endpoints: RouterEndpointCapabilities {
            props: props_evidence,
            list_models: RouterFeatureEvidence::supported(
                "GET /models returned a valid live router registry",
            ),
            // These endpoints mutate state or establish a long-lived stream. #38 deliberately
            // does not pretend they are supported merely because a newer llama.cpp documents
            // them; #39 verifies control operations against the selected runtime.
            reload_models: RouterFeatureEvidence::unknown(
                "not mutated during discovery; verify GET /models?reload=1 before enabling",
            ),
            load_model: RouterFeatureEvidence::unknown(
                "not mutated during discovery; verify POST /models/load before enabling",
            ),
            unload_model: RouterFeatureEvidence::unknown(
                "not mutated during discovery; verify POST /models/unload before enabling",
            ),
            model_events: RouterFeatureEvidence::unknown(
                "SSE endpoint is not opened during one-shot discovery",
            ),
        },
        models,
        observed_at_unix_ms: now_ms(),
    })
}

fn help_has_option(help: &str, option: &str) -> bool {
    help.split_whitespace().any(|token| {
        token
            .trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '[' | ']' | '(' | ')' | '`'))
            == option
    })
}

#[derive(Debug)]
struct RawHttpResponse {
    status_code: u16,
    body: String,
}

fn request_json(
    endpoint: &ServerEndpoint,
    method: &str,
    path: &str,
    body: Option<&str>,
    timeout: Duration,
) -> Result<RawHttpResponse, RouterDiscoveryError> {
    let mut stream = connect(endpoint, timeout, path)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| transport_error(path, error))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| transport_error(path, error))?;

    let body = body.unwrap_or("");
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n",
        endpoint.authority()
    );
    if let Some(api_key) = endpoint.api_key.as_ref() {
        request.push_str("Authorization: Bearer ");
        request.push_str(api_key);
        request.push_str("\r\n");
    }
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    request.push_str(body);

    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|error| transport_error(path, error))?;

    let mut bytes = Vec::new();
    stream
        .take(MAX_ROUTER_RESPONSE_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| transport_error(path, error))?;

    let response = String::from_utf8_lossy(&bytes);
    let status_line = response
        .lines()
        .next()
        .ok_or_else(|| RouterDiscoveryError::Transport {
            path: path.into(),
            message: "empty HTTP response".into(),
        })?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| RouterDiscoveryError::Transport {
            path: path.into(),
            message: format!("invalid HTTP status line: {status_line}"),
        })?;
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, value)| value)
        .unwrap_or("")
        .to_string();

    Ok(RawHttpResponse { status_code, body })
}

fn connect(
    endpoint: &ServerEndpoint,
    timeout: Duration,
    path: &str,
) -> Result<TcpStream, RouterDiscoveryError> {
    if endpoint.port == 0 {
        return Err(RouterDiscoveryError::HostResolution {
            host: endpoint.host.clone(),
            message: "port must be in 1..=65535".into(),
        });
    }

    let addresses: Vec<_> = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| RouterDiscoveryError::HostResolution {
            host: endpoint.host.clone(),
            message: error.to_string(),
        })?
        .collect();
    if addresses.is_empty() {
        return Err(RouterDiscoveryError::HostResolution {
            host: endpoint.host.clone(),
            message: "host resolved to no addresses".into(),
        });
    }
    if !endpoint.allow_non_loopback && addresses.iter().any(|address| !address.ip().is_loopback()) {
        return Err(RouterDiscoveryError::HostResolution {
            host: endpoint.host.clone(),
            message: "non-loopback router target requires explicit opt-in".into(),
        });
    }

    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }

    Err(RouterDiscoveryError::Transport {
        path: path.into(),
        message: last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "no resolved address accepted the connection".into()),
    })
}

fn transport_error(path: &str, error: std::io::Error) -> RouterDiscoveryError {
    RouterDiscoveryError::Transport {
        path: path.into(),
        message: error.to_string(),
    }
}

fn ensure_success(path: &str, response: &RawHttpResponse) -> Result<(), RouterDiscoveryError> {
    match response.status_code {
        200..=299 => Ok(()),
        401 | 403 => Err(RouterDiscoveryError::AuthenticationRejected {
            path: path.into(),
            status_code: response.status_code,
        }),
        404 | 405 => Err(RouterDiscoveryError::EndpointUnsupported {
            path: path.into(),
            status_code: response.status_code,
        }),
        status_code => Err(RouterDiscoveryError::HttpFailure {
            path: path.into(),
            status_code,
            body_excerpt: excerpt(&response.body),
        }),
    }
}

fn parse_role(body: &str) -> Result<RouterRole, RouterDiscoveryError> {
    let value: Value = serde_json::from_str(body).map_err(|error| protocol_drift(
        "/props",
        format!("response was not valid JSON: {error}"),
        body,
    ))?;
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_drift(
            "/props",
            "expected string field `role`",
            body,
        ))?;

    match role {
        "router" => Ok(RouterRole::Router),
        "model" | "server" | "single-model" | "single_model" => Ok(RouterRole::SingleModel),
        other => Err(protocol_drift(
            "/props",
            format!("unrecognized role `{other}`"),
            body,
        )),
    }
}

fn parse_models(body: &str) -> Result<Vec<RouterModel>, RouterDiscoveryError> {
    let value: Value = serde_json::from_str(body).map_err(|error| protocol_drift(
        "/models",
        format!("response was not valid JSON: {error}"),
        body,
    ))?;
    let entries = match &value {
        Value::Array(entries) => entries,
        Value::Object(object) => object
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| protocol_drift(
                "/models",
                "expected an array response or object field `data` containing an array",
                body,
            ))?,
        _ => {
            return Err(protocol_drift(
                "/models",
                "expected an array response or object response",
                body,
            ));
        }
    };

    entries
        .iter()
        .map(|entry| parse_model(entry, body))
        .collect()
}

fn parse_model(entry: &Value, full_body: &str) -> Result<RouterModel, RouterDiscoveryError> {
    let object = entry.as_object().ok_or_else(|| protocol_drift(
        "/models",
        "model entry was not an object",
        full_body,
    ))?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| protocol_drift(
            "/models",
            "model entry is missing non-empty string field `id`",
            full_body,
        ))?
        .to_string();

    let mut routing_targets = BTreeSet::from([id.clone()]);
    if let Some(alias) = object.get("alias").and_then(Value::as_str) {
        if !alias.is_empty() {
            routing_targets.insert(alias.to_string());
        }
    }
    if let Some(aliases) = object.get("aliases").and_then(Value::as_array) {
        for alias in aliases.iter().filter_map(Value::as_str).filter(|alias| !alias.is_empty()) {
            routing_targets.insert(alias.to_string());
        }
    }

    let path = object
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let sha256 = object
        .get("sha256")
        .or_else(|| object.get("file_sha256"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let resident = object
        .get("resident")
        .or_else(|| object.get("is_resident"))
        .and_then(Value::as_bool);

    let status = parse_status(object.get("status"));
    let architecture = object.get("architecture");
    let input_modalities = architecture
        .and_then(|value| value.get("input_modalities"))
        .and_then(Value::as_array)
        .map(string_array)
        .unwrap_or_default();
    let output_modalities = architecture
        .and_then(|value| value.get("output_modalities"))
        .and_then(Value::as_array)
        .map(string_array)
        .unwrap_or_default();

    Ok(RouterModel {
        id,
        routing_targets: routing_targets.into_iter().collect(),
        path,
        sha256,
        status,
        resident,
        input_modalities,
        output_modalities,
        library_link: RouterLibraryLink::unmatched(
            "no M2 ModelStore was supplied for evidence-backed identity mapping",
        ),
    })
}

fn parse_status(value: Option<&Value>) -> RouterModelStatus {
    let (phase_text, failed, exit_code, args, progress) = match value {
        Some(Value::String(phase)) => (Some(phase.as_str()), false, None, Vec::new(), None),
        Some(Value::Object(status)) => (
            status.get("value").and_then(Value::as_str),
            status.get("failed").and_then(Value::as_bool).unwrap_or(false),
            status.get("exit_code").and_then(Value::as_i64),
            status
                .get("args")
                .and_then(Value::as_array)
                .map(string_array)
                .unwrap_or_default(),
            status.get("progress").cloned(),
        ),
        _ => (None, false, None, Vec::new(), None),
    };

    let phase = match phase_text {
        Some("unloaded") => RouterModelPhase::Unloaded,
        Some("downloading") => RouterModelPhase::Downloading,
        Some("loading") => RouterModelPhase::Loading,
        Some("loaded") => RouterModelPhase::Loaded,
        Some("sleeping") => RouterModelPhase::Sleeping,
        Some(other) => RouterModelPhase::Unknown(other.to_string()),
        None => RouterModelPhase::Unknown("missing".into()),
    };

    RouterModelStatus {
        phase,
        failed,
        exit_code,
        args,
        progress,
    }
}

fn string_array(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn map_to_library(
    model: &RouterModel,
    store: &ModelStore,
) -> Result<RouterLibraryLink, RouterDiscoveryError> {
    if let Some(path) = model.path.as_deref() {
        let location = store
            .model_location_by_path(path)
            .map_err(|error| RouterDiscoveryError::LibraryLookup {
                message: error.to_string(),
            })?;
        if let Some(location) = location {
            return Ok(RouterLibraryLink {
                kind: RouterLibraryLinkKind::ExactPath,
                model_id: Some(location.model_id.clone()),
                candidates: vec![location.model_id],
                reason: format!(
                    "router path exactly matches a persisted M2 model location: {}",
                    path.display()
                ),
            });
        }
    }

    if let Some(sha256) = model.sha256.as_deref() {
        let ids = store
            .model_ids_by_sha(sha256)
            .map_err(|error| RouterDiscoveryError::LibraryLookup {
                message: error.to_string(),
            })?;
        return match ids.as_slice() {
            [model_id] => Ok(RouterLibraryLink {
                kind: RouterLibraryLinkKind::Sha256,
                model_id: Some(model_id.clone()),
                candidates: ids,
                reason: "router-provided SHA-256 uniquely matches persisted M2 model evidence".into(),
            }),
            [] => Ok(RouterLibraryLink::unmatched(
                "router path/SHA-256 did not match persisted M2 evidence",
            )),
            _ => Ok(RouterLibraryLink {
                kind: RouterLibraryLinkKind::AmbiguousSha256,
                model_id: None,
                candidates: ids,
                reason: "router-provided SHA-256 matches multiple persisted model identities; refusing to guess"
                    .into(),
            }),
        };
    }

    Ok(RouterLibraryLink::unmatched(
        "router supplied neither an exact persisted path match nor content SHA-256; filename matching is intentionally not used",
    ))
}

fn protocol_drift(
    path: &str,
    message: impl Into<String>,
    body: &str,
) -> RouterDiscoveryError {
    RouterDiscoveryError::ProtocolDrift {
        path: path.into(),
        message: message.into(),
        body_excerpt: excerpt(body),
    }
}

fn excerpt(body: &str) -> String {
    body.chars().take(MAX_ERROR_BODY_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_capabilities_do_not_use_union_capabilities_as_server_authority() {
        let installation = LlamaInstallation {
            id: "test".into(),
            name: "test".into(),
            root_path: PathBuf::from("."),
            server: Some(crate::llama::ToolEvidence {
                path: PathBuf::from("llama-server"),
                sha256: "abc".into(),
                version_output: "version".into(),
                help_output: "--models-preset X --models-max N --models-autoload".into(),
                device_output: String::new(),
            }),
            bench: None,
            fit_params: None,
            backend: None,
            capabilities: BTreeSet::from(["--models-dir".into()]),
            discovered_at_unix_ms: 0,
        };

        let evidence = RouterStaticCapabilities::from_installation(&installation);
        assert!(evidence.router_cli_observed);
        assert!(!evidence.models_dir);
        assert!(evidence.models_preset);
        assert!(evidence.models_max);
        assert!(evidence.models_autoload);
    }

    #[test]
    fn unknown_model_status_is_retained_instead_of_converted_to_loaded() {
        let value = serde_json::json!({"value": "future-state", "failed": false});
        let status = parse_status(Some(&value));
        assert_eq!(
            status.phase,
            RouterModelPhase::Unknown("future-state".into())
        );
    }

    #[test]
    fn explicit_residency_only_is_reported() {
        let model = parse_model(
            &serde_json::json!({
                "id": "a",
                "status": {"value": "loaded"}
            }),
            "fixture",
        )
        .unwrap();
        assert_eq!(model.status.phase, RouterModelPhase::Loaded);
        assert_eq!(model.resident, None);
    }

    #[test]
    fn routing_targets_are_explicit_and_deduplicated() {
        let model = parse_model(
            &serde_json::json!({
                "id": "canonical",
                "alias": "short",
                "aliases": ["short", "other"],
                "status": "unloaded"
            }),
            "fixture",
        )
        .unwrap();
        assert_eq!(
            model.routing_targets,
            vec!["canonical".to_string(), "other".to_string(), "short".to_string()]
        );
    }

    #[test]
    fn tracker_marks_retained_snapshot_stale_on_disconnect_and_recovers() {
        let registry = RouterRegistry {
            endpoint: "127.0.0.1:8080".into(),
            role: RouterRole::Router,
            static_capabilities: RouterStaticCapabilities {
                server_sha256: None,
                server_version: None,
                router_cli_observed: false,
                models_dir: false,
                models_preset: false,
                models_max: false,
                models_autoload: false,
                observed_options: BTreeSet::new(),
            },
            endpoints: RouterEndpointCapabilities {
                props: RouterFeatureEvidence::supported("test"),
                list_models: RouterFeatureEvidence::supported("test"),
                reload_models: RouterFeatureEvidence::unknown("test"),
                load_model: RouterFeatureEvidence::unknown("test"),
                unload_model: RouterFeatureEvidence::unknown("test"),
                model_events: RouterFeatureEvidence::unknown("test"),
            },
            models: Vec::new(),
            observed_at_unix_ms: 1,
        };
        let mut tracker = RouterRegistryTracker::default();
        tracker.reconcile(Ok(registry.clone()));
        assert!(tracker.is_live());

        tracker.reconcile(Err(RouterDiscoveryError::Transport {
            path: "/props".into(),
            message: "offline".into(),
        }));
        assert!(!tracker.is_live());
        assert_eq!(tracker.current, Some(registry.clone()));
        assert!(tracker.disconnect.is_some());

        let mut newer = registry;
        newer.observed_at_unix_ms = 2;
        tracker.reconcile(Ok(newer.clone()));
        assert!(tracker.is_live());
        assert_eq!(tracker.current, Some(newer));
        assert!(tracker.disconnect.is_none());
    }

    #[test]
    fn path_helper_accepts_windows_style_path_as_opaque_evidence() {
        let path = Path::new(r"C:\models\model.gguf");
        assert!(path.to_string_lossy().contains("model.gguf"));
    }
}
