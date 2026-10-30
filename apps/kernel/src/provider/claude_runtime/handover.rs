//! A Claude Code turn that outlives one kernel program. On a hot reload the
//! process keeps running and its pipes stay open across `exec`; the next
//! program adopts them with the turn's state, so the turn completes normally
//! without the prompt being sent again.

use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::process::{ClaudeChild, ClaudeOutput, ClaudeRuntimeMessage};
use super::state::ClaudeRuntimeState;
use super::tool_transcript::ClaudeToolTranscript;
use crate::pty::ReaderGate;

const HOLD_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_MS: libc::c_int = 100;

/// A pipe read line by line on its own thread. A hot reload stops it between
/// two reads; bytes read but not yet passed on stay in `unread`.
pub(super) struct LineReader {
    gate: Arc<ReaderGate>,
    fd: RawFd,
    unread: Arc<Mutex<Vec<u8>>>,
    ended: Arc<AtomicBool>,
}

impl LineReader {
    /// A `held` reader reads nothing until it is released.
    pub(super) fn start(
        fd: OwnedFd,
        unread: Vec<u8>,
        held: bool,
        mut on_line: impl FnMut(String) + Send + 'static,
    ) -> Self {
        let reader = Self {
            gate: Arc::default(),
            fd: fd.as_raw_fd(),
            unread: Arc::new(Mutex::new(unread)),
            ended: Arc::default(),
        };
        if held {
            reader.gate.hold(Duration::ZERO);
        }
        let gate = Arc::clone(&reader.gate);
        let unread = Arc::clone(&reader.unread);
        let ended = Arc::clone(&reader.ended);
        std::thread::spawn(move || {
            let mut chunk = [0_u8; 8192];
            loop {
                gate.checkpoint();
                while let Some(line) = next_line(&unread) {
                    on_line(line);
                }
                match read_ready(&fd, &mut chunk) {
                    Ok(0) => break,
                    Ok(read) => lock(&unread).extend_from_slice(&chunk[..read]),
                    Err(error) if is_transient(&error) => {}
                    Err(_) => break,
                }
            }
            let rest = std::mem::take(&mut *lock(&unread));
            for line in String::from_utf8_lossy(&rest).lines() {
                on_line(line.to_string());
            }
            ended.store(true, Ordering::SeqCst);
            gate.finish();
        });
        reader
    }

    /// Stops the reader between two reads; `None` when it ended or did not stop.
    fn hold(&self) -> Option<(RawFd, Vec<u8>)> {
        if !self.gate.hold(HOLD_TIMEOUT) || self.ended.load(Ordering::SeqCst) {
            return None;
        }
        Some((self.fd, lock(&self.unread).clone()))
    }

    pub(super) fn release(&self) {
        self.gate.release();
    }
}

fn lock(unread: &Mutex<Vec<u8>>) -> std::sync::MutexGuard<'_, Vec<u8>> {
    unread
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn next_line(unread: &Mutex<Vec<u8>>) -> Option<String> {
    let mut unread = lock(unread);
    let end = unread.iter().position(|byte| *byte == b'\n')?;
    let line: Vec<u8> = unread.drain(..=end).collect();
    Some(String::from_utf8_lossy(&line[..end]).into_owned())
}

/// Waits up to one poll interval for the pipe; `Ok(0)` only at end of file.
fn read_ready(fd: &OwnedFd, chunk: &mut [u8]) -> std::io::Result<usize> {
    let mut poll = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    match unsafe { libc::poll(&mut poll, 1, POLL_MS) } {
        0 => Err(std::io::ErrorKind::WouldBlock.into()),
        ready if ready < 0 => Err(std::io::Error::last_os_error()),
        _ => {
            let read =
                unsafe { libc::read(fd.as_raw_fd(), chunk.as_mut_ptr().cast(), chunk.len()) };
            if read < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(read as usize)
            }
        }
    }
}

fn is_transient(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
    )
}

/// What the next kernel program needs to continue a Claude Code turn.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ClaudeHandover {
    pid: u32,
    cgroup: Option<PathBuf>,
    stdin: RawFd,
    stdout: RawFd,
    stderr: RawFd,
    stdout_unread: Vec<u8>,
    stderr_unread: Vec<u8>,
    // The running process's hooks read these paths from its environment.
    context_file: Option<PathBuf>,
    settings_file: Option<PathBuf>,
    usage_file: Option<PathBuf>,
    turn: ClaudeTurn,
    // Passed through memory, never through the handover file.
    #[serde(skip)]
    credentials: crate::provider::ProviderCredentialEnvironment,
}

