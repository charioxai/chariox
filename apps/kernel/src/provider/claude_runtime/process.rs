use crate::runtime::bounded_process::BoundedChildExt as _;
use std::collections::BTreeMap;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

use super::handover::LineReader;
use crate::error::DaemonError;

struct ClaudeOwnedChild {
    child: ClaudeChild,
    #[cfg(unix)]
    signals: crate::runtime::owned_process_signals::OwnedProcessSignals,
}

impl ClaudeOwnedChild {
    fn new(mut child: ClaudeChild) -> std::io::Result<Self> {
        #[cfg(unix)]
        let signals = match match &child {
            ClaudeChild::Started(child) => {
                crate::runtime::owned_process_signals::OwnedProcessSignals::for_child(child)
            }
            ClaudeChild::Adopted { pid, .. } => {
                crate::runtime::owned_process_signals::OwnedProcessSignals::for_pid(*pid)
            }
        } {
            Ok(signals) => signals,
            Err(error) => {
                // The spawn handle still owns this exact unreaped child.
                child.kill_and_wait();
                return Err(error);
            }
        };
        Ok(Self {
            child,
            #[cfg(unix)]
            signals,
        })
    }

    fn poll_stop(&mut self) -> std::io::Result<bool> {
        #[cfg(unix)]
        self.signals.kill_owned_processes()?;
        #[cfg(not(unix))]
        if self.child.try_wait()?.is_none() {
            self.child.kill_and_wait();
        }
        let exited = self.child.try_wait()?.is_some();
        #[cfg(unix)]
        return Ok(exited && !self.signals.has_running_owned_processes()?);
        #[cfg(not(unix))]
        Ok(exited)
    }
}

struct ClaudeProcessOwnership {
    current: Arc<Mutex<ClaudeOwnedChild>>,
    abort_child: Option<Arc<Mutex<ClaudeOwnedChild>>>,
    prompt_abort_context: Option<String>,
    retiring: bool,
    #[cfg(test)]
    cleanup_pending: Arc<std::sync::atomic::AtomicBool>,
}

/// Physical ownership remains reachable while an actor leases its I/O state.
#[derive(Clone)]
pub(crate) struct ClaudeRuntimeProcess(Arc<Mutex<ClaudeProcessOwnership>>);

impl ClaudeRuntimeProcess {
    pub(super) fn new(child: ClaudeChild) -> std::io::Result<Self> {
        Ok(Self(Arc::new(Mutex::new(ClaudeProcessOwnership {
            current: Arc::new(Mutex::new(ClaudeOwnedChild::new(child)?)),
            abort_child: None,
            prompt_abort_context: None,
            retiring: false,
            #[cfg(test)]
            cleanup_pending: Arc::default(),
        }))))
    }

    fn current(&self) -> Arc<Mutex<ClaudeOwnedChild>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .current
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn defer_cleanup_for_test(&self) -> Arc<std::sync::atomic::AtomicBool> {
        let flag = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cleanup_pending
            .clone();
        flag.store(true, std::sync::atomic::Ordering::Release);
        flag
    }

    #[cfg(test)]
    pub(crate) fn retiring_for_test(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retiring
    }

    pub(super) fn id(&self) -> u32 {
        self.current()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .child
            .id()
    }

    pub(super) fn try_wait(&self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let child = self.current();
        let mut child = child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        #[cfg(unix)]
        child.signals.refresh()?;
        child.child.try_wait()
    }

    pub(crate) fn capture_prompt_abort(&self) -> std::io::Result<()> {
        let mut ownership = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if ownership.abort_child.is_none() {
            ownership.abort_child = Some(ownership.current.clone());
        }
        #[cfg(unix)]
        ownership
            .abort_child
            .as_ref()
            .expect("abort child was retained")
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .signals
            .refresh()?;
        Ok(())
    }

