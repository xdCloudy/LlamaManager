use std::{thread, time::Duration};

pub use crate::passive_inference_metrics_legacy::{
    PassiveInferenceMetricsError, PassiveInferenceMetricsSnapshot,
};
use crate::{passive_inference_metrics_legacy, server_readiness::ServerEndpoint};

// A local llama.cpp router can replace a child process between GET /models and the subsequent
// GET /metrics. Under heavy prompt/decode load Windows can also transiently miss a short socket
// deadline even though the child is still healthy. Do not surface either case as STALE until an
// immediate re-resolution/retry window has been exhausted.
const RETRY_ATTEMPTS: usize = 4;
const RETRY_BACKOFF_MS: [u64; RETRY_ATTEMPTS - 1] = [0, 75, 175];

pub fn poll_passive_inference_metrics(
    configured_endpoint: &ServerEndpoint,
    timeout: Duration,
) -> Result<PassiveInferenceMetricsSnapshot, PassiveInferenceMetricsError> {
    let mut last_error = None;

    for attempt in 0..RETRY_ATTEMPTS {
        match passive_inference_metrics_legacy::poll_passive_inference_metrics(
            configured_endpoint,
            timeout,
        ) {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) if should_retry(&error) && attempt + 1 < RETRY_ATTEMPTS => {
                last_error = Some(error);
                let delay_ms = RETRY_BACKOFF_MS[attempt];
                if delay_ms != 0 {
                    thread::sleep(Duration::from_millis(delay_ms));
                }
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.expect("retry loop must retain the last transient error"))
}

fn should_retry(error: &PassiveInferenceMetricsError) -> bool {
    match error {
        PassiveInferenceMetricsError::Connect { .. }
        | PassiveInferenceMetricsError::Io { .. }
        | PassiveInferenceMetricsError::MissingHeaders
        | PassiveInferenceMetricsError::InvalidStatusLine => true,
        PassiveInferenceMetricsError::MetricsHttpRejected { status_code } => {
            matches!(*status_code, 408 | 425 | 429 | 500 | 502 | 503 | 504)
        }
        PassiveInferenceMetricsError::InvalidPort
        | PassiveInferenceMetricsError::InvalidApiKey
        | PassiveInferenceMetricsError::HostResolution { .. }
        | PassiveInferenceMetricsError::NonLoopbackDenied { .. }
        | PassiveInferenceMetricsError::ResponseTooLarge { .. }
        | PassiveInferenceMetricsError::MetricsUnsupported { .. }
        | PassiveInferenceMetricsError::InvalidUtf8
        | PassiveInferenceMetricsError::NoRecognizedMetrics => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_transient_transport_and_busy_gateway_failures() {
        assert!(should_retry(&PassiveInferenceMetricsError::Io {
            phase: "metrics read",
            message: "timed out".to_owned(),
        }));
        assert!(should_retry(&PassiveInferenceMetricsError::Connect {
            endpoint: "127.0.0.1:50973".to_owned(),
            message: "refused".to_owned(),
        }));
        assert!(should_retry(
            &PassiveInferenceMetricsError::MetricsHttpRejected { status_code: 503 }
        ));
    }

    #[test]
    fn does_not_retry_capability_or_configuration_failures() {
        assert!(!should_retry(
            &PassiveInferenceMetricsError::MetricsUnsupported { status_code: 501 }
        ));
        assert!(!should_retry(&PassiveInferenceMetricsError::InvalidApiKey));
        assert!(!should_retry(
            &PassiveInferenceMetricsError::NoRecognizedMetrics
        ));
    }
}
