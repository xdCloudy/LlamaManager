use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::telemetry_history::{
    MetricSupport, SeriesKey, TimeSeriesSample, TimeSeriesState,
};

pub const DEFAULT_ALERT_HISTORY_CAPACITY: usize = 512;
pub const DEFAULT_ALERT_EVIDENCE_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertComparator {
    Above,
    Below,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AlertThreshold {
    pub trigger: f64,
    pub clear: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AlertValueRange {
    pub min: f64,
    pub max: f64,
}

impl AlertValueRange {
    pub fn validate(self) -> Result<Self, AlertError> {
        if !self.min.is_finite() || !self.max.is_finite() {
            return Err(AlertError::InvalidRule(
                "alert value range must be finite".to_owned(),
            ));
        }
        if self.min >= self.max {
            return Err(AlertError::InvalidRule(
                "alert value range minimum must be less than maximum".to_owned(),
            ));
        }
        Ok(self)
    }

    fn contains(self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertRule {
    pub id: String,
    pub metric: String,
    pub source_provider: String,
    pub source_api: String,
    pub severity: AlertSeverity,
    pub comparator: AlertComparator,
    pub threshold: AlertThreshold,
    pub window_ms: u64,
    pub debounce_ms: u64,
    pub min_live_samples: usize,
    pub valid_value_range: Option<AlertValueRange>,
    pub reason: String,
}

impl AlertRule {
    pub fn validate(&self) -> Result<(), AlertError> {
        for (name, value) in [
            ("id", self.id.as_str()),
            ("metric", self.metric.as_str()),
            ("source_provider", self.source_provider.as_str()),
            ("source_api", self.source_api.as_str()),
            ("reason", self.reason.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(AlertError::InvalidRule(format!(
                    "alert rule {name} cannot be empty"
                )));
            }
        }

        if !self.threshold.trigger.is_finite() || !self.threshold.clear.is_finite() {
            return Err(AlertError::InvalidRule(
                "alert thresholds must be finite".to_owned(),
            ));
        }
        if self.window_ms == 0 {
            return Err(AlertError::InvalidRule(
                "alert window_ms must be greater than zero".to_owned(),
            ));
        }
        if self.min_live_samples == 0 {
            return Err(AlertError::InvalidRule(
                "min_live_samples must be greater than zero".to_owned(),
            ));
        }

        match self.comparator {
            AlertComparator::Above if self.threshold.clear >= self.threshold.trigger => {
                return Err(AlertError::InvalidRule(
                    "above-threshold alerts require clear < trigger for hysteresis".to_owned(),
                ));
            }
            AlertComparator::Below if self.threshold.clear <= self.threshold.trigger => {
                return Err(AlertError::InvalidRule(
                    "below-threshold alerts require clear > trigger for hysteresis".to_owned(),
                ));
            }
            _ => {}
        }

        if let Some(range) = self.valid_value_range {
            let range = range.validate()?;
            if !range.contains(self.threshold.trigger) || !range.contains(self.threshold.clear) {
                return Err(AlertError::InvalidRule(format!(
                    "alert thresholds [{}, {}] fall outside valid metric range [{}, {}]",
                    self.threshold.trigger, self.threshold.clear, range.min, range.max
                )));
            }
        }

        Ok(())
    }

    fn matches(&self, key: &SeriesKey) -> bool {
        self.metric == key.metric
            && self.source_provider == key.source_provider
            && self.source_api == key.source_api
    }

    fn trigger_holds(&self, value: f64) -> bool {
        match self.comparator {
            AlertComparator::Above => value >= self.threshold.trigger,
            AlertComparator::Below => value <= self.threshold.trigger,
        }
    }

    fn clear_holds(&self, value: f64) -> bool {
        match self.comparator {
            AlertComparator::Above => value <= self.threshold.clear,
            AlertComparator::Below => value >= self.threshold.clear,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum AlertError {
    #[error("invalid alert rule: {0}")]
    InvalidRule(String),
    #[error("duplicate alert rule id: {0}")]
    DuplicateRuleId(String),
    #[error("unknown alert rule id: {0}")]
    UnknownRule(String),
    #[error("alert history capacity must be greater than zero")]
    InvalidHistoryCapacity,
    #[error("alert evidence capacity must be greater than zero")]
    InvalidEvidenceCapacity,
    #[error("sample source does not match the series key")]
    SourceMismatch,
    #[error("sample timestamp moved backwards for alert state: previous={previous}, incoming={incoming}")]
    OutOfOrderSample { previous: u64, incoming: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertPresentationState {
    Inactive,
    Pending,
    Active,
    Suppressed,
    Clearing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertEventKind {
    Fired,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertEvidenceSample {
    pub timestamp_unix_ms: u64,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertEvidence {
    pub rule_id: String,
    pub severity: AlertSeverity,
    pub metric: String,
    pub unit: String,
    pub source_provider: String,
    pub source_api: String,
    pub identity_disclosure: String,
    pub comparator: AlertComparator,
    pub trigger_threshold: f64,
    pub clear_threshold: f64,
    pub window_ms: u64,
    pub debounce_ms: u64,
    pub min_live_samples: usize,
    pub reason: String,
    pub window_started_at_unix_ms: u64,
    pub observed_at_unix_ms: u64,
    pub samples: Vec<AlertEvidenceSample>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertEvent {
    pub kind: AlertEventKind,
    pub occurred_at_unix_ms: u64,
    pub rule_id: String,
    pub severity: AlertSeverity,
    pub series_key: SeriesKey,
    pub evidence: AlertEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertEvaluation {
    pub rule_id: String,
    pub series_key: SeriesKey,
    pub state: AlertPresentationState,
    pub suppression_reason: Option<String>,
    pub transition: Option<AlertEvent>,
}

#[derive(Debug, Clone)]
struct PendingWindow {
    started_at_unix_ms: u64,
    samples: VecDeque<AlertEvidenceSample>,
}

impl PendingWindow {
    fn new(timestamp_unix_ms: u64, value: f64, capacity: usize) -> Self {
        let mut samples = VecDeque::with_capacity(capacity.min(16));
        samples.push_back(AlertEvidenceSample {
            timestamp_unix_ms,
            value,
        });
        Self {
            started_at_unix_ms: timestamp_unix_ms,
            samples,
        }
    }

    fn push(&mut self, timestamp_unix_ms: u64, value: f64, capacity: usize) {
        self.samples.push_back(AlertEvidenceSample {
            timestamp_unix_ms,
            value,
        });
        while self.samples.len() > capacity {
            self.samples.pop_front();
        }
    }

    fn sample_count(&self) -> usize {
        self.samples.len()
    }

    fn elapsed_ms(&self, timestamp_unix_ms: u64) -> u64 {
        timestamp_unix_ms.saturating_sub(self.started_at_unix_ms)
    }
}

#[derive(Debug, Clone, Default)]
struct InstanceState {
    active: bool,
    trigger_window: Option<PendingWindow>,
    clear_window: Option<PendingWindow>,
    last_transition_at_unix_ms: Option<u64>,
    last_observed_at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InstanceKey {
    rule_id: String,
    series_key: SeriesKey,
}

#[derive(Debug, Clone)]
pub struct AlertEngine {
    rules: Vec<AlertRule>,
    states: HashMap<InstanceKey, InstanceState>,
    history: VecDeque<AlertEvent>,
    history_capacity: usize,
    evidence_capacity: usize,
}

impl AlertEngine {
    pub fn new(rules: Vec<AlertRule>) -> Result<Self, AlertError> {
        Self::with_capacities(
            rules,
            DEFAULT_ALERT_HISTORY_CAPACITY,
            DEFAULT_ALERT_EVIDENCE_CAPACITY,
        )
    }

    pub fn with_capacities(
        rules: Vec<AlertRule>,
        history_capacity: usize,
        evidence_capacity: usize,
    ) -> Result<Self, AlertError> {
        if history_capacity == 0 {
            return Err(AlertError::InvalidHistoryCapacity);
        }
        if evidence_capacity == 0 {
            return Err(AlertError::InvalidEvidenceCapacity);
        }

        let mut seen = std::collections::HashSet::new();
        for rule in &rules {
            rule.validate()?;
            if !seen.insert(rule.id.clone()) {
                return Err(AlertError::DuplicateRuleId(rule.id.clone()));
            }
        }

        Ok(Self {
            rules,
            states: HashMap::new(),
            history: VecDeque::with_capacity(history_capacity.min(64)),
            history_capacity,
            evidence_capacity,
        })
    }

    pub fn rules(&self) -> &[AlertRule] {
        &self.rules
    }

    pub fn history(&self) -> &VecDeque<AlertEvent> {
        &self.history
    }

    pub fn update_threshold(
        &mut self,
        rule_id: &str,
        threshold: AlertThreshold,
    ) -> Result<(), AlertError> {
        let rule = self
            .rules
            .iter_mut()
            .find(|rule| rule.id == rule_id)
            .ok_or_else(|| AlertError::UnknownRule(rule_id.to_owned()))?;
        let previous = rule.threshold;
        rule.threshold = threshold;
        if let Err(error) = rule.validate() {
            rule.threshold = previous;
            return Err(error);
        }

        self.states.retain(|key, _| key.rule_id != rule_id);
        Ok(())
    }

    pub fn observe(
        &mut self,
        key: &SeriesKey,
        sample: &TimeSeriesSample,
    ) -> Result<Vec<AlertEvaluation>, AlertError> {
        validate_sample_source(key, sample)?;
        let matching: Vec<AlertRule> = self
            .rules
            .iter()
            .filter(|rule| rule.matches(key))
            .cloned()
            .collect();

        let mut evaluations = Vec::with_capacity(matching.len());
        for rule in matching {
            let instance_key = InstanceKey {
                rule_id: rule.id.clone(),
                series_key: key.clone(),
            };
            let state = self.states.entry(instance_key).or_default();
            if let Some(previous) = state.last_observed_at_unix_ms
                && sample.timestamp_unix_ms < previous
            {
                return Err(AlertError::OutOfOrderSample {
                    previous,
                    incoming: sample.timestamp_unix_ms,
                });
            }
            state.last_observed_at_unix_ms = Some(sample.timestamp_unix_ms);

            let (presentation, suppression_reason, transition) = match live_value(sample) {
                Some(value) => evaluate_live(
                    &rule,
                    key,
                    state,
                    sample.timestamp_unix_ms,
                    value,
                    self.evidence_capacity,
                ),
                None => {
                    state.trigger_window = None;
                    state.clear_window = None;
                    (
                        AlertPresentationState::Suppressed,
                        Some(non_live_reason(sample)),
                        None,
                    )
                }
            };

            if let Some(event) = transition.as_ref() {
                self.history.push_back(event.clone());
                while self.history.len() > self.history_capacity {
                    self.history.pop_front();
                }
            }

            evaluations.push(AlertEvaluation {
                rule_id: rule.id,
                series_key: key.clone(),
                state: presentation,
                suppression_reason,
                transition,
            });
        }

        Ok(evaluations)
    }
}

fn evaluate_live(
    rule: &AlertRule,
    key: &SeriesKey,
    state: &mut InstanceState,
    timestamp_unix_ms: u64,
    value: f64,
    evidence_capacity: usize,
) -> (AlertPresentationState, Option<String>, Option<AlertEvent>) {
    if state.active {
        state.trigger_window = None;
        if rule.clear_holds(value) {
            let window = state.clear_window.get_or_insert_with(|| {
                PendingWindow::new(timestamp_unix_ms, value, evidence_capacity)
            });
            if window.started_at_unix_ms != timestamp_unix_ms {
                window.push(timestamp_unix_ms, value, evidence_capacity);
            }

            if window.elapsed_ms(timestamp_unix_ms) >= rule.window_ms
                && window.sample_count() >= rule.min_live_samples
                && debounce_satisfied(rule, state, timestamp_unix_ms)
            {
                let window = state
                    .clear_window
                    .take()
                    .expect("clear window exists after threshold check");
                state.active = false;
                state.last_transition_at_unix_ms = Some(timestamp_unix_ms);
                return (
                    AlertPresentationState::Inactive,
                    None,
                    Some(event_from_window(
                        AlertEventKind::Resolved,
                        rule,
                        key,
                        timestamp_unix_ms,
                        window,
                    )),
                );
            }
            return (AlertPresentationState::Clearing, None, None);
        }

        state.clear_window = None;
        return (AlertPresentationState::Active, None, None);
    }

    state.clear_window = None;
    if !rule.trigger_holds(value) {
        state.trigger_window = None;
        return (AlertPresentationState::Inactive, None, None);
    }

    if !debounce_satisfied(rule, state, timestamp_unix_ms) {
        state.trigger_window = None;
        return (AlertPresentationState::Inactive, None, None);
    }

    let window = state.trigger_window.get_or_insert_with(|| {
        PendingWindow::new(timestamp_unix_ms, value, evidence_capacity)
    });
    if window.started_at_unix_ms != timestamp_unix_ms {
        window.push(timestamp_unix_ms, value, evidence_capacity);
    }

    if window.elapsed_ms(timestamp_unix_ms) >= rule.window_ms
        && window.sample_count() >= rule.min_live_samples
    {
        let window = state
            .trigger_window
            .take()
            .expect("trigger window exists after threshold check");
        state.active = true;
        state.last_transition_at_unix_ms = Some(timestamp_unix_ms);
        return (
            AlertPresentationState::Active,
            None,
            Some(event_from_window(
                AlertEventKind::Fired,
                rule,
                key,
                timestamp_unix_ms,
                window,
            )),
        );
    }

    (AlertPresentationState::Pending, None, None)
}

fn debounce_satisfied(rule: &AlertRule, state: &InstanceState, timestamp_unix_ms: u64) -> bool {
    state
        .last_transition_at_unix_ms
        .map(|previous| timestamp_unix_ms.saturating_sub(previous) >= rule.debounce_ms)
        .unwrap_or(true)
}

fn event_from_window(
    kind: AlertEventKind,
    rule: &AlertRule,
    key: &SeriesKey,
    occurred_at_unix_ms: u64,
    window: PendingWindow,
) -> AlertEvent {
    AlertEvent {
        kind,
        occurred_at_unix_ms,
        rule_id: rule.id.clone(),
        severity: rule.severity,
        series_key: key.clone(),
        evidence: AlertEvidence {
            rule_id: rule.id.clone(),
            severity: rule.severity,
            metric: rule.metric.clone(),
            unit: key.unit.clone(),
            source_provider: rule.source_provider.clone(),
            source_api: rule.source_api.clone(),
            identity_disclosure: key.identity.disclosure(),
            comparator: rule.comparator,
            trigger_threshold: rule.threshold.trigger,
            clear_threshold: rule.threshold.clear,
            window_ms: rule.window_ms,
            debounce_ms: rule.debounce_ms,
            min_live_samples: rule.min_live_samples,
            reason: rule.reason.clone(),
            window_started_at_unix_ms: window.started_at_unix_ms,
            observed_at_unix_ms: occurred_at_unix_ms,
            samples: window.samples.into_iter().collect(),
        },
    }
}

fn validate_sample_source(key: &SeriesKey, sample: &TimeSeriesSample) -> Result<(), AlertError> {
    if key.metric != sample.source.metric
        || key.source_provider != sample.source.provider
        || key.source_api != sample.source.api
    {
        return Err(AlertError::SourceMismatch);
    }
    Ok(())
}

fn live_value(sample: &TimeSeriesSample) -> Option<f64> {
    if sample.support != MetricSupport::Supported || !matches!(sample.state, TimeSeriesState::Live) {
        return None;
    }
    sample.value.filter(|value| value.is_finite())
}

fn non_live_reason(sample: &TimeSeriesSample) -> String {
    match &sample.state {
        TimeSeriesState::Live if sample.support != MetricSupport::Supported => {
            format!("metric support state is {:?}", sample.support)
        }
        TimeSeriesState::Live => "live sample has no finite value".to_owned(),
        TimeSeriesState::Stale { reason, .. } => format!("stale telemetry: {reason}"),
        TimeSeriesState::Disconnected { reason } => format!("telemetry disconnected: {reason}"),
        TimeSeriesState::Paused { reason } => format!("telemetry paused: {reason}"),
        TimeSeriesState::Unavailable { reason } => format!("metric unavailable: {reason}"),
        TimeSeriesState::Error { message } => format!("telemetry error: {message}"),
        TimeSeriesState::Reset { reason } => format!("metric reset: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry_history::{SampleSource, SeriesIdentity};

    fn key() -> SeriesKey {
        SeriesKey::new(
            "gpu.temperature",
            "celsius",
            "nvidia-nvml",
            "nvmlDeviceGetTemperature",
            SeriesIdentity::new("gpu-uuid", "GPU-1").with_display_name("RTX test"),
        )
    }

    fn rule() -> AlertRule {
        AlertRule {
            id: "gpu-temperature-warning".to_owned(),
            metric: "gpu.temperature".to_owned(),
            source_provider: "nvidia-nvml".to_owned(),
            source_api: "nvmlDeviceGetTemperature".to_owned(),
            severity: AlertSeverity::Warning,
            comparator: AlertComparator::Above,
            threshold: AlertThreshold {
                trigger: 80.0,
                clear: 75.0,
            },
            window_ms: 2_000,
            debounce_ms: 3_000,
            min_live_samples: 3,
            valid_value_range: Some(AlertValueRange {
                min: -20.0,
                max: 150.0,
            }),
            reason: "configured sustained GPU temperature threshold".to_owned(),
        }
    }

    fn live(timestamp: u64, value: f64) -> TimeSeriesSample {
        TimeSeriesSample::live(timestamp, value, SampleSource::from_key(&key())).unwrap()
    }

    #[test]
    fn rule_rejects_invalid_hysteresis_and_out_of_range_thresholds() {
        let mut invalid = rule();
        invalid.threshold.clear = 81.0;
        assert!(invalid.validate().is_err());

        let mut outside = rule();
        outside.threshold.trigger = 151.0;
        assert!(outside.validate().is_err());
    }

    #[test]
    fn sustained_live_threshold_fires_with_complete_evidence() {
        let key = key();
        let mut engine = AlertEngine::new(vec![rule()]).unwrap();

        assert_eq!(
            engine.observe(&key, &live(1_000, 81.0)).unwrap()[0].state,
            AlertPresentationState::Pending
        );
        assert_eq!(
            engine.observe(&key, &live(2_000, 82.0)).unwrap()[0].state,
            AlertPresentationState::Pending
        );
        let evaluation = engine.observe(&key, &live(3_000, 83.0)).unwrap().remove(0);
        assert_eq!(evaluation.state, AlertPresentationState::Active);
        let event = evaluation.transition.unwrap();
        assert_eq!(event.kind, AlertEventKind::Fired);
        assert_eq!(event.evidence.metric, "gpu.temperature");
        assert_eq!(event.evidence.source_provider, "nvidia-nvml");
        assert_eq!(event.evidence.source_api, "nvmlDeviceGetTemperature");
        assert_eq!(event.evidence.trigger_threshold, 80.0);
        assert_eq!(event.evidence.clear_threshold, 75.0);
        assert_eq!(event.evidence.window_ms, 2_000);
        assert_eq!(event.evidence.samples.len(), 3);
        assert!(event.evidence.identity_disclosure.contains("GPU-1"));
        assert_eq!(engine.history().len(), 1);
    }

    #[test]
    fn stale_unavailable_and_error_samples_cannot_fire_live_alerts() {
        let key = key();
        let source = SampleSource::from_key(&key);
        let mut engine = AlertEngine::new(vec![rule()]).unwrap();

        engine.observe(&key, &live(1_000, 90.0)).unwrap();
        engine.observe(&key, &live(2_000, 90.0)).unwrap();

        let stale = TimeSeriesSample::stale(
            3_000,
            Some(90.0),
            source.clone(),
            Some(2_000),
            "provider late",
        )
        .unwrap();
        let evaluation = engine.observe(&key, &stale).unwrap().remove(0);
        assert_eq!(evaluation.state, AlertPresentationState::Suppressed);
        assert!(evaluation.transition.is_none());
        assert!(engine.history().is_empty());

        let unavailable = TimeSeriesSample::unavailable(4_000, source.clone(), "unsupported");
        assert_eq!(
            engine.observe(&key, &unavailable).unwrap()[0].state,
            AlertPresentationState::Suppressed
        );
        let error = TimeSeriesSample::error(5_000, source, "provider failed");
        assert_eq!(
            engine.observe(&key, &error).unwrap()[0].state,
            AlertPresentationState::Suppressed
        );
        assert!(engine.history().is_empty());
    }

    #[test]
    fn hysteresis_and_window_prevent_alert_flapping() {
        let key = key();
        let mut engine = AlertEngine::new(vec![rule()]).unwrap();
        for (timestamp, value) in [(1_000, 82.0), (2_000, 83.0), (3_000, 84.0)] {
            engine.observe(&key, &live(timestamp, value)).unwrap();
        }
        assert_eq!(engine.history().len(), 1);

        for (timestamp, value) in [(4_000, 78.0), (5_000, 76.0), (6_000, 79.0)] {
            let evaluation = engine.observe(&key, &live(timestamp, value)).unwrap();
            assert_eq!(evaluation[0].state, AlertPresentationState::Active);
        }
        assert_eq!(engine.history().len(), 1);

        assert_eq!(
            engine.observe(&key, &live(7_000, 74.0)).unwrap()[0].state,
            AlertPresentationState::Clearing
        );
        assert_eq!(
            engine.observe(&key, &live(8_000, 73.0)).unwrap()[0].state,
            AlertPresentationState::Clearing
        );
        let resolved = engine.observe(&key, &live(9_000, 72.0)).unwrap().remove(0);
        assert_eq!(resolved.state, AlertPresentationState::Inactive);
        assert_eq!(resolved.transition.unwrap().kind, AlertEventKind::Resolved);
        assert_eq!(engine.history().len(), 2);

        for (timestamp, value) in [(10_000, 90.0), (11_000, 90.0)] {
            assert_eq!(
                engine.observe(&key, &live(timestamp, value)).unwrap()[0].state,
                AlertPresentationState::Inactive
            );
        }
        assert_eq!(engine.history().len(), 2);
    }

    #[test]
    fn active_alert_becomes_suppressed_when_data_goes_stale_without_false_resolution() {
        let key = key();
        let source = SampleSource::from_key(&key);
        let mut engine = AlertEngine::new(vec![rule()]).unwrap();
        for (timestamp, value) in [(1_000, 82.0), (2_000, 83.0), (3_000, 84.0)] {
            engine.observe(&key, &live(timestamp, value)).unwrap();
        }

        let stale = TimeSeriesSample::stale(
            4_000,
            Some(84.0),
            source,
            Some(3_000),
            "NVML temporarily unavailable",
        )
        .unwrap();
        let evaluation = engine.observe(&key, &stale).unwrap().remove(0);
        assert_eq!(evaluation.state, AlertPresentationState::Suppressed);
        assert!(evaluation.suppression_reason.unwrap().contains("stale"));
        assert_eq!(engine.history().len(), 1);

        assert_eq!(
            engine.observe(&key, &live(5_000, 82.0)).unwrap()[0].state,
            AlertPresentationState::Active
        );
        assert_eq!(engine.history().len(), 1);
    }

    #[test]
    fn update_threshold_is_validated_and_resets_rule_state() {
        let key = key();
        let mut engine = AlertEngine::new(vec![rule()]).unwrap();
        engine.observe(&key, &live(1_000, 82.0)).unwrap();

        assert!(engine
            .update_threshold(
                "gpu-temperature-warning",
                AlertThreshold {
                    trigger: 70.0,
                    clear: 75.0,
                }
            )
            .is_err());
        assert_eq!(engine.rules()[0].threshold.trigger, 80.0);

        engine
            .update_threshold(
                "gpu-temperature-warning",
                AlertThreshold {
                    trigger: 85.0,
                    clear: 80.0,
                },
            )
            .unwrap();
        assert_eq!(engine.rules()[0].threshold.trigger, 85.0);
        assert_eq!(
            engine.observe(&key, &live(2_000, 82.0)).unwrap()[0].state,
            AlertPresentationState::Inactive
        );
    }

    #[test]
    fn state_is_isolated_per_series_identity() {
        let first = key();
        let mut second = key();
        second.identity = SeriesIdentity::new("gpu-uuid", "GPU-2");
        let mut engine = AlertEngine::new(vec![rule()]).unwrap();

        for timestamp in [1_000, 2_000, 3_000] {
            engine.observe(&first, &live(timestamp, 90.0)).unwrap();
        }

        let second_sample = TimeSeriesSample::live(
            3_000,
            90.0,
            SampleSource::from_key(&second),
        )
        .unwrap();
        let evaluation = engine.observe(&second, &second_sample).unwrap().remove(0);
        assert_eq!(evaluation.state, AlertPresentationState::Pending);
        assert_eq!(engine.history().len(), 1);
    }

    #[test]
    fn alert_history_is_bounded() {
        let key = key();
        let mut fast_rule = rule();
        fast_rule.window_ms = 1;
        fast_rule.debounce_ms = 0;
        fast_rule.min_live_samples = 2;
        let mut engine = AlertEngine::with_capacities(vec![fast_rule], 2, 8).unwrap();

        for cycle in 0..3_u64 {
            let base = cycle * 10_000;
            engine.observe(&key, &live(base + 1_000, 90.0)).unwrap();
            engine.observe(&key, &live(base + 1_001, 90.0)).unwrap();
            engine.observe(&key, &live(base + 2_000, 70.0)).unwrap();
            engine.observe(&key, &live(base + 2_001, 70.0)).unwrap();
        }

        assert_eq!(engine.history().len(), 2);
        assert_eq!(engine.history().back().unwrap().kind, AlertEventKind::Resolved);
    }
}
