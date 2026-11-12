use crate::runtime::bounded_process::AsyncChildOwner as _;
use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Child;
use tokio::sync::{mpsc, watch};

use super::buffer::PIPE_READ_BYTES;
use super::{error, CancelReason, ExecOutcome, ExecStream, Record, RoomExecService};
use crate::error::DaemonError;
use crate::runtime::owned_process_signals::OwnedProcessSignals;

#[cfg(test)]
mod tests;

pub(super) struct Process {
    child: Child,
    signals: OwnedProcessSignals,
    _domain: crate::runtime::process_resources::ProcessDomain,
}

impl Drop for Process {
    fn drop(&mut self) {
        // The witness checks each current member and its birth before signalling.
        let _ = self.signals.kill_group();
        let _ = self.signals.kill_owned_processes();
    }
}

pub(super) fn spawn(
    cwd: &Path,
    argv: &[String],
    permit: &crate::runtime::process_resources::LaunchPermit,
    intent: &super::LaunchIntent,
) -> Result<Process, DaemonError> {
    spawn_with_environment(cwd, argv, permit, intent, None)
}

pub(super) fn spawn_job(
    cwd: &Path,
    argv: &[String],
    permit: &crate::runtime::process_resources::LaunchPermit,
    intent: &super::LaunchIntent,
) -> Result<Process, DaemonError> {
    let environment = [
        "PATH", "HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "LC_CTYPE", "TZ", "TMPDIR",
    ]
    .into_iter()
    .filter_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| (key.to_string(), value))
    })
    .collect();
    let launch = crate::provider::managed_isolated_utility_launch(
        argv[0].clone(),
        argv[1..].to_vec(),
        environment,
        Some(cwd.to_path_buf()),
        "agent-job",
    )?;
    let mut command = vec![launch
        .pty_program
        .ok_or_else(|| error("exec_spawn_failed", "Job isolation has no executable"))?];
    command.extend(launch.pty_args);
    spawn_with_environment(
        launch.working_directory.as_deref().unwrap_or(cwd),
        &command,
        permit,
        intent,
        Some(&launch.pty_env),
    )
}

fn spawn_with_environment(
    cwd: &Path,
    argv: &[String],
    permit: &crate::runtime::process_resources::LaunchPermit,
    intent: &super::LaunchIntent,
    environment: Option<&std::collections::BTreeMap<String, String>>,
) -> Result<Process, DaemonError> {
    let prepared = permit.prepare().map_err(|failure| {
        error(
            "exec_spawn_failed",
            format!("cannot prepare command resource domain: {failure}"),
        )
    })?;
    let (program, args) = prepared.command(argv[0].clone(), argv[1..].to_vec());
    let mut command = crate::process_spawn::async_command(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(environment) = environment {
        command.env_clear().envs(environment);
    }
    for name in crate::room_terminal::KERNEL_SECRET_ENVIRONMENT {
        command.env_remove(name);
    }
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|failure| {
        error(
            "exec_spawn_failed",
            format!("cannot start command: {failure}; check the program and Room directory"),
        )
    })?;
    let mut signals = match OwnedProcessSignals::for_tokio_session_child(&child) {
        Ok(signals) => signals,
        Err(failure) => {
            let _ = child.start_kill();
            return Err(error(
                "exec_spawn_failed",
                format!("cannot retain command ownership: {failure}; retry the command"),
            ));
        }
    };
    let domain = match prepared.start(child.id().expect("owned child")) {
        Ok(domain) => domain,
        Err(failure) => {
            let _ = signals.kill_group();
            let _ = signals.kill_owned_processes();
            let _ = child.start_kill();
            return Err(error(
                "exec_spawn_failed",
                format!("cannot watch command resources: {failure}"),
            ));
        }
    };
    domain.bind_key(permit.id());
    domain.list_as(&intent.session_id, intent.process_owner());
    Ok(Process {
        child,
        signals,
        _domain: domain,
    })
}

pub(super) async fn cancelled(cancel: &mut watch::Receiver<Option<CancelReason>>) -> CancelReason {
    if let Some(reason) = *cancel.borrow() {
        return reason;
    }
    let _ = cancel.changed().await;
    (*cancel.borrow()).unwrap_or(CancelReason::Canceled)
}

enum OutputEnd {
    Eof,
    Canceled(CancelReason),
}

async fn output(
    mut pipe: impl AsyncRead + Unpin,
    record: Arc<Record>,
    stream: ExecStream,
    mut cancel: watch::Receiver<Option<CancelReason>>,
) -> io::Result<OutputEnd> {
    let mut bytes = vec![0; PIPE_READ_BYTES];
    loop {
        let length = tokio::select! {
            biased;
            read = pipe.read(&mut bytes) => read,
            reason = cancelled(&mut cancel) => return Ok(OutputEnd::Canceled(reason)),
        };
        let length = match length {
            Ok(length) => length,
            Err(failure) => {
                record.cancel.send_replace(Some(CancelReason::Canceled));
                return Err(failure);
            }
        };
        if length == 0 {
            return Ok(OutputEnd::Eof);
        }
        loop {
            let space = record.space.notified();
            tokio::pin!(space);
            space.as_mut().enable();
            if record
                .output
                .lock()
                .expect("exec output poisoned")
                .push(stream, &bytes[..length])
            {
                record.changed.notify_waiters();
                break;
            }
            tokio::select! {
                reason = cancelled(&mut cancel) => return Ok(OutputEnd::Canceled(reason)),
                _ = space => {},
            }
        }
    }
}

