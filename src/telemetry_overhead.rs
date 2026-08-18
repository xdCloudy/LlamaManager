use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_IDLE_CPU_BUDGET_PERCENT_TOTAL_CAPACITY: f64 = 1.0;
pub const DEFAULT_ACTIVE_CPU_BUDGET_PERCENT_TOTAL_CAPACITY: f64 = 2.0;
pub const DEFAULT_PRIVATE_MEMORY_GROWTH_BUDGET_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_P95_POLL_FRACTION_OF_CADENCE: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverheadPhase {
    Idle,
    ActiveInference,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OverheadBudget {
    pub max_idle_cpu_percent_total_capacity: f64,
    pub max_active_cpu_percent_total_capacity: f64,
    pub max_peak_private_growth_bytes: u64,
    pub max_p95_poll_fraction_of_cadence: f64,
}

impl Default for OverheadBudget {
    fn default() -> Self {
        Self {
            max_idle_cpu_percent_total_capacity: DEFAULT_IDLE_CPU_BUDGET_PERCENT_TOTAL_CAPACITY,
            max_active_cpu_percent_total_capacity: DEFAULT_ACTIVE_CPU_BUDGET_PERCENT_TOTAL_CAPACITY,
            max_peak_private_growth_bytes: DEFAULT_PRIVATE_MEMORY_GROWTH_BUDGET_BYTES,
            max_p95_poll_fraction_of_cadence: DEFAULT_P95_POLL_FRACTION_OF_CADENCE,
        }
    }
}

impl OverheadBudget {
    pub fn validate(self) -> Result<Self, OverheadError> {
        for (name, value) in [
            (
                "max_idle_cpu_percent_total_capacity",
                self.max_idle_cpu_percent_total_capacity,
            ),
            (
                "max_active_cpu_percent_total_capacity",
                self.max_active_cpu_percent_total_capacity,
            ),
            (
                "max_p95_poll_fraction_of_cadence",
                self.max_p95_poll_fraction_of_cadence,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(OverheadError::InvalidBudget(format!(
                    "{name} must be finite and non-negative"
                )));
            }
        }
        if self.max_p95_poll_fraction_of_cadence > 1.0 {
            return Err(OverheadError::InvalidBudget(
                "max_p95_poll_fraction_of_cadence cannot exceed 1.0".to_owned(),
            ));
        }
        Ok(self)
    }

    pub fn cpu_limit_for(self, phase: OverheadPhase) -> f64 {
        match phase {
            OverheadPhase::Idle => self.max_idle_cpu_percent_total_capacity,
            OverheadPhase::ActiveInference => self.max_active_cpu_percent_total_capacity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessResourceSample {
    pub captured_at_unix_ms: u64,
    pub process_cpu_time_100ns: u64,
    pub private_bytes: u64,
    pub working_set_bytes: u64,
    pub logical_processor_count: u32,
}

pub fn capture_current_process_resources() -> Result<ProcessResourceSample, OverheadError> {
    platform::capture_current_process_resources()
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PollTimingSample {
    pub started_at_unix_ms: u64,
    pub duration_ms: f64,
}

impl PollTimingSample {
    pub fn from_duration(started_at_unix_ms: u64, duration: Duration) -> Self {
        Self {
            started_at_unix_ms,
            duration_ms: duration.as_secs_f64() * 1_000.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetMetric {
    CpuPercentTotalCapacity,
    PeakPrivateGrowthBytes,
    P95PollFractionOfCadence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetViolation {
    pub metric: BudgetMetric,
    pub observed: f64,
    pub limit: f64,
    pub unit: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetAssessment {
    pub within_budget: bool,
    pub violations: Vec<BudgetViolation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryOverheadMeasurement {
    pub phase: OverheadPhase,
    pub cadence_ms: u64,
    pub elapsed_ms: u64,
    pub poll_sample_count: usize,
    pub logical_processor_count: u32,
    pub process_cpu_percent_total_capacity: f64,
    pub mean_poll_duration_ms: f64,
    pub p95_poll_duration_ms: f64,
    pub max_poll_duration_ms: f64,
    pub p95_poll_fraction_of_cadence: f64,
    pub start_private_bytes: u64,
    pub end_private_bytes: u64,
    pub peak_private_bytes: u64,
    pub peak_private_growth_bytes: u64,
    pub start_working_set_bytes: u64,
    pub end_working_set_bytes: u64,
    pub peak_working_set_bytes: u64,
    pub budget: OverheadBudget,
    pub assessment: BudgetAssessment,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum OverheadError {
    #[error("invalid telemetry overhead budget: {0}")]
    InvalidBudget(String),
    #[error("telemetry sampling cadence must be greater than zero")]
    InvalidCadence,
    #[error("at least one polling timing sample is required")]
    NoPollSamples,
    #[error("logical processor count must be greater than zero")]
    InvalidLogicalProcessorCount,
    #[error("logical processor count changed during measurement: start={start}, end={end}")]
    LogicalProcessorCountChanged { start: u32, end: u32 },
    #[error("process CPU counter moved backwards: start={start}, end={end}")]
    CpuCounterMovedBackwards { start: u64, end: u64 },
    #[error("wall clock moved backwards: start={start}, end={end}")]
    ClockMovedBackwards { start: u64, end: u64 },
    #[error("measurement duration must be greater than zero")]
    ZeroDuration,
    #[error("Windows process resource probe failed: {0}")]
    Platform(String),
    #[error("process resource probe is unsupported on this operating system")]
    UnsupportedPlatform,
}

#[derive(Debug, Clone)]
pub struct TelemetryOverheadRecorder {
    phase: OverheadPhase,
    cadence_ms: u64,
    budget: OverheadBudget,
    start: ProcessResourceSample,
    last: ProcessResourceSample,
    peak_private_bytes: u64,
    peak_working_set_bytes: u64,
    polls: Vec<PollTimingSample>,
}

impl TelemetryOverheadRecorder {
    pub fn new(
        phase: OverheadPhase,
        cadence: Duration,
        budget: OverheadBudget,
        start: ProcessResourceSample,
    ) -> Result<Self, OverheadError> {
        let cadence_ms = u64::try_from(cadence.as_millis()).unwrap_or(u64::MAX);
        if cadence_ms == 0 {
            return Err(OverheadError::InvalidCadence);
        }
        if start.logical_processor_count == 0 {
            return Err(OverheadError::InvalidLogicalProcessorCount);
        }
        Ok(Self {
            phase,
            cadence_ms,
            budget: budget.validate()?,
            start,
            last: start,
            peak_private_bytes: start.private_bytes,
            peak_working_set_bytes: start.working_set_bytes,
            polls: Vec::new(),
        })
    }

    pub fn record_poll(
        &mut self,
        timing: PollTimingSample,
        resources: ProcessResourceSample,
    ) -> Result<(), OverheadError> {
        validate_resource_progression(self.last, resources)?;
        if !timing.duration_ms.is_finite() || timing.duration_ms < 0.0 {
            return Err(OverheadError::InvalidBudget(
                "poll duration must be finite and non-negative".to_owned(),
            ));
        }
        self.peak_private_bytes = self.peak_private_bytes.max(resources.private_bytes);
        self.peak_working_set_bytes = self.peak_working_set_bytes.max(resources.working_set_bytes);
        self.last = resources;
        self.polls.push(timing);
        Ok(())
    }

    pub fn finish(
        mut self,
        end: ProcessResourceSample,
    ) -> Result<TelemetryOverheadMeasurement, OverheadError> {
        validate_resource_progression(self.last, end)?;
        if self.polls.is_empty() {
            return Err(OverheadError::NoPollSamples);
        }
        self.peak_private_bytes = self.peak_private_bytes.max(end.private_bytes);
        self.peak_working_set_bytes = self.peak_working_set_bytes.max(end.working_set_bytes);

        let elapsed_ms = end
            .captured_at_unix_ms
            .checked_sub(self.start.captured_at_unix_ms)
            .ok_or(OverheadError::ClockMovedBackwards {
                start: self.start.captured_at_unix_ms,
                end: end.captured_at_unix_ms,
            })?;
        if elapsed_ms == 0 {
            return Err(OverheadError::ZeroDuration);
        }
        let cpu_delta_100ns = end
            .process_cpu_time_100ns
            .checked_sub(self.start.process_cpu_time_100ns)
            .ok_or(OverheadError::CpuCounterMovedBackwards {
                start: self.start.process_cpu_time_100ns,
                end: end.process_cpu_time_100ns,
            })?;
        let elapsed_100ns = (elapsed_ms as f64) * 10_000.0;
        let cpu_percent_total_capacity = (cpu_delta_100ns as f64 / elapsed_100ns)
            / f64::from(self.start.logical_processor_count)
            * 100.0;

        let mut poll_durations: Vec<f64> =
            self.polls.iter().map(|sample| sample.duration_ms).collect();
        poll_durations.sort_by(f64::total_cmp);
        let sum: f64 = poll_durations.iter().sum();
        let mean_poll_duration_ms = sum / poll_durations.len() as f64;
        let p95_poll_duration_ms = percentile_nearest_rank(&poll_durations, 0.95);
        let max_poll_duration_ms = *poll_durations
            .last()
            .expect("non-empty poll durations checked above");
        let p95_poll_fraction_of_cadence = p95_poll_duration_ms / self.cadence_ms as f64;
        let peak_private_growth_bytes = self
            .peak_private_bytes
            .saturating_sub(self.start.private_bytes);

        let assessment = assess_budget(
            self.phase,
            self.budget,
            cpu_percent_total_capacity,
            peak_private_growth_bytes,
            p95_poll_fraction_of_cadence,
        );

        Ok(TelemetryOverheadMeasurement {
            phase: self.phase,
            cadence_ms: self.cadence_ms,
            elapsed_ms,
            poll_sample_count: poll_durations.len(),
            logical_processor_count: self.start.logical_processor_count,
            process_cpu_percent_total_capacity: cpu_percent_total_capacity,
            mean_poll_duration_ms,
            p95_poll_duration_ms,
            max_poll_duration_ms,
            p95_poll_fraction_of_cadence,
            start_private_bytes: self.start.private_bytes,
            end_private_bytes: end.private_bytes,
            peak_private_bytes: self.peak_private_bytes,
            peak_private_growth_bytes,
            start_working_set_bytes: self.start.working_set_bytes,
            end_working_set_bytes: end.working_set_bytes,
            peak_working_set_bytes: self.peak_working_set_bytes,
            budget: self.budget,
            assessment,
        })
    }
}

fn validate_resource_progression(
    start: ProcessResourceSample,
    current: ProcessResourceSample,
) -> Result<(), OverheadError> {
    if current.logical_processor_count == 0 {
        return Err(OverheadError::InvalidLogicalProcessorCount);
    }
    if current.logical_processor_count != start.logical_processor_count {
        return Err(OverheadError::LogicalProcessorCountChanged {
            start: start.logical_processor_count,
            end: current.logical_processor_count,
        });
    }
    if current.captured_at_unix_ms < start.captured_at_unix_ms {
        return Err(OverheadError::ClockMovedBackwards {
            start: start.captured_at_unix_ms,
            end: current.captured_at_unix_ms,
        });
    }
    if current.process_cpu_time_100ns < start.process_cpu_time_100ns {
        return Err(OverheadError::CpuCounterMovedBackwards {
            start: start.process_cpu_time_100ns,
            end: current.process_cpu_time_100ns,
        });
    }
    Ok(())
}

fn percentile_nearest_rank(sorted_values: &[f64], percentile: f64) -> f64 {
    debug_assert!(!sorted_values.is_empty());
    let rank = (percentile * sorted_values.len() as f64).ceil() as usize;
    sorted_values[rank.saturating_sub(1).min(sorted_values.len() - 1)]
}

fn assess_budget(
    phase: OverheadPhase,
    budget: OverheadBudget,
    cpu_percent_total_capacity: f64,
    peak_private_growth_bytes: u64,
    p95_poll_fraction_of_cadence: f64,
) -> BudgetAssessment {
    let mut violations = Vec::new();
    let cpu_limit = budget.cpu_limit_for(phase);
    if cpu_percent_total_capacity > cpu_limit {
        violations.push(BudgetViolation {
            metric: BudgetMetric::CpuPercentTotalCapacity,
            observed: cpu_percent_total_capacity,
            limit: cpu_limit,
            unit: "percent_total_host_cpu_capacity".to_owned(),
            reason: format!("monitor process CPU exceeded the {:?} phase budget", phase),
        });
    }
    if peak_private_growth_bytes > budget.max_peak_private_growth_bytes {
        violations.push(BudgetViolation {
            metric: BudgetMetric::PeakPrivateGrowthBytes,
            observed: peak_private_growth_bytes as f64,
            limit: budget.max_peak_private_growth_bytes as f64,
            unit: "bytes".to_owned(),
            reason: "monitor process private-memory growth exceeded the configured budget"
                .to_owned(),
        });
    }
    if p95_poll_fraction_of_cadence > budget.max_p95_poll_fraction_of_cadence {
        violations.push(BudgetViolation {
            metric: BudgetMetric::P95PollFractionOfCadence,
            observed: p95_poll_fraction_of_cadence,
            limit: budget.max_p95_poll_fraction_of_cadence,
            unit: "fraction_of_sampling_cadence".to_owned(),
            reason: "p95 telemetry polling work consumed too much of the sampling interval"
                .to_owned(),
        });
    }

    BudgetAssessment {
        within_budget: violations.is_empty(),
        violations,
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(windows)]
mod platform {
    use std::{ffi::c_void, io, mem};

    use super::{OverheadError, ProcessResourceSample, now_unix_ms};

    const ALL_PROCESSOR_GROUPS: u16 = 0xffff;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileTime {
        low_date_time: u32,
        high_date_time: u32,
    }

    impl FileTime {
        fn as_u64(self) -> u64 {
            (u64::from(self.high_date_time) << 32) | u64::from(self.low_date_time)
        }
    }

    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCountersEx {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessTimes(
            process: *mut c_void,
            creation_time: *mut FileTime,
            exit_time: *mut FileTime,
            kernel_time: *mut FileTime,
            user_time: *mut FileTime,
        ) -> i32;
        fn GetActiveProcessorCount(group_number: u16) -> u32;
        fn K32GetProcessMemoryInfo(process: *mut c_void, counters: *mut c_void, cb: u32) -> i32;
    }

    pub(super) fn capture_current_process_resources() -> Result<ProcessResourceSample, OverheadError>
    {
        // SAFETY: GetCurrentProcess returns a process pseudo-handle valid in the current process.
        let process = unsafe { GetCurrentProcess() };
        if process.is_null() {
            return Err(OverheadError::Platform(
                "GetCurrentProcess returned a null pseudo-handle".to_owned(),
            ));
        }

        let mut creation_time = FileTime::default();
        let mut exit_time = FileTime::default();
        let mut kernel_time = FileTime::default();
        let mut user_time = FileTime::default();
        // SAFETY: all FILETIME pointers refer to initialized writable storage and `process` is the
        // current-process pseudo-handle returned by GetCurrentProcess.
        let times_ok = unsafe {
            GetProcessTimes(
                process,
                &mut creation_time,
                &mut exit_time,
                &mut kernel_time,
                &mut user_time,
            )
        };
        if times_ok == 0 {
            return Err(OverheadError::Platform(format!(
                "GetProcessTimes failed: {}",
                io::Error::last_os_error()
            )));
        }

        let mut counters = ProcessMemoryCountersEx {
            cb: mem::size_of::<ProcessMemoryCountersEx>() as u32,
            ..ProcessMemoryCountersEx::default()
        };
        // SAFETY: `counters` is a writable repr(C) structure with the documented size field and the
        // current-process pseudo-handle remains valid for the call.
        let memory_ok = unsafe {
            K32GetProcessMemoryInfo(
                process,
                (&mut counters as *mut ProcessMemoryCountersEx).cast(),
                counters.cb,
            )
        };
        if memory_ok == 0 {
            return Err(OverheadError::Platform(format!(
                "K32GetProcessMemoryInfo(current process) failed: {}",
                io::Error::last_os_error()
            )));
        }

        // SAFETY: GetActiveProcessorCount has no pointer arguments and ALL_PROCESSOR_GROUPS is the
        // documented sentinel for all processor groups.
        let logical_processor_count = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) };
        if logical_processor_count == 0 {
            return Err(OverheadError::Platform(format!(
                "GetActiveProcessorCount failed: {}",
                io::Error::last_os_error()
            )));
        }

        Ok(ProcessResourceSample {
            captured_at_unix_ms: now_unix_ms(),
            process_cpu_time_100ns: kernel_time.as_u64().saturating_add(user_time.as_u64()),
            private_bytes: counters.private_usage as u64,
            working_set_bytes: counters.working_set_size as u64,
            logical_processor_count,
        })
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{OverheadError, ProcessResourceSample};

    pub(super) fn capture_current_process_resources() -> Result<ProcessResourceSample, OverheadError>
    {
        Err(OverheadError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(
        timestamp_ms: u64,
        cpu_100ns: u64,
        private_bytes: u64,
        working_set_bytes: u64,
    ) -> ProcessResourceSample {
        ProcessResourceSample {
            captured_at_unix_ms: timestamp_ms,
            process_cpu_time_100ns: cpu_100ns,
            private_bytes,
            working_set_bytes,
            logical_processor_count: 20,
        }
    }

    #[test]
    fn cpu_is_normalized_to_total_host_capacity() {
        let start = sample(1_000, 1_000_000, 100, 200);
        let mut recorder = TelemetryOverheadRecorder::new(
            OverheadPhase::Idle,
            Duration::from_secs(1),
            OverheadBudget {
                max_idle_cpu_percent_total_capacity: 10.0,
                ..OverheadBudget::default()
            },
            start,
        )
        .unwrap();
        recorder
            .record_poll(
                PollTimingSample {
                    started_at_unix_ms: 1_000,
                    duration_ms: 1.0,
                },
                sample(6_000, 2_000_000, 120, 230),
            )
            .unwrap();

        let measurement = recorder
            .finish(sample(11_000, 3_000_000, 110, 220))
            .unwrap();
        // 2,000,000 * 100ns = 200ms process CPU over 10s, divided across 20 logical CPUs.
        assert!((measurement.process_cpu_percent_total_capacity - 0.1).abs() < 1e-9);
    }

    #[test]
    fn p95_poll_duration_uses_nearest_rank_and_budget_fraction() {
        let start = sample(0, 0, 1_000, 2_000);
        let mut recorder = TelemetryOverheadRecorder::new(
            OverheadPhase::Idle,
            Duration::from_millis(100),
            OverheadBudget {
                max_idle_cpu_percent_total_capacity: 100.0,
                max_p95_poll_fraction_of_cadence: 0.5,
                ..OverheadBudget::default()
            },
            start,
        )
        .unwrap();
        for index in 0..20_u64 {
            let duration_ms = if index == 19 {
                80.0
            } else {
                (index + 1) as f64
            };
            recorder
                .record_poll(
                    PollTimingSample {
                        started_at_unix_ms: index * 100,
                        duration_ms,
                    },
                    sample(index * 100 + 1, index * 10, 1_000, 2_000),
                )
                .unwrap();
        }
        let measurement = recorder.finish(sample(2_100, 200, 1_000, 2_000)).unwrap();
        assert_eq!(measurement.p95_poll_duration_ms, 19.0);
        assert!((measurement.p95_poll_fraction_of_cadence - 0.19).abs() < 1e-9);
        assert!(measurement.assessment.within_budget);
    }

    #[test]
    fn assessment_names_each_budget_violation() {
        let budget = OverheadBudget {
            max_idle_cpu_percent_total_capacity: 0.01,
            max_active_cpu_percent_total_capacity: 0.01,
            max_peak_private_growth_bytes: 50,
            max_p95_poll_fraction_of_cadence: 0.01,
        };
        let start = sample(1_000, 0, 1_000, 2_000);
        let mut recorder = TelemetryOverheadRecorder::new(
            OverheadPhase::ActiveInference,
            Duration::from_millis(100),
            budget,
            start,
        )
        .unwrap();
        recorder
            .record_poll(
                PollTimingSample {
                    started_at_unix_ms: 1_100,
                    duration_ms: 20.0,
                },
                sample(1_100, 500_000, 2_000, 3_000),
            )
            .unwrap();
        let measurement = recorder
            .finish(sample(2_000, 1_000_000, 1_500, 2_500))
            .unwrap();
        assert!(!measurement.assessment.within_budget);
        assert_eq!(measurement.assessment.violations.len(), 3);
        assert!(
            measurement
                .assessment
                .violations
                .iter()
                .any(|item| item.metric == BudgetMetric::CpuPercentTotalCapacity)
        );
        assert!(
            measurement
                .assessment
                .violations
                .iter()
                .any(|item| item.metric == BudgetMetric::PeakPrivateGrowthBytes)
        );
        assert!(
            measurement
                .assessment
                .violations
                .iter()
                .any(|item| item.metric == BudgetMetric::P95PollFractionOfCadence)
        );
    }

    #[test]
    fn invalid_budget_and_counter_regressions_are_rejected() {
        assert!(
            OverheadBudget {
                max_p95_poll_fraction_of_cadence: 1.1,
                ..OverheadBudget::default()
            }
            .validate()
            .is_err()
        );

        let start = sample(1_000, 100, 1_000, 2_000);
        let mut recorder = TelemetryOverheadRecorder::new(
            OverheadPhase::Idle,
            Duration::from_secs(1),
            OverheadBudget::default(),
            start,
        )
        .unwrap();
        assert_eq!(
            recorder.record_poll(
                PollTimingSample {
                    started_at_unix_ms: 1_100,
                    duration_ms: 1.0,
                },
                sample(1_100, 99, 1_000, 2_000),
            ),
            Err(OverheadError::CpuCounterMovedBackwards {
                start: 100,
                end: 99
            })
        );
    }

    #[test]
    fn intermediate_resource_regressions_are_rejected() {
        let start = sample(1_000, 100, 1_000, 2_000);
        let mut recorder = TelemetryOverheadRecorder::new(
            OverheadPhase::Idle,
            Duration::from_secs(1),
            OverheadBudget::default(),
            start,
        )
        .unwrap();
        recorder
            .record_poll(
                PollTimingSample {
                    started_at_unix_ms: 2_000,
                    duration_ms: 1.0,
                },
                sample(2_000, 200, 1_000, 2_000),
            )
            .unwrap();

        assert_eq!(
            recorder.record_poll(
                PollTimingSample {
                    started_at_unix_ms: 2_100,
                    duration_ms: 1.0,
                },
                sample(1_500, 250, 1_000, 2_000),
            ),
            Err(OverheadError::ClockMovedBackwards {
                start: 2_000,
                end: 1_500
            })
        );
        assert_eq!(
            recorder.record_poll(
                PollTimingSample {
                    started_at_unix_ms: 2_100,
                    duration_ms: 1.0,
                },
                sample(2_100, 150, 1_000, 2_000),
            ),
            Err(OverheadError::CpuCounterMovedBackwards {
                start: 200,
                end: 150
            })
        );

        let mut finish_recorder = TelemetryOverheadRecorder::new(
            OverheadPhase::Idle,
            Duration::from_secs(1),
            OverheadBudget::default(),
            start,
        )
        .unwrap();
        finish_recorder
            .record_poll(
                PollTimingSample {
                    started_at_unix_ms: 2_000,
                    duration_ms: 1.0,
                },
                sample(2_000, 200, 1_000, 2_000),
            )
            .unwrap();
        assert_eq!(
            finish_recorder.finish(sample(1_500, 250, 1_000, 2_000)),
            Err(OverheadError::ClockMovedBackwards {
                start: 2_000,
                end: 1_500
            })
        );
    }
}
