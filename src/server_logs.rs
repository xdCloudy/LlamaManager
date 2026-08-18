use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::server_process::{ManagedProcessIdentity, ProcessExitEvidence, ProcessExitKind};

pub const DEFAULT_SERVER_LOG_RETENTION_BYTES: usize = 2 * 1024 * 1024;
pub const DEFAULT_SERVER_LOG_DISK_RETENTION_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerLogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerLogSeverity {
    Info,
    Warning,
    Error,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerLogEntry {
    pub sequence: u64,
    pub observed_at_unix_ms: u64,
    pub pid: u32,
    pub stream: ServerLogStream,
    pub text: String,
    pub truncated: bool,
}

impl ServerLogEntry {
    /// Presentation-only severity. Lifecycle success/failure is never inferred from log text.
    pub fn presentation_severity(&self) -> ServerLogSeverity {
        let lower = self.text.to_ascii_lowercase();
        if self.stream == ServerLogStream::Stderr
            && (lower.contains("fatal") || lower.contains("panic"))
        {
            ServerLogSeverity::Fatal
        } else if lower.contains("warning") || lower.contains("warn:") {
            ServerLogSeverity::Warning
        } else if self.stream == ServerLogStream::Stderr {
            ServerLogSeverity::Error
        } else {
            ServerLogSeverity::Info
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerLogSnapshot {
    pub entries: Vec<ServerLogEntry>,
    pub retained_bytes: usize,
    pub retention_limit_bytes: usize,
    pub evicted_entries: u64,
}

impl ServerLogSnapshot {
    pub fn redacted(&self, secrets: &[String]) -> Self {
        let mut snapshot = self.clone();
        for entry in &mut snapshot.entries {
            entry.text = redact_text(&entry.text, secrets);
        }
        snapshot
    }

    pub fn export_redacted(&self, path: &Path, secrets: &[String]) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(path)?;
        for entry in self.redacted(secrets).entries {
            file.write_all(&encode_entry(&entry))?;
        }
        file.flush()
    }
}

#[derive(Debug)]
struct ServerLogBufferInner {
    entries: VecDeque<ServerLogEntry>,
    retained_bytes: usize,
    next_sequence: u64,
    evicted_entries: u64,
}

#[derive(Debug, Clone)]
pub struct ServerLogBuffer {
    retention_limit_bytes: usize,
    inner: Arc<Mutex<ServerLogBufferInner>>,
}

impl ServerLogBuffer {
    pub fn new(retention_limit_bytes: usize) -> Self {
        Self {
            retention_limit_bytes: retention_limit_bytes.max(1),
            inner: Arc::new(Mutex::new(ServerLogBufferInner {
                entries: VecDeque::new(),
                retained_bytes: 0,
                next_sequence: 0,
                evicted_entries: 0,
            })),
        }
    }

    pub fn push(&self, pid: u32, stream: ServerLogStream, text: impl Into<String>) -> ServerLogEntry {
        let mut text = text.into();
        let mut truncated = false;
        if text.len() > self.retention_limit_bytes {
            let keep_from = text.len() - self.retention_limit_bytes;
            let keep_from = next_char_boundary(&text, keep_from);
            text = text[keep_from..].to_owned();
            truncated = true;
        }

        let bytes = text.len();
        let mut inner = self.inner.lock().expect("server log mutex poisoned");
        while inner.retained_bytes.saturating_add(bytes) > self.retention_limit_bytes {
            let Some(evicted) = inner.entries.pop_front() else {
                break;
            };
            inner.retained_bytes = inner.retained_bytes.saturating_sub(evicted.text.len());
            inner.evicted_entries = inner.evicted_entries.saturating_add(1);
        }

        let entry = ServerLogEntry {
            sequence: inner.next_sequence,
            observed_at_unix_ms: now_unix_ms(),
            pid,
            stream,
            text,
            truncated,
        };
        inner.next_sequence = inner.next_sequence.saturating_add(1);
        inner.retained_bytes = inner.retained_bytes.saturating_add(bytes);
        inner.entries.push_back(entry.clone());
        entry
    }

    pub fn snapshot(&self) -> ServerLogSnapshot {
        let inner = self.inner.lock().expect("server log mutex poisoned");
        ServerLogSnapshot {
            entries: inner.entries.iter().cloned().collect(),
            retained_bytes: inner.retained_bytes,
            retention_limit_bytes: self.retention_limit_bytes,
            evicted_entries: inner.evicted_entries,
        }
    }
}

impl Default for ServerLogBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_SERVER_LOG_RETENTION_BYTES)
    }
}

