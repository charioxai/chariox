//! The pipe supervisor for a local job. Only this task reads the job's pipes, so it can
//! stop between reads and leave both the unread pipes and the bytes already buffered.
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::ExitStatusExt as _;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt as _;
use tokio::net::unix::pipe::Receiver;
use tokio::sync::{mpsc, oneshot, watch};

use super::process::{self, Process};
use super::{
    buffer::PIPE_READ_BYTES, CancelReason, ExecOutcome, ExecStream, Record, RoomExecService,
};
use crate::runtime::bounded_process::AsyncChildOwner as _;
use crate::runtime::owned_process_signals::{OwnedProcessSignals, ProcessWitness};
use crate::runtime::process_resources::{LaunchPermit, OomKill, PreparedDomain, ProcessDomain};

pub(super) type TransferRequest = oneshot::Sender<DetachedProcess>;

#[derive(Debug, Serialize, Deserialize)]
struct ChildHandover {
    witness: ProcessWitness,
    domain: String,
    cgroup: Option<std::path::PathBuf>,
    memory_stop: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StreamsHandover {
    child: ChildState,
    pub(super) stdout: Option<i32>,
    pub(super) stderr: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
enum ChildState {
    Running(ChildHandover),
    Ended(ChildExit),
}

#[derive(Debug, Serialize, Deserialize)]
struct ChildExit {
    outcome: ExecOutcome,
    oom: Option<OomKill>,
}

impl ChildExit {
    fn notice(self, record: &Record) -> ExecOutcome {
        process::name_the_kill(record, self.outcome, self.oom)
    }
}

pub(super) struct DetachedProcess {
    pub(super) handover: StreamsHandover,
    pub(super) pipes: Vec<OwnedFd>,
    pub(super) guard: ProcessGuard,
}

pub(super) struct ProcessGuard {
    // On a failed exec these guards still end the old child and its resource domain.
    _child: Option<JobChild>,
    _permit: Option<LaunchPermit>,
}

impl StreamsHandover {
    pub(super) fn pid(&self) -> Option<u32> {
        match &self.child {
            ChildState::Running(child) => Some(child.witness.pid()),
            ChildState::Ended(_) => None,
        }
    }

    pub(super) fn discard(self) {
        if let ChildState::Running(child) = self.child {
            let pid = child.witness.pid();
            let mut signals = child.witness.into_signals();
            let _ = signals.kill_group();
            let _ = signals.kill_owned_processes();
            let _ = try_reap(pid);
        }
    }
}

enum JobChild {
    Started(Process),
    Adopted(AdoptedChild),
}

struct AdoptedChild {
    pid: u32,
    signals: OwnedProcessSignals,
    domain: Option<ProcessDomain>,
    unit: String,
    memory_stop: Option<u64>,
}

impl Drop for AdoptedChild {
    fn drop(&mut self) {
        let _ = self.signals.kill_group();
        let _ = self.signals.kill_owned_processes();
        let _ = try_reap(self.pid);
    }
}

impl JobChild {
    fn pid(&self) -> u32 {
        match self {
            Self::Started(process) => process.child.id().expect("the job child is still owned"),
            Self::Adopted(process) => process.pid,
        }
    }

    fn domain(&self) -> Option<&ProcessDomain> {
        match self {
            Self::Started(process) => Some(&process.domain),
            Self::Adopted(process) => process.domain.as_ref(),
        }
    }

    fn signals(&mut self) -> &mut OwnedProcessSignals {
        match self {
            Self::Started(process) => &mut process.signals,
            Self::Adopted(process) => &mut process.signals,
        }
    }

    fn memory_stop(&self) -> Option<u64> {
        let remembered = match self {
            Self::Started(_) => None,
            Self::Adopted(process) => process.memory_stop,
        };
        self.domain()
            .and_then(ProcessDomain::memory_limit_bytes)
            .or(remembered)
    }

    fn hand_over(&mut self, permit: Option<&LaunchPermit>) -> ChildHandover {
        let domain = match self {
            Self::Started(_) => permit.expect("a launched job retains its admission").id(),
            Self::Adopted(process) => &process.unit,
        }
        .to_owned();
        let cgroup = self.domain().and_then(ProcessDomain::cgroup_path);
        let memory_stop = self.memory_stop();
        ChildHandover {
            witness: self.signals().witness(),
            domain,
            cgroup,
            memory_stop,
        }
    }

    fn adopt(handover: ChildHandover, record: &Record) -> io::Result<Self> {
        let pid = handover.witness.pid();
        let mut process = AdoptedChild {
            pid,
            signals: handover.witness.into_signals(),
            domain: None,
            unit: handover.domain,
            memory_stop: handover.memory_stop,
        };
        if !process::exited_pid(pid)? {
            let domain =
                PreparedDomain::adopted(&process.unit, handover.cgroup.as_deref())?.start(pid)?;
            domain.bind_key(&process.unit);
            domain.list_as(&record.intent.session_id, record.intent.process_owner());
            process.domain = Some(domain);
        }
        Ok(Self::Adopted(process))
    }

    async fn end(&mut self, inspection: Option<io::Error>) -> ChildExit {
        let cleanup = self
            .signals()
            .kill_group()
            .and_then(|_| self.signals().kill_owned_processes());
        let status = match self {
            Self::Started(process) => {
                if cleanup.is_err() {
                    let _ = process.child.start_kill();
                }
                process.child.supervised_wait().await
            }
            Self::Adopted(process) => reap(process.pid).await,
        };
        let outcome = if let Some(error) = inspection.or_else(|| cleanup.err()) {
            ExecOutcome::Failed {
                message: format!("command cleanup failed: {error}"),
            }
        } else if let Some(limit_bytes) = self.memory_stop() {
            ExecOutcome::MemoryLimit { limit_bytes }
        } else {
            process::exit_outcome(status)
        };
        ChildExit {
            outcome,
            oom: self.domain().and_then(ProcessDomain::oom_kill),
        }
    }
}

fn try_reap(pid: u32) -> io::Result<Option<std::process::ExitStatus>> {
    if pid <= 1 || pid > i32::MAX as u32 {
        return Err(io::Error::other("invalid owned job PID"));
    }
    let mut status = 0;
    // The successful exec keeps this process the child's parent. Only this supervisor reaps it.
    match unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) } {
        -1 => Err(io::Error::last_os_error()),
        0 => Ok(None),
        _ => Ok(Some(std::process::ExitStatus::from_raw(status))),
    }
}

