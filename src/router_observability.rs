use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::PathBuf,
    thread,
    time::Duration,
};

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    llama::{LlamaInstallation, now_ms},
    model_store::ModelStore,
    paths::AppPaths,
    persistence::Database,
    router::{
        RouterDiscoveryError, RouterModel, RouterModelPhase, RouterRegistry, RouterRole,
        discover_router_registry,
    },
    server_readiness::ServerEndpoint,
};

const MAX_OBSERVABILITY_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_OBSERVABILITY_ERROR_CHARS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceAvailability {
    Observed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceValue<T> {
    pub availability: EvidenceAvailability,
    pub value: Option<T>,
    pub reason: String,
}

impl<T> EvidenceValue<T> {
    fn observed(value: T, reason: impl Into<String>) -> Self {
        Self {
            availability: EvidenceAvailability::Observed,
            value: Some(value),
            reason: reason.into(),
        }
    }

    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            availability: EvidenceAvailability::Unavailable,
            value: None,
            reason: reason.into(),
        }
    }

    pub fn is_observed(&self) -> bool {
        self.availability == EvidenceAvailability::Observed && self.value.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterEvictionSafety {
    SafeObserved,
    Busy { active_requests: u64 },
    RouterDenied,
    Unknown { reason: String },
    NotApplicable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterModelObservability {
    pub model: RouterModel,
    pub residency: EvidenceValue<bool>,
    pub active_requests: EvidenceValue<u64>,
    /// Router-relative last-use value. It is never converted into wall-clock time.
    pub last_used_ms: EvidenceValue<i64>,
    pub lru_rank: EvidenceValue<u64>,
    pub evictable: EvidenceValue<bool>,
}

impl RouterModelObservability {
    pub fn eviction_safety(&self) -> RouterEvictionSafety {
        if !matches!(
            &self.model.status.phase,
            RouterModelPhase::Loaded | RouterModelPhase::Sleeping
        ) {
            return RouterEvictionSafety::NotApplicable {
                reason: format!(
                    "router status is {:?}; no loaded-model eviction is implied",
                    self.model.status.phase
                ),
            };
        }

        let Some(active_requests) = self.active_requests.value else {
            return RouterEvictionSafety::Unknown {
                reason: self.active_requests.reason.clone(),
            };
        };
        if active_requests > 0 {
            return RouterEvictionSafety::Busy { active_requests };
        }

        match self.evictable.value {
            Some(true) => RouterEvictionSafety::SafeObserved,
            Some(false) => RouterEvictionSafety::RouterDenied,
            None => RouterEvictionSafety::Unknown {
                reason: self.evictable.reason.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterObservabilitySnapshot {
    pub registry: RouterRegistry,
    pub models: Vec<RouterModelObservability>,
    /// A supplemental observability read can fail while the canonical registry read remains valid.
    /// In that case every supplemental field remains unavailable and this error is retained.
    pub supplemental_error: Option<String>,
    pub observed_at_unix_ms: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterSnapshotFreshness {
    Empty,
    Loading,
    Live,
    Stale,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RouterObservabilityTracker {
    pub current: Option<RouterObservabilitySnapshot>,
    pub last_error: Option<String>,
    pub loading: bool,
}

impl RouterObservabilityTracker {
    pub fn begin_refresh(&mut self) {
        self.loading = true;
        self.last_error = None;
    }

    pub fn reconcile(&mut self, result: Result<RouterObservabilitySnapshot, String>) {
        self.loading = false;
        match result {
            Ok(snapshot) => {
                self.current = Some(snapshot);
                self.last_error = None;
            }
            Err(error) => {
                self.last_error = Some(error);
            }
        }
    }

    pub fn freshness(&self) -> RouterSnapshotFreshness {
        if self.loading {
            RouterSnapshotFreshness::Loading
        } else if self.last_error.is_some() && self.current.is_some() {
            RouterSnapshotFreshness::Stale
        } else if self.last_error.is_some() {
            RouterSnapshotFreshness::Failed
        } else if self.current.is_some() {
            RouterSnapshotFreshness::Live
        } else {
            RouterSnapshotFreshness::Empty
        }
    }

    pub fn is_live(&self) -> bool {
        self.freshness() == RouterSnapshotFreshness::Live
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RouterObservabilityError {
    #[error("router discovery failed: {0}")]
    Discovery(String),

    #[error("router observability host {host} could not be resolved: {message}")]
    HostResolution { host: String, message: String },

    #[error("router observability transport failed for {path}: {message}")]
    Transport { path: String, message: String },

    #[error("router observability authentication failed at {path} with HTTP {status_code}")]
    AuthenticationRejected { path: String, status_code: u16 },

    #[error("router observability endpoint {path} returned HTTP {status_code}: {body_excerpt}")]
    HttpFailure {
        path: String,
        status_code: u16,
        body_excerpt: String,
    },

    #[error("router observability protocol drift at {path}: {message}; response={body_excerpt}")]
    ProtocolDrift {
        path: String,
        message: String,
        body_excerpt: String,
    },
}

impl From<RouterDiscoveryError> for RouterObservabilityError {
    fn from(value: RouterDiscoveryError) -> Self {
        Self::Discovery(value.to_string())
    }
}

#[derive(Debug, Clone, Default)]
struct SupplementalModelEvidence {
    resident: Option<bool>,
    active_requests: Option<u64>,
    last_used_ms: Option<i64>,
    lru_rank: Option<u64>,
    evictable: Option<bool>,
}

pub fn discover_router_observability(
    installation: &LlamaInstallation,
    endpoint: &ServerEndpoint,
    model_store: Option<&ModelStore>,
    timeout: Duration,
) -> Result<RouterObservabilitySnapshot, RouterObservabilityError> {
    let registry = discover_router_registry(installation, endpoint, model_store, timeout)?;

    if registry.role != RouterRole::Router {
        return Ok(RouterObservabilitySnapshot {
            registry,
            models: Vec::new(),
            supplemental_error: None,
            observed_at_unix_ms: now_ms(),
        });
    }

    let supplemental = fetch_supplemental_model_evidence(endpoint, timeout);
    let (supplemental_by_id, supplemental_error) = match supplemental {
        Ok(evidence) => (Some(evidence), None),
        Err(error) => (None, Some(error.to_string())),
    };

    let models = registry
        .models
        .iter()
        .cloned()
        .map(|model| {
            let entry = supplemental_by_id
                .as_ref()
                .and_then(|values| values.get(&model.id));
            observability_for_model(model, entry, supplemental_error.as_deref())
        })
        .collect();

    Ok(RouterObservabilitySnapshot {
        registry,
        models,
        supplemental_error,
        observed_at_unix_ms: now_ms(),
    })
}

fn observability_for_model(
    model: RouterModel,
    supplemental: Option<&SupplementalModelEvidence>,
    supplemental_error: Option<&str>,
) -> RouterModelObservability {
    let unavailable_reason = |field: &str| {
        if let Some(error) = supplemental_error {
            format!("supplemental /models evidence unavailable for {field}: {error}")
        } else if supplemental.is_none() {
            format!(
                "supplemental /models response did not contain model `{}`",
                model.id
            )
        } else {
            format!("selected router /models payload does not expose {field}")
        }
    };

    let residency = model
        .resident
        .or_else(|| supplemental.and_then(|value| value.resident))
        .map(|value| {
            EvidenceValue::observed(
                value,
                "router /models payload explicitly reported residency",
            )
        })
        .unwrap_or_else(|| EvidenceValue::unavailable(unavailable_reason("residency")));

    let active_requests = supplemental
        .and_then(|value| value.active_requests)
        .map(|value| {
            EvidenceValue::observed(
                value,
                "router /models payload explicitly reported active-request count",
            )
        })
        .unwrap_or_else(|| EvidenceValue::unavailable(unavailable_reason("active-request count")));

    let last_used_ms = supplemental
        .and_then(|value| value.last_used_ms)
        .map(|value| {
            EvidenceValue::observed(
                value,
                "router /models payload explicitly reported router-relative last-use value",
            )
        })
        .unwrap_or_else(|| EvidenceValue::unavailable(unavailable_reason("last-use/LRU value")));

    let lru_rank = supplemental
        .and_then(|value| value.lru_rank)
        .map(|value| {
            EvidenceValue::observed(
                value,
                "router /models payload explicitly reported LRU/eviction rank",
            )
        })
        .unwrap_or_else(|| EvidenceValue::unavailable(unavailable_reason("LRU/eviction rank")));

    let evictable = supplemental
        .and_then(|value| value.evictable)
        .map(|value| {
            EvidenceValue::observed(
                value,
                "router /models payload explicitly reported eviction eligibility",
            )
        })
        .unwrap_or_else(|| EvidenceValue::unavailable(unavailable_reason("eviction eligibility")));

    RouterModelObservability {
        model,
        residency,
        active_requests,
        last_used_ms,
        lru_rank,
        evictable,
    }
}

fn fetch_supplemental_model_evidence(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<BTreeMap<String, SupplementalModelEvidence>, RouterObservabilityError> {
    let body = http_get_json(endpoint, "/models", timeout)?;
    parse_supplemental_model_evidence(&body)
}

fn parse_supplemental_model_evidence(
    body: &str,
) -> Result<BTreeMap<String, SupplementalModelEvidence>, RouterObservabilityError> {
    let value: Value = serde_json::from_str(body).map_err(|error| {
        protocol_error(
            "/models",
            format!("response was not valid JSON: {error}"),
            body,
        )
    })?;
    let entries = match &value {
        Value::Array(entries) => entries,
        Value::Object(object) => object
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                protocol_error(
                    "/models",
                    "expected array response or object field `data` containing an array",
                    body,
                )
            })?,
        _ => {
            return Err(protocol_error(
                "/models",
                "expected array response or object response",
                body,
            ));
        }
    };

    let mut parsed = BTreeMap::new();
    for entry in entries {
        let object = entry
            .as_object()
            .ok_or_else(|| protocol_error("/models", "model entry was not an object", body))?;
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                protocol_error(
                    "/models",
                    "model entry is missing non-empty string field `id`",
                    body,
                )
            })?;
        if parsed.contains_key(id) {
            return Err(protocol_error(
                "/models",
                format!("duplicate model id `{id}` in observability payload"),
                body,
            ));
        }

        let resident = first_bool(object, &["resident", "is_resident"]);
        let active_requests =
            first_u64(object, &["active_requests", "req_count", "requests_active"]);
        let last_used_ms = first_i64(object, &["last_used_ms", "last_used"]);
        let lru_rank = first_u64(object, &["lru_rank", "eviction_rank"]);
        let evictable = first_bool(object, &["evictable", "can_evict"]);

        parsed.insert(
            id.to_string(),
            SupplementalModelEvidence {
                resident,
                active_requests,
                last_used_ms,
                lru_rank,
                evictable,
            },
        );
    }
    Ok(parsed)
}

fn first_bool(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_bool))
}

fn first_u64(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_u64))
}

fn first_i64(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        })
    })
}

