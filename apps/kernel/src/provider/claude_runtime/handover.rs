//! A Claude Code turn that outlives one kernel program. On a hot reload the
//! process keeps running and its pipes stay open across `exec`; the next
//! program adopts them with the turn's state, so the turn completes normally
//! without the prompt being sent again.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
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
use super::watchdog::ClaudeStartState;
use crate::pty::ReaderGate;

const HOLD_TIMEOUT: Duration = Duration::from_secs(2);
/// A reader waits for its pipe or for a hold request, whichever comes first,
/// so this is only a backstop: at 100 ms every Claude pipe of a Room of idle
/// agents woke ten times a second for nothing.
const POLL_MS: libc::c_int = 5_000;

/// A pipe read line by line on its own thread. A hot reload stops it between
/// two reads; bytes read but not yet passed on stay in `unread`.
pub(super) struct LineReader {
    gate: Arc<ReaderGate>,
    fd: RawFd,
    unread: Arc<Mutex<Vec<u8>>>,
    ended: Arc<AtomicBool>,
    /// Written to when a hold is requested, so a reader waiting on its pipe
    /// stops at once.
    wake: Arc<(UnixStream, UnixStream)>,
}

impl LineReader {
    /// A `held` reader reads nothing until it is released.
    pub(super) fn start(
        fd: OwnedFd,
        unread: Vec<u8>,
        held: bool,
        wake: WakePair,
        mut on_line: impl FnMut(String) + Send + 'static,
    ) -> Self {
        let reader = Self {
            gate: Arc::default(),
            fd: fd.as_raw_fd(),
            unread: Arc::new(Mutex::new(unread)),
            ended: Arc::default(),
            wake: Arc::new(wake),
        };
        if held {
            reader.gate.hold(Duration::ZERO);
        }
        let gate = Arc::clone(&reader.gate);
        let unread = Arc::clone(&reader.unread);
        let ended = Arc::clone(&reader.ended);
        // Keep both endpoints alive until the reader ends. Dropping its
        // controller must not turn the wake socket into a permanent hangup.
        let wake = Arc::clone(&reader.wake);
        std::thread::spawn(move || {
            let mut chunk = [0_u8; 8192];
            loop {
                gate.checkpoint();
                while let Some(line) = next_line(&unread) {
                    on_line(line);
                }
                match read_ready(&fd, &wake.0, &mut chunk) {
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
        self.hold_with_wake(|| self.wake_reader())
    }

    fn wake_reader(&self) -> std::io::Result<()> {
        // A full nonblocking socket is already readable. Retry an interrupted
        // write so a quiet reader never has to wait for the poll backstop.
        loop {
            match (&self.wake.1).write(&[1]) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Ok(_) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
            }
        }
    }

    fn hold_with_wake(
        &self,
        wake: impl FnOnce() -> std::io::Result<()>,
    ) -> Option<(RawFd, Vec<u8>)> {
        self.gate.request_hold();
        if wake().is_err() {
            self.gate.release();
            return None;
        }
        if !self.gate.wait_held(HOLD_TIMEOUT) || self.ended.load(Ordering::SeqCst) {
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

/// Both ends are nonblocking and close on exec on every supported Unix.
pub(super) type WakePair = (UnixStream, UnixStream);

pub(super) fn wake_pair() -> std::io::Result<WakePair> {
    #[cfg(test)]
    if WAKE_PAIRS_BEFORE_FAILURE.with(|count| {
        let remaining = count.get();
        count.set(remaining.saturating_sub(1));
        remaining == 1
    }) {
        return Err(std::io::Error::from_raw_os_error(libc::EMFILE));
    }
    let (read, write) = UnixStream::pair()?;
    read.set_nonblocking(true)?;
    write.set_nonblocking(true)?;
    Ok((read, write))
}

#[cfg(test)]
thread_local! {
    static WAKE_PAIRS_BEFORE_FAILURE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn with_second_wake_failure<T>(run: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            WAKE_PAIRS_BEFORE_FAILURE.with(|count| count.set(0));
        }
    }
    WAKE_PAIRS_BEFORE_FAILURE.with(|count| {
        assert_eq!(count.replace(2), 0);
    });
    let _reset = Reset;
    run()
}

/// Waits for the pipe or a hold request, at most one poll interval;
/// `Ok(0)` only at end of file.
fn read_ready(fd: &OwnedFd, wake: &UnixStream, chunk: &mut [u8]) -> std::io::Result<usize> {
    let mut polls = [
        libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: wake.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    match unsafe { libc::poll(polls.as_mut_ptr(), 2, POLL_MS) } {
        0 => Err(std::io::ErrorKind::WouldBlock.into()),
        ready if ready < 0 => Err(std::io::Error::last_os_error()),
        _ => {
            if polls[1].revents != 0 {
                let mut count = [0_u8; 8];
                unsafe { libc::read(wake.as_raw_fd(), count.as_mut_ptr().cast(), count.len()) };
                // Pipe data that arrived with the request is read on the next pass.
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
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
    // A predecessor program (L12) hands neither over: its turns had no own-turn state.
    #[serde(default)]
    own_turn: bool,
    #[serde(default)]
    background_notification_pending: bool,
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
    // The compactions this run has had and the one still running: the next program numbers the next
    // row after them, so a reload never reuses the key of a row a client already shows. A predecessor
    // program hands neither over.
    #[serde(default)]
    compactions: u64,
    #[serde(default)]
    compaction_started_at_ms: Option<u64>,
    start: ClaudeStartState,
}

impl ClaudeTurn {
    /// The turn the state is in, taken out of it for the next program.
    fn of(state: &mut ClaudeRuntimeState) -> Self {
        Self {
            session_id: state.session_id.clone(),
            active_turn_id: state.active_turn_id.clone(),
            active_prompt_message: state.active_prompt_message.clone(),
            own_turn: state.own_turn,
            background_notification_pending: state.background_notification_pending,
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
            compactions: state.compactions,
            compaction_started_at_ms: state.compaction_started_at_ms,
            start: state
                .turn_watchdog
                .start_state(std::time::Instant::now(), crate::session::unix_epoch_ms()),
        }
    }
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
) -> Result<ClaudeHandover, Box<ClaudeRuntimeState>> {
    if state.active_turn_id.is_none() || !matches!(state.child.try_wait(), Ok(None)) {
        return Err(Box::new(state));
    }
    let (Some((stdout, mut stdout_unread)), Some((stderr, mut stderr_unread))) =
        (state.output.stdout.hold(), state.output.stderr.hold())
    else {
        state.output.stdout.release();
        state.output.stderr.release();
        return Err(Box::new(state));
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
        turn: ClaudeTurn::of(&mut state),
        credentials: state.provider_credential_env.clone(),
    };
    // Nothing of this program may stop the process or close its pipes: the
    // readers stay held and the `exec` replaces this program.
    std::mem::forget(state);
    Ok(handover)
}

/// The handed-over process joins its owner's memory unit in this program.
pub(super) fn adopted_resources(
    provider_run_id: &str,
    resource_unit: &str,
    handover: &ClaudeHandover,
) -> Result<crate::runtime::process_resources::ProcessDomain, crate::error::DaemonError> {
    let resources = crate::runtime::process_resources::PreparedDomain::adopted(
        resource_unit,
        handover.cgroup.as_deref(),
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
) -> std::io::Result<(ClaudeChild, ChildStdin, ClaudeOutput, Adopted)> {
    let (stdin, stdout, stderr) = unsafe {
        (
            OwnedFd::from_raw_fd(handover.stdin),
            OwnedFd::from_raw_fd(handover.stdout),
            OwnedFd::from_raw_fd(handover.stderr),
        )
    };
    let mut child = ClaudeChild::Adopted {
        pid: handover.pid,
        status: None,
    };
    // Held until the recovered prompt is bound to the turn again. On setup
    // failure the inherited child is stopped and reaped before pipes close.
    let output = child.start_output(
        stdout,
        handover.stdout_unread,
        stderr,
        handover.stderr_unread,
        true,
        output_notifier,
    )?;
    Ok((
        child,
        ChildStdin::from(stdin),
        output,
        Adopted {
            turn: handover.turn,
            context_file: handover.context_file,
            settings_file: handover.settings_file,
            usage_file: handover.usage_file,
        },
    ))
}

impl Adopted {
    /// Puts the turn that was running back into `state`.
    pub(super) fn restore_turn(self, state: &mut ClaudeRuntimeState) -> AdoptedTurn {
        let turn = self.turn;
        state.context_file = self.context_file;
        state.settings_file = self.settings_file;
        state.usage_file = self.usage_file;
        state.session_id = turn.session_id;
        state.own_turn = turn.own_turn;
        state.background_notification_pending = turn.background_notification_pending;
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
        state.compactions = turn.compactions;
        state.compaction_started_at_ms = turn.compaction_started_at_ms;
        AdoptedTurn {
            turn_id: turn.active_turn_id,
            prompt_message: turn.active_prompt_message,
            start: turn.start,
        }
    }
}

/// The turn the process is already running: the recovered prompt's submit
/// resumes it instead of sending the prompt again.
#[derive(Debug)]
pub(super) struct AdoptedTurn {
    pub(super) turn_id: Option<String>,
    pub(super) prompt_message: Option<Value>,
    pub(super) start: ClaudeStartState,
}

#[cfg(test)]
mod tests {
    use super::*;

    // A completed compaction 0 and a running compaction 1 survive a hot reload: the boundary the next
    // program reads settles row 1, and the row after it is 2, never a reused key.
    #[test]
    fn a_hot_reload_keeps_the_compaction_count_and_the_running_one() {
        let (mut before, _batch) = super::super::tests::parser_state();
        before.compactions = 1;
        before.compaction_started_at_ms = Some(7_000);
        let turn: ClaudeTurn =
            serde_json::from_slice(&serde_json::to_vec(&ClaudeTurn::of(&mut before)).unwrap())
                .unwrap();
        let (mut after, _batch) = super::super::tests::parser_state();
        assert_eq!(
            (after.compactions, after.compaction_started_at_ms),
            (0, None)
        );
        Adopted {
            turn,
            context_file: None,
            settings_file: None,
            usage_file: None,
        }
        .restore_turn(&mut after);
        assert_eq!(after.compactions, 1);
        assert_eq!(after.compaction_started_at_ms, Some(7_000));
    }
}

#[cfg(test)]
mod line_reader_tests {
    use super::*;
    use std::time::Instant;

    fn pipe() -> (OwnedFd, OwnedFd) {
        let mut fds = [0 as RawFd; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
    }

    /// A reader of an idle pipe stops for a hot reload when asked, not when
    /// its poll interval runs out; the interval is long so that an idle Room
    /// does not wake every reader ten times a second.
    #[test]
    fn an_idle_reader_stops_for_a_hold_at_once() {
        let (read_end, _write_end) = pipe();
        let reader = LineReader::start(read_end, Vec::new(), false, wake_pair().unwrap(), |_| {});
        std::thread::sleep(Duration::from_millis(200));
        let mut took = Vec::new();
        for _ in 0..15 {
            let started = Instant::now();
            assert!(reader.hold().is_some(), "the reader did not stop");
            took.push(started.elapsed());
            reader.release();
            std::thread::sleep(Duration::from_millis(37));
        }
        took.sort();
        let median = took[took.len() / 2];
        assert!(
            median < Duration::from_millis(20),
            "an idle reader took {median:?} (median) to stop; it waited out its poll interval"
        );
    }

    /// Force the reader to acknowledge inside the wake operation, before the
    /// requester can start its normal wait. Wake-before-register loses here.
    #[test]
    fn a_hold_is_registered_before_the_reader_consumes_its_wake() {
        let (read_end, write_end) = pipe();
        let (lines, received) = std::sync::mpsc::channel();
        let reader = LineReader::start(
            read_end,
            Vec::new(),
            false,
            wake_pair().unwrap(),
            move |line| {
                lines.send(line).unwrap();
            },
        );
        assert!(reader
            .hold_with_wake(|| {
                reader.wake_reader()?;
                assert!(
                    reader.gate.wait_held(HOLD_TIMEOUT),
                    "wake preceded hold registration"
                );
                Ok(())
            })
            .is_some());
        let bytes = b"once\n";
        assert_eq!(
            unsafe { libc::write(write_end.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) },
            bytes.len() as isize
        );
        assert!(received.try_recv().is_err());
        reader.release();
        assert_eq!(received.recv_timeout(HOLD_TIMEOUT).as_deref(), Ok("once"));
        drop(write_end);
        assert!(received.recv_timeout(HOLD_TIMEOUT).is_err());
    }

    #[test]
    fn wake_sockets_are_nonblocking_and_close_on_exec() {
        let (read, write) = wake_pair().unwrap();
        for fd in [&read, &write] {
            let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
            assert!(flags >= 0 && flags & libc::O_NONBLOCK != 0);
            let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
            assert!(flags >= 0 && flags & libc::FD_CLOEXEC != 0);
        }
    }

    #[test]
    fn a_line_written_while_idle_is_still_delivered_after_a_hold() {
        let (read_end, write_end) = pipe();
        let (lines, received) = std::sync::mpsc::channel();
        let reader = LineReader::start(
            read_end,
            Vec::new(),
            false,
            wake_pair().unwrap(),
            move |line| {
                let _ = lines.send(line);
            },
        );
        std::thread::sleep(Duration::from_millis(100));
        assert!(reader.hold().is_some());
        reader.release();
        let bytes = b"hello\n";
        unsafe { libc::write(write_end.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).as_deref(),
            Ok("hello")
        );
    }
}

#[cfg(test)]
mod wake_allocation_tests {
    use super::super::process::{spawn_claude_child, stop_child};
    use super::*;
    use std::io::Read;
    use std::os::fd::IntoRawFd;
    use std::process::Stdio;

    // A real owned child, but not a live Claude provider. The guard also
    // cleans up when testing the broken path fails or panics.
    struct Fixture {
        child: ClaudeChild,
        stdin: Option<OwnedFd>,
        stdout: Option<OwnedFd>,
        stderr: Option<OwnedFd>,
        _state: crate::test_temp_dir::TestTempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let state = crate::test_temp_dir::TestTempDir::new("claude-wake-failure");
            let mut command = crate::process_spawn::command("/bin/sh");
            command
                .args(["-c", "exec cat >/dev/null"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = crate::process_spawn::spawn_command(command).unwrap();
            std::fs::write(state.join("owned.pid"), child.id().to_string()).unwrap();
            Self {
                stdin: Some(child.stdin.take().unwrap().into()),
                stdout: Some(child.stdout.take().unwrap().into()),
                stderr: Some(child.stderr.take().unwrap().into()),
                child: ClaudeChild::Started(child),
                _state: state,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            stop_child(&mut self.child);
        }
    }

    fn assert_reaped(pid: u32) {
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }

    #[test]
    fn second_wake_failure_starts_neither_reader() {
        let (stdout, mut stdout_peer) = UnixStream::pair().unwrap();
        let (stderr, mut stderr_peer) = UnixStream::pair().unwrap();
        let result = std::panic::catch_unwind(|| {
            with_second_wake_failure(|| {
                ClaudeOutput::start(
                    stdout.into(),
                    Vec::new(),
                    stderr.into(),
                    Vec::new(),
                    false,
                    &Default::default(),
                )
            })
        });
        assert!(
            result.is_ok(),
            "second wake allocation failure must not panic"
        );
        drop(result);
        // EOF proves no thread retained either input after the second
        // reservation failed. Nonblocking reads cannot hide a leaked reader.
        for peer in [&mut stdout_peer, &mut stderr_peer] {
            peer.set_nonblocking(true).unwrap();
            assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        }
    }

    #[test]
    fn second_wake_failure_reaps_started_child() {
        let mut fixture = Fixture::new();
        let pid = fixture.child.id();
        let result = with_second_wake_failure(|| {
            fixture.child.start_output(
                fixture.stdout.take().unwrap(),
                Vec::new(),
                fixture.stderr.take().unwrap(),
                Vec::new(),
                false,
                &Default::default(),
            )
        });
        assert_eq!(result.err().unwrap().raw_os_error(), Some(libc::EMFILE));
        assert!(fixture.child.try_wait().unwrap().is_some());
        assert_reaped(pid);
    }

    #[test]
    fn second_wake_failure_returns_a_startup_error() {
        let result = with_second_wake_failure(|| {
            spawn_claude_child(
                "claude-second-wake-failure",
                "claude-second-wake-failure",
                "/bin/sh",
                &["-c".into(), "exec cat >/dev/null".into()],
                &BTreeMap::new(),
                &Default::default(),
                &[],
                None,
                "initialize_claude_runtime",
                &Default::default(),
            )
        });
        let error = result
            .err()
            .expect("startup must fail without panicking")
            .to_string();
        assert!(
            error.contains("failed to set up Claude Code output"),
            "{error}"
        );
        assert!(
            error.contains(&std::io::Error::from_raw_os_error(libc::EMFILE).to_string()),
            "{error}"
        );
    }

    #[test]
    fn second_wake_failure_reaps_adopted_child_and_closes_inherited_pipes() {
        let mut fixture = Fixture::new();
        let pid = fixture.child.id();
        let turn = serde_json::from_value(serde_json::json!({
            "session_id": null, "active_turn_id": "turn", "active_prompt_message": null,
            "own_turn": false, "background_notification_pending": false,
            "active_stream_message_id": null, "next_turn_number": 1, "result_number": 1,
            "emitted_text_by_block": {}, "completed_text_blocks": [],
            "tool_transcript": serde_json::to_value(ClaudeToolTranscript::default()).unwrap(),
            "last_usage_file_contents": null, "last_context_tokens": null,
            "last_context_window": null, "observed_context_model": null,
            "main_stream_has_context": true,
            "start": {"process_started": false, "waiting_since_unix_ms": null}
        }))
        .unwrap();
        let handover = ClaudeHandover {
            pid,
            cgroup: None,
            stdin: fixture.stdin.take().unwrap().into_raw_fd(),
            stdout: fixture.stdout.take().unwrap().into_raw_fd(),
            stderr: fixture.stderr.take().unwrap().into_raw_fd(),
            stdout_unread: Vec::new(),
            stderr_unread: Vec::new(),
            context_file: None,
            settings_file: None,
            usage_file: None,
            turn,
            credentials: Default::default(),
        };
        let fds = handover.fds();
        // SAFETY: these are only the pipes owned by this fixture's child;
        // their ownership was transferred into the handover above.
        let result = with_second_wake_failure(|| unsafe { adopt(handover, &Default::default()) });
        assert_eq!(result.err().unwrap().raw_os_error(), Some(libc::EMFILE));
        assert_reaped(pid);
        for fd in fds {
            assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EBADF)
            );
        }
    }
}