async fn reap(pid: u32) -> io::Result<std::process::ExitStatus> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = try_reap(pid)? {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the job child did not exit"))?
}

struct Supervisor {
    child: Option<JobChild>,
    permit: Option<LaunchPermit>,
    writer: Option<tokio::task::JoinHandle<()>>,
    stdout: Option<Receiver>,
    stderr: Option<Receiver>,
    outcome: Option<ChildExit>,
}

pub(super) fn start(
    mut process: Process,
    input: mpsc::Receiver<Option<Vec<u8>>>,
    cancel: watch::Receiver<Option<CancelReason>>,
    service: RoomExecService,
    record: Arc<Record>,
    permit: LaunchPermit,
    requests: mpsc::Receiver<TransferRequest>,
) {
    let stdout = process
        .child
        .stdout
        .take()
        .expect("piped stdout")
        .into_owned_fd()
        .and_then(Receiver::from_owned_fd);
    let stderr = process
        .child
        .stderr
        .take()
        .expect("piped stderr")
        .into_owned_fd()
        .and_then(Receiver::from_owned_fd);
    let (Ok(stdout), Ok(stderr)) = (stdout, stderr) else {
        service.finish(
            &record,
            ExecOutcome::Failed {
                message: "cannot supervise job output pipes".into(),
            },
        );
        return;
    };
    let stdin = process.child.stdin.take().expect("piped stdin");
    let writer = process::start_input(stdin, input, cancel.clone());
    let supervisor = Supervisor {
        child: Some(JobChild::Started(process)),
        permit: Some(permit),
        writer: Some(writer),
        stdout: Some(stdout),
        stderr: Some(stderr),
        outcome: None,
    };
    tokio::spawn(supervisor.run(service, record, cancel, requests));
}

pub(super) fn resume(
    handover: StreamsHandover,
    stdout: Option<OwnedFd>,
    stderr: Option<OwnedFd>,
    service: RoomExecService,
    record: Arc<Record>,
    requests: mpsc::Receiver<TransferRequest>,
) -> io::Result<()> {
    let (child, outcome) = match handover.child {
        ChildState::Running(child) => (Some(JobChild::adopt(child, &record)?), None),
        ChildState::Ended(outcome) => (None, Some(outcome)),
    };
    let stdout = stdout.map(Receiver::from_owned_fd).transpose()?;
    let stderr = stderr.map(Receiver::from_owned_fd).transpose()?;
    let cancel = record.cancel.subscribe();
    let supervisor = Supervisor {
        child,
        permit: None,
        writer: None,
        stdout,
        stderr,
        outcome,
    };
    tokio::spawn(supervisor.run(service, record, cancel, requests));
    Ok(())
}

enum Step {
    Transfer(TransferRequest),
    Cancel,
    Inspect,
    Stdout(io::Result<usize>),
    Stderr(io::Result<usize>),
}