fn http_get_json(
    endpoint: &ServerEndpoint,
    path: &str,
    timeout: Duration,
) -> Result<String, RouterObservabilityError> {
    let mut stream = connect(endpoint, timeout, path)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| transport_error(path, error))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| transport_error(path, error))?;

    let mut request = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n",
        endpoint.authority()
    );
    if let Some(api_key) = endpoint.api_key.as_ref() {
        request.push_str("Authorization: Bearer ");
        request.push_str(api_key);
        request.push_str("\r\n");
    }
    request.push_str("Content-Length: 0\r\n\r\n");

    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|error| transport_error(path, error))?;

    let mut bytes = Vec::new();
    stream
        .take(MAX_OBSERVABILITY_RESPONSE_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| transport_error(path, error))?;
    parse_http_json_response(path, &bytes)
}

fn connect(
    endpoint: &ServerEndpoint,
    timeout: Duration,
    path: &str,
) -> Result<TcpStream, RouterObservabilityError> {
    if endpoint.port == 0 {
        return Err(RouterObservabilityError::HostResolution {
            host: endpoint.host.clone(),
            message: "port must be in 1..=65535".into(),
        });
    }
    let addresses: Vec<_> = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| RouterObservabilityError::HostResolution {
            host: endpoint.host.clone(),
            message: error.to_string(),
        })?
        .collect();
    if addresses.is_empty() {
        return Err(RouterObservabilityError::HostResolution {
            host: endpoint.host.clone(),
            message: "host resolved to no addresses".into(),
        });
    }
    if !endpoint.allow_non_loopback && addresses.iter().any(|address| !address.ip().is_loopback()) {
        return Err(RouterObservabilityError::HostResolution {
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
    Err(RouterObservabilityError::Transport {
        path: path.into(),
        message: last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "no resolved address accepted the connection".into()),
    })
}

fn parse_http_json_response(path: &str, bytes: &[u8]) -> Result<String, RouterObservabilityError> {
    let response = String::from_utf8_lossy(bytes);
    let status_line =
        response
            .lines()
            .next()
            .ok_or_else(|| RouterObservabilityError::Transport {
                path: path.into(),
                message: "empty HTTP response".into(),
            })?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| RouterObservabilityError::Transport {
            path: path.into(),
            message: format!("invalid HTTP status line: {status_line}"),
        })?;
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, value)| value)
        .unwrap_or("")
        .to_string();

    match status_code {
        200..=299 => Ok(body),
        401 | 403 => Err(RouterObservabilityError::AuthenticationRejected {
            path: path.into(),
            status_code,
        }),
        _ => Err(RouterObservabilityError::HttpFailure {
            path: path.into(),
            status_code,
            body_excerpt: excerpt(&body),
        }),
    }
}