#[derive(Debug)]
struct BoundedServerLogFile {
    path: PathBuf,
    retention_limit_bytes: u64,
    current_bytes: u64,
    secrets: Vec<String>,
}

impl BoundedServerLogFile {
    fn open(path: PathBuf, retention_limit_bytes: u64, secrets: Vec<String>) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let current_bytes = fs::metadata(&path).map(|metadata| metadata.len()).unwrap_or(0);
        Ok(Self {
            path,
            retention_limit_bytes: retention_limit_bytes.max(1),
            current_bytes,
            secrets,
        })
    }

    fn append(&mut self, entry: &ServerLogEntry) -> io::Result<()> {
        let mut redacted = entry.clone();
        redacted.text = redact_text(&redacted.text, &self.secrets);
        let mut encoded = encode_entry(&redacted);
        if encoded.len() as u64 > self.retention_limit_bytes {
            let limit = usize::try_from(self.retention_limit_bytes).unwrap_or(usize::MAX);
            encoded = tail_bytes_on_char_boundary(&encoded, limit);
        }

        let encoded_len = encoded.len() as u64;
        let rotate = self.current_bytes.saturating_add(encoded_len) > self.retention_limit_bytes;
        let mut options = OpenOptions::new();
        options.create(true).write(true);
        if rotate {
            options.truncate(true);
            self.current_bytes = 0;
        } else {
            options.append(true);
        }
        let mut file = options.open(&self.path)?;
        file.write_all(&encoded)?;
        file.flush()?;
        self.current_bytes = self.current_bytes.saturating_add(encoded_len);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ServerLogCapture {
    buffer: ServerLogBuffer,
    disk: Option<Arc<Mutex<BoundedServerLogFile>>>,
}

impl ServerLogCapture {
    pub fn memory_only(retention_limit_bytes: usize) -> Self {
        Self {
            buffer: ServerLogBuffer::new(retention_limit_bytes),
            disk: None,
        }
    }

    pub fn with_disk(
        path: PathBuf,
        memory_retention_bytes: usize,
        disk_retention_bytes: u64,
        secrets: Vec<String>,
    ) -> io::Result<Self> {
        Ok(Self {
            buffer: ServerLogBuffer::new(memory_retention_bytes),
            disk: Some(Arc::new(Mutex::new(BoundedServerLogFile::open(
                path,
                disk_retention_bytes,
                secrets,
            )?))),
        })
    }

    pub fn push(&self, pid: u32, stream: ServerLogStream, text: impl Into<String>) {
        let entry = self.buffer.push(pid, stream, text);
        if let Some(disk) = &self.disk
            && let Ok(mut disk) = disk.lock()
        {
            let _ = disk.append(&entry);
        }
    }

    pub fn snapshot(&self) -> ServerLogSnapshot {
        self.buffer.snapshot()
    }
}

impl Default for ServerLogCapture {
    fn default() -> Self {
        Self::memory_only(DEFAULT_SERVER_LOG_RETENTION_BYTES)
    }
}

fn encode_entry(entry: &ServerLogEntry) -> Vec<u8> {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\n",
        entry.sequence,
        entry.observed_at_unix_ms,
        entry.pid,
        match entry.stream {
            ServerLogStream::Stdout => "stdout",
            ServerLogStream::Stderr => "stderr",
        },
        if entry.truncated { "truncated" } else { "complete" },
        entry.text.replace('\n', "\\n").replace('\r', "\\r")
    )
    .into_bytes()
}

fn tail_bytes_on_char_boundary(bytes: &[u8], limit: usize) -> Vec<u8> {
    if bytes.len() <= limit {
        return bytes.to_vec();
    }
    let mut start = bytes.len() - limit;
    while start < bytes.len() && (bytes[start] & 0b1100_0000) == 0b1000_0000 {
        start += 1;
    }
    bytes[start..].to_vec()
}

fn next_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

pub fn redact_text(text: &str, secrets: &[String]) -> String {
    let mut redacted = text.to_owned();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        redacted = redacted.replace(secret, "<redacted>");
    }
    redact_bearer_tokens(&redacted)
}

