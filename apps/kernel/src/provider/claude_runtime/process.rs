use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

use crate::error::DaemonError;

struct ClaudeOwnedChild {
    child: Child,
    #[cfg(unix)]
    signals: crate::runtime::owned_process_signals::OwnedProcessSignals,
}

impl ClaudeOwnedChild {
    fn new(mut child: Child) -> std::io::Result<Self> {
        #[cfg(unix)]
        let signals =
            match crate::runtime::owned_process_signals::OwnedProcessSignals::for_child(&child) {
                Ok(signals) => signals,
                Err(error) => {
                    // The spawn handle still owns this exact unreaped child.
                    let _ = child.kill();
                    let _ = child.wait();
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
            self.child.kill()?;
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
    retiring: bool,
    #[cfg(test)]
    cleanup_pending: Arc<std::sync::atomic::AtomicBool>,
}

/// Physical ownership remains reachable while an actor leases its I/O state.
#[derive(Clone)]
pub(crate) struct ClaudeRuntimeProcess(Arc<Mutex<ClaudeProcessOwnership>>);

impl ClaudeRuntimeProcess {
    pub(crate) fn new(child: Child) -> std::io::Result<Self> {
        Ok(Self(Arc::new(Mutex::new(ClaudeProcessOwnership {
            current: Arc::new(Mutex::new(ClaudeOwnedChild::new(child)?)),
            abort_child: None,
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
        spawn: impl FnOnce() -> Result<(Child, T), DaemonError>,
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
    program: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    provider_credential_env: &crate::provider::ProviderCredentialEnvironment,
    env_remove: &[String],
    working_directory: Option<&PathBuf>,
    operation: &'static str,
    output_notifier: &crate::provider::output_ready::ProviderEventNotifier,
) -> Result<(Child, ChildStdin, Receiver<ClaudeRuntimeMessage>), DaemonError> {
    let mut command = Command::new(program);
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
    if let Some(working_directory) = working_directory {
        command.current_dir(working_directory);
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
    let (tx, receiver) = mpsc::channel();
    {
        let tx = tx.clone();
        let output_notifier = output_notifier.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) if line.trim().is_empty() => {}
                    Ok(line) => match serde_json::from_str::<Value>(&line) {
                        Ok(value) => {
                            let _ = output_notifier.send(&tx, ClaudeRuntimeMessage::Stdout(value));
                        }
                        Err(error) => {
                            let _ = output_notifier.send(
                                &tx,
                                ClaudeRuntimeMessage::StdoutParseError(format!("{error}: {line}")),
                            );
                        }
                    },
                    Err(error) => {
                        let _ = output_notifier.send(
                            &tx,
                            ClaudeRuntimeMessage::StdoutParseError(error.to_string()),
                        );
                        break;
                    }
                }
            }
        });
    }
    let output_notifier = output_notifier.clone();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) if line.trim().is_empty() => {}
                Ok(line) => {
                    let _ = output_notifier.send(&tx, ClaudeRuntimeMessage::Stderr(line));
                }
                Err(error) => {
                    let _ =
                        output_notifier.send(&tx, ClaudeRuntimeMessage::Stderr(error.to_string()));
                    break;
                }
            }
        }
    });

    Ok((child, stdin, receiver))
}

pub(super) fn stop_child(child: &mut ClaudeRuntimeProcess) {
    child.stop_until_exited(false);
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