fn transport_error(path: &str, error: std::io::Error) -> RouterObservabilityError {
    RouterObservabilityError::Transport {
        path: path.into(),
        message: error.to_string(),
    }
}

fn protocol_error(path: &str, message: impl Into<String>, body: &str) -> RouterObservabilityError {
    RouterObservabilityError::ProtocolDrift {
        path: path.into(),
        message: message.into(),
        body_excerpt: excerpt(body),
    }
}

fn excerpt(value: &str) -> String {
    value.chars().take(MAX_OBSERVABILITY_ERROR_CHARS).collect()
}

const ROUTER_OBSERVABILITY_CSS: &str = r#"
.ro-page{min-height:100vh;padding:30px 34px 92px;color:#f6eaff;background:radial-gradient(circle at 78% 9%,rgba(255,0,190,.13),transparent 34%),radial-gradient(circle at 8% 80%,rgba(0,255,255,.08),transparent 36%),#07000e;font-family:"Cascadia Mono","Cascadia Code",Consolas,monospace;box-sizing:border-box}.ro-page *{box-sizing:border-box}.ro-header{display:flex;justify-content:space-between;gap:24px;align-items:flex-start;padding-bottom:18px;border-bottom:1px solid rgba(0,255,255,.42)}.ro-kicker{color:#00ffff;font-size:9px;font-weight:900;letter-spacing:.15em}.ro-header h1{margin:7px 0 8px;font-size:clamp(26px,3vw,40px)}.ro-header p,.ro-muted{margin:0;color:#a996bb;font-size:10px;line-height:1.65}.ro-fresh{text-align:right;min-width:190px}.ro-fresh strong{display:block;margin-top:5px;font-size:18px}.ro-badge{display:inline-flex;align-items:center;min-height:22px;padding:0 7px;border:1px solid rgba(0,255,255,.45);color:#76ffe6;font-size:8px;font-weight:900;letter-spacing:.07em;text-transform:uppercase}.ro-badge.warn{border-color:#ffd36b;color:#ffd36b}.ro-badge.error{border-color:#ff3d7f;color:#ff7ba9}.ro-panel{margin-top:14px;min-width:0;border:1px solid rgba(0,255,255,.32);background:linear-gradient(180deg,rgba(29,5,47,.83),rgba(7,0,15,.92))}.ro-panel-head{display:flex;justify-content:space-between;align-items:center;gap:12px;padding:12px 14px;border-bottom:1px solid rgba(0,255,255,.25)}.ro-panel-head h2{margin:4px 0 0;font-size:16px}.ro-panel-body{padding:13px;min-width:0}.ro-fields{display:grid;grid-template-columns:minmax(0,1fr) 150px;gap:9px}.ro-field.wide{grid-column:1/-1}.ro-field label{display:block;margin-bottom:5px;color:#9b80a9;font-size:8px;letter-spacing:.08em;text-transform:uppercase}.ro-input{width:100%;min-height:34px;padding:7px 9px;border:1px solid rgba(0,255,255,.3);border-radius:0;background:#030008;color:#f6eaff;font:inherit;font-size:10px}.ro-input:focus-visible,.ro-button:focus-visible{outline:2px solid #ff00ff;outline-offset:2px}.ro-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:11px}.ro-button{min-height:34px;padding:0 12px;border:1px solid #00dbe7;border-radius:0;background:transparent;color:#00f5ff;font:inherit;font-size:8px;font-weight:900;letter-spacing:.08em;text-transform:uppercase;cursor:pointer}.ro-button:hover:not(:disabled),.ro-button.primary{background:#00ffff;color:#050009}.ro-button.magenta{border-color:#ff00d4;color:#ff55e7}.ro-button:disabled{opacity:.34;cursor:not-allowed}.ro-notice{margin-top:12px;padding:9px 11px;border:1px solid rgba(117,255,226,.48);background:rgba(0,20,18,.6);color:#baffed;font-size:9px;line-height:1.55;overflow-wrap:anywhere}.ro-notice.error{border-color:rgba(255,50,110,.58);background:rgba(40,0,18,.58);color:#ff91b5}.ro-runtime{margin-top:10px;padding:9px;border-left:2px solid #00ffff;background:rgba(0,0,0,.34);color:#cbb9d7;font-size:9px;line-height:1.55;overflow-wrap:anywhere;word-break:break-word}.ro-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px;margin-top:12px}.ro-card{min-width:0;border:1px solid rgba(106,74,126,.55);background:rgba(0,0,0,.28)}.ro-card-head{padding:10px 11px;border-bottom:1px solid rgba(106,74,126,.38)}.ro-card-head strong{display:block;font-size:12px;overflow-wrap:anywhere}.ro-targets{margin-top:5px;color:#a996bb;font-size:8px;overflow-wrap:anywhere}.ro-stats{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:1px;background:rgba(106,74,126,.22)}.ro-stat{min-width:0;padding:9px 10px;background:#090011}.ro-stat span{display:block;color:#887597;font-size:7px;text-transform:uppercase;letter-spacing:.05em}.ro-stat strong{display:block;margin-top:5px;font-size:10px;overflow-wrap:anywhere}.ro-stat small{display:block;margin-top:4px;color:#806d8d;font-size:7px;line-height:1.4;overflow-wrap:anywhere}.ro-state-live{color:#76ffe6}.ro-state-warn{color:#ffd36b}.ro-state-error{color:#ff7ba9}.ro-empty{padding:44px 14px;text-align:center;color:#8e789b;font-size:9px}.ro-detail{margin-top:10px;padding:9px;border-left:2px solid #785b91;background:rgba(0,0,0,.24);color:#a996bb;font-size:8px;line-height:1.55;overflow-wrap:anywhere}@media(max-width:980px){.ro-page{padding:22px 22px 92px}.ro-header{flex-direction:column}.ro-fresh{text-align:left;min-width:0}.ro-grid{grid-template-columns:1fr}}@media(max-width:620px){.ro-fields{grid-template-columns:1fr}.ro-field.wide{grid-column:auto}.ro-stats{grid-template-columns:1fr}}@media(prefers-reduced-motion:reduce){.ro-page *,.ro-page *::before,.ro-page *::after{transition:none!important;animation:none!important}}
"#;

type RouterUiSignal = Signal<RouterUiState, SyncStorage>;

#[derive(Debug, Clone)]
struct RouterUiState {
    paths: Option<AppPaths>,
    installation: Option<LlamaInstallation>,
    host: String,
    port: String,
    api_key: String,
    allow_non_loopback: bool,
    tracker: RouterObservabilityTracker,
    notice: Option<(bool, String)>,
}

impl RouterUiState {
    fn load() -> Self {
        let mut notice = None;
        let paths = match AppPaths::detect() {
            Ok(paths) => Some(paths),
            Err(error) => {
                notice = Some((
                    false,
                    format!("Could not resolve application paths: {error}"),
                ));
                None
            }
        };
        let installation = paths.as_ref().and_then(|paths| {
            match Database::open(paths.database.clone()).and_then(|db| db.latest_installation()) {
                Ok(installation) => installation,
                Err(error) => {
                    notice = Some((
                        false,
                        format!("Could not reload persisted runtime evidence: {error}"),
                    ));
                    None
                }
            }
        });

        Self {
            paths,
            installation,
            host: "127.0.0.1".into(),
            port: "8080".into(),
            api_key: String::new(),
            allow_non_loopback: false,
            tracker: RouterObservabilityTracker::default(),
            notice,
        }
    }
}

fn endpoint_from_ui(state: &RouterUiState) -> Result<ServerEndpoint, String> {
    let host = state.host.trim();
    if host.is_empty() {
        return Err("Host cannot be empty.".into());
    }
    let port = state
        .port
        .trim()
        .parse::<u16>()
        .map_err(|_| "Port must be an integer in 1..=65535.".to_string())?;
    if port == 0 {
        return Err("Port must be in 1..=65535.".into());
    }
    let api_key = state.api_key.trim();
    Ok(ServerEndpoint {
        host: host.into(),
        port,
        api_key: (!api_key.is_empty()).then(|| api_key.to_string()),
        allow_non_loopback: state.allow_non_loopback,
    })
}

fn refresh_persisted_runtime(mut state: RouterUiSignal) {
    let snapshot = state.read().clone();
    let Some(paths) = snapshot.paths else {
        state.write().notice = Some((false, "Application storage paths are unavailable.".into()));
        return;
    };

    thread::spawn(move || {
        let result = Database::open(paths.database.clone()).and_then(|db| db.latest_installation());
        let mut current = state.write();
        match result {
            Ok(installation) => {
                current.installation = installation;
                current.notice = Some((
                    true,
                    "Reloaded persisted llama.cpp runtime evidence.".into(),
                ));
            }
            Err(error) => current.notice = Some((false, error.to_string())),
        }
    });
}

fn refresh_router(mut state: RouterUiSignal) {
    let snapshot = state.read().clone();
    let Some(installation) = snapshot.installation else {
        state.write().notice = Some((
            false,
            "No persisted llama.cpp installation is selected. Select one in CORE LAB first.".into(),
        ));
        return;
    };
    if installation.server.is_none() {
        state.write().notice = Some((
            false,
            "The selected llama.cpp installation does not contain llama-server.".into(),
        ));
        return;
    }
    let endpoint = match endpoint_from_ui(&snapshot) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            let mut current = state.write();
            current.tracker.reconcile(Err(error.clone()));
            current.notice = Some((false, error));
            return;
        }
    };
    let paths = snapshot.paths;

    {
        let mut current = state.write();
        current.tracker.begin_refresh();
        current.notice = None;
    }

    thread::spawn(move || {
        let result = (|| -> Result<RouterObservabilitySnapshot, String> {
            let store = if let Some(paths) = paths {
                Database::open(paths.database.clone()).map_err(|error| error.to_string())?;
                Some(ModelStore::open(paths.database).map_err(|error| error.to_string())?)
            } else {
                None
            };
            discover_router_observability(
                &installation,
                &endpoint,
                store.as_ref(),
                Duration::from_secs(4),
            )
            .map_err(|error| error.to_string())
        })();

        let mut current = state.write();
        let succeeded = result.is_ok();
        let error = result.as_ref().err().cloned();
        current.tracker.reconcile(result);
        if succeeded {
            current.notice = Some((
                true,
                "Router observability snapshot reconciled from live evidence.".into(),
            ));
        } else if let Some(error) = error {
            current.notice = Some((false, error));
        }
    });
}

