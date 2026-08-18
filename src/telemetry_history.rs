use std::{
    collections::{HashMap, VecDeque},
    fmt::Write,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_HISTORY_RETENTION_MS: u64 = 30 * 60 * 1_000;
pub const DEFAULT_RAW_RETENTION_MS: u64 = 2 * 60 * 1_000;
pub const DEFAULT_DOWNSAMPLE_BUCKET_MS: u64 = 5_000;
pub const DEFAULT_MAX_POINTS_PER_SERIES: usize = 8_192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricSupport {
    Supported,
    Unavailable,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TimeSeriesState {
    Live,
    Stale {
        reason: String,
        last_observed_at_unix_ms: Option<u64>,
    },
    Disconnected {
        reason: String,
    },
    Paused {
        reason: String,
    },
    Unavailable {
        reason: String,
    },
    Error {
        message: String,
    },
    Reset {
        reason: String,
    },
}

impl TimeSeriesState {
    fn render_class(&self) -> Option<RenderClass> {
        match self {
            Self::Live => Some(RenderClass::Live),
            Self::Stale { .. } => Some(RenderClass::Stale),
            Self::Disconnected { .. }
            | Self::Paused { .. }
            | Self::Unavailable { .. }
            | Self::Error { .. }
            | Self::Reset { .. } => None,
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            Self::Live => None,
            Self::Stale { reason, .. }
            | Self::Disconnected { reason }
            | Self::Paused { reason }
            | Self::Unavailable { reason }
            | Self::Reset { reason } => Some(reason),
            Self::Error { message } => Some(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SeriesIdentity {
    pub namespace: String,
    pub stable_id: String,
    pub display_name: Option<String>,
}

impl SeriesIdentity {
    pub fn new(namespace: impl Into<String>, stable_id: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            stable_id: stable_id.into(),
            display_name: None,
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn disclosure(&self) -> String {
        match &self.display_name {
            Some(display_name) => format!("{}:{} ({display_name})", self.namespace, self.stable_id),
            None => format!("{}:{}", self.namespace, self.stable_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SeriesKey {
    pub metric: String,
    pub unit: String,
    pub source_provider: String,
    pub source_api: String,
    pub identity: SeriesIdentity,
}

impl SeriesKey {
    pub fn new(
        metric: impl Into<String>,
        unit: impl Into<String>,
        source_provider: impl Into<String>,
        source_api: impl Into<String>,
        identity: SeriesIdentity,
    ) -> Self {
        Self {
            metric: metric.into(),
            unit: unit.into(),
            source_provider: source_provider.into(),
            source_api: source_api.into(),
            identity,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleSource {
    pub provider: String,
    pub api: String,
    pub metric: String,
}

impl SampleSource {
    pub fn from_key(key: &SeriesKey) -> Self {
        Self {
            provider: key.source_provider.clone(),
            api: key.source_api.clone(),
            metric: key.metric.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeSeriesSample {
    pub timestamp_unix_ms: u64,
    pub value: Option<f64>,
    pub source: SampleSource,
    pub support: MetricSupport,
    pub state: TimeSeriesState,
}

impl TimeSeriesSample {
    pub fn live(
        timestamp_unix_ms: u64,
        value: f64,
        source: SampleSource,
    ) -> Result<Self, HistoryError> {
        validate_finite(value)?;
        Ok(Self {
            timestamp_unix_ms,
            value: Some(value),
            source,
            support: MetricSupport::Supported,
            state: TimeSeriesState::Live,
        })
    }

    pub fn stale(
        timestamp_unix_ms: u64,
        last_value: Option<f64>,
        source: SampleSource,
        last_observed_at_unix_ms: Option<u64>,
        reason: impl Into<String>,
    ) -> Result<Self, HistoryError> {
        if let Some(value) = last_value {
            validate_finite(value)?;
        }
        Ok(Self {
            timestamp_unix_ms,
            value: last_value,
            source,
            support: MetricSupport::Supported,
            state: TimeSeriesState::Stale {
                reason: reason.into(),
                last_observed_at_unix_ms,
            },
        })
    }

    pub fn disconnected(
        timestamp_unix_ms: u64,
        source: SampleSource,
        reason: impl Into<String>,
    ) -> Self {
        Self::marker(
            timestamp_unix_ms,
            source,
            MetricSupport::Supported,
            TimeSeriesState::Disconnected {
                reason: reason.into(),
            },
        )
    }

    pub fn paused(timestamp_unix_ms: u64, source: SampleSource, reason: impl Into<String>) -> Self {
        Self::marker(
            timestamp_unix_ms,
            source,
            MetricSupport::Supported,
            TimeSeriesState::Paused {
                reason: reason.into(),
            },
        )
    }

    pub fn unavailable(
        timestamp_unix_ms: u64,
        source: SampleSource,
        reason: impl Into<String>,
    ) -> Self {
        Self::marker(
            timestamp_unix_ms,
            source,
            MetricSupport::Unavailable,
            TimeSeriesState::Unavailable {
                reason: reason.into(),
            },
        )
    }

    pub fn error(timestamp_unix_ms: u64, source: SampleSource, message: impl Into<String>) -> Self {
        Self::marker(
            timestamp_unix_ms,
            source,
            MetricSupport::Error,
            TimeSeriesState::Error {
                message: message.into(),
            },
        )
    }

    pub fn reset(timestamp_unix_ms: u64, source: SampleSource, reason: impl Into<String>) -> Self {
        Self::marker(
            timestamp_unix_ms,
            source,
            MetricSupport::Supported,
            TimeSeriesState::Reset {
                reason: reason.into(),
            },
        )
    }

    fn marker(
        timestamp_unix_ms: u64,
        source: SampleSource,
        support: MetricSupport,
        state: TimeSeriesState,
    ) -> Self {
        Self {
            timestamp_unix_ms,
            value: None,
            source,
            support,
            state,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPolicy {
    pub retention_ms: u64,
    pub raw_retention_ms: u64,
    pub downsample_bucket_ms: u64,
    pub max_points_per_series: usize,
}

impl Default for HistoryPolicy {
    fn default() -> Self {
        Self {
            retention_ms: DEFAULT_HISTORY_RETENTION_MS,
            raw_retention_ms: DEFAULT_RAW_RETENTION_MS,
            downsample_bucket_ms: DEFAULT_DOWNSAMPLE_BUCKET_MS,
            max_points_per_series: DEFAULT_MAX_POINTS_PER_SERIES,
        }
    }
}

impl HistoryPolicy {
    pub fn validate(self) -> Result<Self, HistoryError> {
        if self.retention_ms == 0 {
            return Err(HistoryError::InvalidPolicy(
                "retention_ms must be greater than zero".to_owned(),
            ));
        }
        if self.raw_retention_ms > self.retention_ms {
            return Err(HistoryError::InvalidPolicy(
                "raw_retention_ms cannot exceed retention_ms".to_owned(),
            ));
        }
        if self.downsample_bucket_ms == 0 {
            return Err(HistoryError::InvalidPolicy(
                "downsample_bucket_ms must be greater than zero".to_owned(),
            ));
        }
        if self.max_points_per_series < 8 {
            return Err(HistoryError::InvalidPolicy(
                "max_points_per_series must be at least 8".to_owned(),
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum HistoryError {
    #[error("invalid history policy: {0}")]
    InvalidPolicy(String),
    #[error("sample value must be finite")]
    NonFiniteValue,
    #[error("sample timestamp moved backwards: previous={previous}, incoming={incoming}")]
    OutOfOrderSample { previous: u64, incoming: u64 },
    #[error("sample source does not match series key")]
    SourceMismatch,
    #[error("chart dimensions must be non-zero")]
    InvalidChartDimensions,
}

fn validate_finite(value: f64) -> Result<(), HistoryError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(HistoryError::NonFiniteValue)
    }
}

#[derive(Debug, Clone)]
pub struct TimeSeries {
    key: SeriesKey,
    policy: HistoryPolicy,
    samples: VecDeque<TimeSeriesSample>,
    last_compaction_unix_ms: Option<u64>,
}

impl TimeSeries {
    pub fn new(key: SeriesKey, policy: HistoryPolicy) -> Result<Self, HistoryError> {
        Ok(Self {
            key,
            policy: policy.validate()?,
            samples: VecDeque::new(),
            last_compaction_unix_ms: None,
        })
    }

    pub fn key(&self) -> &SeriesKey {
        &self.key
    }

    pub fn policy(&self) -> HistoryPolicy {
        self.policy
    }

    pub fn samples(&self) -> &VecDeque<TimeSeriesSample> {
        &self.samples
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn push(&mut self, sample: TimeSeriesSample) -> Result<(), HistoryError> {
        self.validate_source(&sample)?;
        if let Some(previous) = self.samples.back() {
            if sample.timestamp_unix_ms < previous.timestamp_unix_ms {
                return Err(HistoryError::OutOfOrderSample {
                    previous: previous.timestamp_unix_ms,
                    incoming: sample.timestamp_unix_ms,
                });
            }
        }

        let newest = sample.timestamp_unix_ms;
        self.samples.push_back(sample);
        self.prune_expired(newest);

        let should_compact = self.samples.len() > self.policy.max_points_per_series
            || self
                .last_compaction_unix_ms
                .map(|last| newest.saturating_sub(last) >= self.policy.downsample_bucket_ms)
                .unwrap_or(true);
        if should_compact {
            self.compact(newest);
            self.last_compaction_unix_ms = Some(newest);
        }

        Ok(())
    }

    pub fn presentation_state(&self) -> SeriesPresentationState {
        self.samples
            .back()
            .map(|sample| SeriesPresentationState::from(&sample.state))
            .unwrap_or(SeriesPresentationState::Empty)
    }

    pub fn project(&self, options: ChartOptions) -> Result<ChartProjection, HistoryError> {
        options.validate()?;
        ChartProjection::build(self, options)
    }

    fn validate_source(&self, sample: &TimeSeriesSample) -> Result<(), HistoryError> {
        if sample.source.metric != self.key.metric
            || sample.source.provider != self.key.source_provider
            || sample.source.api != self.key.source_api
        {
            return Err(HistoryError::SourceMismatch);
        }
        Ok(())
    }

    fn prune_expired(&mut self, newest: u64) {
        let oldest_allowed = newest.saturating_sub(self.policy.retention_ms);
        while self
            .samples
            .front()
            .is_some_and(|sample| sample.timestamp_unix_ms < oldest_allowed)
        {
            self.samples.pop_front();
        }
    }

    fn compact(&mut self, newest: u64) {
        let raw_cutoff = newest.saturating_sub(self.policy.raw_retention_ms);
        let mut output = VecDeque::with_capacity(self.samples.len());
        let mut bucket: Option<DownsampleBucket> = None;

        while let Some(sample) = self.samples.pop_front() {
            if sample.timestamp_unix_ms > raw_cutoff
                || sample.state.render_class().is_none()
                || sample.value.is_none()
            {
                flush_bucket(&mut bucket, &mut output);
                output.push_back(sample);
                continue;
            }

            let bucket_id = sample.timestamp_unix_ms / self.policy.downsample_bucket_ms;
            let render_class = sample
                .state
                .render_class()
                .expect("render class checked above");

            let same_bucket = bucket.as_ref().is_some_and(|current| {
                current.bucket_id == bucket_id && current.class == render_class
            });
            if !same_bucket {
                flush_bucket(&mut bucket, &mut output);
                bucket = Some(DownsampleBucket::new(bucket_id, render_class, sample));
            } else if let Some(current) = bucket.as_mut() {
                current.observe(sample);
            }
        }
        flush_bucket(&mut bucket, &mut output);

        if output.len() > self.policy.max_points_per_series {
            let excess = output.len() - self.policy.max_points_per_series;
            output.drain(..excess);
        }
        self.samples = output;
    }
}

#[derive(Debug, Clone)]
pub struct TimeSeriesStore {
    policy: HistoryPolicy,
    series: HashMap<SeriesKey, TimeSeries>,
}

impl TimeSeriesStore {
    pub fn new(policy: HistoryPolicy) -> Result<Self, HistoryError> {
        Ok(Self {
            policy: policy.validate()?,
            series: HashMap::new(),
        })
    }

    pub fn push(
        &mut self,
        key: SeriesKey,
        sample: TimeSeriesSample,
    ) -> Result<&TimeSeries, HistoryError> {
        let entry = match self.series.entry(key.clone()) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(TimeSeries::new(key, self.policy)?)
            }
        };
        entry.push(sample)?;
        Ok(entry)
    }

    pub fn series(&self, key: &SeriesKey) -> Option<&TimeSeries> {
        self.series.get(key)
    }

    pub fn series_for_metric<'a>(
        &'a self,
        metric: &'a str,
    ) -> impl Iterator<Item = &'a TimeSeries> + 'a {
        self.series
            .values()
            .filter(move |series| series.key.metric == metric)
    }

    pub fn len(&self) -> usize {
        self.series.len()
    }

    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderClass {
    Live,
    Stale,
}

#[derive(Debug)]
struct DownsampleBucket {
    bucket_id: u64,
    class: RenderClass,
    first: TimeSeriesSample,
    min: TimeSeriesSample,
    max: TimeSeriesSample,
    last: TimeSeriesSample,
}

impl DownsampleBucket {
    fn new(bucket_id: u64, class: RenderClass, sample: TimeSeriesSample) -> Self {
        Self {
            bucket_id,
            class,
            first: sample.clone(),
            min: sample.clone(),
            max: sample.clone(),
            last: sample,
        }
    }

    fn observe(&mut self, sample: TimeSeriesSample) {
        let value = sample.value.expect("renderable samples carry values");
        if value < self.min.value.expect("renderable samples carry values") {
            self.min = sample.clone();
        }
        if value > self.max.value.expect("renderable samples carry values") {
            self.max = sample.clone();
        }
        self.last = sample;
    }

    fn selected(self) -> Vec<TimeSeriesSample> {
        let mut selected = vec![self.first, self.min, self.max, self.last];
        selected.sort_by_key(|sample| sample.timestamp_unix_ms);
        selected.dedup_by_key(|sample| sample.timestamp_unix_ms);
        selected
    }
}

fn flush_bucket(bucket: &mut Option<DownsampleBucket>, output: &mut VecDeque<TimeSeriesSample>) {
    if let Some(current) = bucket.take() {
        output.extend(current.selected());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChartOptions {
    pub width_px: u32,
    pub height_px: u32,
    pub missing_gap_after_ms: Option<u64>,
}

impl Default for ChartOptions {
    fn default() -> Self {
        Self {
            width_px: 800,
            height_px: 220,
            missing_gap_after_ms: None,
        }
    }
}

impl ChartOptions {
    fn validate(self) -> Result<Self, HistoryError> {
        if self.width_px == 0 || self.height_px == 0 {
            return Err(HistoryError::InvalidChartDimensions);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesPresentationState {
    Empty,
    Live,
    Stale,
    Disconnected,
    Paused,
    Unavailable,
    Error,
    Reset,
}

impl From<&TimeSeriesState> for SeriesPresentationState {
    fn from(value: &TimeSeriesState) -> Self {
        match value {
            TimeSeriesState::Live => Self::Live,
            TimeSeriesState::Stale { .. } => Self::Stale,
            TimeSeriesState::Disconnected { .. } => Self::Disconnected,
            TimeSeriesState::Paused { .. } => Self::Paused,
            TimeSeriesState::Unavailable { .. } => Self::Unavailable,
            TimeSeriesState::Error { .. } => Self::Error,
            TimeSeriesState::Reset { .. } => Self::Reset,
        }
    }
}

impl SeriesPresentationState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Live => "live",
            Self::Stale => "stale",
            Self::Disconnected => "disconnected",
            Self::Paused => "paused",
            Self::Unavailable => "unavailable",
            Self::Error => "error",
            Self::Reset => "reset",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartSegmentState {
    Live,
    Stale,
}

impl ChartSegmentState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Stale => "stale",
        }
    }
}

impl From<RenderClass> for ChartSegmentState {
    fn from(value: RenderClass) -> Self {
        match value {
            RenderClass::Live => Self::Live,
            RenderClass::Stale => Self::Stale,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartPoint {
    pub timestamp_unix_ms: u64,
    pub value: f64,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartSegment {
    pub state: ChartSegmentState,
    pub points: Vec<ChartPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartGapKind {
    MissingSamples,
    Stale,
    Disconnected,
    Paused,
    Unavailable,
    Error,
    Reset,
}

impl ChartGapKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::MissingSamples => "missing_samples",
            Self::Stale => "stale",
            Self::Disconnected => "disconnected",
            Self::Paused => "paused",
            Self::Unavailable => "unavailable",
            Self::Error => "error",
            Self::Reset => "reset",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartGap {
    pub kind: ChartGapKind,
    pub from_unix_ms: u64,
    pub to_unix_ms: u64,
    pub x_start: f64,
    pub x_end: f64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartProjection {
    pub width_px: u32,
    pub height_px: u32,
    pub key: SeriesKey,
    pub identity_disclosure: String,
    pub presentation_state: SeriesPresentationState,
    pub timeline_start_unix_ms: Option<u64>,
    pub timeline_end_unix_ms: Option<u64>,
    pub y_min: Option<f64>,
    pub y_max: Option<f64>,
    pub segments: Vec<ChartSegment>,
    pub gaps: Vec<ChartGap>,
}

impl ChartProjection {
    fn build(series: &TimeSeries, options: ChartOptions) -> Result<Self, HistoryError> {
        let samples = series.samples();
        let timeline_start = samples.front().map(|sample| sample.timestamp_unix_ms);
        let timeline_end = samples.back().map(|sample| sample.timestamp_unix_ms);
        let values: Vec<f64> = samples
            .iter()
            .filter_map(|sample| sample.state.render_class().and_then(|_| sample.value))
            .collect();

        let (y_min, y_max) = if values.is_empty() {
            (None, None)
        } else {
            let min = values.iter().copied().fold(f64::INFINITY, f64::min);
            let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if (max - min).abs() < f64::EPSILON {
                let padding = if min.abs() < 1.0 {
                    1.0
                } else {
                    min.abs() * 0.05
                };
                (Some(min - padding), Some(max + padding))
            } else {
                (Some(min), Some(max))
            }
        };

        let mut segments = Vec::new();
        let mut gaps = Vec::new();
        let mut current: Option<(RenderClass, Vec<ChartPoint>)> = None;
        let mut previous_renderable_timestamp: Option<u64> = None;
        let mut previous_timestamp: Option<u64> = None;

        for sample in samples {
            match sample.state.render_class() {
                Some(class) if sample.value.is_some() => {
                    if let (Some(previous), Some(limit)) =
                        (previous_renderable_timestamp, options.missing_gap_after_ms)
                    {
                        if sample.timestamp_unix_ms.saturating_sub(previous) > limit {
                            flush_segment(&mut current, &mut segments, options.width_px);
                            gaps.push(project_gap(
                                ChartGapKind::MissingSamples,
                                previous,
                                sample.timestamp_unix_ms,
                                "sample interval exceeded chart gap threshold",
                                timeline_start,
                                timeline_end,
                                options.width_px,
                            ));
                        }
                    }

                    if current.as_ref().is_some_and(|(state, _)| *state != class) {
                        flush_segment(&mut current, &mut segments, options.width_px);
                    }

                    let point = project_point(
                        sample.timestamp_unix_ms,
                        sample.value.expect("checked above"),
                        timeline_start,
                        timeline_end,
                        y_min,
                        y_max,
                        options,
                    );
                    current
                        .get_or_insert_with(|| (class, Vec::new()))
                        .1
                        .push(point);
                    previous_renderable_timestamp = Some(sample.timestamp_unix_ms);
                }
                _ => {
                    flush_segment(&mut current, &mut segments, options.width_px);
                    let from = previous_timestamp.unwrap_or(sample.timestamp_unix_ms);
                    if let Some((kind, reason)) = gap_for_state(&sample.state) {
                        gaps.push(project_gap(
                            kind,
                            from,
                            sample.timestamp_unix_ms,
                            reason,
                            timeline_start,
                            timeline_end,
                            options.width_px,
                        ));
                    }
                    previous_renderable_timestamp = None;
                }
            }
            previous_timestamp = Some(sample.timestamp_unix_ms);
        }
        flush_segment(&mut current, &mut segments, options.width_px);

        Ok(Self {
            width_px: options.width_px,
            height_px: options.height_px,
            key: series.key().clone(),
            identity_disclosure: series.key().identity.disclosure(),
            presentation_state: series.presentation_state(),
            timeline_start_unix_ms: timeline_start,
            timeline_end_unix_ms: timeline_end,
            y_min,
            y_max,
            segments,
            gaps,
        })
    }

    pub fn to_svg(&self) -> String {
        let width_px = self.width_px;
        let height_px = self.height_px;
        let mut output = String::new();
        let _ = write!(
            output,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width_px} {height_px}\" role=\"img\" data-series-state=\"{}\" data-identity=\"{}\">",
            self.presentation_state.as_str(),
            escape_xml(&self.identity_disclosure)
        );

        for gap in &self.gaps {
            let x_start = gap.x_start.min(gap.x_end);
            let width = (gap.x_end - gap.x_start).abs().max(1.0);
            let _ = write!(
                output,
                "<rect class=\"telemetry-gap telemetry-gap-{}\" x=\"{x_start:.2}\" y=\"0\" width=\"{width:.2}\" height=\"{height_px}\" data-reason=\"{}\" />",
                gap.kind.as_str(),
                escape_xml(&gap.reason)
            );
        }

        for segment in &self.segments {
            if segment.points.is_empty() {
                continue;
            }
            if segment.points.len() == 1 {
                let point = &segment.points[0];
                let _ = write!(
                    output,
                    "<circle class=\"telemetry-point telemetry-{}\" cx=\"{:.2}\" cy=\"{:.2}\" r=\"2\" />",
                    segment.state.as_str(),
                    point.x,
                    point.y
                );
            } else {
                let mut points = String::new();
                for (index, point) in segment.points.iter().enumerate() {
                    if index > 0 {
                        points.push(' ');
                    }
                    let _ = write!(points, "{:.2},{:.2}", point.x, point.y);
                }
                let _ = write!(
                    output,
                    "<polyline class=\"telemetry-segment telemetry-{}\" points=\"{points}\" />",
                    segment.state.as_str()
                );
            }
        }

        output.push_str("</svg>");
        output
    }
}

fn project_point(
    timestamp_unix_ms: u64,
    value: f64,
    timeline_start: Option<u64>,
    timeline_end: Option<u64>,
    y_min: Option<f64>,
    y_max: Option<f64>,
    options: ChartOptions,
) -> ChartPoint {
    let x = project_x(
        timestamp_unix_ms,
        timeline_start,
        timeline_end,
        options.width_px,
    );
    let y = match (y_min, y_max) {
        (Some(min), Some(max)) if max > min => {
            let normalized = (value - min) / (max - min);
            f64::from(options.height_px) * (1.0 - normalized.clamp(0.0, 1.0))
        }
        _ => f64::from(options.height_px) / 2.0,
    };
    ChartPoint {
        timestamp_unix_ms,
        value,
        x,
        y,
    }
}

fn project_x(
    timestamp_unix_ms: u64,
    timeline_start: Option<u64>,
    timeline_end: Option<u64>,
    width_px: u32,
) -> f64 {
    match (timeline_start, timeline_end) {
        (Some(start), Some(end)) if end > start => {
            let elapsed = timestamp_unix_ms.saturating_sub(start) as f64;
            let span = (end - start) as f64;
            (elapsed / span) * f64::from(width_px)
        }
        _ => 0.0,
    }
}

fn project_gap(
    kind: ChartGapKind,
    from_unix_ms: u64,
    to_unix_ms: u64,
    reason: impl Into<String>,
    timeline_start: Option<u64>,
    timeline_end: Option<u64>,
    width_px: u32,
) -> ChartGap {
    ChartGap {
        kind,
        from_unix_ms,
        to_unix_ms,
        x_start: project_x(from_unix_ms, timeline_start, timeline_end, width_px),
        x_end: project_x(to_unix_ms, timeline_start, timeline_end, width_px),
        reason: reason.into(),
    }
}

fn gap_for_state(state: &TimeSeriesState) -> Option<(ChartGapKind, &str)> {
    let reason = state.reason()?;
    let kind = match state {
        TimeSeriesState::Stale { .. } => ChartGapKind::Stale,
        TimeSeriesState::Disconnected { .. } => ChartGapKind::Disconnected,
        TimeSeriesState::Paused { .. } => ChartGapKind::Paused,
        TimeSeriesState::Unavailable { .. } => ChartGapKind::Unavailable,
        TimeSeriesState::Error { .. } => ChartGapKind::Error,
        TimeSeriesState::Reset { .. } => ChartGapKind::Reset,
        TimeSeriesState::Live => return None,
    };
    Some((kind, reason))
}

fn flush_segment(
    current: &mut Option<(RenderClass, Vec<ChartPoint>)>,
    segments: &mut Vec<ChartSegment>,
    width_px: u32,
) {
    if let Some((state, points)) = current.take() {
        if !points.is_empty() {
            segments.push(ChartSegment {
                state: state.into(),
                points: decimate_for_pixels(points, width_px),
            });
        }
    }
}

fn decimate_for_pixels(points: Vec<ChartPoint>, width_px: u32) -> Vec<ChartPoint> {
    let budget = usize::try_from(width_px)
        .unwrap_or(usize::MAX / 2)
        .saturating_mul(2)
        .max(2);
    if points.len() <= budget {
        return points;
    }

    let mut output = Vec::with_capacity(budget.saturating_add(2));
    let mut index = 0;
    while index < points.len() {
        let bucket = points[index].x.floor() as i64;
        let start = index;
        while index < points.len() && points[index].x.floor() as i64 == bucket {
            index += 1;
        }
        let slice = &points[start..index];
        let min = slice
            .iter()
            .min_by(|left, right| left.value.total_cmp(&right.value))
            .expect("slice is non-empty");
        let max = slice
            .iter()
            .max_by(|left, right| left.value.total_cmp(&right.value))
            .expect("slice is non-empty");

        if min.timestamp_unix_ms <= max.timestamp_unix_ms {
            output.push(min.clone());
            if min.timestamp_unix_ms != max.timestamp_unix_ms {
                output.push(max.clone());
            }
        } else {
            output.push(max.clone());
            output.push(min.clone());
        }
    }
    output
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterPolicy {
    pub inclusive_max: Option<u64>,
}

impl CounterPolicy {
    pub fn monotonic() -> Self {
        Self {
            inclusive_max: None,
        }
    }

    pub fn wrapping(inclusive_max: u64) -> Self {
        Self {
            inclusive_max: Some(inclusive_max),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CounterObservation {
    First { value: u64 },
    Delta { delta: u64 },
    Wrapped { delta: u64 },
    Reset { previous: u64, current: u64 },
}

#[derive(Debug, Clone)]
pub struct CounterTracker {
    policy: CounterPolicy,
    previous: Option<u64>,
}

impl CounterTracker {
    pub fn new(policy: CounterPolicy) -> Self {
        Self {
            policy,
            previous: None,
        }
    }

    pub fn observe(&mut self, current: u64) -> CounterObservation {
        let Some(previous) = self.previous.replace(current) else {
            return CounterObservation::First { value: current };
        };

        if current >= previous {
            return CounterObservation::Delta {
                delta: current - previous,
            };
        }

        match self.policy.inclusive_max {
            Some(maximum) if previous <= maximum && current <= maximum => {
                CounterObservation::Wrapped {
                    delta: maximum
                        .saturating_sub(previous)
                        .saturating_add(1)
                        .saturating_add(current),
                }
            }
            _ => CounterObservation::Reset { previous, current },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(identity: &str) -> SeriesKey {
        SeriesKey::new(
            "gpu.utilization",
            "percent",
            "nvidia-nvml",
            "nvmlDeviceGetUtilizationRates",
            SeriesIdentity::new("gpu-uuid", identity),
        )
    }

    fn test_policy(max_points: usize) -> HistoryPolicy {
        HistoryPolicy {
            retention_ms: 60_000,
            raw_retention_ms: 1_000,
            downsample_bucket_ms: 500,
            max_points_per_series: max_points,
        }
    }

    #[test]
    fn history_is_bounded_and_downsamples_old_live_samples() {
        let key = key("GPU-1");
        let source = SampleSource::from_key(&key);
        let mut series = TimeSeries::new(key, test_policy(32)).unwrap();

        for index in 0..200_u64 {
            series
                .push(TimeSeriesSample::live(index * 100, index as f64, source.clone()).unwrap())
                .unwrap();
        }

        assert!(series.len() <= 32);
        assert_eq!(series.samples().back().unwrap().timestamp_unix_ms, 19_900);
    }

    #[test]
    fn markers_survive_downsampling_until_retention_or_hard_cap_requires_eviction() {
        let key = key("GPU-1");
        let source = SampleSource::from_key(&key);
        let mut series = TimeSeries::new(key, test_policy(64)).unwrap();

        for index in 0..20_u64 {
            series
                .push(TimeSeriesSample::live(index * 100, 50.0, source.clone()).unwrap())
                .unwrap();
        }
        series
            .push(TimeSeriesSample::disconnected(
                2_100,
                source.clone(),
                "server stopped",
            ))
            .unwrap();
        for index in 22..40_u64 {
            series
                .push(TimeSeriesSample::live(index * 100, 55.0, source.clone()).unwrap())
                .unwrap();
        }

        assert!(
            series
                .samples()
                .iter()
                .any(|sample| matches!(sample.state, TimeSeriesState::Disconnected { .. }))
        );
    }

    #[test]
    fn store_never_mixes_incompatible_identities() {
        let mut store = TimeSeriesStore::new(test_policy(64)).unwrap();
        let first = key("GPU-1");
        let second = key("GPU-2");

        store
            .push(
                first.clone(),
                TimeSeriesSample::live(1_000, 10.0, SampleSource::from_key(&first)).unwrap(),
            )
            .unwrap();
        store
            .push(
                second.clone(),
                TimeSeriesSample::live(1_000, 90.0, SampleSource::from_key(&second)).unwrap(),
            )
            .unwrap();

        assert_eq!(store.len(), 2);
        assert_eq!(store.series_for_metric("gpu.utilization").count(), 2);
        assert_eq!(
            store
                .series(&first)
                .unwrap()
                .samples()
                .back()
                .unwrap()
                .value,
            Some(10.0)
        );
        assert_eq!(
            store
                .series(&second)
                .unwrap()
                .samples()
                .back()
                .unwrap()
                .value,
            Some(90.0)
        );
    }

    #[test]
    fn disconnect_and_reconnect_create_separate_chart_segments() {
        let key = key("GPU-1");
        let source = SampleSource::from_key(&key);
        let mut series = TimeSeries::new(key, test_policy(64)).unwrap();
        series
            .push(TimeSeriesSample::live(1_000, 10.0, source.clone()).unwrap())
            .unwrap();
        series
            .push(TimeSeriesSample::live(2_000, 20.0, source.clone()).unwrap())
            .unwrap();
        series
            .push(TimeSeriesSample::disconnected(
                2_500,
                source.clone(),
                "provider disconnected",
            ))
            .unwrap();
        series
            .push(TimeSeriesSample::live(4_000, 40.0, source).unwrap())
            .unwrap();

        let projection = series
            .project(ChartOptions {
                width_px: 400,
                height_px: 100,
                missing_gap_after_ms: None,
            })
            .unwrap();

        assert_eq!(projection.segments.len(), 2);
        assert!(
            projection
                .gaps
                .iter()
                .any(|gap| gap.kind == ChartGapKind::Disconnected)
        );
        assert_eq!(projection.segments[0].points.len(), 2);
        assert_eq!(projection.segments[1].points.len(), 1);
    }

    #[test]
    fn stale_values_render_as_a_distinct_segment() {
        let key = key("GPU-1");
        let source = SampleSource::from_key(&key);
        let mut series = TimeSeries::new(key, test_policy(64)).unwrap();
        series
            .push(TimeSeriesSample::live(1_000, 10.0, source.clone()).unwrap())
            .unwrap();
        series
            .push(
                TimeSeriesSample::stale(
                    2_000,
                    Some(10.0),
                    source,
                    Some(1_000),
                    "last provider sample is stale",
                )
                .unwrap(),
            )
            .unwrap();

        let projection = series.project(ChartOptions::default()).unwrap();
        assert_eq!(projection.segments.len(), 2);
        assert_eq!(projection.segments[0].state, ChartSegmentState::Live);
        assert_eq!(projection.segments[1].state, ChartSegmentState::Stale);
        assert_eq!(
            projection.presentation_state,
            SeriesPresentationState::Stale
        );
    }

    #[test]
    fn missing_interval_breaks_the_line_without_an_explicit_marker() {
        let key = key("GPU-1");
        let source = SampleSource::from_key(&key);
        let mut series = TimeSeries::new(key, test_policy(64)).unwrap();
        series
            .push(TimeSeriesSample::live(1_000, 10.0, source.clone()).unwrap())
            .unwrap();
        series
            .push(TimeSeriesSample::live(10_000, 20.0, source).unwrap())
            .unwrap();

        let projection = series
            .project(ChartOptions {
                width_px: 400,
                height_px: 100,
                missing_gap_after_ms: Some(2_000),
            })
            .unwrap();

        assert_eq!(projection.segments.len(), 2);
        assert_eq!(projection.gaps.len(), 1);
        assert_eq!(projection.gaps[0].kind, ChartGapKind::MissingSamples);
    }

    #[test]
    fn pixel_decimation_preserves_extrema_and_bounds_render_points() {
        let key = key("GPU-1");
        let source = SampleSource::from_key(&key);
        let policy = HistoryPolicy {
            retention_ms: 1_000_000,
            raw_retention_ms: 1_000_000,
            downsample_bucket_ms: 5_000,
            max_points_per_series: 8_192,
        };
        let mut series = TimeSeries::new(key, policy).unwrap();

        for index in 0..4_000_u64 {
            let value = if index == 2_000 { 100.0 } else { 10.0 };
            series
                .push(TimeSeriesSample::live(index, value, source.clone()).unwrap())
                .unwrap();
        }

        let projection = series
            .project(ChartOptions {
                width_px: 200,
                height_px: 100,
                missing_gap_after_ms: None,
            })
            .unwrap();

        let points = &projection.segments[0].points;
        assert!(points.len() <= 402);
        assert!(points.iter().any(|point| point.value == 100.0));
    }

    #[test]
    fn counter_tracker_reports_reset_instead_of_negative_delta() {
        let mut tracker = CounterTracker::new(CounterPolicy::monotonic());
        assert_eq!(
            tracker.observe(100),
            CounterObservation::First { value: 100 }
        );
        assert_eq!(
            tracker.observe(120),
            CounterObservation::Delta { delta: 20 }
        );
        assert_eq!(
            tracker.observe(5),
            CounterObservation::Reset {
                previous: 120,
                current: 5
            }
        );
    }

    #[test]
    fn counter_tracker_can_handle_known_wrap_modulus() {
        let mut tracker = CounterTracker::new(CounterPolicy::wrapping(255));
        assert_eq!(
            tracker.observe(250),
            CounterObservation::First { value: 250 }
        );
        assert_eq!(tracker.observe(3), CounterObservation::Wrapped { delta: 9 });
    }

    #[test]
    fn svg_exposes_gap_stale_and_identity_semantics() {
        let key = key("GPU<&1");
        let source = SampleSource::from_key(&key);
        let mut series = TimeSeries::new(key, test_policy(64)).unwrap();
        series
            .push(TimeSeriesSample::live(1_000, 10.0, source.clone()).unwrap())
            .unwrap();
        series
            .push(TimeSeriesSample::disconnected(
                2_000,
                source.clone(),
                "driver <restart>",
            ))
            .unwrap();
        series
            .push(
                TimeSeriesSample::stale(3_000, Some(10.0), source, Some(1_000), "waiting & stale")
                    .unwrap(),
            )
            .unwrap();

        let projection = series.project(ChartOptions::default()).unwrap();
        let svg = projection.to_svg();

        assert!(svg.contains("telemetry-gap-disconnected"));
        assert!(svg.contains("telemetry-stale"));
        assert!(svg.contains("gpu-uuid:GPU&lt;&amp;1"));
        assert!(svg.contains("driver &lt;restart&gt;"));
        assert!(!svg.contains("<restart>"));
    }
}
