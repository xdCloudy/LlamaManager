use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::hardware_telemetry::TelemetryState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceMetricUnit {
    TokensPerSecond,
    Milliseconds,
    Tokens,
    Ratio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceMetricSource {
    pub provider: String,
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferenceMetric<T> {
    pub state: TelemetryState<T>,
    pub unit: InferenceMetricUnit,
    pub source: InferenceMetricSource,
    pub observed_at_unix_ms: u64,
}

impl<T> InferenceMetric<T> {
    pub fn live_value(&self) -> Option<&T> {
        match &self.state {
            TelemetryState::Live { value } => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceRequestIdentity {
    pub request_id: String,
    pub endpoint: String,
    pub server_pid: Option<u32>,
    pub requested_model: Option<String>,
    pub reported_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferenceRequestObservation {
    pub request_id: String,
    pub endpoint: String,
    pub server_pid: Option<u32>,
    pub requested_model: Option<String>,
    pub request_latency_ms: f64,
    pub ttft_ms: Option<f64>,
    pub observed_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InferenceConnectionState {
    Live,
    Stale {
        reason: String,
        observed_at_unix_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferenceTelemetrySnapshot {
    pub identity: InferenceRequestIdentity,
    pub connection: InferenceConnectionState,
    pub prompt_tps: InferenceMetric<f64>,
    pub decode_tps: InferenceMetric<f64>,
    pub ttft_ms: InferenceMetric<f64>,
    pub request_latency_ms: InferenceMetric<f64>,
    pub prompt_tokens: InferenceMetric<u64>,
    pub decode_tokens: InferenceMetric<u64>,
    pub cached_prompt_tokens: InferenceMetric<u64>,
    pub context_tokens: InferenceMetric<u64>,
    pub context_capacity_tokens: InferenceMetric<u64>,
    pub batch_tokens: InferenceMetric<u64>,
    pub kv_cache_tokens: InferenceMetric<u64>,
    pub speculative_generated_tokens: InferenceMetric<u64>,
    pub speculative_accepted_tokens: InferenceMetric<u64>,
    pub speculative_acceptance_rate: InferenceMetric<f64>,
    pub speculative_mean_run_length: InferenceMetric<f64>,
    pub mtp_generated_tokens: InferenceMetric<u64>,
    pub mtp_accepted_tokens: InferenceMetric<u64>,
    pub mtp_acceptance_rate: InferenceMetric<f64>,
    pub mtp_mean_run_length: InferenceMetric<f64>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum InferenceTelemetryParseError {
    #[error("inference response is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("inference response root must be a JSON object")]
    InvalidRoot,
}

#[derive(Debug, Default)]
pub struct InferenceTelemetryTracker {
    last: Option<InferenceTelemetrySnapshot>,
}

impl InferenceTelemetryTracker {
    pub fn ingest(
        &mut self,
        body: &str,
        observation: InferenceRequestObservation,
    ) -> Result<&InferenceTelemetrySnapshot, InferenceTelemetryParseError> {
        self.last = Some(parse_llama_cpp_completion(body, observation)?);
        Ok(self.last.as_ref().expect("snapshot was just stored"))
    }

    pub fn last(&self) -> Option<&InferenceTelemetrySnapshot> {
        self.last.as_ref()
    }

    pub fn mark_disconnected(
        &mut self,
        reason: impl Into<String>,
        observed_at_unix_ms: u64,
    ) -> Option<&InferenceTelemetrySnapshot> {
        let reason = reason.into();
        let snapshot = self.last.as_mut()?;
        snapshot.connection = InferenceConnectionState::Stale {
            reason: reason.clone(),
            observed_at_unix_ms,
        };
        stale_metric(&mut snapshot.prompt_tps, &reason);
        stale_metric(&mut snapshot.decode_tps, &reason);
        stale_metric(&mut snapshot.ttft_ms, &reason);
        stale_metric(&mut snapshot.request_latency_ms, &reason);
        stale_metric(&mut snapshot.prompt_tokens, &reason);
        stale_metric(&mut snapshot.decode_tokens, &reason);
        stale_metric(&mut snapshot.cached_prompt_tokens, &reason);
        stale_metric(&mut snapshot.context_tokens, &reason);
        stale_metric(&mut snapshot.context_capacity_tokens, &reason);
        stale_metric(&mut snapshot.batch_tokens, &reason);
        stale_metric(&mut snapshot.kv_cache_tokens, &reason);
        stale_metric(&mut snapshot.speculative_generated_tokens, &reason);
        stale_metric(&mut snapshot.speculative_accepted_tokens, &reason);
        stale_metric(&mut snapshot.speculative_acceptance_rate, &reason);
        stale_metric(&mut snapshot.speculative_mean_run_length, &reason);
        stale_metric(&mut snapshot.mtp_generated_tokens, &reason);
        stale_metric(&mut snapshot.mtp_accepted_tokens, &reason);
        stale_metric(&mut snapshot.mtp_acceptance_rate, &reason);
        stale_metric(&mut snapshot.mtp_mean_run_length, &reason);
        Some(snapshot)
    }
}

pub fn parse_llama_cpp_completion(
    body: &str,
    observation: InferenceRequestObservation,
) -> Result<InferenceTelemetrySnapshot, InferenceTelemetryParseError> {
    let root: Value = serde_json::from_str(body)
        .map_err(|error| InferenceTelemetryParseError::InvalidJson(error.to_string()))?;
    if !root.is_object() {
        return Err(InferenceTelemetryParseError::InvalidRoot);
    }

    let observed_at = observation.observed_at_unix_ms;
    let reported_model = root
        .get("model")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    let prompt_tps = number_metric(
        &root,
        &[["timings", "prompt_per_second"].as_slice()],
        InferenceMetricUnit::TokensPerSecond,
        "timings.prompt_per_second",
        observed_at,
    );
    let decode_tps = number_metric(
        &root,
        &[["timings", "predicted_per_second"].as_slice()],
        InferenceMetricUnit::TokensPerSecond,
        "timings.predicted_per_second",
        observed_at,
    );
    let prompt_tokens = integer_metric(
        &root,
        &[
            ["timings", "prompt_n"].as_slice(),
            ["tokens_evaluated"].as_slice(),
        ],
        InferenceMetricUnit::Tokens,
        "timings.prompt_n | tokens_evaluated",
        observed_at,
    );
    let decode_tokens = integer_metric(
        &root,
        &[
            ["timings", "predicted_n"].as_slice(),
            ["tokens_predicted"].as_slice(),
        ],
        InferenceMetricUnit::Tokens,
        "timings.predicted_n | tokens_predicted",
        observed_at,
    );
    let cached_prompt_tokens = integer_metric(
        &root,
        &[
            ["timings", "cache_n"].as_slice(),
            ["tokens_cached"].as_slice(),
        ],
        InferenceMetricUnit::Tokens,
        "timings.cache_n | tokens_cached",
        observed_at,
    );
    let context_tokens = sum_context_tokens(
        &cached_prompt_tokens,
        &prompt_tokens,
        &decode_tokens,
        observed_at,
    );
    let context_capacity_tokens = integer_metric(
        &root,
        &[
            ["n_ctx"].as_slice(),
            ["context_size"].as_slice(),
            ["generation_settings", "n_ctx"].as_slice(),
        ],
        InferenceMetricUnit::Tokens,
        "n_ctx | context_size | generation_settings.n_ctx",
        observed_at,
    );
    let batch_tokens = integer_metric(
        &root,
        &[
            ["n_batch"].as_slice(),
            ["batch_size"].as_slice(),
            ["generation_settings", "n_batch"].as_slice(),
            ["generation_settings", "batch_size"].as_slice(),
        ],
        InferenceMetricUnit::Tokens,
        "n_batch | batch_size | generation_settings.n_batch",
        observed_at,
    );
    let kv_cache_tokens = integer_metric(
        &root,
        &[
            ["kv_cache_tokens"].as_slice(),
            ["kv_cache", "tokens"].as_slice(),
        ],
        InferenceMetricUnit::Tokens,
        "kv_cache_tokens | kv_cache.tokens",
        observed_at,
    );

    let speculative_generated_tokens = integer_metric(
        &root,
        &[
            ["timings", "draft_n"].as_slice(),
            ["timings", "draft_tokens"].as_slice(),
        ],
        InferenceMetricUnit::Tokens,
        "timings.draft_n | timings.draft_tokens",
        observed_at,
    );
    let speculative_accepted_tokens = integer_metric(
        &root,
        &[
            ["timings", "draft_n_accepted"].as_slice(),
            ["timings", "draft_tokens_accepted"].as_slice(),
        ],
        InferenceMetricUnit::Tokens,
        "timings.draft_n_accepted | timings.draft_tokens_accepted",
        observed_at,
    );
    let speculative_acceptance_rate = speculative_acceptance_metric(
        &root,
        &speculative_generated_tokens,
        &speculative_accepted_tokens,
        observed_at,
    );
    let speculative_mean_run_length = number_metric(
        &root,
        &[
            ["timings", "draft_mean_len"].as_slice(),
            ["timings", "draft_mean_run"].as_slice(),
            ["timings", "mean_accepted_run"].as_slice(),
        ],
        InferenceMetricUnit::Tokens,
        "timings.draft_mean_len | timings.draft_mean_run | timings.mean_accepted_run",
        observed_at,
    );

    let mtp_mode = explicit_mtp_mode(&root);
    let mtp_generated_tokens = mtp_metric_from_speculative(
        &speculative_generated_tokens,
        &mtp_mode,
        "timings.draft_n | timings.draft_tokens",
    );
    let mtp_accepted_tokens = mtp_metric_from_speculative(
        &speculative_accepted_tokens,
        &mtp_mode,
        "timings.draft_n_accepted | timings.draft_tokens_accepted",
    );
    let mtp_acceptance_rate = mtp_metric_from_speculative(
        &speculative_acceptance_rate,
        &mtp_mode,
        "timings.draft_accept_ratio | draft_n_accepted / draft_n",
    );
    let mtp_mean_run_length = mtp_metric_from_speculative(
        &speculative_mean_run_length,
        &mtp_mode,
        "timings.draft_mean_len | timings.draft_mean_run | timings.mean_accepted_run",
    );

    Ok(InferenceTelemetrySnapshot {
        identity: InferenceRequestIdentity {
            request_id: observation.request_id,
            endpoint: observation.endpoint,
            server_pid: observation.server_pid,
            requested_model: observation.requested_model,
            reported_model,
        },
        connection: InferenceConnectionState::Live,
        prompt_tps,
        decode_tps,
        ttft_ms: observed_metric(
            observation.ttft_ms,
            InferenceMetricUnit::Milliseconds,
            "client.first_token_elapsed_ms",
            observed_at,
            "the request transport did not observe a first token",
        ),
        request_latency_ms: observed_metric(
            Some(observation.request_latency_ms),
            InferenceMetricUnit::Milliseconds,
            "client.request_elapsed_ms",
            observed_at,
            "request latency was not observed",
        ),
        prompt_tokens,
        decode_tokens,
        cached_prompt_tokens,
        context_tokens,
        context_capacity_tokens,
        batch_tokens,
        kv_cache_tokens,
        speculative_generated_tokens,
        speculative_accepted_tokens,
        speculative_acceptance_rate,
        speculative_mean_run_length,
        mtp_generated_tokens,
        mtp_accepted_tokens,
        mtp_acceptance_rate,
        mtp_mean_run_length,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MtpModeEvidence {
    Explicit,
    NotMtp(String),
    Missing,
    Invalid(String),
}

fn explicit_mtp_mode(root: &Value) -> MtpModeEvidence {
    let value = match lookup_path(root, &["generation_settings", "speculative.types"]) {
        Lookup::Missing => return MtpModeEvidence::Missing,
        Lookup::Invalid(message) => return MtpModeEvidence::Invalid(message),
        Lookup::Found(value) => value,
    };

    let modes = if let Some(mode) = value.as_str() {
        vec![mode.to_owned()]
    } else if let Some(values) = value.as_array() {
        let mut modes = Vec::with_capacity(values.len());
        for item in values {
            let Some(mode) = item.as_str() else {
                return MtpModeEvidence::Invalid(
                    "generation_settings.speculative.types array contains a non-string value"
                        .to_owned(),
                );
            };
            modes.push(mode.to_owned());
        }
        modes
    } else if value.is_null() {
        return MtpModeEvidence::Missing;
    } else {
        return MtpModeEvidence::Invalid(
            "generation_settings.speculative.types must be a string or string array".to_owned(),
        );
    };

    if modes
        .iter()
        .any(|mode| mode.to_ascii_lowercase().contains("mtp"))
    {
        MtpModeEvidence::Explicit
    } else {
        MtpModeEvidence::NotMtp(if modes.is_empty() {
            "empty speculative mode list".to_owned()
        } else {
            modes.join(",")
        })
    }
}

fn mtp_metric_from_speculative<T: Clone>(
    speculative: &InferenceMetric<T>,
    mode: &MtpModeEvidence,
    source_field: &str,
) -> InferenceMetric<T> {
    match mode {
        MtpModeEvidence::Explicit => InferenceMetric {
            state: speculative.state.clone(),
            unit: speculative.unit,
            source: InferenceMetricSource {
                provider: "llama.cpp-mtp".to_owned(),
                field: source_field.to_owned(),
            },
            observed_at_unix_ms: speculative.observed_at_unix_ms,
        },
        MtpModeEvidence::NotMtp(mode) => unavailable_metric(
            speculative.unit,
            "llama.cpp-mtp",
            source_field,
            speculative.observed_at_unix_ms,
            format!("runtime explicitly reported non-MTP speculative mode {mode:?}"),
        ),
        MtpModeEvidence::Missing => unavailable_metric(
            speculative.unit,
            "llama.cpp-mtp",
            source_field,
            speculative.observed_at_unix_ms,
            "response did not explicitly identify an MTP speculative mode".to_owned(),
        ),
        MtpModeEvidence::Invalid(message) => error_metric(
            speculative.unit,
            "llama.cpp-mtp",
            source_field,
            speculative.observed_at_unix_ms,
            message.clone(),
        ),
    }
}

fn observed_metric(
    value: Option<f64>,
    unit: InferenceMetricUnit,
    field: &str,
    observed_at_unix_ms: u64,
    unavailable_reason: &str,
) -> InferenceMetric<f64> {
    match value {
        Some(value) if value.is_finite() && value >= 0.0 => {
            live_metric(value, unit, "client-observed", field, observed_at_unix_ms)
        }
        Some(value) => error_metric(
            unit,
            "client-observed",
            field,
            observed_at_unix_ms,
            format!("observed metric must be finite and non-negative, got {value}"),
        ),
        None => unavailable_metric(
            unit,
            "client-observed",
            field,
            observed_at_unix_ms,
            unavailable_reason.to_owned(),
        ),
    }
}

fn sum_context_tokens(
    cached: &InferenceMetric<u64>,
    prompt: &InferenceMetric<u64>,
    predicted: &InferenceMetric<u64>,
    observed_at_unix_ms: u64,
) -> InferenceMetric<u64> {
    match (
        cached.live_value().copied(),
        prompt.live_value().copied(),
        predicted.live_value().copied(),
    ) {
        (Some(cache), Some(prompt), Some(predicted)) => cache
            .checked_add(prompt)
            .and_then(|value| value.checked_add(predicted))
            .map(|value| {
                live_metric(
                    value,
                    InferenceMetricUnit::Tokens,
                    "llama.cpp",
                    "timings.cache_n + timings.prompt_n + timings.predicted_n",
                    observed_at_unix_ms,
                )
            })
            .unwrap_or_else(|| {
                error_metric(
                    InferenceMetricUnit::Tokens,
                    "llama.cpp",
                    "timings.cache_n + timings.prompt_n + timings.predicted_n",
                    observed_at_unix_ms,
                    "context token count overflowed u64".to_owned(),
                )
            }),
        _ => unavailable_metric(
            InferenceMetricUnit::Tokens,
            "llama.cpp",
            "timings.cache_n + timings.prompt_n + timings.predicted_n",
            observed_at_unix_ms,
            "context usage requires cache_n, prompt_n and predicted_n from the same response"
                .to_owned(),
        ),
    }
}

fn speculative_acceptance_metric(
    root: &Value,
    generated: &InferenceMetric<u64>,
    accepted: &InferenceMetric<u64>,
    observed_at_unix_ms: u64,
) -> InferenceMetric<f64> {
    let direct = number_metric(
        root,
        &[["timings", "draft_accept_ratio"].as_slice()],
        InferenceMetricUnit::Ratio,
        "timings.draft_accept_ratio",
        observed_at_unix_ms,
    );
    if let TelemetryState::Live { value } = &direct.state {
        if *value <= 1.0 {
            return direct;
        }
        return error_metric(
            InferenceMetricUnit::Ratio,
            "llama.cpp",
            "timings.draft_accept_ratio",
            observed_at_unix_ms,
            format!("acceptance ratio must be in 0..=1, got {value}"),
        );
    }
    if !matches!(&direct.state, TelemetryState::Unavailable { .. }) {
        return direct;
    }

    match (
        generated.live_value().copied(),
        accepted.live_value().copied(),
    ) {
        (Some(0), _) => unavailable_metric(
            InferenceMetricUnit::Ratio,
            "llama.cpp",
            "timings.draft_n_accepted / timings.draft_n",
            observed_at_unix_ms,
            "no speculative draft tokens were generated for this request".to_owned(),
        ),
        (Some(generated), Some(accepted)) if accepted <= generated => live_metric(
            accepted as f64 / generated as f64,
            InferenceMetricUnit::Ratio,
            "llama.cpp",
            "timings.draft_n_accepted / timings.draft_n",
            observed_at_unix_ms,
        ),
        (Some(generated), Some(accepted)) => error_metric(
            InferenceMetricUnit::Ratio,
            "llama.cpp",
            "timings.draft_n_accepted / timings.draft_n",
            observed_at_unix_ms,
            format!(
                "accepted speculative tokens {accepted} exceed generated draft tokens {generated}"
            ),
        ),
        _ => unavailable_metric(
            InferenceMetricUnit::Ratio,
            "llama.cpp",
            "timings.draft_accept_ratio | draft_n_accepted / draft_n",
            observed_at_unix_ms,
            "response did not expose enough speculative timing evidence".to_owned(),
        ),
    }
}

fn number_metric(
    root: &Value,
    paths: &[&[&str]],
    unit: InferenceMetricUnit,
    field: &str,
    observed_at_unix_ms: u64,
) -> InferenceMetric<f64> {
    for path in paths {
        match lookup_path(root, path) {
            Lookup::Missing => continue,
            Lookup::Invalid(message) => {
                return error_metric(unit, "llama.cpp", field, observed_at_unix_ms, message);
            }
            Lookup::Found(value) if value.is_null() => continue,
            Lookup::Found(value) => {
                return match value.as_f64() {
                    Some(value) if value.is_finite() && value >= 0.0 => live_metric(
                        value,
                        unit,
                        "llama.cpp",
                        &path.join("."),
                        observed_at_unix_ms,
                    ),
                    Some(value) => error_metric(
                        unit,
                        "llama.cpp",
                        &path.join("."),
                        observed_at_unix_ms,
                        format!("metric must be finite and non-negative, got {value}"),
                    ),
                    None => error_metric(
                        unit,
                        "llama.cpp",
                        &path.join("."),
                        observed_at_unix_ms,
                        format!("metric {} is not numeric", path.join(".")),
                    ),
                };
            }
        }
    }
    unavailable_metric(
        unit,
        "llama.cpp",
        field,
        observed_at_unix_ms,
        format!("response did not expose {field}"),
    )
}

fn integer_metric(
    root: &Value,
    paths: &[&[&str]],
    unit: InferenceMetricUnit,
    field: &str,
    observed_at_unix_ms: u64,
) -> InferenceMetric<u64> {
    for path in paths {
        match lookup_path(root, path) {
            Lookup::Missing => continue,
            Lookup::Invalid(message) => {
                return error_metric(unit, "llama.cpp", field, observed_at_unix_ms, message);
            }
            Lookup::Found(value) if value.is_null() => continue,
            Lookup::Found(value) => {
                return match value.as_u64() {
                    Some(value) => live_metric(
                        value,
                        unit,
                        "llama.cpp",
                        &path.join("."),
                        observed_at_unix_ms,
                    ),
                    None => error_metric(
                        unit,
                        "llama.cpp",
                        &path.join("."),
                        observed_at_unix_ms,
                        format!(
                            "metric {} must be a non-negative integer, got {value}",
                            path.join(".")
                        ),
                    ),
                };
            }
        }
    }
    unavailable_metric(
        unit,
        "llama.cpp",
        field,
        observed_at_unix_ms,
        format!("response did not expose {field}"),
    )
}

enum Lookup<'a> {
    Missing,
    Invalid(String),
    Found(&'a Value),
}

fn lookup_path<'a>(root: &'a Value, path: &[&str]) -> Lookup<'a> {
    let mut current = root;
    for (index, segment) in path.iter().enumerate() {
        let Some(object) = current.as_object() else {
            return Lookup::Invalid(format!(
                "{} must be an object before reading {}",
                path[..index].join("."),
                path.join(".")
            ));
        };
        let Some(next) = object.get(*segment) else {
            return Lookup::Missing;
        };
        current = next;
    }
    Lookup::Found(current)
}

fn live_metric<T>(
    value: T,
    unit: InferenceMetricUnit,
    provider: &str,
    field: &str,
    observed_at_unix_ms: u64,
) -> InferenceMetric<T> {
    InferenceMetric {
        state: TelemetryState::Live { value },
        unit,
        source: InferenceMetricSource {
            provider: provider.to_owned(),
            field: field.to_owned(),
        },
        observed_at_unix_ms,
    }
}

fn unavailable_metric<T>(
    unit: InferenceMetricUnit,
    provider: &str,
    field: &str,
    observed_at_unix_ms: u64,
    reason: String,
) -> InferenceMetric<T> {
    InferenceMetric {
        state: TelemetryState::Unavailable { reason },
        unit,
        source: InferenceMetricSource {
            provider: provider.to_owned(),
            field: field.to_owned(),
        },
        observed_at_unix_ms,
    }
}

fn error_metric<T>(
    unit: InferenceMetricUnit,
    provider: &str,
    field: &str,
    observed_at_unix_ms: u64,
    message: String,
) -> InferenceMetric<T> {
    InferenceMetric {
        state: TelemetryState::Error { message },
        unit,
        source: InferenceMetricSource {
            provider: provider.to_owned(),
            field: field.to_owned(),
        },
        observed_at_unix_ms,
    }
}

fn stale_metric<T: Clone>(metric: &mut InferenceMetric<T>, reason: &str) {
    metric.state = match &metric.state {
        TelemetryState::Live { value } => TelemetryState::Stale {
            last_value: Some(value.clone()),
            last_observed_at_unix_ms: Some(metric.observed_at_unix_ms),
            reason: reason.to_owned(),
        },
        TelemetryState::Stale {
            last_value,
            last_observed_at_unix_ms,
            ..
        } => TelemetryState::Stale {
            last_value: last_value.clone(),
            last_observed_at_unix_ms: *last_observed_at_unix_ms,
            reason: reason.to_owned(),
        },
        TelemetryState::Unavailable { reason } => TelemetryState::Unavailable {
            reason: reason.clone(),
        },
        TelemetryState::Error { message } => TelemetryState::Error {
            message: message.clone(),
        },
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> InferenceRequestObservation {
        InferenceRequestObservation {
            request_id: "request-1".to_owned(),
            endpoint: "127.0.0.1:8080".to_owned(),
            server_pid: Some(42),
            requested_model: Some("model.gguf".to_owned()),
            request_latency_ms: 42.5,
            ttft_ms: Some(12.25),
            observed_at_unix_ms: 1234,
        }
    }

    #[test]
    fn parses_native_completion_timings_and_identity() {
        let body = r#"{
            "model":"model.gguf",
            "tokens_cached":3,
            "tokens_evaluated":5,
            "tokens_predicted":7,
            "timings":{
                "cache_n":3,
                "prompt_n":5,
                "prompt_per_second":100.5,
                "predicted_n":7,
                "predicted_per_second":25.25
            },
            "generation_settings":{"speculative.types":"none"}
        }"#;
        let snapshot = parse_llama_cpp_completion(body, observation()).unwrap();
        assert_eq!(snapshot.identity.request_id, "request-1");
        assert_eq!(snapshot.identity.server_pid, Some(42));
        assert_eq!(
            snapshot.identity.reported_model.as_deref(),
            Some("model.gguf")
        );
        assert_eq!(snapshot.prompt_tps.live_value(), Some(&100.5));
        assert_eq!(snapshot.decode_tps.live_value(), Some(&25.25));
        assert_eq!(snapshot.ttft_ms.live_value(), Some(&12.25));
        assert_eq!(snapshot.request_latency_ms.live_value(), Some(&42.5));
        assert_eq!(snapshot.context_tokens.live_value(), Some(&15));
        assert!(matches!(
            &snapshot.mtp_generated_tokens.state,
            TelemetryState::Unavailable { .. }
        ));
    }

    #[test]
    fn generic_draft_counters_do_not_imply_mtp() {
        let body = r#"{
            "timings":{
                "draft_n":12,
                "draft_n_accepted":9,
                "draft_mean_len":2.5
            },
            "generation_settings":{"speculative.types":"draft"}
        }"#;
        let snapshot = parse_llama_cpp_completion(body, observation()).unwrap();
        assert_eq!(snapshot.speculative_generated_tokens.live_value(), Some(&12));
        assert_eq!(snapshot.speculative_accepted_tokens.live_value(), Some(&9));
        assert_eq!(snapshot.speculative_acceptance_rate.live_value(), Some(&0.75));
        assert_eq!(snapshot.speculative_mean_run_length.live_value(), Some(&2.5));
        assert!(matches!(
            &snapshot.mtp_generated_tokens.state,
            TelemetryState::Unavailable { .. }
        ));
    }

    #[test]
    fn explicit_mtp_mode_allows_mtp_projection() {
        let body = r#"{
            "timings":{
                "draft_n":12,
                "draft_n_accepted":9,
                "draft_accept_ratio":0.75,
                "draft_mean_len":2.5
            },
            "generation_settings":{"speculative.types":"draft-mtp"}
        }"#;
        let snapshot = parse_llama_cpp_completion(body, observation()).unwrap();
        assert_eq!(snapshot.mtp_generated_tokens.live_value(), Some(&12));
        assert_eq!(snapshot.mtp_accepted_tokens.live_value(), Some(&9));
        assert_eq!(snapshot.mtp_acceptance_rate.live_value(), Some(&0.75));
        assert_eq!(snapshot.mtp_mean_run_length.live_value(), Some(&2.5));
    }

    #[test]
    fn invalid_acceptance_ratio_is_error() {
        let body = r#"{"timings":{"draft_accept_ratio":1.2}}"#;
        let snapshot = parse_llama_cpp_completion(body, observation()).unwrap();
        assert!(matches!(
            &snapshot.speculative_acceptance_rate.state,
            TelemetryState::Error { .. }
        ));
    }

    #[test]
    fn missing_and_null_metrics_are_unavailable_not_zero() {
        let snapshot = parse_llama_cpp_completion(
            r#"{"timings":{"prompt_per_second":null}}"#,
            observation(),
        )
        .unwrap();
        assert!(matches!(
            &snapshot.prompt_tps.state,
            TelemetryState::Unavailable { .. }
        ));
        assert!(matches!(
            &snapshot.context_capacity_tokens.state,
            TelemetryState::Unavailable { .. }
        ));
        assert!(matches!(
            &snapshot.batch_tokens.state,
            TelemetryState::Unavailable { .. }
        ));
        assert!(matches!(
            &snapshot.kv_cache_tokens.state,
            TelemetryState::Unavailable { .. }
        ));
    }

    #[test]
    fn malformed_metric_type_is_error_without_poisoning_other_fields() {
        let body = r#"{
            "timings":{
                "prompt_per_second":"fast",
                "predicted_per_second":12.0,
                "cache_n":0,
                "prompt_n":2,
                "predicted_n":1
            }
        }"#;
        let snapshot = parse_llama_cpp_completion(body, observation()).unwrap();
        assert!(matches!(
            &snapshot.prompt_tps.state,
            TelemetryState::Error { .. }
        ));
        assert_eq!(snapshot.decode_tps.live_value(), Some(&12.0));
        assert_eq!(snapshot.context_tokens.live_value(), Some(&3));
    }

    #[test]
    fn version_changed_timings_shape_is_error_not_bogus_data() {
        let snapshot =
            parse_llama_cpp_completion(r#"{"timings":"changed-shape"}"#, observation()).unwrap();
        assert!(matches!(
            &snapshot.prompt_tps.state,
            TelemetryState::Error { .. }
        ));
        assert!(matches!(
            &snapshot.decode_tps.state,
            TelemetryState::Error { .. }
        ));
    }

    #[test]
    fn invalid_json_and_non_object_root_are_rejected() {
        assert!(matches!(
            parse_llama_cpp_completion("{", observation()),
            Err(InferenceTelemetryParseError::InvalidJson(_))
        ));
        assert_eq!(
            parse_llama_cpp_completion("[]", observation()).unwrap_err(),
            InferenceTelemetryParseError::InvalidRoot
        );
    }

    #[test]
    fn accepted_tokens_cannot_exceed_generated_tokens() {
        let snapshot = parse_llama_cpp_completion(
            r#"{"timings":{"draft_n":2,"draft_n_accepted":3}}"#,
            observation(),
        )
        .unwrap();
        assert!(matches!(
            &snapshot.speculative_acceptance_rate.state,
            TelemetryState::Error { .. }
        ));
    }

    #[test]
    fn tracker_marks_live_metrics_stale_on_disconnect() {
        let body = r#"{
            "timings":{
                "cache_n":1,
                "prompt_n":2,
                "prompt_per_second":10.0,
                "predicted_n":3,
                "predicted_per_second":20.0
            }
        }"#;
        let mut tracker = InferenceTelemetryTracker::default();
        tracker.ingest(body, observation()).unwrap();
        let stale = tracker.mark_disconnected("server restarted", 2000).unwrap();
        assert!(matches!(
            &stale.connection,
            InferenceConnectionState::Stale { .. }
        ));
        assert_eq!(
            stale.prompt_tps.state,
            TelemetryState::Stale {
                last_value: Some(10.0),
                last_observed_at_unix_ms: Some(1234),
                reason: "server restarted".to_owned(),
            }
        );
    }

    #[test]
    fn new_request_or_server_identity_does_not_use_counter_deltas() {
        let mut tracker = InferenceTelemetryTracker::default();
        let first = r#"{"timings":{"cache_n":10,"prompt_n":20,"prompt_per_second":30.0,"predicted_n":40,"predicted_per_second":50.0}}"#;
        tracker.ingest(first, observation()).unwrap();

        let mut second_observation = observation();
        second_observation.request_id = "request-2".to_owned();
        second_observation.server_pid = Some(99);
        let second = r#"{"timings":{"cache_n":0,"prompt_n":1,"prompt_per_second":8.0,"predicted_n":2,"predicted_per_second":9.0}}"#;
        let current = tracker.ingest(second, second_observation).unwrap();
        assert_eq!(current.identity.request_id, "request-2");
        assert_eq!(current.identity.server_pid, Some(99));
        assert_eq!(current.prompt_tps.live_value(), Some(&8.0));
        assert_eq!(current.decode_tps.live_value(), Some(&9.0));
        assert_eq!(current.context_tokens.live_value(), Some(&3));
    }
}