fn freshness_class(freshness: RouterSnapshotFreshness) -> &'static str {
    match freshness {
        RouterSnapshotFreshness::Live => "ro-badge",
        RouterSnapshotFreshness::Loading | RouterSnapshotFreshness::Stale => "ro-badge warn",
        RouterSnapshotFreshness::Failed => "ro-badge error",
        RouterSnapshotFreshness::Empty => "ro-badge warn",
    }
}

fn phase_label(phase: &RouterModelPhase) -> String {
    match phase {
        RouterModelPhase::Unloaded => "UNLOADED".into(),
        RouterModelPhase::Downloading => "DOWNLOADING".into(),
        RouterModelPhase::Loading => "LOADING".into(),
        RouterModelPhase::Loaded => "LOADED".into(),
        RouterModelPhase::Sleeping => "SLEEPING".into(),
        RouterModelPhase::Unknown(value) => format!("UNKNOWN · {value}"),
    }
}

fn evidence_bool(value: &EvidenceValue<bool>) -> String {
    match value.value {
        Some(true) => "YES · OBSERVED".into(),
        Some(false) => "NO · OBSERVED".into(),
        None => "UNAVAILABLE".into(),
    }
}

fn evidence_u64(value: &EvidenceValue<u64>) -> String {
    value
        .value
        .map(|value| format!("{value} · OBSERVED"))
        .unwrap_or_else(|| "UNAVAILABLE".into())
}

