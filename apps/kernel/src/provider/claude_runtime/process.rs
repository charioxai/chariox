use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::Value;

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

pub(super) fn stop_child(child: &mut Child) {
    // Signal only the exact child created by this runtime.
    if child.id() <= 1 {
        return;
    }
    match child.try_wait() {
        Ok(Some(_)) => {}
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
        }
        Err(_) => {}
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