    pub(crate) fn set_prompt_abort_context(&self, prompt_id: Option<&str>) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .prompt_abort_context = prompt_id.map(str::to_string);
    }

    /// A capture for this already-admitted prompt must survive its late write.
    pub(crate) fn clear_previous_prompt_abort_context(&self, prompt_id: &str) -> bool {
        let mut ownership = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if ownership.prompt_abort_context.as_deref() == Some(prompt_id) {
            return false;
        }
        ownership.prompt_abort_context = None;
        true
    }

    pub(crate) fn prompt_abort_context(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .prompt_abort_context
            .clone()
    }

    /// A successful restart already settled its old child. A forced retirement
    /// also closes the current child and prevents a late actor from replacing it.
    pub(crate) fn settle_prompt_abort(&self, force_retirement: bool) -> std::io::Result<bool> {
        let mut ownership = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let child = ownership
            .abort_child
            .clone()
            .unwrap_or_else(|| ownership.current.clone());
        ownership.abort_child = Some(child.clone());
        ownership.retiring |= force_retirement || Arc::ptr_eq(&child, &ownership.current);
        #[cfg(test)]
        if ownership
            .cleanup_pending
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "owned child cleanup held by test",
            ));
        }
        let mut stopped = child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .poll_stop()?;
        if ownership.retiring && !Arc::ptr_eq(&child, &ownership.current) {
            stopped &= ownership
                .current
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .poll_stop()?;
        }
        if !stopped {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "the owned Claude child or its descendants have not exited",
            ));
        }
        ownership.abort_child = None;
        Ok(ownership.retiring)
    }

    pub(super) fn replace_with<T>(
        &self,
        spawn: impl FnOnce() -> Result<(ClaudeChild, T), DaemonError>,
    ) -> Result<T, DaemonError> {
        // Admission and retirement share this lock: a cancelled leased actor
        // cannot start another child after cancellation releases the turn.
        let mut ownership = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if ownership.retiring {
            return Err(DaemonError::LocalTransport {
                operation: "claude_replace_retired_process",
                message: "the Claude provider process was retired".into(),
            });
        }
        let (child, result) = spawn()?;
        let child = ClaudeOwnedChild::new(child).map_err(|error| DaemonError::LocalTransport {
            operation: "claude_capture_replacement_process",
            message: error.to_string(),
        })?;
        ownership.current = Arc::new(Mutex::new(child));
        Ok(result)
    }

    fn stop_until_exited(&self, retire: bool) {
        let mut reported = false;
        loop {
            let result = if retire {
                self.settle_prompt_abort(true).map(|_| true)
            } else {
                self.current()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .poll_stop()
            };
            match result {
                Ok(true) => return,
                Err(error) if !reported => {
                    crate::logging::warn_with_fields(
                        "daemon.provider",
                        "Claude process cleanup is pending",
                        serde_json::json!({"pid": self.id(), "error": error.to_string()}),
                    );
                    reported = true;
                }
                _ => {}
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    pub(super) fn stop(&self) {
        self.stop_until_exited(false);
    }

    pub(crate) fn begin_retirement(&self) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retiring = true;
    }

    pub(crate) fn retire(&self) {
        self.stop_until_exited(true);
    }
}

pub(super) enum ClaudeRuntimeMessage {
    Stdout(Value),
    StdoutParseError(String),
    Stderr(String),
}

#[allow(
    clippy::too_many_arguments,
    reason = "Preserve the existing spawn_claude_child operation signature and explicit context arguments"
)]
pub(super) fn spawn_claude_child(
    provider_run_id: &str,
    resource_unit: &str,
    program: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    provider_credential_env: &crate::provider::ProviderCredentialEnvironment,
    env_remove: &[String],
    working_directory: Option<&PathBuf>,
    operation: &'static str,
    output_notifier: &crate::provider::output_ready::ProviderEventNotifier,
) -> Result<
    (
        ClaudeChild,
        ChildStdin,
        ClaudeOutput,
        crate::runtime::process_resources::ProcessDomain,
    ),
    DaemonError,
> {
    let prepared =
        crate::runtime::process_resources::PreparedDomain::new(resource_unit).map_err(|error| {
            DaemonError::LocalTransport {
                operation,
                message: error.to_string(),
            }
        })?;
    #[cfg(unix)]
    let inner_program = program;
    let (program, args) = prepared.command(program.to_string(), args.to_vec());
    let mut command = crate::process_spawn::command(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, _) in std::env::vars() {
        if crate::secret::secret_like_env_name(&name) {
            command.env_remove(name);
        }
    }
    for name in env_remove {
        command.env_remove(name);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    for (name, value) in provider_credential_env.iter() {
        command.env(name, value);
    }
    crate::provider::process_owner::stamp(&mut command);
    // Row 554: what the agent's tools leave running is attributable to this run, wherever it is
    // reparented to. Run ids repeat from kernel to kernel: the owner marker above tells them apart.
    command.env("CHARIOX_PROVIDER_RUN_ID", provider_run_id);
    if let Some(working_directory) = working_directory {
        command.current_dir(working_directory);
    }

    #[cfg(unix)]
    {
        let path = match command.get_envs().find(|(name, _)| *name == "PATH") {
            Some((_, value)) => value.map(std::ffi::OsStr::to_os_string),
            None => std::env::var_os("PATH"),
        };
        crate::runtime::process_resources::validate_executable(inner_program, command.get_current_dir(), path.as_deref())
            .map_err(|_| DaemonError::LocalTransport {
                operation,
                message: "Claude Code executable is missing or not executable; reinstall Claude Code and retry".into(),
            })?;
    }

    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = crate::process_spawn::spawn_command(command).map_err(|error| {
        DaemonError::LocalTransport {
            operation,
            message: format!("failed to start Claude Code for `{provider_run_id}`: {error}"),
        }
    })?;
    let resources = match prepared.start(child.id()) {
        Ok(resources) => resources,
        Err(error) => {
            let _ = child.kill();
            let _ = child.bounded_wait();
            return Err(DaemonError::LocalTransport {
                operation,
                message: error.to_string(),
            });
        }
    };
    resources.bind_key(provider_run_id);
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: "Claude Code did not expose stdin".to_string(),
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: "Claude Code did not expose stdout".to_string(),
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: "Claude Code did not expose stderr".to_string(),
        })?;
    let output = ClaudeOutput::start(
        stdout.into(),
        Vec::new(),
        stderr.into(),
        Vec::new(),
        false,
        output_notifier,
    );

    Ok((ClaudeChild::Started(child), stdin, output, resources))
}

