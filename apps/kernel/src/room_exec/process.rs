use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};

use super::buffer::PIPE_READ_BYTES;
use super::{error, CancelReason, ExecOutcome, ExecStream, Record, RoomExecService};
use crate::error::DaemonError;
use crate::runtime::owned_process_signals::OwnedProcessSignals;

pub(super) struct Process {
    child: Child,
    signals: OwnedProcessSignals,
}

impl Drop for Process {
    fn drop(&mut self) {
        // The witness checks each current member and its birth before signalling.
        let _ = self.signals.kill_group();
        let _ = self.signals.kill_owned_processes();
    }
}

pub(super) fn spawn(cwd: &Path, argv: &[String]) -> Result<Process, DaemonError> {
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
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
    let signals = match OwnedProcessSignals::for_tokio_session_child(&child) {
        Ok(signals) => signals,
        Err(failure) => {
            let _ = child.start_kill();
            return Err(error(
                "exec_spawn_failed",
                format!("cannot retain command ownership: {failure}; retry the command"),
            ));
        }
    };
    Ok(Process { child, signals })
}

async fn cancelled(cancel: &mut watch::Receiver<Option<CancelReason>>) -> CancelReason {
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
) {
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
        let outcome = supervise(&mut process, &mut cancel).await;
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
        // Only the reaped process and settled pipes publish the final outcome.
        record.output.lock().expect("exec output poisoned").outcome = Some(outcome);
        record.changed.notify_waiters();
        service.inner.running.fetch_sub(1, Ordering::AcqRel);
        service.report_work();
    });
}

async fn supervise(
    process: &mut Process,
    cancel: &mut watch::Receiver<Option<CancelReason>>,
) -> ExecOutcome {
    // Observe exit without reaping first: the root's birth still witnesses its
    // complete private session when descendants must be cleaned up.
    let failure = loop {
        tokio::select! {
            _ = cancelled(cancel) => break None,
            _ = tokio::time::sleep(Duration::from_millis(25)) => {},
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
    let status = process.child.wait().await;
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
