//! MP-08 / MP-09 / MP-10 / MP-11 A03: kernel-owned watched processes.
//! The kernel starts argv (no shell) in the agent's workspace, owns the
//! process group, drains bounded sanitized output and reports exit or one
//! output match. It never attaches to an arbitrary PID.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentWake, Operation};
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};

const TAIL_BYTES: usize = 4_096;
const LINE_BYTES: usize = 1_024;
const ENV_ALLOW: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "TMPDIR",
    "TERM",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
    "SSH_AUTH_SOCK",
    "GH_CONFIG_DIR",
];

type Owned = Arc<std::sync::Mutex<Child>>;

#[derive(Default)]
pub(crate) struct WatchedProcesses {
    live: std::sync::Mutex<HashMap<String, Owned>>,
}

impl WatchedProcesses {
    pub(crate) fn count(&self) -> usize {
        self.live.lock().map(|l| l.len()).unwrap_or(0)
    }
    pub(super) fn contains(&self, id: &str) -> bool {
        self.live.lock().is_ok_and(|l| l.contains_key(id))
    }
    /// Signals the owned group only while its leader is unreaped, so the
    /// group id cannot have been reused; SIGKILL follows after a grace period.
    pub(super) fn terminate(&self, id: &str) -> bool {
        let Some(child) = self.live.lock().ok().and_then(|l| l.get(id).cloned()) else {
            return false;
        };
        if !signal_owned_group(&child, libc::SIGTERM) {
            return false;
        }
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            signal_owned_group(&child, libc::SIGKILL);
        });
        true
    }
}

fn signal_owned_group(child: &Owned, signal: libc::c_int) -> bool {
    let Ok(mut child) = child.lock() else {
        return false;
    };
    if !matches!(child.try_wait(), Ok(None)) {
        return false;
    }
    signal_group(child.id(), signal)
}

/// Rejects 0, 1, -1 and out-of-range ids before any signal is sent.
fn signal_group(pid: u32, signal: libc::c_int) -> bool {
    match libc::pid_t::try_from(pid) {
        Ok(group) if group > 1 => unsafe { libc::kill(-group, signal) == 0 },
        _ => false,
    }
}

fn spawn(argv: &[String], cwd: &Path) -> std::io::Result<Child> {
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in ENV_ALLOW {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    #[cfg(target_os = "linux")]
    // SAFETY: prctl is async-signal-safe; the child dies with the kernel.
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    crate::process_spawn::spawn_command(command)
}

/// Strips terminal escapes/control bytes and redacts secrets before any
/// output is matched, retained or shown to the model.
fn sanitize(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(n) = chars.next() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else if c == '\t' || !c.is_control() {
            out.push(c);
        }
    }
    crate::secret_redaction::redact_secrets(&out).into_owned()
}

enum Signal {
    Matched(String),
    Exited(i32, String),
}

struct Output {
    tail: std::sync::Mutex<std::collections::VecDeque<String>>,
    matched: std::sync::atomic::AtomicBool,
}

impl Output {
    fn push(&self, line: String) {
        let Ok(mut tail) = self.tail.lock() else {
            return;
        };
        tail.push_back(line);
        while tail.iter().map(|l| l.len() + 1).sum::<usize>() > TAIL_BYTES {
            tail.pop_front();
        }
    }
    fn text(&self) -> String {
        self.tail
            .lock()
            .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }
}

fn drain(
    mut stream: impl Read,
    output: Arc<Output>,
    pattern: Option<String>,
    sender: tokio::sync::mpsc::UnboundedSender<Signal>,
) {
    let mut buffer = [0u8; 4096];
    let mut line = Vec::new();
    let emit = |line: &mut Vec<u8>| {
        let text = sanitize(line);
        line.clear();
        if let Some(pattern) = &pattern {
            if text.contains(pattern.as_str())
                && !output
                    .matched
                    .swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                let _ = sender.send(Signal::Matched(text.clone()));
            }
        }
        output.push(text);
    };
    while let Ok(n) = stream.read(&mut buffer) {
        if n == 0 {
            break;
        }
        for byte in &buffer[..n] {
            if *byte == b'\n' {
                emit(&mut line);
            } else if line.len() < LINE_BYTES {
                line.push(*byte);
            }
        }
    }
    if !line.is_empty() {
        emit(&mut line);
    }
}