/// The Claude Code process: started by this kernel program, or started by the
/// previous one and inherited across a hot reload (still this process's child).
pub(super) enum ClaudeChild {
    Started(Child),
    Adopted {
        pid: u32,
        status: Option<ExitStatus>,
    },
}

impl ClaudeChild {
    pub(super) fn id(&self) -> u32 {
        match self {
            Self::Started(child) => child.id(),
            Self::Adopted { pid, .. } => *pid,
        }
    }

    pub(super) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        match self {
            Self::Started(child) => child.try_wait(),
            Self::Adopted { pid, status } => {
                if status.is_none() {
                    *status = reap(*pid, libc::WNOHANG)?;
                }
                Ok(*status)
            }
        }
    }

    fn kill_and_wait(&mut self) {
        match self {
            Self::Started(child) => {
                let _ = child.kill();
                let _ = child.bounded_wait();
            }
            Self::Adopted { pid, status } => {
                unsafe { libc::kill(*pid as libc::pid_t, libc::SIGKILL) };
                *status = reap(*pid, 0).ok().flatten();
            }
        }
    }
}

fn reap(pid: u32, options: libc::c_int) -> std::io::Result<Option<ExitStatus>> {
    use std::os::unix::process::ExitStatusExt;
    let mut raw = 0;
    loop {
        match unsafe { libc::waitpid(pid as libc::pid_t, &mut raw, options) } {
            0 => return Ok(None),
            reaped if reaped > 0 => return Ok(Some(ExitStatus::from_raw(raw))),
            _ => {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
}

/// Claude Code's stdout and stderr, read line by line into one channel.
pub(super) struct ClaudeOutput {
    pub(super) receiver: Receiver<ClaudeRuntimeMessage>,
    pub(super) stdout: LineReader,
    pub(super) stderr: LineReader,
}

impl ClaudeOutput {
    #[cfg(test)]
    pub(super) fn detached() -> Self {
        let null = || OwnedFd::from(std::fs::File::open("/dev/null").expect("open /dev/null"));
        Self::start(
            null(),
            Vec::new(),
            null(),
            Vec::new(),
            false,
            &Default::default(),
        )
    }

    pub(super) fn start(
        stdout: OwnedFd,
        stdout_unread: Vec<u8>,
        stderr: OwnedFd,
        stderr_unread: Vec<u8>,
        held: bool,
        output_notifier: &crate::provider::output_ready::ProviderEventNotifier,
    ) -> Self {
        let (tx, receiver) = mpsc::channel();
        let stdout = {
            let tx = tx.clone();
            let output_notifier = output_notifier.clone();
            LineReader::start(stdout, stdout_unread, held, move |line| {
                let message = match serde_json::from_str::<Value>(&line) {
                    Ok(value) => ClaudeRuntimeMessage::Stdout(value),
                    Err(error) => {
                        ClaudeRuntimeMessage::StdoutParseError(format!("{error}: {line}"))
                    }
                };
                let _ = output_notifier.send(&tx, message);
            })
        };
        let output_notifier = output_notifier.clone();
        let stderr = LineReader::start(stderr, stderr_unread, held, move |line| {
            let _ = output_notifier.send(&tx, ClaudeRuntimeMessage::Stderr(line));
        });
        Self {
            receiver,
            stdout,
            stderr,
        }
    }

    pub(super) fn release(&self) {
        self.stdout.release();
        self.stderr.release();
    }
}

pub(super) fn stop_child(child: &mut ClaudeChild) {
    // Signal only the exact child created by this runtime.
    if child.id() <= 1 {
        return;
    }
    if let Ok(None) = child.try_wait() {
        child.kill_and_wait();
    }
}

pub(super) fn write_json_line(stdin: &mut ChildStdin, value: &Value) -> Result<(), DaemonError> {
    serde_json::to_writer(&mut *stdin, value).map_err(|error| DaemonError::LocalTransport {
        operation: "claude_write_stdin",
        message: error.to_string(),
    })?;
    stdin
        .write_all(b"\n")
        .and_then(|_| stdin.flush())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "claude_write_stdin",
            message: error.to_string(),
        })
}
