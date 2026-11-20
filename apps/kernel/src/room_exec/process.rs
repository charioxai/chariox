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
use super::{error, CancelReason, CommandExit, ExecOutcome, ExecStream, Record, RoomExecService};
use crate::error::DaemonError;
use crate::runtime::owned_process_signals::OwnedProcessSignals;

#[cfg(test)]
mod tests;

pub(super) struct Process {
    pub(super) child: Child,
    pub(super) signals: OwnedProcessSignals,
    pub(super) domain: crate::runtime::process_resources::ProcessDomain,
}

impl Drop for Process {
    fn drop(&mut self) {
        // The witness checks each current member and its birth before signalling.
        let _ = self.signals.kill_group();
        let _ = self.signals.kill_owned_processes();
    }
}

#[cfg(test)]
pub(super) fn spawn(
    cwd: &Path,
    argv: &[String],
    permit: &crate::runtime::process_resources::LaunchPermit,
    intent: &super::LaunchIntent,
) -> Result<Process, DaemonError> {
    spawn_command(cwd, argv, permit, intent, None)
}

pub(super) fn spawn_command(
    cwd: &Path,
    argv: &[String],
    permit: &crate::runtime::process_resources::LaunchPermit,
    intent: &super::LaunchIntent,
    environment: Option<&crate::provider::ProviderCredentialEnvironment>,
) -> Result<Process, DaemonError> {
    spawn_with_environment(cwd, argv, permit, intent, None, environment)
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
        None,
    )
}

fn spawn_with_environment(
    cwd: &Path,
    argv: &[String],
    permit: &crate::runtime::process_resources::LaunchPermit,
    intent: &super::LaunchIntent,
    environment: Option<&std::collections::BTreeMap<String, String>>,
    project_environment: Option<&crate::provider::ProviderCredentialEnvironment>,
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
    for name in crate::provider::managed_provider_control_env_remove() {
        command.env_remove(name);
    }
    if let Some(project_environment) = project_environment {
        command.envs(project_environment.iter());
    }
    // Everything the command starts inherits this claim, so cleanup owns a job that detached
    // before anything looked at the command's tree.
    let claim = format!("room-exec-{:032x}", rand::random::<u128>());
    command.env(
        crate::runtime::owned_process_signals::CLAIM_VARIABLE,
        &claim,
    );
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
            format!("cannot start command: {failure}; check the program and working directory"),
        )
    })?;
    let mut signals = match OwnedProcessSignals::for_claimed_tokio_session_child(&child, &claim) {
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
    if intent.session_id != super::KERNEL_SCOPE {
        domain.list_as(&intent.session_id, intent.process_owner());
    }
    Ok(Process {
        child,
        signals,
        domain,
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
        record
            .output
            .lock()
            .expect("exec output poisoned")
            .push(stream, &bytes[..length]);
        record.changed.notify_waiters();
    }
}

/// A command that ended by a signal, or failed after the memory controller
/// killed one of its processes, says so in its own output: the reader of its
/// outcome otherwise sees only an exit code. An OOM kill is the typed
/// memory-limit outcome, since the exit code of a tool that lost a child says
/// nothing of why.
pub(super) fn name_the_kill(
    record: &Record,
    outcome: ExecOutcome,
    oom: Option<crate::runtime::process_resources::OomKill>,
) -> ExecOutcome {
    let failed = match &outcome {
        ExecOutcome::Signaled { .. } => true,
        ExecOutcome::Exited { code } => *code != 0,
        ExecOutcome::MemoryLimit { .. }
        | ExecOutcome::Failed { .. }
        | ExecOutcome::TimedOut { .. } => return outcome,
    };
    let (line, outcome) = match (oom, &outcome) {
        (Some(kill), _) if failed => (
            kill.notice(),
            ExecOutcome::MemoryLimit {
                limit_bytes: kill.bytes,
            },
        ),
        (None, ExecOutcome::Signaled { signal }) => (
            crate::runtime::process_resources::signal_notice(*signal),
            outcome,
        ),
        _ => return outcome,
    };
    record
        .output
        .lock()
        .expect("exec output poisoned")
        .push(ExecStream::Stderr, format!("\n{line}\n").as_bytes());
    outcome
}

pub(super) fn run(
    mut process: Process,
    record: Arc<Record>,
    input: mpsc::Receiver<Option<Vec<u8>>>,
    mut cancel: watch::Receiver<Option<CancelReason>>,
    service: RoomExecService,
    permit: crate::runtime::process_resources::LaunchPermit,
) {
    let mut permit = Some(permit);
    let stdin = process.child.stdin.take().expect("piped stdin");
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
    let writer = start_input(stdin, input, cancel.clone());
    tokio::spawn(async move {
        let deadline = record.deadline().zip(record.intent.timeout_ms);
        let outcome = supervise(&mut process, &mut cancel, &mut permit, deadline).await;
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
        let outcome = match process.domain.memory_limit_bytes() {
            Some(limit_bytes) => ExecOutcome::MemoryLimit { limit_bytes },
            None => name_the_kill(&record, outcome, process.domain.oom_kill()),
        };
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

pub(super) fn start_input(
    mut stdin: tokio::process::ChildStdin,
    mut input: mpsc::Receiver<Option<Vec<u8>>>,
    mut cancel: watch::Receiver<Option<CancelReason>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::select! {
            _ = cancelled(&mut cancel) => {},
            _ = async {
                while let Some(Some(bytes)) = input.recv().await {
                    if stdin.write_all(&bytes).await.is_err() { break; }
                }
            } => {},
        }
    })
}

async fn supervise(
    process: &mut Process,
    cancel: &mut watch::Receiver<Option<CancelReason>>,
    permit: &mut Option<crate::runtime::process_resources::LaunchPermit>,
    deadline: Option<(tokio::time::Instant, u64)>,
) -> ExecOutcome {
    // Observe exit without reaping first: the root's birth still witnesses its
    // complete private session when descendants must be cleaned up.
    let mut sampled = std::time::Instant::now();
    let deadline = async {
        match deadline {
            Some((deadline, timeout_ms)) => {
                tokio::time::sleep_until(deadline).await;
                timeout_ms
            }
            None => std::future::pending().await,
        }
    };
    tokio::pin!(deadline);
    let mut timed_out = None;
    let failure = loop {
        tokio::select! {
            _ = cancelled(cancel) => break None,
            limit = &mut deadline => {
                match exited(&process.child) {
                    Ok(true) => {}
                    Ok(false) => timed_out = Some(limit),
                    Err(failure) => break Some(failure.to_string()),
                }
                break None;
            },
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
    match (timed_out, exit_outcome(status)) {
        (Some(timeout_ms), ExecOutcome::Exited { code }) => ExecOutcome::TimedOut {
            timeout_ms,
            exit: CommandExit::Exited { code },
        },
        (Some(timeout_ms), ExecOutcome::Signaled { signal }) => ExecOutcome::TimedOut {
            timeout_ms,
            exit: CommandExit::Signaled { signal },
        },
        (_, outcome) => outcome,
    }
}

pub(super) fn exit_outcome(status: io::Result<std::process::ExitStatus>) -> ExecOutcome {
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
        .ok_or_else(|| io::Error::other("invalid owned child PID"))?;
    exited_pid(pid)
}

#[cfg(unix)]
pub(super) fn exited_pid(pid: u32) -> io::Result<bool> {
    if pid <= 1 || pid > i32::MAX as u32 {
        return Err(io::Error::other("invalid owned child PID"));
    }
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