impl Supervisor {
    async fn run(
        mut self,
        service: RoomExecService,
        record: Arc<Record>,
        mut cancel: watch::Receiver<Option<CancelReason>>,
        mut requests: mpsc::Receiver<TransferRequest>,
    ) {
        let mut tick = tokio::time::interval(Duration::from_millis(25));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut sampled = std::time::Instant::now();
        let mut stdout = vec![0u8; PIPE_READ_BYTES];
        let mut stderr = vec![0u8; PIPE_READ_BYTES];
        let outcome = loop {
            if self.child.is_none() && self.stdout.is_none() && self.stderr.is_none() {
                break self
                    .outcome
                    .take()
                    .expect("ended job has an outcome")
                    .notice(&record);
            }
            let step = tokio::select! {
                biased;
                Some(request) = requests.recv() => Step::Transfer(request),
                _ = process::cancelled(&mut cancel) => Step::Cancel,
                _ = tick.tick(), if self.child.is_some() => Step::Inspect,
                next = read(&mut self.stdout, &mut self.stderr, &mut stdout, &mut stderr) => next,
            };
            match step {
                Step::Transfer(request) => {
                    match self.detach().await {
                        Ok(transfer) => {
                            // If the reader went away, dropping the transfer still ends its child.
                            let _ = request.send(transfer);
                            return;
                        }
                        Err(error) => {
                            self.end_child(None).await;
                            break ExecOutcome::Failed {
                                message: format!("cannot retain the job's pipes: {error}"),
                            };
                        }
                    }
                }
                Step::Cancel => {
                    self.end_child(None).await;
                    break self
                        .outcome
                        .take()
                        .expect("canceled job has an outcome")
                        .notice(&record);
                }
                Step::Inspect => {
                    let child = self.child.as_mut().expect("inspects only running jobs");
                    let inspection = process::exited_pid(child.pid());
                    if !matches!(inspection, Ok(false)) {
                        self.end_child(inspection.err()).await;
                    } else if sampled.elapsed() >= Duration::from_millis(250) {
                        if let Some(mut permit) = self.permit.take() {
                            let observed = tokio::task::spawn_blocking(move || {
                                permit.observe();
                                permit
                            })
                            .await;
                            match observed {
                                Ok(permit) => self.permit = Some(permit),
                                Err(error) => {
                                    self.end_child(None).await;
                                    break ExecOutcome::Failed {
                                        message: format!("resource sampling failed: {error}"),
                                    };
                                }
                            }
                        }
                        sampled = std::time::Instant::now();
                    }
                }
                Step::Stdout(Ok(0)) => self.stdout = None,
                Step::Stderr(Ok(0)) => self.stderr = None,
                Step::Stdout(Ok(length)) => push(&record, ExecStream::Stdout, &stdout[..length]),
                Step::Stderr(Ok(length)) => push(&record, ExecStream::Stderr, &stderr[..length]),
                Step::Stdout(Err(error)) | Step::Stderr(Err(error)) => {
                    self.end_child(None).await;
                    break ExecOutcome::Failed {
                        message: format!("job output pipe failed: {error}"),
                    };
                }
            }
        };
        service.finish(&record, outcome);
    }

    async fn detach(&mut self) -> io::Result<DetachedProcess> {
        if let Some(mut writer) = self.writer.take() {
            match tokio::time::timeout(Duration::from_secs(1), &mut writer).await {
                Ok(result) => result.map_err(io::Error::other)?,
                Err(_) => {
                    writer.abort();
                    return Err(io::Error::other(
                        "job input did not settle before the reload",
                    ));
                }
            }
        }
        let mut pipes = Vec::new();
        let mut retain = |pipe: Option<Receiver>| -> io::Result<Option<i32>> {
            Ok(pipe
                .map(Receiver::into_nonblocking_fd)
                .transpose()?
                .map(|fd| {
                    let raw = fd.as_raw_fd();
                    pipes.push(fd);
                    raw
                }))
        };
        let stdout = retain(self.stdout.take())?;
        let stderr = retain(self.stderr.take())?;
        let child = match &mut self.child {
            Some(child) => ChildState::Running(child.hand_over(self.permit.as_ref())),
            None => ChildState::Ended(self.outcome.take().expect("ended job has an outcome")),
        };
        Ok(DetachedProcess {
            handover: StreamsHandover {
                child,
                stdout,
                stderr,
            },
            pipes,
            guard: ProcessGuard {
                _child: self.child.take(),
                _permit: self.permit.take(),
            },
        })
    }

    async fn end_child(&mut self, inspection: Option<io::Error>) {
        if let Some(child) = &mut self.child {
            self.outcome = Some(child.end(inspection).await);
            if let Some(writer) = self.writer.take() {
                writer.abort();
            }
            let child = self.child.take();
            let permit = self.permit.take();
            let _ = tokio::task::spawn_blocking(move || {
                drop(child);
                drop(permit);
            })
            .await;
        }
    }
}

async fn read(
    stdout: &mut Option<Receiver>,
    stderr: &mut Option<Receiver>,
    out: &mut [u8],
    err: &mut [u8],
) -> Step {
    tokio::select! {
        result = async { stdout.as_mut().unwrap().read(out).await }, if stdout.is_some() => Step::Stdout(result),
        result = async { stderr.as_mut().unwrap().read(err).await }, if stderr.is_some() => Step::Stderr(result),
        else => std::future::pending().await,
    }
}

fn push(record: &Record, stream: ExecStream, bytes: &[u8]) {
    record
        .output
        .lock()
        .expect("exec output poisoned")
        .push(stream, bytes);
    record.changed.notify_waiters();
}
