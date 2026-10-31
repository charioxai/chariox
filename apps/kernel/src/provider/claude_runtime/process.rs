use std::collections::BTreeMap;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};

use serde_json::Value;

use super::handover::LineReader;
use crate::error::DaemonError;

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
    command.env(
        crate::provider::PROVIDER_OWNER_PID_ENV,
        std::process::id().to_string(),
    );
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
            let _ = child.wait();
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
                let _ = child.wait();
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

#[cfg(test)]
impl ClaudeChild {
    pub(super) fn kill(&mut self) -> std::io::Result<()> {
        match self {
            Self::Started(child) => child.kill(),
            Self::Adopted { .. } => Ok(()),
        }
    }

    pub(super) fn wait(&mut self) {
        self.kill_and_wait();
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