fn evidence_i64(value: &EvidenceValue<i64>) -> String {
    value
        .value
        .map(|value| format!("{value} · OBSERVED"))
        .unwrap_or_else(|| "UNAVAILABLE".into())
}

fn eviction_label(value: &RouterModelObservability) -> (String, &'static str) {
    match value.eviction_safety() {
        RouterEvictionSafety::SafeObserved => ("SAFE · OBSERVED".into(), "ro-state-live"),
        RouterEvictionSafety::Busy { active_requests } => (
            format!("BLOCKED · {active_requests} ACTIVE"),
            "ro-state-error",
        ),
        RouterEvictionSafety::RouterDenied => ("ROUTER DENIED".into(), "ro-state-error"),
        RouterEvictionSafety::Unknown { .. } => ("UNKNOWN".into(), "ro-state-warn"),
        RouterEvictionSafety::NotApplicable { .. } => ("N/A".into(), "ro-state-warn"),
    }
}

fn eviction_reason(value: &RouterModelObservability) -> String {
    match value.eviction_safety() {
        RouterEvictionSafety::SafeObserved => {
            "router reports zero active requests and explicit eviction eligibility".into()
        }
        RouterEvictionSafety::Busy { active_requests } => format!(
            "do not evict: router reports {active_requests} active request(s) for this model"
        ),
        RouterEvictionSafety::RouterDenied => {
            "router explicitly reports that this model is not evictable".into()
        }
        RouterEvictionSafety::Unknown { reason } => reason,
        RouterEvictionSafety::NotApplicable { reason } => reason,
    }
}