pub(super) fn run(
    mut process: Process,
    record: Arc<Record>,
    mut input: mpsc::Receiver<Option<Vec<u8>>>,
    mut cancel: watch::Receiver<Option<CancelReason>>,
    service: RoomExecService,
    permit: crate::runtime::process_resources::LaunchPermit,
) {
    let mut permit = Some(permit);
    let mut stdin = process.child.stdin.take().expect("piped stdin");
    let stdout = process.child.stdout.take().expect("piped stdout");
    let stderr = process.child.stderr.take().expect("piped stderr");
    let out = tokio::spawn(output(
        stdout,
        record.clone(),
        ExecStream::Stdout,
        cancel.clone(),
    ));
    let err = tokio::spawn(output(
        stderr,
        record.clone(),
        ExecStream::Stderr,
        cancel.clone(),
    ));
    let mut input_cancel = cancel.clone();
    let writer = tokio::spawn(async move {
        tokio::select! {
            _ = cancelled(&mut input_cancel) => {},
            _ = async {
                while let Some(Some(bytes)) = input.recv().await {
                    if stdin.write_all(&bytes).await.is_err() { break; }
                }
            } => {},
        }
    });
    tokio::spawn(async move {
        let outcome = supervise(&mut process, &mut cancel, &mut permit).await;
        writer.abort();
        let _ = writer.await;
        let (out, err) = tokio::join!(out, err);
        let outcome = match (out, err) {
            (Ok(Ok(OutputEnd::Eof)), Ok(Ok(OutputEnd::Eof))) => outcome,
            (Ok(Err(failure)), _) | (_, Ok(Err(failure))) => ExecOutcome::Failed {
                message: format!("command output pipe failed: {failure}; the stream is incomplete"),
            },
            (Err(failure), _) | (_, Err(failure)) => ExecOutcome::Failed {
                message: format!("command output task failed: {failure}; the stream is incomplete"),
            },
            (Ok(Ok(OutputEnd::Canceled(reason))), _) | (_, Ok(Ok(OutputEnd::Canceled(reason)))) => {
                ExecOutcome::Failed {
                    message: format!(
                        "command {} before its output ended; the stream is incomplete",
                        match reason {
                            CancelReason::Canceled => "canceled",
                            CancelReason::Expired => "expired",
                        }
                    ),
                }
            }
        };
        let outcome = process
            ._domain
            .memory_limit_bytes()
            .map_or(outcome, |limit_bytes| ExecOutcome::MemoryLimit {
                limit_bytes,
            });
        // Only the reaped process and settled pipes publish the final outcome.
        // Stop the resource watch outside the async executor: its Drop joins a thread.
        let _ = tokio::task::spawn_blocking(move || {
            drop(process);
            drop(permit);
        })
        .await;
        service.finish(&record, outcome);
    });
}

async fn supervise(
    process: &mut Process,
    cancel: &mut watch::Receiver<Option<CancelReason>>,
    permit: &mut Option<crate::runtime::process_resources::LaunchPermit>,
) -> ExecOutcome {
    // Observe exit without reaping first: the root's birth still witnesses its
    // complete private session when descendants must be cleaned up.
    let mut sampled = std::time::Instant::now();
    let failure = loop {
        tokio::select! {
            _ = cancelled(cancel) => break None,
            _ = tokio::time::sleep(Duration::from_millis(25)) => {},
        }
        if sampled.elapsed() >= Duration::from_millis(250) {
            // Sampling takes the process-resources registry lock, which worker
            // cleanup can hold: keep it off the runtime thread.
            let Some(mut held) = permit.take() else {
                break Some("the launch permit was lost".to_string());
            };
            match tokio::task::spawn_blocking(move || {
                held.observe();
                held
            })
            .await
            {
                Ok(held) => *permit = Some(held),
                Err(failure) => break Some(format!("resource sampling failed: {failure}")),
            }
            sampled = std::time::Instant::now();
        }
        match exited(&process.child) {
            Ok(true) => break None,
            Ok(false) => {}
            Err(failure) => break Some(failure.to_string()),
        }
    };
    let cleanup = process
        .signals
        .kill_group()
        .and_then(|_| process.signals.kill_owned_processes());
    if cleanup.is_err() {
        // The handle still owns the unreaped child even when group inspection
        // fails; cancel its root rather than wait forever for a live command.
        let _ = process.child.start_kill();
    }
    let status = process.child.supervised_wait().await;
    if let Some(message) = failure.or_else(|| cleanup.err().map(|error| error.to_string())) {
        return ExecOutcome::Failed {
            message: format!("command cleanup failed: {message}"),
        };
    }
    match status {
        Ok(status) => {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(signal) = status.signal() {
                    return ExecOutcome::Signaled { signal };
                }
            }
            status.code().map_or_else(
                || ExecOutcome::Failed {
                    message: "command exited without a status".to_string(),
                },
                |code| ExecOutcome::Exited { code },
            )
        }
        Err(failure) => ExecOutcome::Failed {
            message: format!("cannot reap command: {failure}"),
        },
    }
}

#[cfg(unix)]
fn exited(child: &Child) -> io::Result<bool> {
    let pid = child
        .id()
        .filter(|pid| *pid > 1 && *pid <= i32::MAX as u32)
        .ok_or_else(|| io::Error::other("invalid owned child PID"))?;
    let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    if unsafe {
        libc::waitid(
            libc::P_PID,
            pid,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { info.assume_init().si_pid() } != 0)
}

#[cfg(not(unix))]
fn exited(_child: &Child) -> io::Result<bool> {
    Err(io::Error::other(
        "Room exec requires Unix process ownership",
    ))
}