impl KernelRuntimeState {
    /// Launch intent is durable before the process starts; a failed or
    /// uncertain launch is reported, never retried automatically.
    pub(super) async fn start_agent_process(
        &self,
        wake: AgentWake,
        argv: Vec<String>,
        cwd: PathBuf,
        task: &str,
        prompt: &str,
    ) -> Result<AgentWake, DaemonError> {
        let store = &self.owned.durable_state_store;
        store.agent_lifecycle(Operation::CreateWake {
            task: task.into(),
            prompt: prompt.into(),
            wake: wake.clone(),
        })?;
        let mut child = match spawn(&argv, &cwd) {
            Ok(child) => child,
            Err(e) => {
                store.agent_lifecycle(Operation::ProcessExited {
                    id: wake.id.clone(),
                    exit_code: None,
                    tail: format!("launch failed: {e}"),
                    now: crate::session::unix_epoch_ms(),
                })?;
                return Err(crate::durable_state::agent_lifecycle::error(format!(
                    "process launch failed: {e}"
                )));
            }
        };
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let output = Arc::new(Output {
            tail: Default::default(),
            matched: Default::default(),
        });
        let readers = [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .map(|stream| {
            let (output, pattern, sender) =
                (output.clone(), wake.match_text.clone(), sender.clone());
            std::thread::spawn(move || drain(stream, output, pattern, sender))
        })
        .collect::<Vec<_>>();
        let pid = child.id();
        let owned: Owned = Arc::new(std::sync::Mutex::new(child));
        let started = store.agent_lifecycle(Operation::ProcessStarted {
            id: wake.id.clone(),
            pid,
            now: crate::session::unix_epoch_ms(),
        });
        if let Ok(mut live) = self.owned.agent_wakes.processes.live.lock() {
            live.insert(wake.id.clone(), owned.clone());
        }
        if let Err(error) = started {
            self.owned.agent_wakes.processes.terminate(&wake.id);
            return Err(error);
        }
        self.owned.record_managed_activity_transition();
        std::thread::spawn(move || {
            let code = loop {
                match owned.lock().map(|mut c| c.try_wait()) {
                    Ok(Ok(Some(status))) => {
                        break status
                            .code()
                            .or_else(|| status.signal().map(|s| 128 + s))
                            .unwrap_or(-1);
                    }
                    Ok(Ok(None)) => std::thread::sleep(Duration::from_millis(200)),
                    _ => break -1,
                }
            };
            // A detached grandchild may keep a pipe open; bound the drain.
            let deadline = Instant::now() + Duration::from_secs(1);
            for reader in readers {
                while !reader.is_finished() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            let _ = sender.send(Signal::Exited(code, output.text()));
        });
        let state = self.clone();
        let id = wake.id.clone();
        tokio::spawn(async move {
            while let Some(signal) = receiver.recv().await {
                let now = crate::session::unix_epoch_ms();
                let (op, exited) = match signal {
                    Signal::Matched(line) => (
                        Operation::ProcessMatched {
                            id: id.clone(),
                            line,
                            now,
                        },
                        false,
                    ),
                    Signal::Exited(code, tail) => (
                        Operation::ProcessExited {
                            id: id.clone(),
                            exit_code: Some(code),
                            tail,
                            now,
                        },
                        true,
                    ),
                };
                match state.owned.durable_state_store.agent_lifecycle(op) {
                    Ok(crate::durable_state::agent_lifecycle::Outcome::Wakes(wakes)) => {
                        for wake in wakes {
                            let text = match (exited, wake.exit_code) {
                                (true, Some(code)) => format!(
                                    "Watched process '{}' exited with code {code}",
                                    wake.label
                                ),
                                _ => format!("Watched process '{}' printed its match", wake.label),
                            };
                            state.wake_notice(&wake, text);
                            state.schedule_wake_delivery(&wake);
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(%error, "MP-08/MP-09/MP-10/MP-11 A03: process outcome retained for recovery")
                    }
                }
                if exited {
                    if let Ok(mut live) = state.owned.agent_wakes.processes.live.lock() {
                        live.remove(&id);
                    }
                    state.owned.record_managed_activity_transition();
                    break;
                }
            }
        });
        let started = self
            .owned
            .durable_state_store
            .agent_wakes(Some(&wake.room_id), Some(&wake.agent_id))?
            .into_iter()
            .find(|w| w.id == wake.id)
            .unwrap_or(wake);
        Ok(started)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a03_signal_guard_rejects_reserved_and_invalid_targets() {
        for pid in [0, 1, u32::MAX, (i32::MAX as u32) + 1] {
            assert!(!signal_group(pid, 0), "pid {pid} must be rejected");
        }
    }

    #[test]
    fn a03_output_is_sanitized_bounded_and_matches_once() {
        assert_eq!(sanitize(b"\x1b[31mred\x1b[0m ok\x07"), "red ok");
        let output = Arc::new(Output {
            tail: Default::default(),
            matched: Default::default(),
        });
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut input = b"boot\nready one\nready two\n".to_vec();
        input.extend(std::iter::repeat_n(b'x', 10_000));
        drain(&input[..], output.clone(), Some("ready".into()), sender);
        let mut matches = vec![];
        while let Ok(Signal::Matched(line)) = receiver.try_recv() {
            matches.push(line);
        }
        assert_eq!(matches, vec!["ready one".to_string()]);
        let tail = output.text();
        assert!(tail.len() <= TAIL_BYTES && tail.ends_with(&"x".repeat(LINE_BYTES)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a03_terminate_settles_the_owned_group_only_while_unreaped() {
        let processes = WatchedProcesses::default();
        let child = spawn(&["sleep".into(), "30".into()], Path::new("/")).unwrap();
        let owned: Owned = Arc::new(std::sync::Mutex::new(child));
        processes
            .live
            .lock()
            .unwrap()
            .insert("w".into(), owned.clone());
        assert!(processes.terminate("w"));
        let status = owned.lock().unwrap().wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGTERM));
        assert!(
            !processes.terminate("w"),
            "a reaped leader is never signalled"
        );
        assert!(!processes.terminate("unknown"));
    }
}
