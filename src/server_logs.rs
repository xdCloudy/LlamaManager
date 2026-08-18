use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::server_process::{ManagedProcessIdentity, ProcessExitEvidence, ProcessExitKind};

pub const DEFAULT_SERVER_LOG_RETENTION_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerLogStream {
    Stdout,
    Stderr,
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
        let mut file = File::create(path)?;
        for entry in self.redacted(secrets).entries {
            writeln!(
                file,
                "{}\t{}\t{}\t{}\t{}",
                entry.sequence,
                entry.observed_at_unix_ms,
                entry.pid,
                match entry.stream {
                    ServerLogStream::Stdout => "stdout",
                    ServerLogStream::Stderr => "stderr",
                },
                entry.text.replace('\n', "\\n").replace('\r', "\\r")
            )?;
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

    pub fn push(&self, pid: u32, stream: ServerLogStream, text: impl Into<String>) {
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

        let sequence = inner.next_sequence;
        inner.next_sequence = inner.next_sequence.saturating_add(1);
        inner.retained_bytes = inner.retained_bytes.saturating_add(bytes);
        inner.entries.push_back(ServerLogEntry {
            sequence,
            observed_at_unix_ms: now_unix_ms(),
            pid,
            stream,
            text,
            truncated,
        });
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
    redacted = redact_bearer_tokens(&redacted);
    redacted
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
        let token_end = remaining
            .find(char::is_whitespace)
            .unwrap_or(remaining.len());
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
        self.set(
            ServerLifecyclePhase::Starting,
            Some(identity),
            None,
            None,
        );
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
            (_, ProcessExitKind::Natural, Some(0)) if self.snapshot.phase != ServerLifecyclePhase::Ready => {
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
        assert!(snapshot.entries.windows(2).all(|pair| pair[0].sequence < pair[1].sequence));
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