fn redact_bearer_tokens(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut remaining = text;
    loop {
        let lower = remaining.to_ascii_lowercase();
        let Some(index) = lower.find("bearer ") else {
            output.push_str(remaining);
            break;
        };
        let prefix_end = index + "bearer ".len();
        output.push_str(&remaining[..prefix_end]);
        remaining = &remaining[prefix_end..];
        let token_end = remaining.find(char::is_whitespace).unwrap_or(remaining.len());
        output.push_str("<redacted>");
        remaining = &remaining[token_end..];
    }
    output
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerLifecyclePhase {
    Stopped,
    Starting,
    Ready,
    Stopping,
    Failed,
    Crashed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerLifecycleSnapshot {
    pub phase: ServerLifecyclePhase,
    pub process: Option<ManagedProcessIdentity>,
    pub last_exit: Option<ProcessExitEvidence>,
    pub detail: Option<String>,
    pub observed_at_unix_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ServerLifecycleTracker {
    snapshot: ServerLifecycleSnapshot,
}

impl Default for ServerLifecycleTracker {
    fn default() -> Self {
        Self {
            snapshot: ServerLifecycleSnapshot {
                phase: ServerLifecyclePhase::Stopped,
                process: None,
                last_exit: None,
                detail: None,
                observed_at_unix_ms: now_unix_ms(),
            },
        }
    }
}

impl ServerLifecycleTracker {
    pub fn snapshot(&self) -> &ServerLifecycleSnapshot {
        &self.snapshot
    }

    pub fn mark_starting(&mut self, identity: ManagedProcessIdentity) {
        self.set(ServerLifecyclePhase::Starting, Some(identity), None, None);
    }

    pub fn mark_ready(&mut self) {
        self.snapshot.phase = ServerLifecyclePhase::Ready;
        self.snapshot.detail = None;
        self.snapshot.observed_at_unix_ms = now_unix_ms();
    }

    pub fn mark_stopping(&mut self) {
        self.snapshot.phase = ServerLifecyclePhase::Stopping;
        self.snapshot.observed_at_unix_ms = now_unix_ms();
    }

    pub fn mark_failed(&mut self, detail: impl Into<String>) {
        self.snapshot.phase = ServerLifecyclePhase::Failed;
        self.snapshot.detail = Some(detail.into());
        self.snapshot.observed_at_unix_ms = now_unix_ms();
    }

    pub fn mark_exit(&mut self, evidence: ProcessExitEvidence) {
        let phase = match (self.snapshot.phase, evidence.kind, evidence.code) {
            (ServerLifecyclePhase::Stopping, _, _) => ServerLifecyclePhase::Stopped,
            (_, ProcessExitKind::ForceKilled | ProcessExitKind::DropCleanup, _) => {
                ServerLifecyclePhase::Stopped
            }
            (_, ProcessExitKind::Natural, Some(0))
                if self.snapshot.phase != ServerLifecyclePhase::Ready =>
            {
                ServerLifecyclePhase::Stopped
            }
            _ => ServerLifecyclePhase::Crashed,
        };
        self.snapshot.phase = phase;
        self.snapshot.last_exit = Some(evidence);
        self.snapshot.observed_at_unix_ms = now_unix_ms();
        if phase == ServerLifecyclePhase::Stopped {
            self.snapshot.process = None;
        }
    }

    pub fn reconcile_after_restart(
        &mut self,
        previously_managed: Option<ManagedProcessIdentity>,
        process_still_exists: bool,
    ) {
        if process_still_exists {
            self.set(
                ServerLifecyclePhase::Unknown,
                previously_managed,
                None,
                Some(
                    "a previously managed process still exists, but this app instance does not own its Job Object/process handle"
                        .into(),
                ),
            );
        } else {
            self.set(ServerLifecyclePhase::Stopped, None, None, None);
        }
    }

    fn set(
        &mut self,
        phase: ServerLifecyclePhase,
        process: Option<ManagedProcessIdentity>,
        last_exit: Option<ProcessExitEvidence>,
        detail: Option<String>,
    ) {
        self.snapshot = ServerLifecycleSnapshot {
            phase,
            process,
            last_exit,
            detail,
            observed_at_unix_ms: now_unix_ms(),
        };
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn identity(pid: u32) -> ManagedProcessIdentity {
        ManagedProcessIdentity {
            pid,
            executable: PathBuf::from(r"C:\llama cpp 模型\llama-server.exe"),
            started_at_unix_ms: 1,
        }
    }

    #[test]
    fn bounded_buffer_evicts_old_entries_without_exceeding_limit() {
        let logs = ServerLogBuffer::new(128);
        for index in 0..1000 {
            logs.push(42, ServerLogStream::Stdout, format!("line-{index:04}-xxxxxxxx"));
        }
        let snapshot = logs.snapshot();
        assert!(snapshot.retained_bytes <= 128);
        assert!(snapshot.evicted_entries > 0);
        assert!(!snapshot.entries.is_empty());
        assert!(snapshot
            .entries
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence));
    }

    #[test]
    fn giant_unicode_entry_is_truncated_on_a_char_boundary() {
        let logs = ServerLogBuffer::new(17);
        logs.push(7, ServerLogStream::Stderr, "模型模型模型模型模型模型模型");
        let snapshot = logs.snapshot();
        let entry = snapshot.entries.last().unwrap();
        assert!(entry.truncated);
        assert!(entry.text.len() <= 17);
        assert!(entry.text.is_char_boundary(0));
    }

    #[test]
    fn redaction_hides_explicit_and_bearer_secrets_but_not_original_snapshot() {
        let logs = ServerLogBuffer::new(1024);
        logs.push(
            9,
            ServerLogStream::Stderr,
            "Authorization: Bearer abc123 api-key=secret-value",
        );
        let original = logs.snapshot();
        let redacted = original.redacted(&["secret-value".into()]);
        assert!(original.entries[0].text.contains("abc123"));
        assert!(!redacted.entries[0].text.contains("abc123"));
        assert!(!redacted.entries[0].text.contains("secret-value"));
        assert!(redacted.entries[0].text.contains("<redacted>"));
    }

    #[test]
    fn bounded_disk_capture_stays_within_limit_and_redacts_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("server logs 模型").join("server.log");
        let capture = ServerLogCapture::with_disk(
            path.clone(),
            1024,
            256,
            vec!["secret-value".into()],
        )
        .unwrap();
        for index in 0..100 {
            capture.push(
                12,
                ServerLogStream::Stderr,
                format!("warning {index} secret-value Authorization: Bearer tok-{index}"),
            );
        }
        let bytes = fs::metadata(&path).unwrap().len();
        assert!(bytes <= 256, "disk retention exceeded limit: {bytes}");
        let text = fs::read_to_string(path).unwrap();
        assert!(!text.contains("secret-value"));
        assert!(!text.contains("Bearer tok-"));
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn presentation_severity_never_changes_lifecycle_truth() {
        let logs = ServerLogBuffer::new(1024);
        let warning = logs.push(1, ServerLogStream::Stdout, "warning: warmup");
        let fatal = logs.push(1, ServerLogStream::Stderr, "fatal: model load failed");
        assert_eq!(warning.presentation_severity(), ServerLogSeverity::Warning);
        assert_eq!(fatal.presentation_severity(), ServerLogSeverity::Fatal);
    }

    #[test]
    fn lifecycle_records_crash_stop_and_restart_unknown_truthfully() {
        let mut tracker = ServerLifecycleTracker::default();
        tracker.mark_starting(identity(100));
        tracker.mark_ready();
        tracker.mark_exit(ProcessExitEvidence {
            code: Some(7),
            kind: ProcessExitKind::Natural,
            observed_at_unix_ms: 2,
        });
        assert_eq!(tracker.snapshot().phase, ServerLifecyclePhase::Crashed);
        assert_eq!(tracker.snapshot().last_exit.as_ref().unwrap().code, Some(7));

        tracker.reconcile_after_restart(Some(identity(100)), true);
        assert_eq!(tracker.snapshot().phase, ServerLifecyclePhase::Unknown);
        assert!(tracker.snapshot().detail.is_some());

        tracker.reconcile_after_restart(Some(identity(100)), false);
        assert_eq!(tracker.snapshot().phase, ServerLifecyclePhase::Stopped);
    }

    #[test]
    fn lifecycle_distinguishes_stopping_force_kill_from_crash() {
        let mut tracker = ServerLifecycleTracker::default();
        tracker.mark_starting(identity(101));
        tracker.mark_ready();
        tracker.mark_stopping();
        tracker.mark_exit(ProcessExitEvidence {
            code: None,
            kind: ProcessExitKind::ForceKilled,
            observed_at_unix_ms: 3,
        });
        assert_eq!(tracker.snapshot().phase, ServerLifecyclePhase::Stopped);
        assert_eq!(
            tracker.snapshot().last_exit.as_ref().unwrap().kind,
            ProcessExitKind::ForceKilled
        );
    }
}