#[allow(non_snake_case)]
pub fn RouterObservabilityView() -> Element {
    let mut state = use_signal_sync(RouterUiState::load);
    let snapshot = state.read().clone();
    let freshness = snapshot.tracker.freshness();
    let current = snapshot.tracker.current.as_ref();
    let loading = freshness == RouterSnapshotFreshness::Loading;

    rsx! {
        style { dangerous_inner_html: ROUTER_OBSERVABILITY_CSS }
        main { class: "ro-page",
            header { class: "ro-header",
                div {
                    div { class: "ro-kicker", "> LLAMAWAVE / ROUTER OBSERVATORY" }
                    h1 { "ROUTER STATE, WITHOUT GUESSING" }
                    p { "Live registry status, aliases, residency, LRU/eviction and active-request evidence. Missing router fields stay unavailable; disconnected snapshots stay visibly stale." }
                }
                div { class: "ro-fresh",
                    div { class: "ro-kicker", "SNAPSHOT" }
                    strong { "{freshness:?}" }
                    span { class: freshness_class(freshness), "{freshness:?}" }
                }
            }

            if let Some((success, message)) = snapshot.notice.as_ref() {
                div { class: if *success { "ro-notice" } else { "ro-notice error" }, "{message}" }
            }
            if let Some(error) = snapshot.tracker.last_error.as_ref() {
                if snapshot.tracker.current.is_some() {
                    div { class: "ro-notice error", "STALE SNAPSHOT · latest refresh failed: {error}" }
                }
            }

            section { class: "ro-panel",
                div { class: "ro-panel-head",
                    div {
                        div { class: "ro-kicker", "LIVE TARGET" }
                        h2 { "ROUTER ENDPOINT" }
                    }
                    button {
                        class: "ro-button",
                        disabled: loading,
                        onclick: move |_| refresh_persisted_runtime(state),
                        "REFRESH RUNTIME"
                    }
                }
                div { class: "ro-panel-body",
                    div { class: "ro-fields",
                        div { class: "ro-field",
                            label { "HOST" }
                            input {
                                class: "ro-input",
                                value: "{snapshot.host}",
                                disabled: loading,
                                oninput: move |event| state.write().host = event.value(),
                            }
                        }
                        div { class: "ro-field",
                            label { "PORT" }
                            input {
                                class: "ro-input",
                                value: "{snapshot.port}",
                                disabled: loading,
                                oninput: move |event| state.write().port = event.value(),
                            }
                        }
                        div { class: "ro-field wide",
                            label { "API KEY · OPTIONAL · NEVER DISPLAYED" }
                            input {
                                class: "ro-input",
                                r#type: "password",
                                value: "{snapshot.api_key}",
                                disabled: loading,
                                oninput: move |event| state.write().api_key = event.value(),
                            }
                        }
                    }
                    div { class: "ro-actions",
                        button {
                            class: if snapshot.allow_non_loopback { "ro-button magenta" } else { "ro-button" },
                            disabled: loading,
                            onclick: move |_| {
                                let enabled = state.read().allow_non_loopback;
                                state.write().allow_non_loopback = !enabled;
                            },
                            if snapshot.allow_non_loopback { "LAN OPT-IN ON" } else { "LAN OPT-IN OFF" }
                        }
                        button {
                            class: "ro-button primary",
                            disabled: loading || snapshot.installation.is_none(),
                            onclick: move |_| refresh_router(state),
                            if loading { "REFRESHING..." } else { "REFRESH LIVE ROUTER" }
                        }
                    }
                    div { class: "ro-runtime",
                        strong { "SELECTED RUNTIME\n" }
                        if let Some(installation) = snapshot.installation.as_ref() {
                            if let Some(server) = installation.server.as_ref() {
                                "{server.path.display()}\nSHA-256 {server.sha256}"
                            } else {
                                "selected installation has no llama-server"
                            }
                        } else {
                            "no persisted runtime selected"
                        }
                    }
                }
            }

            section { class: "ro-panel",
                div { class: "ro-panel-head",
                    div {
                        div { class: "ro-kicker", "CANONICAL REGISTRY" }
                        h2 { "MODEL ROUTING + RESIDENCY" }
                    }
                    if let Some(current) = current {
                        span { class: "ro-muted", {format!("observed {} · {} models", current.observed_at_unix_ms, current.models.len())} }
                    }
                }
                div { class: "ro-panel-body",
                    if let Some(current) = current {
                        if current.registry.role != RouterRole::Router {
                            div { class: "ro-empty", "Selected endpoint reports single-model server role. Router observability is unavailable by contract." }
                        } else if current.models.is_empty() {
                            div { class: "ro-empty", "Router returned an empty model registry." }
                        } else {
                            div { class: "ro-grid",
                                for observed in current.models.iter() {
                                    {
                                        let phase = phase_label(&observed.model.status.phase);
                                        let targets = observed.model.routing_targets.join(", ");
                                        let residency = evidence_bool(&observed.residency);
                                        let active = evidence_u64(&observed.active_requests);
                                        let last_used = evidence_i64(&observed.last_used_ms);
                                        let lru = evidence_u64(&observed.lru_rank);
                                        let evictable = evidence_bool(&observed.evictable);
                                        let (eviction, eviction_class) = eviction_label(observed);
                                        let eviction_reason = eviction_reason(observed);
                                        rsx! {
                                            article { class: "ro-card",
                                                div { class: "ro-card-head",
                                                    strong { "{observed.model.id}" }
                                                    div { class: "ro-targets", "ROUTING TARGETS · {targets}" }
                                                }
                                                div { class: "ro-stats",
                                                    div { class: "ro-stat",
                                                        span { "ROUTER STATUS" }
                                                        strong { class: if matches!(&observed.model.status.phase, RouterModelPhase::Loaded | RouterModelPhase::Sleeping) { "ro-state-live" } else { "ro-state-warn" }, "{phase}" }
                                                        small { "direct /models status evidence" }
                                                    }
                                                    div { class: "ro-stat",
                                                        span { "RESIDENCY" }
                                                        strong { class: if observed.residency.is_observed() { "ro-state-live" } else { "ro-state-warn" }, "{residency}" }
                                                        small { "{observed.residency.reason}" }
                                                    }
                                                    div { class: "ro-stat",
                                                        span { "ACTIVE REQUESTS" }
                                                        strong { class: if observed.active_requests.is_observed() { "ro-state-live" } else { "ro-state-warn" }, "{active}" }
                                                        small { "{observed.active_requests.reason}" }
                                                    }
                                                    div { class: "ro-stat",
                                                        span { "LAST USED · ROUTER-RELATIVE" }
                                                        strong { class: if observed.last_used_ms.is_observed() { "ro-state-live" } else { "ro-state-warn" }, "{last_used}" }
                                                        small { "{observed.last_used_ms.reason}" }
                                                    }
                                                    div { class: "ro-stat",
                                                        span { "LRU / EVICTION RANK" }
                                                        strong { class: if observed.lru_rank.is_observed() { "ro-state-live" } else { "ro-state-warn" }, "{lru}" }
                                                        small { "{observed.lru_rank.reason}" }
                                                    }
                                                    div { class: "ro-stat",
                                                        span { "EVICTABLE" }
                                                        strong { class: if observed.evictable.is_observed() { "ro-state-live" } else { "ro-state-warn" }, "{evictable}" }
                                                        small { "{observed.evictable.reason}" }
                                                    }
                                                    div { class: "ro-stat",
                                                        span { "EVICTION SAFETY" }
                                                        strong { class: eviction_class, "{eviction}" }
                                                        small { "{eviction_reason}" }
                                                    }
                                                    div { class: "ro-stat",
                                                        span { "M2 IDENTITY" }
                                                        strong { "{observed.model.library_link.kind:?}" }
                                                        small { "{observed.model.library_link.reason}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(error) = current.supplemental_error.as_ref() {
                            div { class: "ro-detail", "SUPPLEMENTAL OBSERVABILITY ERROR · {error}. Canonical registry state above remains live; unsupported fields remain unavailable." }
                        }
                    } else if loading {
                        div { class: "ro-empty", "Loading live router evidence. No success state is shown until the full snapshot reconciles." }
                    } else {
                        div { class: "ro-empty", "No router snapshot yet. Refresh a live router endpoint to populate evidence." }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::router::{
        RouterEndpointCapabilities, RouterFeatureEvidence, RouterFeatureState, RouterLibraryLink,
        RouterLibraryLinkKind, RouterModelStatus, RouterStaticCapabilities,
    };

    fn model(phase: RouterModelPhase) -> RouterModel {
        RouterModel {
            id: "alpha".into(),
            routing_targets: vec!["alpha".into(), "alias-a".into()],
            path: None,
            sha256: None,
            status: RouterModelStatus {
                phase,
                failed: false,
                exit_code: None,
                args: Vec::new(),
                progress: None,
            },
            resident: None,
            input_modalities: vec!["text".into()],
            output_modalities: vec!["text".into()],
            library_link: RouterLibraryLink {
                kind: RouterLibraryLinkKind::Unmatched,
                model_id: None,
                candidates: Vec::new(),
                reason: "fixture".into(),
            },
        }
    }

    fn registry() -> RouterRegistry {
        let unsupported = RouterFeatureEvidence {
            state: RouterFeatureState::Unknown,
            reason: "fixture".into(),
        };
        RouterRegistry {
            endpoint: "127.0.0.1:8080".into(),
            role: RouterRole::Router,
            static_capabilities: RouterStaticCapabilities {
                server_sha256: None,
                server_version: None,
                router_cli_observed: true,
                models_dir: true,
                models_preset: false,
                models_max: true,
                models_autoload: true,
                observed_options: BTreeSet::new(),
            },
            endpoints: RouterEndpointCapabilities {
                props: unsupported.clone(),
                list_models: unsupported.clone(),
                reload_models: unsupported.clone(),
                load_model: unsupported.clone(),
                unload_model: unsupported.clone(),
                model_events: unsupported,
            },
            models: vec![model(RouterModelPhase::Loaded)],
            observed_at_unix_ms: 1,
        }
    }

    #[test]
    fn absent_router_fields_stay_unavailable() {
        let observed = observability_for_model(
            model(RouterModelPhase::Loaded),
            Some(&SupplementalModelEvidence::default()),
            None,
        );
        assert_eq!(
            observed.residency.availability,
            EvidenceAvailability::Unavailable
        );
        assert_eq!(
            observed.active_requests.availability,
            EvidenceAvailability::Unavailable
        );
        assert_eq!(
            observed.lru_rank.availability,
            EvidenceAvailability::Unavailable
        );
        assert!(matches!(
            observed.eviction_safety(),
            RouterEvictionSafety::Unknown { .. }
        ));
    }

    #[test]
    fn busy_model_is_never_presented_as_safe_to_evict() {
        let observed = observability_for_model(
            model(RouterModelPhase::Loaded),
            Some(&SupplementalModelEvidence {
                active_requests: Some(3),
                evictable: Some(true),
                ..SupplementalModelEvidence::default()
            }),
            None,
        );
        assert_eq!(
            observed.eviction_safety(),
            RouterEvictionSafety::Busy { active_requests: 3 }
        );
    }

    #[test]
    fn lru_values_are_not_derived_when_only_last_use_is_observed() {
        let observed = observability_for_model(
            model(RouterModelPhase::Loaded),
            Some(&SupplementalModelEvidence {
                active_requests: Some(0),
                last_used_ms: Some(1234),
                evictable: Some(true),
                ..SupplementalModelEvidence::default()
            }),
            None,
        );
        assert_eq!(observed.last_used_ms.value, Some(1234));
        assert_eq!(observed.lru_rank.value, None);
        assert_eq!(
            observed.eviction_safety(),
            RouterEvictionSafety::SafeObserved
        );
    }

    #[test]
    fn tracker_marks_retained_snapshot_stale_on_disconnect_and_recovers() {
        let mut tracker = RouterObservabilityTracker::default();
        tracker.reconcile(Ok(RouterObservabilitySnapshot {
            registry: registry(),
            models: Vec::new(),
            supplemental_error: None,
            observed_at_unix_ms: 1,
        }));
        assert_eq!(tracker.freshness(), RouterSnapshotFreshness::Live);

        tracker.begin_refresh();
        assert_eq!(tracker.freshness(), RouterSnapshotFreshness::Loading);
        tracker.reconcile(Err("disconnect".into()));
        assert_eq!(tracker.freshness(), RouterSnapshotFreshness::Stale);
        assert!(tracker.current.is_some());
    }

    #[test]
    fn supplemental_parser_retains_explicit_active_lru_and_eviction_fields() {
        let parsed = parse_supplemental_model_evidence(
            r#"{"data":[{"id":"alpha","resident":true,"active_requests":2,"last_used_ms":88,"lru_rank":4,"evictable":false}]}"#,
        )
        .unwrap();
        let alpha = parsed.get("alpha").unwrap();
        assert_eq!(alpha.resident, Some(true));
        assert_eq!(alpha.active_requests, Some(2));
        assert_eq!(alpha.last_used_ms, Some(88));
        assert_eq!(alpha.lru_rank, Some(4));
        assert_eq!(alpha.evictable, Some(false));
    }
}
