use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::hardware_telemetry::{SamplingCadence, TelemetryState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuTelemetryUnit {
    Percent,
    Bytes,
    Celsius,
    Megahertz,
    Milliwatts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuTelemetrySource {
    pub provider: String,
    pub api: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuTelemetryReading<T> {
    pub state: TelemetryState<T>,
    pub unit: GpuTelemetryUnit,
    pub source: GpuTelemetrySource,
    pub sampled_at_unix_ms: u64,
}

impl<T> GpuTelemetryReading<T> {
    pub fn live_value(&self) -> Option<&T> {
        match &self.state {
            TelemetryState::Live { value } => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuAdapterIdentity {
    pub vendor: String,
    pub index: u32,
    pub uuid: Option<String>,
    pub name: Option<String>,
    pub stable_for_evidence: bool,
    pub identity_note: Option<String>,
}

impl GpuAdapterIdentity {
    fn key(&self) -> String {
        self.uuid
            .as_ref()
            .map(|uuid| format!("uuid:{uuid}"))
            .unwrap_or_else(|| format!("volatile-index:{}", self.index))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuAdapterTelemetry {
    pub identity: GpuAdapterIdentity,
    pub gpu_utilization_percent: GpuTelemetryReading<u32>,
    pub memory_utilization_percent: GpuTelemetryReading<u32>,
    pub memory_used_bytes: GpuTelemetryReading<u64>,
    pub memory_total_bytes: GpuTelemetryReading<u64>,
    pub temperature_celsius: GpuTelemetryReading<u32>,
    pub graphics_clock_mhz: GpuTelemetryReading<u32>,
    pub memory_clock_mhz: GpuTelemetryReading<u32>,
    pub power_milliwatts: GpuTelemetryReading<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuTelemetrySnapshot {
    pub captured_at_unix_ms: u64,
    pub provider_adapter_count: TelemetryState<u32>,
    pub adapters: Vec<GpuAdapterTelemetry>,
}

pub trait GpuTelemetryProvider: Send {
    fn sample(&mut self) -> GpuTelemetrySnapshot;
}

pub struct NvidiaGpuTelemetryProvider {
    collector: GpuTelemetryCollector<platform::NvmlBackend>,
}

impl NvidiaGpuTelemetryProvider {
    pub fn new() -> Self {
        Self {
            collector: GpuTelemetryCollector::new(platform::NvmlBackend::new()),
        }
    }
}

impl Default for NvidiaGpuTelemetryProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuTelemetryProvider for NvidiaGpuTelemetryProvider {
    fn sample(&mut self) -> GpuTelemetrySnapshot {
        self.collector.sample()
    }
}

pub struct GpuTelemetryWorker {
    receiver: Receiver<GpuTelemetrySnapshot>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl GpuTelemetryWorker {
    pub fn spawn(cadence: Duration) -> Self {
        Self::spawn_with_provider(NvidiaGpuTelemetryProvider::new(), cadence)
    }

    pub fn spawn_with_provider<P>(mut provider: P, cadence: Duration) -> Self
    where
        P: GpuTelemetryProvider + 'static,
    {
        let cadence = SamplingCadence::bounded(cadence).interval();
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                if sender.send(provider.sample()).is_err() {
                    break;
                }
                thread::park_timeout(cadence);
            }
        });
        Self {
            receiver,
            stop,
            handle: Some(handle),
        }
    }

    pub fn try_latest(&self) -> Option<GpuTelemetrySnapshot> {
        let mut latest = None;
        while let Ok(snapshot) = self.receiver.try_recv() {
            latest = Some(snapshot);
        }
        latest
    }
}

impl Drop for GpuTelemetryWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BackendProviderError {
    Unavailable(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
enum BackendMetric<T> {
    Live(T),
    Unavailable(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
struct BackendAdapterSample {
    identity: GpuAdapterIdentity,
    gpu_utilization_percent: BackendMetric<u32>,
    memory_utilization_percent: BackendMetric<u32>,
    memory_used_bytes: BackendMetric<u64>,
    memory_total_bytes: BackendMetric<u64>,
    temperature_celsius: BackendMetric<u32>,
    graphics_clock_mhz: BackendMetric<u32>,
    memory_clock_mhz: BackendMetric<u32>,
    power_milliwatts: BackendMetric<u32>,
}

#[derive(Debug, Clone, PartialEq)]
struct BackendSnapshot {
    reported_adapter_count: u32,
    adapters: Vec<BackendAdapterSample>,
}

trait GpuBackend: Send {
    fn sample_adapters(&mut self) -> Result<BackendSnapshot, BackendProviderError>;
}

struct GpuTelemetryCollector<B> {
    backend: B,
    previous: HashMap<String, GpuAdapterTelemetry>,
    previous_provider_count: Option<(u32, u64)>,
}

impl<B: GpuBackend> GpuTelemetryCollector<B> {
    fn new(backend: B) -> Self {
        Self {
            backend,
            previous: HashMap::new(),
            previous_provider_count: None,
        }
    }

    fn sample(&mut self) -> GpuTelemetrySnapshot {
        let captured_at_unix_ms = now_unix_ms();
        let backend_snapshot = match self.backend.sample_adapters() {
            Ok(snapshot) => snapshot,
            Err(error) => return self.provider_failure(error, captured_at_unix_ms),
        };

        let mut adapters =
            Vec::with_capacity(self.previous.len().max(backend_snapshot.adapters.len()));
        let mut seen = HashSet::new();

        for backend_adapter in backend_snapshot.adapters {
            let key = backend_adapter.identity.key();
            let previous = self.previous.get(&key);
            seen.insert(key);
            adapters.push(convert_adapter(
                backend_adapter,
                previous,
                captured_at_unix_ms,
            ));
        }

        for (key, previous) in &self.previous {
            if !seen.contains(key) {
                adapters.push(stale_adapter(
                    previous,
                    captured_at_unix_ms,
                    "adapter disappeared from NVML enumeration",
                ));
            }
        }

        adapters.sort_by_key(|adapter| adapter.identity.index);
        self.previous = adapters
            .iter()
            .cloned()
            .map(|adapter| (adapter.identity.key(), adapter))
            .collect();
        self.previous_provider_count =
            Some((backend_snapshot.reported_adapter_count, captured_at_unix_ms));

        GpuTelemetrySnapshot {
            captured_at_unix_ms,
            provider_adapter_count: TelemetryState::Live {
                value: backend_snapshot.reported_adapter_count,
            },
            adapters,
        }
    }

    fn provider_failure(
        &mut self,
        error: BackendProviderError,
        captured_at_unix_ms: u64,
    ) -> GpuTelemetrySnapshot {
        let reason = match &error {
            BackendProviderError::Unavailable(reason) | BackendProviderError::Error(reason) => {
                reason.clone()
            }
        };
        let mut adapters: Vec<_> = self
            .previous
            .values()
            .map(|previous| stale_adapter(previous, captured_at_unix_ms, &reason))
            .collect();
        adapters.sort_by_key(|adapter| adapter.identity.index);
        self.previous = adapters
            .iter()
            .cloned()
            .map(|adapter| (adapter.identity.key(), adapter))
            .collect();

        let provider_adapter_count = match self.previous_provider_count {
            Some((value, observed_at)) => TelemetryState::Stale {
                last_value: Some(value),
                last_observed_at_unix_ms: Some(observed_at),
                reason,
            },
            None => match error {
                BackendProviderError::Unavailable(reason) => TelemetryState::Unavailable { reason },
                BackendProviderError::Error(message) => TelemetryState::Error { message },
            },
        };

        GpuTelemetrySnapshot {
            captured_at_unix_ms,
            provider_adapter_count,
            adapters,
        }
    }
}

fn convert_adapter(
    backend: BackendAdapterSample,
    previous: Option<&GpuAdapterTelemetry>,
    sampled_at_unix_ms: u64,
) -> GpuAdapterTelemetry {
    GpuAdapterTelemetry {
        identity: backend.identity,
        gpu_utilization_percent: convert_metric(
            backend.gpu_utilization_percent,
            previous.map(|adapter| &adapter.gpu_utilization_percent),
            GpuTelemetryUnit::Percent,
            "nvmlDeviceGetUtilizationRates(gpu)",
            sampled_at_unix_ms,
        ),
        memory_utilization_percent: convert_metric(
            backend.memory_utilization_percent,
            previous.map(|adapter| &adapter.memory_utilization_percent),
            GpuTelemetryUnit::Percent,
            "nvmlDeviceGetUtilizationRates(memory)",
            sampled_at_unix_ms,
        ),
        memory_used_bytes: convert_metric(
            backend.memory_used_bytes,
            previous.map(|adapter| &adapter.memory_used_bytes),
            GpuTelemetryUnit::Bytes,
            "nvmlDeviceGetMemoryInfo(used)",
            sampled_at_unix_ms,
        ),
        memory_total_bytes: convert_metric(
            backend.memory_total_bytes,
            previous.map(|adapter| &adapter.memory_total_bytes),
            GpuTelemetryUnit::Bytes,
            "nvmlDeviceGetMemoryInfo(total)",
            sampled_at_unix_ms,
        ),
        temperature_celsius: convert_metric(
            backend.temperature_celsius,
            previous.map(|adapter| &adapter.temperature_celsius),
            GpuTelemetryUnit::Celsius,
            "nvmlDeviceGetTemperature(NVML_TEMPERATURE_GPU)",
            sampled_at_unix_ms,
        ),
        graphics_clock_mhz: convert_metric(
            backend.graphics_clock_mhz,
            previous.map(|adapter| &adapter.graphics_clock_mhz),
            GpuTelemetryUnit::Megahertz,
            "nvmlDeviceGetClockInfo(NVML_CLOCK_GRAPHICS)",
            sampled_at_unix_ms,
        ),
        memory_clock_mhz: convert_metric(
            backend.memory_clock_mhz,
            previous.map(|adapter| &adapter.memory_clock_mhz),
            GpuTelemetryUnit::Megahertz,
            "nvmlDeviceGetClockInfo(NVML_CLOCK_MEM)",
            sampled_at_unix_ms,
        ),
        power_milliwatts: convert_metric(
            backend.power_milliwatts,
            previous.map(|adapter| &adapter.power_milliwatts),
            GpuTelemetryUnit::Milliwatts,
            "nvmlDeviceGetPowerUsage",
            sampled_at_unix_ms,
        ),
    }
}

fn convert_metric<T: Clone>(
    backend: BackendMetric<T>,
    previous: Option<&GpuTelemetryReading<T>>,
    unit: GpuTelemetryUnit,
    api: &str,
    sampled_at_unix_ms: u64,
) -> GpuTelemetryReading<T> {
    let state = match backend {
        BackendMetric::Live(value) => TelemetryState::Live { value },
        BackendMetric::Unavailable(reason) => TelemetryState::Unavailable { reason },
        BackendMetric::Error(message) => stale_or_error(previous, message),
    };
    GpuTelemetryReading {
        state,
        unit,
        source: GpuTelemetrySource {
            provider: "nvidia-nvml".to_owned(),
            api: api.to_owned(),
        },
        sampled_at_unix_ms,
    }
}

fn stale_or_error<T: Clone>(
    previous: Option<&GpuTelemetryReading<T>>,
    message: String,
) -> TelemetryState<T> {
    match previous.map(|reading| &reading.state) {
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
    }
}

fn stale_adapter(
    previous: &GpuAdapterTelemetry,
    sampled_at_unix_ms: u64,
    reason: &str,
) -> GpuAdapterTelemetry {
    GpuAdapterTelemetry {
        identity: previous.identity.clone(),
        gpu_utilization_percent: stale_reading(
            &previous.gpu_utilization_percent,
            sampled_at_unix_ms,
            reason,
        ),
        memory_utilization_percent: stale_reading(
            &previous.memory_utilization_percent,
            sampled_at_unix_ms,
            reason,
        ),
        memory_used_bytes: stale_reading(&previous.memory_used_bytes, sampled_at_unix_ms, reason),
        memory_total_bytes: stale_reading(&previous.memory_total_bytes, sampled_at_unix_ms, reason),
        temperature_celsius: stale_reading(
            &previous.temperature_celsius,
            sampled_at_unix_ms,
            reason,
        ),
        graphics_clock_mhz: stale_reading(&previous.graphics_clock_mhz, sampled_at_unix_ms, reason),
        memory_clock_mhz: stale_reading(&previous.memory_clock_mhz, sampled_at_unix_ms, reason),
        power_milliwatts: stale_reading(&previous.power_milliwatts, sampled_at_unix_ms, reason),
    }
}

fn stale_reading<T: Clone>(
    previous: &GpuTelemetryReading<T>,
    sampled_at_unix_ms: u64,
    reason: &str,
) -> GpuTelemetryReading<T> {
    let state = match &previous.state {
        TelemetryState::Live { value } => TelemetryState::Stale {
            last_value: Some(value.clone()),
            last_observed_at_unix_ms: Some(previous.sampled_at_unix_ms),
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
    GpuTelemetryReading {
        state,
        unit: previous.unit,
        source: previous.source.clone(),
        sampled_at_unix_ms,
    }
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
    use std::{
        ffi::{CStr, c_char, c_void},
        mem, ptr,
    };

    use super::{
        BackendAdapterSample, BackendMetric, BackendProviderError, BackendSnapshot,
        GpuAdapterIdentity, GpuBackend,
    };

    const NVML_SUCCESS: i32 = 0;
    const NVML_ERROR_NOT_SUPPORTED: i32 = 3;
    const NVML_ERROR_NO_PERMISSION: i32 = 4;
    const NVML_ERROR_DRIVER_NOT_LOADED: i32 = 9;
    const NVML_ERROR_LIBRARY_NOT_FOUND: i32 = 12;
    const NVML_ERROR_FUNCTION_NOT_FOUND: i32 = 13;
    const NVML_ERROR_GPU_NOT_FOUND: i32 = 28;
    const NVML_CLOCK_GRAPHICS: u32 = 0;
    const NVML_CLOCK_MEM: u32 = 2;
    const NVML_TEMPERATURE_GPU: u32 = 0;
    const STRING_BUFFER_LEN: usize = 96;

    type NvmlReturn = i32;
    type NvmlDevice = *mut c_void;
    type NvmlInitV2 = unsafe extern "C" fn() -> NvmlReturn;
    type NvmlShutdown = unsafe extern "C" fn() -> NvmlReturn;
    type NvmlDeviceGetCountV2 = unsafe extern "C" fn(*mut u32) -> NvmlReturn;
    type NvmlDeviceGetHandleByIndexV2 = unsafe extern "C" fn(u32, *mut NvmlDevice) -> NvmlReturn;
    type NvmlDeviceGetString = unsafe extern "C" fn(NvmlDevice, *mut c_char, u32) -> NvmlReturn;
    type NvmlDeviceGetUtilizationRates =
        unsafe extern "C" fn(NvmlDevice, *mut NvmlUtilization) -> NvmlReturn;
    type NvmlDeviceGetMemoryInfo = unsafe extern "C" fn(NvmlDevice, *mut NvmlMemory) -> NvmlReturn;
    type NvmlDeviceGetTemperature = unsafe extern "C" fn(NvmlDevice, u32, *mut u32) -> NvmlReturn;
    type NvmlDeviceGetClockInfo = unsafe extern "C" fn(NvmlDevice, u32, *mut u32) -> NvmlReturn;
    type NvmlDeviceGetPowerUsage = unsafe extern "C" fn(NvmlDevice, *mut u32) -> NvmlReturn;

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    struct NvmlUtilization {
        gpu: u32,
        memory: u32,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    struct NvmlMemory {
        total: u64,
        free: u64,
        used: u64,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }

    pub(super) struct NvmlBackend {
        library: Option<NvmlLibrary>,
    }

    impl NvmlBackend {
        pub(super) fn new() -> Self {
            Self { library: None }
        }

        fn library(&mut self) -> Result<&mut NvmlLibrary, BackendProviderError> {
            if self.library.is_none() {
                self.library = Some(NvmlLibrary::load()?);
            }
            Ok(self.library.as_mut().expect("NVML library was initialized"))
        }
    }

    impl GpuBackend for NvmlBackend {
        fn sample_adapters(&mut self) -> Result<BackendSnapshot, BackendProviderError> {
            self.library()?.sample_adapters()
        }
    }

    struct NvmlLibrary {
        module: usize,
        shutdown: NvmlShutdown,
        device_get_count_v2: NvmlDeviceGetCountV2,
        device_get_handle_by_index_v2: NvmlDeviceGetHandleByIndexV2,
        device_get_uuid: NvmlDeviceGetString,
        device_get_name: Option<NvmlDeviceGetString>,
        device_get_utilization_rates: Option<NvmlDeviceGetUtilizationRates>,
        device_get_memory_info: Option<NvmlDeviceGetMemoryInfo>,
        device_get_temperature: Option<NvmlDeviceGetTemperature>,
        device_get_clock_info: Option<NvmlDeviceGetClockInfo>,
        device_get_power_usage: Option<NvmlDeviceGetPowerUsage>,
    }

    impl NvmlLibrary {
        fn load() -> Result<Self, BackendProviderError> {
            let wide_name: Vec<u16> = "nvml.dll".encode_utf16().chain(Some(0)).collect();
            // SAFETY: the string is NUL-terminated and alive for the duration of the call.
            let module = unsafe { LoadLibraryW(wide_name.as_ptr()) };
            if module.is_null() {
                return Err(BackendProviderError::Unavailable(
                    "NVIDIA NVML library nvml.dll is not available; NVIDIA telemetry is unsupported on this system"
                        .to_owned(),
                ));
            }
            let module_value = module as usize;

            let result = (|| {
                let init = required_symbol::<NvmlInitV2>(module_value, b"nvmlInit_v2\0")?;
                let shutdown = required_symbol::<NvmlShutdown>(module_value, b"nvmlShutdown\0")?;
                let device_get_count_v2 = required_symbol::<NvmlDeviceGetCountV2>(
                    module_value,
                    b"nvmlDeviceGetCount_v2\0",
                )?;
                let device_get_handle_by_index_v2 = required_symbol::<NvmlDeviceGetHandleByIndexV2>(
                    module_value,
                    b"nvmlDeviceGetHandleByIndex_v2\0",
                )?;
                let device_get_uuid =
                    required_symbol::<NvmlDeviceGetString>(module_value, b"nvmlDeviceGetUUID\0")?;

                // SAFETY: the symbol was resolved from nvml.dll with the documented signature.
                let init_result = unsafe { init() };
                if init_result != NVML_SUCCESS {
                    return Err(provider_error(
                        init_result,
                        "nvmlInit_v2 failed while initializing NVIDIA telemetry",
                    ));
                }

                Ok(Self {
                    module: module_value,
                    shutdown,
                    device_get_count_v2,
                    device_get_handle_by_index_v2,
                    device_get_uuid,
                    device_get_name: optional_symbol(module_value, b"nvmlDeviceGetName\0"),
                    device_get_utilization_rates: optional_symbol(
                        module_value,
                        b"nvmlDeviceGetUtilizationRates\0",
                    ),
                    device_get_memory_info: optional_symbol(
                        module_value,
                        b"nvmlDeviceGetMemoryInfo\0",
                    ),
                    device_get_temperature: optional_symbol(
                        module_value,
                        b"nvmlDeviceGetTemperature\0",
                    ),
                    device_get_clock_info: optional_symbol(
                        module_value,
                        b"nvmlDeviceGetClockInfo\0",
                    ),
                    device_get_power_usage: optional_symbol(
                        module_value,
                        b"nvmlDeviceGetPowerUsage\0",
                    ),
                })
            })();

            if result.is_err() {
                // SAFETY: `module` is a live LoadLibraryW handle not yet owned by NvmlLibrary.
                unsafe {
                    FreeLibrary(module);
                }
            }
            result
        }

        fn sample_adapters(&mut self) -> Result<BackendSnapshot, BackendProviderError> {
            let mut count = 0_u32;
            // SAFETY: NVML is initialized and `count` is writable.
            let result = unsafe { (self.device_get_count_v2)(&mut count) };
            if result != NVML_SUCCESS {
                return Err(provider_error(
                    result,
                    "nvmlDeviceGetCount_v2 failed while enumerating NVIDIA adapters",
                ));
            }

            let mut adapters = Vec::with_capacity(count as usize);
            for index in 0..count {
                let mut device = ptr::null_mut();
                // SAFETY: `index` is within the just-reported device count and `device` is writable.
                let result = unsafe { (self.device_get_handle_by_index_v2)(index, &mut device) };
                if result != NVML_SUCCESS {
                    return Err(provider_error(
                        result,
                        &format!("nvmlDeviceGetHandleByIndex_v2 failed for adapter {index}"),
                    ));
                }
                adapters.push(self.sample_adapter(index, device));
            }

            Ok(BackendSnapshot {
                reported_adapter_count: count,
                adapters,
            })
        }

        fn sample_adapter(&self, index: u32, device: NvmlDevice) -> BackendAdapterSample {
            let (uuid, uuid_note) =
                self.query_string(self.device_get_uuid, device, "nvmlDeviceGetUUID");
            let (name, name_note) = match self.device_get_name {
                Some(function) => self.query_string(function, device, "nvmlDeviceGetName"),
                None => (
                    None,
                    Some("nvmlDeviceGetName is not exported by the loaded NVML library".to_owned()),
                ),
            };
            let stable_for_evidence = uuid.is_some();
            let identity_note = join_notes(uuid_note, name_note);

            let (gpu_utilization_percent, memory_utilization_percent) =
                self.query_utilization(device);
            let (memory_used_bytes, memory_total_bytes) = self.query_memory(device);

            BackendAdapterSample {
                identity: GpuAdapterIdentity {
                    vendor: "nvidia".to_owned(),
                    index,
                    uuid,
                    name,
                    stable_for_evidence,
                    identity_note,
                },
                gpu_utilization_percent,
                memory_utilization_percent,
                memory_used_bytes,
                memory_total_bytes,
                temperature_celsius: self.query_selected_u32_metric(
                    self.device_get_temperature,
                    device,
                    NVML_TEMPERATURE_GPU,
                    "nvmlDeviceGetTemperature(NVML_TEMPERATURE_GPU)",
                ),
                graphics_clock_mhz: self.query_selected_u32_metric(
                    self.device_get_clock_info,
                    device,
                    NVML_CLOCK_GRAPHICS,
                    "nvmlDeviceGetClockInfo(NVML_CLOCK_GRAPHICS)",
                ),
                memory_clock_mhz: self.query_selected_u32_metric(
                    self.device_get_clock_info,
                    device,
                    NVML_CLOCK_MEM,
                    "nvmlDeviceGetClockInfo(NVML_CLOCK_MEM)",
                ),
                power_milliwatts: self.query_simple_u32_metric(
                    self.device_get_power_usage,
                    device,
                    "nvmlDeviceGetPowerUsage",
                ),
            }
        }

        fn query_string(
            &self,
            function: NvmlDeviceGetString,
            device: NvmlDevice,
            api: &str,
        ) -> (Option<String>, Option<String>) {
            let mut buffer = [0 as c_char; STRING_BUFFER_LEN];
            // SAFETY: the buffer is writable and the symbol has the documented string signature.
            let result = unsafe {
                function(
                    device,
                    buffer.as_mut_ptr(),
                    u32::try_from(buffer.len()).expect("NVML string buffer fits in u32"),
                )
            };
            if result != NVML_SUCCESS {
                return (
                    None,
                    Some(format!("{api} failed: {}", nvml_error_name(result))),
                );
            }
            // SAFETY: successful NVML string queries are NUL-terminated; the buffer starts zeroed.
            let value = unsafe { CStr::from_ptr(buffer.as_ptr()) }
                .to_string_lossy()
                .trim()
                .to_owned();
            if value.is_empty() {
                (
                    None,
                    Some(format!("{api} returned an empty identity string")),
                )
            } else {
                (Some(value), None)
            }
        }

        fn query_utilization(
            &self,
            device: NvmlDevice,
        ) -> (BackendMetric<u32>, BackendMetric<u32>) {
            let Some(function) = self.device_get_utilization_rates else {
                let reason =
                    "nvmlDeviceGetUtilizationRates is not exported by the loaded NVML library"
                        .to_owned();
                return (
                    BackendMetric::Unavailable(reason.clone()),
                    BackendMetric::Unavailable(reason),
                );
            };
            let mut utilization = NvmlUtilization::default();
            // SAFETY: `utilization` is a writable repr(C) structure of the documented shape.
            let result = unsafe { function(device, &mut utilization) };
            (
                metric_result(
                    result,
                    utilization.gpu,
                    "nvmlDeviceGetUtilizationRates(gpu)",
                ),
                metric_result(
                    result,
                    utilization.memory,
                    "nvmlDeviceGetUtilizationRates(memory)",
                ),
            )
        }

        fn query_memory(&self, device: NvmlDevice) -> (BackendMetric<u64>, BackendMetric<u64>) {
            let Some(function) = self.device_get_memory_info else {
                let reason =
                    "nvmlDeviceGetMemoryInfo is not exported by the loaded NVML library".to_owned();
                return (
                    BackendMetric::Unavailable(reason.clone()),
                    BackendMetric::Unavailable(reason),
                );
            };
            let mut memory = NvmlMemory::default();
            // SAFETY: `memory` is a writable repr(C) structure of the documented shape.
            let result = unsafe { function(device, &mut memory) };
            (
                metric_result(result, memory.used, "nvmlDeviceGetMemoryInfo(used)"),
                metric_result(result, memory.total, "nvmlDeviceGetMemoryInfo(total)"),
            )
        }

        fn query_selected_u32_metric(
            &self,
            function: Option<unsafe extern "C" fn(NvmlDevice, u32, *mut u32) -> NvmlReturn>,
            device: NvmlDevice,
            selector: u32,
            api: &str,
        ) -> BackendMetric<u32> {
            let Some(function) = function else {
                return BackendMetric::Unavailable(format!(
                    "{api} is not exported by the loaded NVML library"
                ));
            };
            let mut value = 0_u32;
            // SAFETY: `value` is writable and the selector matches the documented NVML enum.
            let result = unsafe { function(device, selector, &mut value) };
            metric_result(result, value, api)
        }

        fn query_simple_u32_metric(
            &self,
            function: Option<unsafe extern "C" fn(NvmlDevice, *mut u32) -> NvmlReturn>,
            device: NvmlDevice,
            api: &str,
        ) -> BackendMetric<u32> {
            let Some(function) = function else {
                return BackendMetric::Unavailable(format!(
                    "{api} is not exported by the loaded NVML library"
                ));
            };
            let mut value = 0_u32;
            // SAFETY: `value` is writable and the symbol has the documented two-argument shape.
            let result = unsafe { function(device, &mut value) };
            metric_result(result, value, api)
        }
    }

    impl Drop for NvmlLibrary {
        fn drop(&mut self) {
            // SAFETY: this instance owns one successful NVML initialization and one module handle.
            unsafe {
                (self.shutdown)();
                FreeLibrary(self.module as *mut c_void);
            }
        }
    }

    fn metric_result<T>(result: i32, value: T, api: &str) -> BackendMetric<T> {
        if result == NVML_SUCCESS {
            BackendMetric::Live(value)
        } else if matches!(result, NVML_ERROR_NOT_SUPPORTED | NVML_ERROR_NO_PERMISSION) {
            BackendMetric::Unavailable(format!("{api} is unavailable: {}", nvml_error_name(result)))
        } else {
            BackendMetric::Error(format!("{api} failed: {}", nvml_error_name(result)))
        }
    }

    fn provider_error(result: i32, context: &str) -> BackendProviderError {
        let message = format!("{context}: {}", nvml_error_name(result));
        if matches!(
            result,
            NVML_ERROR_DRIVER_NOT_LOADED
                | NVML_ERROR_LIBRARY_NOT_FOUND
                | NVML_ERROR_FUNCTION_NOT_FOUND
                | NVML_ERROR_GPU_NOT_FOUND
                | NVML_ERROR_NO_PERMISSION
        ) {
            BackendProviderError::Unavailable(message)
        } else {
            BackendProviderError::Error(message)
        }
    }

    fn nvml_error_name(result: i32) -> String {
        let name = match result {
            0 => "NVML_SUCCESS",
            1 => "NVML_ERROR_UNINITIALIZED",
            2 => "NVML_ERROR_INVALID_ARGUMENT",
            3 => "NVML_ERROR_NOT_SUPPORTED",
            4 => "NVML_ERROR_NO_PERMISSION",
            5 => "NVML_ERROR_ALREADY_INITIALIZED",
            6 => "NVML_ERROR_NOT_FOUND",
            7 => "NVML_ERROR_INSUFFICIENT_SIZE",
            8 => "NVML_ERROR_INSUFFICIENT_POWER",
            9 => "NVML_ERROR_DRIVER_NOT_LOADED",
            10 => "NVML_ERROR_TIMEOUT",
            11 => "NVML_ERROR_IRQ_ISSUE",
            12 => "NVML_ERROR_LIBRARY_NOT_FOUND",
            13 => "NVML_ERROR_FUNCTION_NOT_FOUND",
            14 => "NVML_ERROR_CORRUPTED_INFOROM",
            15 => "NVML_ERROR_GPU_IS_LOST",
            16 => "NVML_ERROR_RESET_REQUIRED",
            17 => "NVML_ERROR_OPERATING_SYSTEM",
            18 => "NVML_ERROR_LIB_RM_VERSION_MISMATCH",
            19 => "NVML_ERROR_IN_USE",
            20 => "NVML_ERROR_MEMORY",
            21 => "NVML_ERROR_NO_DATA",
            22 => "NVML_ERROR_VGPU_ECC_NOT_SUPPORTED",
            23 => "NVML_ERROR_INSUFFICIENT_RESOURCES",
            24 => "NVML_ERROR_FREQ_NOT_SUPPORTED",
            25 => "NVML_ERROR_ARGUMENT_VERSION_MISMATCH",
            26 => "NVML_ERROR_DEPRECATED",
            27 => "NVML_ERROR_NOT_READY",
            28 => "NVML_ERROR_GPU_NOT_FOUND",
            29 => "NVML_ERROR_INVALID_STATE",
            999 => "NVML_ERROR_UNKNOWN",
            _ => "NVML_ERROR_UNRECOGNIZED",
        };
        format!("{name} ({result})")
    }

    fn join_notes(first: Option<String>, second: Option<String>) -> Option<String> {
        match (first, second) {
            (Some(first), Some(second)) => Some(format!("{first}; {second}")),
            (Some(note), None) | (None, Some(note)) => Some(note),
            (None, None) => None,
        }
    }

    fn required_symbol<T: Copy>(module: usize, name: &[u8]) -> Result<T, BackendProviderError> {
        optional_symbol(module, name).ok_or_else(|| {
            let symbol = String::from_utf8_lossy(name)
                .trim_end_matches('\0')
                .to_owned();
            BackendProviderError::Error(format!(
                "loaded nvml.dll does not export required symbol {symbol}"
            ))
        })
    }

    fn optional_symbol<T: Copy>(module: usize, name: &[u8]) -> Option<T> {
        // SAFETY: `module` is live, `name` is NUL-terminated, and each caller supplies the exact
        // documented function-pointer type for the symbol it requests.
        let address = unsafe { GetProcAddress(module as *mut c_void, name.as_ptr()) };
        if address.is_null() {
            None
        } else {
            debug_assert_eq!(mem::size_of::<T>(), mem::size_of::<*mut c_void>());
            // SAFETY: NVML function pointers and FARPROC are pointer-sized on Windows.
            Some(unsafe { mem::transmute_copy(&address) })
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{BackendProviderError, BackendSnapshot, GpuBackend};

    pub(super) struct NvmlBackend;

    impl NvmlBackend {
        pub(super) fn new() -> Self {
            Self
        }
    }

    impl GpuBackend for NvmlBackend {
        fn sample_adapters(&mut self) -> Result<BackendSnapshot, BackendProviderError> {
            Err(BackendProviderError::Unavailable(
                "NVIDIA NVML telemetry is currently implemented only for Windows".to_owned(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct FakeBackend {
        samples: VecDeque<Result<BackendSnapshot, BackendProviderError>>,
    }

    impl FakeBackend {
        fn new(samples: Vec<Result<BackendSnapshot, BackendProviderError>>) -> Self {
            Self {
                samples: samples.into(),
            }
        }
    }

    impl GpuBackend for FakeBackend {
        fn sample_adapters(&mut self) -> Result<BackendSnapshot, BackendProviderError> {
            self.samples
                .pop_front()
                .expect("fake backend sample sequence exhausted")
        }
    }

    fn adapter(index: u32, uuid: &str, gpu_usage: BackendMetric<u32>) -> BackendAdapterSample {
        BackendAdapterSample {
            identity: GpuAdapterIdentity {
                vendor: "nvidia".to_owned(),
                index,
                uuid: Some(uuid.to_owned()),
                name: Some(format!("GPU {index}")),
                stable_for_evidence: true,
                identity_note: None,
            },
            gpu_utilization_percent: gpu_usage,
            memory_utilization_percent: BackendMetric::Live(10),
            memory_used_bytes: BackendMetric::Live(2_000),
            memory_total_bytes: BackendMetric::Live(12_000),
            temperature_celsius: BackendMetric::Live(55),
            graphics_clock_mhz: BackendMetric::Live(1_800),
            memory_clock_mhz: BackendMetric::Live(7_000),
            power_milliwatts: BackendMetric::Live(125_000),
        }
    }

    fn snapshot(
        adapters: Vec<BackendAdapterSample>,
    ) -> Result<BackendSnapshot, BackendProviderError> {
        Ok(BackendSnapshot {
            reported_adapter_count: adapters.len() as u32,
            adapters,
        })
    }

    #[test]
    fn supports_multiple_uuid_identified_adapters_and_truthful_unavailable_metrics() {
        let mut second = adapter(1, "GPU-b", BackendMetric::Live(80));
        second.power_milliwatts =
            BackendMetric::Unavailable("power reporting unsupported".to_owned());
        let backend = FakeBackend::new(vec![snapshot(vec![
            adapter(0, "GPU-a", BackendMetric::Live(25)),
            second,
        ])]);
        let mut collector = GpuTelemetryCollector::new(backend);
        let result = collector.sample();

        assert_eq!(
            result.provider_adapter_count,
            TelemetryState::Live { value: 2 }
        );
        assert_eq!(result.adapters.len(), 2);
        assert!(
            result
                .adapters
                .iter()
                .all(|adapter| adapter.identity.stable_for_evidence)
        );
        assert_eq!(
            result.adapters[0].gpu_utilization_percent.live_value(),
            Some(&25)
        );
        assert!(matches!(
            result.adapters[1].power_milliwatts.state,
            TelemetryState::Unavailable { .. }
        ));
    }

    #[test]
    fn metric_query_failure_marks_previous_live_value_stale() {
        let backend = FakeBackend::new(vec![
            snapshot(vec![adapter(0, "GPU-a", BackendMetric::Live(25))]),
            snapshot(vec![adapter(
                0,
                "GPU-a",
                BackendMetric::Error("GPU was lost".to_owned()),
            )]),
        ]);
        let mut collector = GpuTelemetryCollector::new(backend);
        collector.sample();
        let result = collector.sample();

        assert!(matches!(
            result.adapters[0].gpu_utilization_percent.state,
            TelemetryState::Stale {
                last_value: Some(25),
                ..
            }
        ));
    }

    #[test]
    fn provider_failure_stales_last_known_adapter_and_count() {
        let backend = FakeBackend::new(vec![
            snapshot(vec![adapter(0, "GPU-a", BackendMetric::Live(25))]),
            Err(BackendProviderError::Error("NVML driver reset".to_owned())),
        ]);
        let mut collector = GpuTelemetryCollector::new(backend);
        collector.sample();
        let result = collector.sample();

        assert!(matches!(
            result.provider_adapter_count,
            TelemetryState::Stale {
                last_value: Some(1),
                ..
            }
        ));
        assert!(matches!(
            result.adapters[0].temperature_celsius.state,
            TelemetryState::Stale {
                last_value: Some(55),
                ..
            }
        ));
    }

    #[test]
    fn disappeared_adapter_remains_stale_and_recovers_by_uuid() {
        let backend = FakeBackend::new(vec![
            snapshot(vec![adapter(0, "GPU-a", BackendMetric::Live(25))]),
            snapshot(Vec::new()),
            snapshot(vec![adapter(0, "GPU-a", BackendMetric::Live(40))]),
        ]);
        let mut collector = GpuTelemetryCollector::new(backend);
        collector.sample();
        let missing = collector.sample();
        assert_eq!(missing.adapters.len(), 1);
        assert!(matches!(
            missing.adapters[0].gpu_utilization_percent.state,
            TelemetryState::Stale {
                last_value: Some(25),
                ..
            }
        ));

        let recovered = collector.sample();
        assert_eq!(
            recovered.adapters[0].gpu_utilization_percent.live_value(),
            Some(&40)
        );
    }

    #[test]
    fn initial_provider_absence_is_unavailable_not_zero() {
        let backend = FakeBackend::new(vec![Err(BackendProviderError::Unavailable(
            "nvml.dll missing".to_owned(),
        ))]);
        let mut collector = GpuTelemetryCollector::new(backend);
        let result = collector.sample();

        assert!(matches!(
            result.provider_adapter_count,
            TelemetryState::Unavailable { .. }
        ));
        assert!(result.adapters.is_empty());
    }
}
