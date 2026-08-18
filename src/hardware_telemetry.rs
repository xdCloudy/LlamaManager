use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const MIN_SAMPLING_INTERVAL_MS: u64 = 250;
pub const DEFAULT_SAMPLING_INTERVAL_MS: u64 = 1_000;
pub const MAX_SAMPLING_INTERVAL_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryUnit {
    Percent,
    Bytes,
    Count,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetrySource {
    pub provider: String,
    pub api: String,
}

impl TelemetrySource {
    fn windows(api: impl Into<String>) -> Self {
        Self {
            provider: "windows-native".to_owned(),
            api: api.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TelemetryState<T> {
    Live {
        value: T,
    },
    Unavailable {
        reason: String,
    },
    Error {
        message: String,
    },
    Stale {
        last_value: Option<T>,
        last_observed_at_unix_ms: Option<u64>,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryReading<T> {
    pub state: TelemetryState<T>,
    pub unit: TelemetryUnit,
    pub source: TelemetrySource,
    pub sampled_at_unix_ms: u64,
}

impl<T> TelemetryReading<T> {
    pub fn live_value(&self) -> Option<&T> {
        match &self.state {
            TelemetryState::Live { value } => Some(value),
            _ => None,
        }
    }

    pub fn is_live(&self) -> bool {
        matches!(self.state, TelemetryState::Live { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamplingCadence {
    interval_ms: u64,
}

impl Default for SamplingCadence {
    fn default() -> Self {
        Self {
            interval_ms: DEFAULT_SAMPLING_INTERVAL_MS,
        }
    }
}

impl SamplingCadence {
    pub fn bounded(requested: Duration) -> Self {
        let requested_ms = requested.as_millis();
        let bounded_ms = requested_ms.clamp(
            u128::from(MIN_SAMPLING_INTERVAL_MS),
            u128::from(MAX_SAMPLING_INTERVAL_MS),
        );
        Self {
            interval_ms: bounded_ms as u64,
        }
    }

    pub fn interval(self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }

    pub fn interval_ms(self) -> u64 {
        self.interval_ms
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CpuTelemetry {
    pub total_usage_percent: TelemetryReading<f64>,
    pub per_logical_processor_usage_percent: TelemetryReading<Vec<f64>>,
    pub logical_processor_count: TelemetryReading<u32>,
    pub physical_core_count: TelemetryReading<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryTelemetry {
    pub total_physical_bytes: TelemetryReading<u64>,
    pub available_physical_bytes: TelemetryReading<u64>,
    pub used_physical_bytes: TelemetryReading<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessMemoryMetrics {
    pub pid: u32,
    pub working_set_bytes: u64,
    pub peak_working_set_bytes: u64,
    pub private_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareTelemetrySnapshot {
    pub captured_at_unix_ms: u64,
    pub cadence_ms: u64,
    pub cpu: CpuTelemetry,
    pub memory: MemoryTelemetry,
    pub managed_process_memory: TelemetryReading<ProcessMemoryMetrics>,
}

pub trait HardwareTelemetryProvider {
    fn cadence(&self) -> SamplingCadence;
    fn set_cadence(&mut self, requested: Duration);
    fn sample(&mut self, managed_process_pid: Option<u32>) -> HardwareTelemetrySnapshot;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuTimes {
    idle: u64,
    kernel: u64,
    user: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct CpuUsageDelta {
    total_percent: f64,
    per_logical_processor_percent: Vec<f64>,
}

#[derive(Debug)]
pub struct WindowsHardwareTelemetryProvider {
    cadence: SamplingCadence,
    previous_cpu_times: Option<Vec<CpuTimes>>,
    previous_snapshot: Option<HardwareTelemetrySnapshot>,
}

impl Default for WindowsHardwareTelemetryProvider {
    fn default() -> Self {
        Self::new(SamplingCadence::default().interval())
    }
}

impl WindowsHardwareTelemetryProvider {
    pub fn new(requested_cadence: Duration) -> Self {
        Self {
            cadence: SamplingCadence::bounded(requested_cadence),
            previous_cpu_times: None,
            previous_snapshot: None,
        }
    }

    pub fn provider_name(&self) -> &'static str {
        "windows-native"
    }
}

impl HardwareTelemetryProvider for WindowsHardwareTelemetryProvider {
    fn cadence(&self) -> SamplingCadence {
        self.cadence
    }

    fn set_cadence(&mut self, requested: Duration) {
        self.cadence = SamplingCadence::bounded(requested);
    }

    fn sample(&mut self, managed_process_pid: Option<u32>) -> HardwareTelemetrySnapshot {
        let captured_at_unix_ms = now_unix_ms();
        let previous_snapshot = self.previous_snapshot.as_ref();

        let (total_usage_percent, per_logical_processor_usage_percent) =
            match platform::query_cpu_times() {
                Ok(current) => {
                    let usage = match self.previous_cpu_times.as_ref() {
                        Some(previous) => cpu_usage_delta(previous, &current),
                        None => Err(
                            "CPU utilization requires two counter samples; no prior sample exists"
                                .to_owned(),
                        ),
                    };
                    self.previous_cpu_times = Some(current);
                    match usage {
                        Ok(usage) => (
                            live_reading(
                                usage.total_percent,
                                TelemetryUnit::Percent,
                                "NtQuerySystemInformation(SystemProcessorPerformanceInformation)",
                                captured_at_unix_ms,
                            ),
                            live_reading(
                                usage.per_logical_processor_percent,
                                TelemetryUnit::Percent,
                                "NtQuerySystemInformation(SystemProcessorPerformanceInformation)",
                                captured_at_unix_ms,
                            ),
                        ),
                        Err(reason) => (
                            unavailable_reading(
                                TelemetryUnit::Percent,
                                "NtQuerySystemInformation(SystemProcessorPerformanceInformation)",
                                captured_at_unix_ms,
                                reason.clone(),
                            ),
                            unavailable_reading(
                                TelemetryUnit::Percent,
                                "NtQuerySystemInformation(SystemProcessorPerformanceInformation)",
                                captured_at_unix_ms,
                                reason,
                            ),
                        ),
                    }
                }
                Err(message) => (
                    failed_or_stale_reading(
                        previous_snapshot.map(|snapshot| &snapshot.cpu.total_usage_percent),
                        TelemetryUnit::Percent,
                        "NtQuerySystemInformation(SystemProcessorPerformanceInformation)",
                        captured_at_unix_ms,
                        message.clone(),
                    ),
                    failed_or_stale_reading(
                        previous_snapshot
                            .map(|snapshot| &snapshot.cpu.per_logical_processor_usage_percent),
                        TelemetryUnit::Percent,
                        "NtQuerySystemInformation(SystemProcessorPerformanceInformation)",
                        captured_at_unix_ms,
                        message,
                    ),
                ),
            };

        let logical_processor_count = match platform::logical_processor_count() {
            Ok(value) => live_reading(
                value,
                TelemetryUnit::Count,
                "GetActiveProcessorCount(ALL_PROCESSOR_GROUPS)",
                captured_at_unix_ms,
            ),
            Err(message) => failed_or_stale_reading(
                previous_snapshot.map(|snapshot| &snapshot.cpu.logical_processor_count),
                TelemetryUnit::Count,
                "GetActiveProcessorCount(ALL_PROCESSOR_GROUPS)",
                captured_at_unix_ms,
                message,
            ),
        };

        let physical_core_count = match platform::physical_core_count() {
            Ok(value) => live_reading(
                value,
                TelemetryUnit::Count,
                "GetLogicalProcessorInformationEx(RelationProcessorCore)",
                captured_at_unix_ms,
            ),
            Err(message) => failed_or_stale_reading(
                previous_snapshot.map(|snapshot| &snapshot.cpu.physical_core_count),
                TelemetryUnit::Count,
                "GetLogicalProcessorInformationEx(RelationProcessorCore)",
                captured_at_unix_ms,
                message,
            ),
        };

        let memory = match platform::query_memory() {
            Ok(memory) => MemoryTelemetry {
                total_physical_bytes: live_reading(
                    memory.total,
                    TelemetryUnit::Bytes,
                    "GlobalMemoryStatusEx",
                    captured_at_unix_ms,
                ),
                available_physical_bytes: live_reading(
                    memory.available,
                    TelemetryUnit::Bytes,
                    "GlobalMemoryStatusEx",
                    captured_at_unix_ms,
                ),
                used_physical_bytes: live_reading(
                    memory.used,
                    TelemetryUnit::Bytes,
                    "GlobalMemoryStatusEx",
                    captured_at_unix_ms,
                ),
            },
            Err(message) => MemoryTelemetry {
                total_physical_bytes: failed_or_stale_reading(
                    previous_snapshot.map(|snapshot| &snapshot.memory.total_physical_bytes),
                    TelemetryUnit::Bytes,
                    "GlobalMemoryStatusEx",
                    captured_at_unix_ms,
                    message.clone(),
                ),
                available_physical_bytes: failed_or_stale_reading(
                    previous_snapshot.map(|snapshot| &snapshot.memory.available_physical_bytes),
                    TelemetryUnit::Bytes,
                    "GlobalMemoryStatusEx",
                    captured_at_unix_ms,
                    message.clone(),
                ),
                used_physical_bytes: failed_or_stale_reading(
                    previous_snapshot.map(|snapshot| &snapshot.memory.used_physical_bytes),
                    TelemetryUnit::Bytes,
                    "GlobalMemoryStatusEx",
                    captured_at_unix_ms,
                    message,
                ),
            },
        };

        let managed_process_memory = match managed_process_pid {
            Some(pid) => match platform::query_process_memory(pid) {
                Ok(metrics) => live_reading(
                    metrics,
                    TelemetryUnit::Bytes,
                    "OpenProcess + K32GetProcessMemoryInfo",
                    captured_at_unix_ms,
                ),
                Err(PlatformProcessError::Unavailable(reason)) => unavailable_reading(
                    TelemetryUnit::Bytes,
                    "OpenProcess + K32GetProcessMemoryInfo",
                    captured_at_unix_ms,
                    reason,
                ),
                Err(PlatformProcessError::Error(message)) => failed_or_stale_reading(
                    previous_snapshot.and_then(|snapshot| {
                        process_reading_for_pid(&snapshot.managed_process_memory, pid)
                    }),
                    TelemetryUnit::Bytes,
                    "OpenProcess + K32GetProcessMemoryInfo",
                    captured_at_unix_ms,
                    message,
                ),
            },
            None => unavailable_reading(
                TelemetryUnit::Bytes,
                "OpenProcess + K32GetProcessMemoryInfo",
                captured_at_unix_ms,
                "no managed process PID was supplied".to_owned(),
            ),
        };

        let snapshot = HardwareTelemetrySnapshot {
            captured_at_unix_ms,
            cadence_ms: self.cadence.interval_ms(),
            cpu: CpuTelemetry {
                total_usage_percent,
                per_logical_processor_usage_percent,
                logical_processor_count,
                physical_core_count,
            },
            memory,
            managed_process_memory,
        };
        self.previous_snapshot = Some(snapshot.clone());
        snapshot
    }
}

fn process_reading_for_pid(
    reading: &TelemetryReading<ProcessMemoryMetrics>,
    pid: u32,
) -> Option<&TelemetryReading<ProcessMemoryMetrics>> {
    match &reading.state {
        TelemetryState::Live { value } if value.pid == pid => Some(reading),
        TelemetryState::Stale {
            last_value: Some(value),
            ..
        } if value.pid == pid => Some(reading),
        _ => None,
    }
}

fn live_reading<T>(
    value: T,
    unit: TelemetryUnit,
    api: &str,
    sampled_at_unix_ms: u64,
) -> TelemetryReading<T> {
    TelemetryReading {
        state: TelemetryState::Live { value },
        unit,
        source: TelemetrySource::windows(api),
        sampled_at_unix_ms,
    }
}

fn unavailable_reading<T>(
    unit: TelemetryUnit,
    api: &str,
    sampled_at_unix_ms: u64,
    reason: String,
) -> TelemetryReading<T> {
    TelemetryReading {
        state: TelemetryState::Unavailable { reason },
        unit,
        source: TelemetrySource::windows(api),
        sampled_at_unix_ms,
    }
}

fn failed_or_stale_reading<T: Clone>(
    previous: Option<&TelemetryReading<T>>,
    unit: TelemetryUnit,
    api: &str,
    sampled_at_unix_ms: u64,
    message: String,
) -> TelemetryReading<T> {
    let state = match previous.map(|reading| &reading.state) {
        Some(TelemetryState::Live { value }) => TelemetryState::Stale {
            last_value: Some(value.clone()),
            last_observed_at_unix_ms: previous.map(|reading| reading.sampled_at_unix_ms),
            reason: message,
        },
        Some(TelemetryState::Stale {
            last_value,
            last_observed_at_unix_ms,
            ..
        }) => TelemetryState::Stale {
            last_value: last_value.clone(),
            last_observed_at_unix_ms: *last_observed_at_unix_ms,
            reason: message,
        },
        _ => TelemetryState::Error { message },
    };
    TelemetryReading {
        state,
        unit,
        source: TelemetrySource::windows(api),
        sampled_at_unix_ms,
    }
}

fn cpu_usage_delta(previous: &[CpuTimes], current: &[CpuTimes]) -> Result<CpuUsageDelta, String> {
    if previous.len() != current.len() {
        return Err(format!(
            "logical processor count changed between CPU samples ({} -> {})",
            previous.len(),
            current.len()
        ));
    }
    if current.is_empty() {
        return Err("CPU performance query returned no logical processors".to_owned());
    }

    let mut aggregate_total = 0_u128;
    let mut aggregate_busy = 0_u128;
    let mut per_cpu = Vec::with_capacity(current.len());

    for (index, (before, after)) in previous.iter().zip(current).enumerate() {
        let idle = checked_counter_delta(before.idle, after.idle, index, "idle")?;
        let kernel = checked_counter_delta(before.kernel, after.kernel, index, "kernel")?;
        let user = checked_counter_delta(before.user, after.user, index, "user")?;
        let total = u128::from(kernel) + u128::from(user);
        if total == 0 {
            return Err(format!(
                "logical processor {index} reported a zero CPU counter delta"
            ));
        }
        let busy = total.saturating_sub(u128::from(idle));
        let usage = ((busy as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
        per_cpu.push(usage);
        aggregate_total += total;
        aggregate_busy += busy;
    }

    if aggregate_total == 0 {
        return Err("aggregate CPU counter delta was zero".to_owned());
    }

    Ok(CpuUsageDelta {
        total_percent: ((aggregate_busy as f64 / aggregate_total as f64) * 100.0)
            .clamp(0.0, 100.0),
        per_logical_processor_percent: per_cpu,
    })
}

fn checked_counter_delta(
    before: u64,
    after: u64,
    processor_index: usize,
    counter_name: &str,
) -> Result<u64, String> {
    after.checked_sub(before).ok_or_else(|| {
        format!(
            "logical processor {processor_index} {counter_name} counter moved backwards ({before} -> {after})"
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlatformMemory {
    total: u64,
    available: u64,
    used: u64,
}

fn platform_memory(total: u64, available: u64) -> Result<PlatformMemory, String> {
    let used = total.checked_sub(available).ok_or_else(|| {
        format!("available physical memory {available} exceeds total physical memory {total}")
    })?;
    Ok(PlatformMemory {
        total,
        available,
        used,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PlatformProcessError {
    Unavailable(String),
    Error(String),
}

fn now_unix_ms() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

#[cfg(windows)]
mod platform {
    use std::{ffi::c_void, io, mem, ptr};

    use super::{CpuTimes, PlatformMemory, PlatformProcessError, ProcessMemoryMetrics, platform_memory};

    const ALL_PROCESSOR_GROUPS: u16 = 0xFFFF;
    const RELATION_PROCESSOR_CORE: u32 = 0;
    const SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION_CLASS: u32 = 8;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const ERROR_INVALID_PARAMETER: i32 = 87;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SystemProcessorPerformanceInformation {
        idle_time: i64,
        kernel_time: i64,
        user_time: i64,
        reserved1: [i64; 2],
        reserved2: u32,
    }

    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    impl Default for MemoryStatusEx {
        fn default() -> Self {
            Self {
                length: mem::size_of::<Self>() as u32,
                memory_load: 0,
                total_phys: 0,
                avail_phys: 0,
                total_page_file: 0,
                avail_page_file: 0,
                total_virtual: 0,
                avail_virtual: 0,
                avail_extended_virtual: 0,
            }
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
        fn GetActiveProcessorCount(group_number: u16) -> u32;
        fn GetLogicalProcessorInformationEx(
            relationship_type: u32,
            buffer: *mut c_void,
            returned_length: *mut u32,
        ) -> i32;
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn K32GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut c_void,
            cb: u32,
        ) -> i32;
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQuerySystemInformation(
            system_information_class: u32,
            system_information: *mut c_void,
            system_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    pub(super) fn logical_processor_count() -> Result<u32, String> {
        // SAFETY: `GetActiveProcessorCount` has no pointer arguments and ALL_PROCESSOR_GROUPS is a
        // documented sentinel value requesting the active logical processor count system-wide.
        let count = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) };
        if count == 0 {
            Err(format!(
                "GetActiveProcessorCount failed: {}",
                io::Error::last_os_error()
            ))
        } else {
            Ok(count)
        }
    }

    pub(super) fn physical_core_count() -> Result<u32, String> {
        let mut required_bytes = 0_u32;
        // SAFETY: a null buffer with length zero is the documented sizing call. The API writes only
        // the required byte count to `required_bytes`.
        unsafe {
            GetLogicalProcessorInformationEx(
                RELATION_PROCESSOR_CORE,
                ptr::null_mut(),
                &mut required_bytes,
            );
        }
        if required_bytes < 8 {
            return Err(format!(
                "GetLogicalProcessorInformationEx sizing failed: {}",
                io::Error::last_os_error()
            ));
        }

        let mut buffer = vec![0_u8; required_bytes as usize];
        let mut returned_bytes = required_bytes;
        // SAFETY: `buffer` has exactly the advertised capacity and remains alive/mutable for the
        // duration of the call. `returned_bytes` is initialized to the buffer size.
        let ok = unsafe {
            GetLogicalProcessorInformationEx(
                RELATION_PROCESSOR_CORE,
                buffer.as_mut_ptr().cast(),
                &mut returned_bytes,
            )
        };
        if ok == 0 {
            return Err(format!(
                "GetLogicalProcessorInformationEx failed: {}",
                io::Error::last_os_error()
            ));
        }

        parse_physical_core_records(&buffer[..returned_bytes as usize])
    }

    fn parse_physical_core_records(buffer: &[u8]) -> Result<u32, String> {
        let mut offset = 0_usize;
        let mut cores = 0_u32;
        while offset < buffer.len() {
            if buffer.len() - offset < 8 {
                return Err("logical processor topology record is truncated".to_owned());
            }
            let relationship = u32::from_ne_bytes(
                buffer[offset..offset + 4]
                    .try_into()
                    .map_err(|_| "invalid topology relationship field".to_owned())?,
            );
            let size = u32::from_ne_bytes(
                buffer[offset + 4..offset + 8]
                    .try_into()
                    .map_err(|_| "invalid topology record size field".to_owned())?,
            ) as usize;
            if size < 8 || offset.saturating_add(size) > buffer.len() {
                return Err(format!(
                    "logical processor topology record has invalid size {size} at offset {offset}"
                ));
            }
            if relationship == RELATION_PROCESSOR_CORE {
                cores = cores
                    .checked_add(1)
                    .ok_or_else(|| "physical core count overflowed u32".to_owned())?;
            }
            offset += size;
        }
        if cores == 0 {
            Err("Windows reported zero active physical processor cores".to_owned())
        } else {
            Ok(cores)
        }
    }

    pub(super) fn query_cpu_times() -> Result<Vec<CpuTimes>, String> {
        let logical_count = logical_processor_count()?;
        let mut counters = vec![
            SystemProcessorPerformanceInformation::default();
            logical_count as usize
        ];
        let byte_len = counters
            .len()
            .checked_mul(mem::size_of::<SystemProcessorPerformanceInformation>())
            .and_then(|bytes| u32::try_from(bytes).ok())
            .ok_or_else(|| "CPU performance buffer size overflowed u32".to_owned())?;
        let mut returned_bytes = 0_u32;
        // SAFETY: `counters` is a writable array sized for every active logical processor reported
        // by Windows. The class and structure layout are documented for
        // SystemProcessorPerformanceInformation.
        let status = unsafe {
            NtQuerySystemInformation(
                SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION_CLASS,
                counters.as_mut_ptr().cast(),
                byte_len,
                &mut returned_bytes,
            )
        };
        if status < 0 {
            return Err(format!(
                "NtQuerySystemInformation(SystemProcessorPerformanceInformation) failed with NTSTATUS 0x{:08X}",
                status as u32
            ));
        }

        let record_size = mem::size_of::<SystemProcessorPerformanceInformation>();
        let returned_count = returned_bytes as usize / record_size;
        if returned_count == 0 || returned_count > counters.len() {
            return Err(format!(
                "CPU performance query returned an invalid byte count {returned_bytes} for {} allocated records",
                counters.len()
            ));
        }
        counters.truncate(returned_count);
        counters
            .into_iter()
            .enumerate()
            .map(|(index, counter)| {
                Ok(CpuTimes {
                    idle: nonnegative_counter(counter.idle_time, index, "idle")?,
                    kernel: nonnegative_counter(counter.kernel_time, index, "kernel")?,
                    user: nonnegative_counter(counter.user_time, index, "user")?,
                })
            })
            .collect()
    }

    fn nonnegative_counter(value: i64, index: usize, name: &str) -> Result<u64, String> {
        u64::try_from(value).map_err(|_| {
            format!("logical processor {index} returned a negative {name} time counter: {value}")
        })
    }

    pub(super) fn query_memory() -> Result<PlatformMemory, String> {
        let mut status = MemoryStatusEx::default();
        // SAFETY: `status.length` is initialized to the exact repr(C) structure size and the pointer
        // remains valid and writable for the duration of the call.
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        if ok == 0 {
            return Err(format!(
                "GlobalMemoryStatusEx failed: {}",
                io::Error::last_os_error()
            ));
        }
        platform_memory(status.total_phys, status.avail_phys)
    }

    pub(super) fn query_process_memory(
        pid: u32,
    ) -> Result<ProcessMemoryMetrics, PlatformProcessError> {
        // SAFETY: `OpenProcess` receives a numeric PID and requests query-only access. No borrowed
        // pointers cross the FFI boundary.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER) {
                Err(PlatformProcessError::Unavailable(format!(
                    "process pid {pid} is no longer available"
                )))
            } else {
                Err(PlatformProcessError::Error(format!(
                    "OpenProcess(pid {pid}) failed: {error}"
                )))
            };
        }

        let mut counters = ProcessMemoryCountersEx {
            cb: mem::size_of::<ProcessMemoryCountersEx>() as u32,
            ..ProcessMemoryCountersEx::default()
        };
        // SAFETY: `handle` is valid from `OpenProcess`; `counters` is a writable repr(C) structure
        // with its size field initialized. The handle is closed below on every path after this call.
        let ok = unsafe {
            K32GetProcessMemoryInfo(
                handle,
                (&mut counters as *mut ProcessMemoryCountersEx).cast(),
                counters.cb,
            )
        };
        let query_error = if ok == 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        // SAFETY: `handle` was returned by `OpenProcess` and has not been closed yet.
        unsafe {
            CloseHandle(handle);
        }
        if let Some(error) = query_error {
            return Err(PlatformProcessError::Error(format!(
                "K32GetProcessMemoryInfo(pid {pid}) failed: {error}"
            )));
        }

        Ok(ProcessMemoryMetrics {
            pid,
            working_set_bytes: counters.working_set_size as u64,
            peak_working_set_bytes: counters.peak_working_set_size as u64,
            private_bytes: counters.private_usage as u64,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::parse_physical_core_records;

        #[test]
        fn topology_parser_counts_variable_sized_core_records() {
            let mut bytes = Vec::new();
            for size in [16_u32, 24_u32] {
                bytes.extend_from_slice(&0_u32.to_ne_bytes());
                bytes.extend_from_slice(&size.to_ne_bytes());
                bytes.resize(bytes.len() + size as usize - 8, 0);
            }
            assert_eq!(parse_physical_core_records(&bytes).unwrap(), 2);
        }

        #[test]
        fn topology_parser_rejects_truncated_record() {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&0_u32.to_ne_bytes());
            bytes.extend_from_slice(&32_u32.to_ne_bytes());
            assert!(parse_physical_core_records(&bytes).is_err());
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{CpuTimes, PlatformMemory, PlatformProcessError, ProcessMemoryMetrics};

    fn unsupported() -> String {
        "the windows-native telemetry provider is unavailable on this operating system".to_owned()
    }

    pub(super) fn logical_processor_count() -> Result<u32, String> {
        Err(unsupported())
    }

    pub(super) fn physical_core_count() -> Result<u32, String> {
        Err(unsupported())
    }

    pub(super) fn query_cpu_times() -> Result<Vec<CpuTimes>, String> {
        Err(unsupported())
    }

    pub(super) fn query_memory() -> Result<PlatformMemory, String> {
        Err(unsupported())
    }

    pub(super) fn query_process_memory(
        _pid: u32,
    ) -> Result<ProcessMemoryMetrics, PlatformProcessError> {
        Err(PlatformProcessError::Unavailable(unsupported()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_cadence_is_bounded() {
        assert_eq!(
            SamplingCadence::bounded(Duration::ZERO).interval_ms(),
            MIN_SAMPLING_INTERVAL_MS
        );
        assert_eq!(
            SamplingCadence::bounded(Duration::from_millis(1_337)).interval_ms(),
            1_337
        );
        assert_eq!(
            SamplingCadence::bounded(Duration::from_secs(600)).interval_ms(),
            MAX_SAMPLING_INTERVAL_MS
        );
    }

    #[test]
    fn cpu_delta_accounts_for_kernel_time_including_idle() {
        let before = [CpuTimes {
            idle: 100,
            kernel: 200,
            user: 100,
        }];
        let after = [CpuTimes {
            idle: 150,
            kernel: 300,
            user: 200,
        }];
        let usage = cpu_usage_delta(&before, &after).unwrap();
        // Delta total = kernel 100 + user 100 = 200. Idle delta is 50, so busy is 150.
        assert!((usage.total_percent - 75.0).abs() < f64::EPSILON);
        assert!((usage.per_logical_processor_percent[0] - 75.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cpu_delta_rejects_counter_rollback() {
        let before = [CpuTimes {
            idle: 100,
            kernel: 200,
            user: 100,
        }];
        let after = [CpuTimes {
            idle: 99,
            kernel: 300,
            user: 200,
        }];
        assert!(cpu_usage_delta(&before, &after).is_err());
    }

    #[test]
    fn memory_used_bytes_are_checked_not_saturated() {
        assert_eq!(platform_memory(1_000, 250).unwrap().used, 750);
        assert!(platform_memory(100, 101).is_err());
    }

    #[test]
    fn failed_refresh_marks_previous_live_value_stale() {
        let previous = live_reading(42_u64, TelemetryUnit::Bytes, "fixture", 10);
        let failed = failed_or_stale_reading(
            Some(&previous),
            TelemetryUnit::Bytes,
            "fixture",
            20,
            "provider failed".to_owned(),
        );
        assert_eq!(
            failed.state,
            TelemetryState::Stale {
                last_value: Some(42),
                last_observed_at_unix_ms: Some(10),
                reason: "provider failed".to_owned(),
            }
        );
    }

    #[test]
    fn failed_refresh_without_prior_live_value_is_error_not_zero() {
        let failed: TelemetryReading<u64> = failed_or_stale_reading(
            None,
            TelemetryUnit::Bytes,
            "fixture",
            20,
            "provider failed".to_owned(),
        );
        assert_eq!(
            failed.state,
            TelemetryState::Error {
                message: "provider failed".to_owned()
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_memory_and_topology_are_sane() {
        let logical = platform::logical_processor_count().unwrap();
        let physical = platform::physical_core_count().unwrap();
        let memory = platform::query_memory().unwrap();
        assert!(logical > 0);
        assert!(physical > 0);
        assert!(physical <= logical);
        assert!(memory.total > 0);
        assert!(memory.available <= memory.total);
    }

    #[cfg(windows)]
    #[test]
    fn windows_process_memory_reads_current_process() {
        let metrics = platform::query_process_memory(std::process::id()).unwrap();
        assert_eq!(metrics.pid, std::process::id());
        assert!(metrics.working_set_bytes > 0);
        assert!(metrics.peak_working_set_bytes >= metrics.working_set_bytes);
        assert!(metrics.private_bytes > 0);
    }
}