#[derive(Serialize, Deserialize)]
struct ClaudeTurn {
    session_id: Option<String>,
    active_turn_id: Option<String>,
    active_prompt_message: Option<Value>,
    active_stream_message_id: Option<String>,
    next_turn_number: u64,
    result_number: u64,
    emitted_text_by_block: BTreeMap<String, String>,
    completed_text_blocks: BTreeSet<String>,
    tool_transcript: ClaudeToolTranscript,
    last_usage_file_contents: Option<String>,
    last_context_tokens: Option<u64>,
    last_context_window: Option<u64>,
    observed_context_model: Option<String>,
    main_stream_has_context: bool,
}

impl std::fmt::Debug for ClaudeTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeTurn")
            .field("session_id", &self.session_id)
            .field("active_turn_id", &self.active_turn_id)
            .finish_non_exhaustive()
    }
}

impl ClaudeHandover {
    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    pub(crate) fn fds(&self) -> [RawFd; 3] {
        [self.stdin, self.stdout, self.stderr]
    }

    /// The launch credentials the process runs with, to restart it later.
    pub(crate) fn credentials(&self) -> &crate::provider::ProviderCredentialEnvironment {
        &self.credentials
    }

    pub(crate) fn set_credentials(
        &mut self,
        credentials: crate::provider::ProviderCredentialEnvironment,
    ) {
        self.credentials = credentials;
    }
}

/// Hands a running turn over to the next kernel program: the process and its
/// pipes are left running and open. Gives the state back when there is no
/// turn to continue, so it stops as for a restart.
pub(super) fn hand_over(
    mut state: ClaudeRuntimeState,
) -> Result<ClaudeHandover, ClaudeRuntimeState> {
    if state.active_turn_id.is_none() || !matches!(state.child.try_wait(), Ok(None)) {
        return Err(state);
    }
    let (Some((stdout, mut stdout_unread)), Some((stderr, mut stderr_unread))) =
        (state.output.stdout.hold(), state.output.stderr.hold())
    else {
        state.output.stdout.release();
        state.output.stderr.release();
        return Err(state);
    };
    // Lines already read but not yet applied come first in the next program.
    let mut stdout_lines = Vec::new();
    let mut stderr_lines = Vec::new();
    while let Ok(message) = state.output.receiver.try_recv() {
        match message {
            ClaudeRuntimeMessage::Stdout(value) => {
                stdout_lines.extend(value.to_string().into_bytes());
                stdout_lines.push(b'\n');
            }
            ClaudeRuntimeMessage::StdoutParseError(_) => {}
            ClaudeRuntimeMessage::Stderr(line) => {
                stderr_lines.extend(line.into_bytes());
                stderr_lines.push(b'\n');
            }
        }
    }
    crate::logging::info_with_fields(
        "hot_reload.claude",
        "handing over a running Claude turn",
        serde_json::json!({
            "pid": state.child.id(),
            "applied_lines_pending": stdout_lines.iter().filter(|b| **b == b'\n').count(),
            "unread_bytes": stdout_unread.len(),
        }),
    );
    stdout_lines.append(&mut stdout_unread);
    stderr_lines.append(&mut stderr_unread);
    // The process read its MCP configuration at start; the next program writes its own.
    drop(state.mcp_config_file.take());
    let handover = ClaudeHandover {
        pid: state.child.id(),
        cgroup: state
            .resources
            .as_ref()
            .and_then(crate::runtime::process_resources::ProcessDomain::cgroup_path),
        stdin: state.stdin.as_raw_fd(),
        stdout,
        stderr,
        stdout_unread: stdout_lines,
        stderr_unread: stderr_lines,
        context_file: state.context_file.clone(),
        settings_file: state.settings_file.clone(),
        usage_file: state.usage_file.clone(),
        turn: ClaudeTurn {
            session_id: state.session_id.clone(),
            active_turn_id: state.active_turn_id.clone(),
            active_prompt_message: state.active_prompt_message.clone(),
            active_stream_message_id: state.active_stream_message_id.clone(),
            next_turn_number: state.next_turn_number,
            result_number: state.result_number,
            emitted_text_by_block: std::mem::take(&mut state.emitted_text_by_block),
            completed_text_blocks: std::mem::take(&mut state.completed_text_blocks),
            tool_transcript: std::mem::take(&mut state.tool_transcript),
            last_usage_file_contents: state.last_usage_file_contents.clone(),
            last_context_tokens: state.last_context_tokens,
            last_context_window: state.last_context_window,
            observed_context_model: state.observed_context_model.clone(),
            main_stream_has_context: state.main_stream_has_context,
        },
        credentials: state.provider_credential_env.clone(),
    };
    // Nothing of this program may stop the process or close its pipes: the
    // readers stay held and the `exec` replaces this program.
    std::mem::forget(state);
    Ok(handover)
}

