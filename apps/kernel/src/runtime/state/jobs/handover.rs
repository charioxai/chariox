//! Transfers job readers after they stop consuming output.
use super::execute::{Output, OutputCheckpoint};
use super::*;
use crate::room_exec::{FrozenJobExec, JobExecBuffer, JobExecHandover};
use serde::{Deserialize, Serialize};
use std::os::fd::{FromRawFd as _, OwnedFd};
use tokio::sync::Notify;

impl KernelRuntimeState {
    pub(super) async fn track_job(&self) -> ActiveJob {
        let supervisors = &self
            .job_wake_driver
            .as_ref()
            .expect("a job has a driver")
            .supervisors;
        match supervisors.enter() {
            Some(active) => active,
            None => std::future::pending().await,
        }
    }

    pub(super) fn freeze_job(
        &self,
        record: &Record,
        output: &mut Output,
        progress: JobProgress,
        exec: Option<FrozenJobExec>,
    ) -> Result<(), DaemonError> {
        let (exec, buffer, pipes, guard) = match exec {
            Some(exec) => (
                Some(exec.handover),
                Some(exec.buffer),
                exec.pipes,
                exec.guard,
            ),
            None => (None, None, Vec::new(), None),
        };
        let log = output
            .keep_log()
            .map_err(|failure| ledger::error(failure.to_string()))?;
        let state = zeroize::Zeroizing::new(
            serde_json::to_vec(&JobState {
                reader: output.take_checkpoint(),
                buffer,
                progress,
            })
            .map_err(|failure| ledger::error(failure.to_string()))?,
        );
        crate::hot_reload::hand_over_job(
            record.job_id.clone(),
            record.wake_id.clone(),
            exec,
            pipes,
            guard,
            log,
            &state,
        )
        .map_err(|failure| ledger::error(failure.to_string()))
    }

    pub(in crate::runtime::state) async fn shutdown_jobs(&self, reload: bool) {
        if let Some(driver) = &self.job_wake_driver {
            driver
                .supervisors
                .shutdown(if reload {
                    Shutdown::Reload
                } else {
                    Shutdown::Restart
                })
                .await;
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) enum Shutdown {
    #[default]
    Running,
    Restart,
    Reload,
}

#[derive(Default)]
pub(super) struct JobSupervisors {
    active: std::sync::Mutex<usize>,
    shutdown: watch::Sender<Shutdown>,
    drained: Notify,
}

impl JobSupervisors {
    pub(super) fn enter(self: &Arc<Self>) -> Option<ActiveJob> {
        let mut active = self.active.lock().expect("active job supervisors");
        let shutdown = self.shutdown.subscribe();
        if !matches!(*shutdown.borrow(), Shutdown::Running) {
            return None;
        }
        *active += 1;
        Some(ActiveJob {
            jobs: Arc::clone(self),
            shutdown,
        })
    }

    async fn shutdown(&self, kind: Shutdown) {
        {
            let _active = self.active.lock().expect("active job supervisors");
            self.shutdown.send_replace(kind);
        }
        let wait = async {
            loop {
                let drained = self.drained.notified();
                tokio::pin!(drained);
                drained.as_mut().enable();
                if *self.active.lock().expect("active job supervisors") == 0 {
                    return;
                }
                drained.await;
            }
        };
        if tokio::time::timeout(Duration::from_secs(2), wait)
            .await
            .is_err()
        {
            tracing::warn!("job supervisors did not stop before kernel shutdown");
        }
    }
}

pub(super) struct ActiveJob {
    jobs: Arc<JobSupervisors>,
    shutdown: watch::Receiver<Shutdown>,
}

impl ActiveJob {
    pub(super) async fn shutdown(&mut self) -> Shutdown {
        *self
            .shutdown
            .wait_for(|kind| !matches!(kind, Shutdown::Running))
            .await
            .expect("the registered job's driver holds its shutdown channel")
    }
}

impl Drop for ActiveJob {
    fn drop(&mut self) {
        let mut active = self.jobs.active.lock().expect("active job supervisors");
        *active -= 1;
        if *active == 0 {
            self.jobs.drained.notify_waiters();
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct JobProgress {
    pub(super) cursor: u64,
    pub(super) stopped: Option<JobOutcome>,
}

#[derive(Serialize, Deserialize)]
struct JobState {
    reader: OutputCheckpoint,
    buffer: Option<JobExecBuffer>,
    progress: JobProgress,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct JobHandover {
    job: String,
    wake: String,
    exec: Option<JobExecHandover>,
    state_fd: i32,
    log_fd: i32,
}

impl JobHandover {
    pub(crate) fn new(
        job: String,
        wake: String,
        exec: Option<JobExecHandover>,
        state_fd: i32,
        log_fd: i32,
    ) -> Self {
        Self {
            job,
            wake,
            exec,
            state_fd,
            log_fd,
        }
    }

    pub(crate) fn id(&self) -> &str {
        &self.job
    }

    pub(crate) fn wake_id(&self) -> &str {
        &self.wake
    }

    pub(crate) fn pid(&self) -> Option<u32> {
        self.exec.as_ref().and_then(JobExecHandover::pid)
    }

    pub(crate) fn fds(&self) -> impl Iterator<Item = i32> + '_ {
        [self.state_fd, self.log_fd]
            .into_iter()
            .chain(self.exec.iter().flat_map(JobExecHandover::fds))
    }

    pub(crate) fn discard(self) {
        // The previous program named both sets of descriptors for this job alone.
        drop(unsafe { OwnedFd::from_raw_fd(self.state_fd) });
        drop(unsafe { OwnedFd::from_raw_fd(self.log_fd) });
        if let Some(exec) = self.exec {
            exec.discard();
        }
    }

    pub(super) fn matches(&self, record: &Record) -> bool {
        self.wake == record.wake_id
            && record.result.is_none()
            && match &self.exec {
                Some(exec) => {
                    record.placement == JobPlacement::Local
                        && exec.matches(&record.room_id, record.exec_id.as_deref())
                }
                None => {
                    record.placement == JobPlacement::Remote
                        && matches!(record.remote, RemoteStage::Running { .. })
                }
            }
    }

    pub(super) fn restore(
        self,
        runtime: &KernelRuntimeState,
        record: &Record,
    ) -> Result<(Output, JobProgress), DaemonError> {
        let state = crate::hot_reload::read_job_state(self.state_fd)
            .and_then(|bytes| {
                serde_json::from_slice::<JobState>(&bytes).map_err(std::io::Error::other)
            })
            .map_err(|failure| ledger::error(format!("cannot restore job output: {failure}")));
        // The log descriptor is shared with the outgoing reader at its last write offset.
        let log = unsafe { std::fs::File::from_raw_fd(self.log_fd) };
        let state = match state {
            Ok(state) => state,
            Err(error) => {
                if let Some(exec) = self.exec {
                    exec.discard();
                }
                return Err(error);
            }
        };
        match (self.exec, state.buffer) {
            (Some(exec), Some(buffer)) => runtime.owned.room_execs.restore_job(exec, buffer)?,
            (None, None) => {}
            (exec, _) => {
                if let Some(exec) = exec {
                    exec.discard();
                }
                return Err(ledger::error(
                    "the job handover has inconsistent command state",
                ));
            }
        }
        Ok((
            Output::reopen(runtime, record, state.reader, log),
            state.progress,
        ))
    }
}
