use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use thiserror::Error;

use crate::server_command::ServerLaunchSpec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProcessIdentity {
    pub pid: u32,
    pub executable: PathBuf,
    pub started_at_unix_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessExitKind {
    Natural,
    ForceKilled,
    DropCleanup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessExitEvidence {
    pub code: Option<i32>,
    pub kind: ProcessExitKind,
    pub observed_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedProcessState {
    Running(ManagedProcessIdentity),
    Exited {
        identity: ManagedProcessIdentity,
        evidence: ProcessExitEvidence,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GracefulStopOutcome {
    Exited(ProcessExitEvidence),
    GracePeriodExpired { waited: Duration },
}

#[derive(Debug, Error)]
pub enum ProcessSupervisorError {
    #[error("a managed server process is already running (pid {pid})")]
    AlreadyRunning { pid: u32 },

    #[error("failed to spawn managed process {executable}: {source}")]
    Spawn {
        executable: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to create/assign Windows Job Object for pid {pid}: {source}")]
    JobObject {
        pid: u32,
        #[source]
        source: io::Error,
    },

    #[error("failed to inspect managed process pid {pid}: {source}")]
    Inspect {
        pid: u32,
        #[source]
        source: io::Error,
    },

    #[error("failed to force-kill managed process tree pid {pid}: {source}")]
    ForceKill {
        pid: u32,
        #[source]
        source: io::Error,
    },

    #[error("graceful shutdown request failed for pid {pid}: {message}")]
    GracefulRequest { pid: u32, message: String },

    #[error("no managed server process exists")]
    NotRunning,
}

#[derive(Debug, Clone)]
struct ManagedProcessSpec {
    executable: PathBuf,
    argv: Vec<OsString>,
    cwd: PathBuf,
    environment: BTreeMap<OsString, OsString>,
}

impl From<&ServerLaunchSpec> for ManagedProcessSpec {
    fn from(value: &ServerLaunchSpec) -> Self {
        Self {
            executable: value.executable.clone(),
            argv: value.argv.clone(),
            cwd: value.cwd.clone(),
            environment: value.environment.clone(),
        }
    }
}

#[derive(Debug)]
pub struct ManagedServerProcess {
    child: Child,
    identity: ManagedProcessIdentity,
    job: PlatformJob,
    exit_evidence: Option<ProcessExitEvidence>,
}

impl ManagedServerProcess {
    pub fn identity(&self) -> &ManagedProcessIdentity {
        &self.identity
    }

    pub fn state(&mut self) -> Result<ManagedProcessState, ProcessSupervisorError> {
        if let Some(evidence) = &self.exit_evidence {
            return Ok(ManagedProcessState::Exited {
                identity: self.identity.clone(),
                evidence: evidence.clone(),
            });
        }

        match self
            .child
            .try_wait()
            .map_err(|source| ProcessSupervisorError::Inspect {
                pid: self.identity.pid,
                source,
            })? {
            Some(status) => {
                let evidence = exit_evidence(status, ProcessExitKind::Natural);
                self.exit_evidence = Some(evidence.clone());
                Ok(ManagedProcessState::Exited {
                    identity: self.identity.clone(),
                    evidence,
                })
            }
            None => Ok(ManagedProcessState::Running(self.identity.clone())),
        }
    }

    pub fn wait_for_cooperative_exit<F>(
        &mut self,
        timeout: Duration,
        request_graceful_shutdown: F,
    ) -> Result<GracefulStopOutcome, ProcessSupervisorError>
    where
        F: FnOnce(&ManagedProcessIdentity) -> Result<(), String>,
    {
        request_graceful_shutdown(&self.identity).map_err(|message| {
            ProcessSupervisorError::GracefulRequest {
                pid: self.identity.pid,
                message,
            }
        })?;

        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.state()? {
                ManagedProcessState::Exited { evidence, .. } => {
                    return Ok(GracefulStopOutcome::Exited(evidence));
                }
                ManagedProcessState::Running(_) if std::time::Instant::now() >= deadline => {
                    return Ok(GracefulStopOutcome::GracePeriodExpired { waited: timeout });
                }
                ManagedProcessState::Running(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    pub fn force_kill(&mut self) -> Result<ProcessExitEvidence, ProcessSupervisorError> {
        if let Some(evidence) = &self.exit_evidence {
            return Ok(evidence.clone());
        }
        if let Some(status) =
            self.child
                .try_wait()
                .map_err(|source| ProcessSupervisorError::Inspect {
                    pid: self.identity.pid,
                    source,
                })?
        {
            let evidence = exit_evidence(status, ProcessExitKind::Natural);
            self.exit_evidence = Some(evidence.clone());
            return Ok(evidence);
        }

        self.job
            .terminate(0xEE)
            .map_err(|source| ProcessSupervisorError::ForceKill {
                pid: self.identity.pid,
                source,
            })?;
        let status = self
            .child
            .wait()
            .map_err(|source| ProcessSupervisorError::Inspect {
                pid: self.identity.pid,
                source,
            })?;
        let evidence = exit_evidence(status, ProcessExitKind::ForceKilled);
        self.exit_evidence = Some(evidence.clone());
        Ok(evidence)
    }
}

impl Drop for ManagedServerProcess {
    fn drop(&mut self) {
        if self.exit_evidence.is_some() {
            return;
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.exit_evidence = Some(exit_evidence(status, ProcessExitKind::Natural));
            }
            Ok(None) => {
                let _ = self.job.terminate(0xED);
                if let Ok(status) = self.child.wait() {
                    self.exit_evidence = Some(exit_evidence(status, ProcessExitKind::DropCleanup));
                }
            }
            Err(_) => {
                let _ = self.job.terminate(0xED);
                let _ = self.child.wait();
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct ServerProcessSupervisor {
    current: Option<ManagedServerProcess>,
}

impl ServerProcessSupervisor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_server(
        &mut self,
        spec: &ServerLaunchSpec,
    ) -> Result<&ManagedProcessIdentity, ProcessSupervisorError> {
        self.start_process(ManagedProcessSpec::from(spec))
    }

    pub fn state(&mut self) -> Result<Option<ManagedProcessState>, ProcessSupervisorError> {
        self.current
            .as_mut()
            .map(ManagedServerProcess::state)
            .transpose()
    }

    pub fn process_mut(&mut self) -> Result<&mut ManagedServerProcess, ProcessSupervisorError> {
        self.current
            .as_mut()
            .ok_or(ProcessSupervisorError::NotRunning)
    }

    pub fn clear_exited(&mut self) -> Result<bool, ProcessSupervisorError> {
        let Some(process) = self.current.as_mut() else {
            return Ok(false);
        };
        if matches!(process.state()?, ManagedProcessState::Exited { .. }) {
            self.current = None;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn start_process(
        &mut self,
        spec: ManagedProcessSpec,
    ) -> Result<&ManagedProcessIdentity, ProcessSupervisorError> {
        if let Some(current) = self.current.as_mut() {
            match current.state()? {
                ManagedProcessState::Running(identity) => {
                    return Err(ProcessSupervisorError::AlreadyRunning { pid: identity.pid });
                }
                ManagedProcessState::Exited { .. } => self.current = None,
            }
        }

        let mut command = Command::new(&spec.executable);
        command
            .args(&spec.argv)
            .current_dir(&spec.cwd)
            .envs(&spec.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command
            .spawn()
            .map_err(|source| ProcessSupervisorError::Spawn {
                executable: spec.executable.clone(),
                source,
            })?;
        let pid = child.id();
        let job = match PlatformJob::create_and_assign(&child) {
            Ok(job) => job,
            Err(source) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProcessSupervisorError::JobObject { pid, source });
            }
        };

        self.current = Some(ManagedServerProcess {
            child,
            identity: ManagedProcessIdentity {
                pid,
                executable: spec.executable,
                started_at_unix_ms: now_unix_ms(),
            },
            job,
            exit_evidence: None,
        });
        Ok(&self.current.as_ref().expect("assigned above").identity)
    }
}

fn exit_evidence(status: ExitStatus, kind: ProcessExitKind) -> ProcessExitEvidence {
    ProcessExitEvidence {
        code: status.code(),
        kind,
        observed_at_unix_ms: now_unix_ms(),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(windows)]
#[derive(Debug)]
struct PlatformJob {
    handle: *mut std::ffi::c_void,
}

#[cfg(windows)]
unsafe impl Send for PlatformJob {}

#[cfg(windows)]
impl PlatformJob {
    fn create_and_assign(child: &Child) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;

        type Handle = *mut std::ffi::c_void;
        const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
        const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

        #[repr(C)]
        #[derive(Default)]
        struct JobObjectBasicLimitInformation {
            per_process_user_time_limit: i64,
            per_job_user_time_limit: i64,
            limit_flags: u32,
            minimum_working_set_size: usize,
            maximum_working_set_size: usize,
            active_process_limit: u32,
            affinity: usize,
            priority_class: u32,
            scheduling_class: u32,
        }

        #[repr(C)]
        #[derive(Default)]
        struct IoCounters {
            read_operation_count: u64,
            write_operation_count: u64,
            other_operation_count: u64,
            read_transfer_count: u64,
            write_transfer_count: u64,
            other_transfer_count: u64,
        }

        #[repr(C)]
        #[derive(Default)]
        struct JobObjectExtendedLimitInformation {
            basic_limit_information: JobObjectBasicLimitInformation,
            io_info: IoCounters,
            process_memory_limit: usize,
            job_memory_limit: usize,
            peak_process_memory_used: usize,
            peak_job_memory_used: usize,
        }

        unsafe extern "system" {
            fn CreateJobObjectW(attributes: *mut std::ffi::c_void, name: *const u16) -> Handle;
            fn SetInformationJobObject(
                job: Handle,
                information_class: i32,
                information: *const std::ffi::c_void,
                information_length: u32,
            ) -> i32;
            fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
            fn CloseHandle(handle: Handle) -> i32;
        }

        let handle = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut information = JobObjectExtendedLimitInformation::default();
        information.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                std::ptr::addr_of!(information).cast(),
                std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
            )
        };
        if configured == 0 {
            let error = io::Error::last_os_error();
            unsafe {
                CloseHandle(handle);
            }
            return Err(error);
        }

        let assigned = unsafe { AssignProcessToJobObject(handle, child.as_raw_handle() as Handle) };
        if assigned == 0 {
            let error = io::Error::last_os_error();
            unsafe {
                CloseHandle(handle);
            }
            return Err(error);
        }

        Ok(Self { handle })
    }

    fn terminate(&self, exit_code: u32) -> io::Result<()> {
        type Handle = *mut std::ffi::c_void;
        unsafe extern "system" {
            fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
        }
        let result = unsafe { TerminateJobObject(self.handle, exit_code) };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
impl Drop for PlatformJob {
    fn drop(&mut self) {
        type Handle = *mut std::ffi::c_void;
        unsafe extern "system" {
            fn CloseHandle(handle: Handle) -> i32;
        }
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

#[cfg(not(windows))]
#[derive(Debug)]
struct PlatformJob;

#[cfg(not(windows))]
impl PlatformJob {
    fn create_and_assign(_child: &Child) -> io::Result<Self> {
        Ok(Self)
    }

    fn terminate(&self, _exit_code: u32) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process-tree Job Object termination is Windows-only",
        ))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{fs, path::Path, process::Command};

    fn pwsh_spec(script: &str, environment: BTreeMap<OsString, OsString>) -> ManagedProcessSpec {
        ManagedProcessSpec {
            executable: PathBuf::from("pwsh.exe"),
            argv: vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-Command".into(),
                script.into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            environment,
        }
    }

    #[test]
    fn spawn_returns_identity_and_duplicate_start_is_rejected() {
        let mut supervisor = ServerProcessSupervisor::new();
        let first = supervisor
            .start_process(pwsh_spec("Start-Sleep -Seconds 30", BTreeMap::new()))
            .unwrap()
            .clone();
        assert!(first.pid > 0);
        assert!(matches!(
            supervisor.state().unwrap(),
            Some(ManagedProcessState::Running(identity)) if identity.pid == first.pid
        ));
        assert!(matches!(
            supervisor.start_process(pwsh_spec("exit 0", BTreeMap::new())),
            Err(ProcessSupervisorError::AlreadyRunning { pid }) if pid == first.pid
        ));
        supervisor.process_mut().unwrap().force_kill().unwrap();
    }

    #[test]
    fn unexpected_exit_is_retained_and_supervisor_can_restart_after_it() {
        let mut supervisor = ServerProcessSupervisor::new();
        let first_pid = supervisor
            .start_process(pwsh_spec("exit 7", BTreeMap::new()))
            .unwrap()
            .pid;
        let evidence = wait_until_exited(&mut supervisor, Duration::from_secs(5));
        assert_eq!(evidence.code, Some(7));
        assert_eq!(evidence.kind, ProcessExitKind::Natural);
        assert!(supervisor.clear_exited().unwrap());

        let second_pid = supervisor
            .start_process(pwsh_spec("Start-Sleep -Seconds 30", BTreeMap::new()))
            .unwrap()
            .pid;
        assert_ne!(first_pid, second_pid);
        supervisor.process_mut().unwrap().force_kill().unwrap();
    }

    #[test]
    fn cooperative_grace_exit_timeout_and_force_kill_are_distinct() {
        let mut supervisor = ServerProcessSupervisor::new();
        supervisor
            .start_process(pwsh_spec(
                "Start-Sleep -Milliseconds 80; exit 0",
                BTreeMap::new(),
            ))
            .unwrap();
        let graceful = supervisor
            .process_mut()
            .unwrap()
            .wait_for_cooperative_exit(Duration::from_secs(5), |_| Ok(()))
            .unwrap();
        assert!(matches!(
            graceful,
            GracefulStopOutcome::Exited(ProcessExitEvidence {
                kind: ProcessExitKind::Natural,
                code: Some(0),
                ..
            })
        ));
        supervisor.clear_exited().unwrap();

        supervisor
            .start_process(pwsh_spec("Start-Sleep -Seconds 30", BTreeMap::new()))
            .unwrap();
        let timeout = supervisor
            .process_mut()
            .unwrap()
            .wait_for_cooperative_exit(Duration::from_millis(80), |_| Ok(()))
            .unwrap();
        assert_eq!(
            timeout,
            GracefulStopOutcome::GracePeriodExpired {
                waited: Duration::from_millis(80)
            }
        );
        let forced = supervisor.process_mut().unwrap().force_kill().unwrap();
        assert_eq!(forced.kind, ProcessExitKind::ForceKilled);
    }

    #[test]
    fn job_object_force_kill_terminates_descendant_process_tree() {
        let temp = tempfile::tempdir().unwrap();
        let pid_file = temp.path().join("grandchild pid.txt");
        let mut environment = BTreeMap::new();
        environment.insert(
            "GRANDCHILD_PID_FILE".into(),
            pid_file.as_os_str().to_os_string(),
        );
        let script = "$p = Start-Process -FilePath pwsh.exe -ArgumentList '-NoLogo','-NoProfile','-Command','Start-Sleep -Seconds 30' -PassThru; Set-Content -LiteralPath $env:GRANDCHILD_PID_FILE -Value $p.Id; Start-Sleep -Seconds 30";

        let mut supervisor = ServerProcessSupervisor::new();
        supervisor
            .start_process(pwsh_spec(script, environment))
            .unwrap();
        wait_for_file(&pid_file, Duration::from_secs(5));
        let grandchild_pid: u32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(windows_process_is_alive(grandchild_pid));

        supervisor.process_mut().unwrap().force_kill().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while windows_process_is_alive(grandchild_pid) && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        assert!(
            !windows_process_is_alive(grandchild_pid),
            "grandchild pid {grandchild_pid} survived Job Object termination"
        );
    }

    #[test]
    fn dropping_supervisor_cleans_up_managed_process_tree() {
        let temp = tempfile::tempdir().unwrap();
        let pid_file = temp.path().join("child pid.txt");
        let pid = {
            let mut supervisor = ServerProcessSupervisor::new();
            let identity = supervisor
                .start_process(pwsh_spec("Start-Sleep -Seconds 30", BTreeMap::new()))
                .unwrap()
                .clone();
            fs::write(&pid_file, identity.pid.to_string()).unwrap();
            identity.pid
        };

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while windows_process_is_alive(pid) && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        assert!(!windows_process_is_alive(pid));
    }

    fn wait_until_exited(
        supervisor: &mut ServerProcessSupervisor,
        timeout: Duration,
    ) -> ProcessExitEvidence {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(ManagedProcessState::Exited { evidence, .. }) = supervisor.state().unwrap()
            {
                return evidence;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "process did not exit in time"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_file(path: &Path, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while !path.is_file() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(path.is_file(), "expected fixture file {}", path.display());
    }

    fn windows_process_is_alive(pid: u32) -> bool {
        let filter = format!("PID eq {pid}");
        let output = Command::new("tasklist.exe")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .output()
            .expect("tasklist must be available on Windows");
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines().any(|line| {
            let fields: Vec<_> = line.trim_matches('"').split("\",\"").collect();
            fields.get(1).and_then(|value| value.parse::<u32>().ok()) == Some(pid)
        })
    }
}