/// The handed-over process keeps its memory domain.
pub(super) fn adopted_resources(
    provider_run_id: &str,
    handover: &ClaudeHandover,
) -> Result<crate::runtime::process_resources::ProcessDomain, crate::error::DaemonError> {
    let resources = crate::runtime::process_resources::PreparedDomain::adopted(
        handover.cgroup.clone(),
        handover.pid,
    )
    .and_then(|domain| domain.start(handover.pid))
    .map_err(|error| crate::error::DaemonError::LocalTransport {
        operation: "adopt_claude_runtime",
        message: error.to_string(),
    })?;
    resources.bind_key(provider_run_id);
    Ok(resources)
}

/// The handed-over turn and the files its process uses, in this program.
pub(super) struct Adopted {
    turn: ClaudeTurn,
    context_file: Option<PathBuf>,
    settings_file: Option<PathBuf>,
    usage_file: Option<PathBuf>,
}

/// # Safety
/// The handover's fds must be the open pipes inherited from the previous
/// program, owned by nothing else in this one.
pub(super) unsafe fn adopt(
    handover: ClaudeHandover,
    output_notifier: &crate::provider::output_ready::ProviderEventNotifier,
) -> (ClaudeChild, ChildStdin, ClaudeOutput, Adopted) {
    let (stdin, stdout, stderr) = unsafe {
        (
            OwnedFd::from_raw_fd(handover.stdin),
            OwnedFd::from_raw_fd(handover.stdout),
            OwnedFd::from_raw_fd(handover.stderr),
        )
    };
    (
        ClaudeChild::Adopted {
            pid: handover.pid,
            status: None,
        },
        ChildStdin::from(stdin),
        // Held until the recovered prompt is bound to the turn again.
        ClaudeOutput::start(
            stdout,
            handover.stdout_unread,
            stderr,
            handover.stderr_unread,
            true,
            output_notifier,
        ),
        Adopted {
            turn: handover.turn,
            context_file: handover.context_file,
            settings_file: handover.settings_file,
            usage_file: handover.usage_file,
        },
    )
}

impl Adopted {
    /// Puts the turn that was running back into `state`.
    pub(super) fn restore_turn(self, state: &mut ClaudeRuntimeState) -> AdoptedTurn {
        let turn = self.turn;
        state.context_file = self.context_file;
        state.settings_file = self.settings_file;
        state.usage_file = self.usage_file;
        state.session_id = turn.session_id;
        state.active_stream_message_id = turn.active_stream_message_id;
        state.next_turn_number = turn.next_turn_number;
        state.result_number = turn.result_number;
        state.emitted_text_by_block = turn.emitted_text_by_block;
        state.completed_text_blocks = turn.completed_text_blocks;
        state.tool_transcript = turn.tool_transcript;
        state.last_usage_file_contents = turn.last_usage_file_contents;
        state.last_context_tokens = turn.last_context_tokens;
        state.last_context_window = turn.last_context_window;
        state.observed_context_model = turn.observed_context_model;
        state.main_stream_has_context = turn.main_stream_has_context;
        AdoptedTurn {
            turn_id: turn.active_turn_id,
            prompt_message: turn.active_prompt_message,
        }
    }
}

/// The turn the process is already running: the recovered prompt's submit
/// resumes it instead of sending the prompt again.
#[derive(Debug)]
pub(super) struct AdoptedTurn {
    pub(super) turn_id: Option<String>,
    pub(super) prompt_message: Option<Value>,
}
