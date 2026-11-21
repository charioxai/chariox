use super::execute::{command_input, note_failed_end, result_from_exit, Output, OutputCheckpoint};
use super::*;
use crate::room_exec::{JobExecBuffer, JobExecHandover};
use serde::{Deserialize, Serialize};
use std::os::fd::{FromRawFd as _, OwnedFd};
use tokio::sync::Notify;
use zeroize::Zeroizing;

impl KernelRuntimeState {
    pub(super) async fn run_local_job(
        &self,
        record: &mut Record,
        environment: &BTreeMap<String, Zeroizing<String>>,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
    ) -> Result<JobResult, DaemonError> {
        let active = self.track_local_job().await;
        record.placement = JobPlacement::Local;
        let session = self.owned.session_store.get_session(&record.room_id)?;
        let execs = &self.owned.room_execs;
        let input = command_input(&record.request, environment, &record.job_id)?;
        let exec_id = execs.open_job_as(
            &record.room_id,
            session.owner_user_id(),
            &record.cwd,
            &validate(&record.request)?,
            &record.request.needs,
            Some(self.clone()),
            Some(record.agent_id.clone()),
        )?;
        let owned = LocalExec {
            service: execs.clone(),
            room: record.room_id.clone(),
            id: exec_id.clone(),
        };
        record.exec_id = Some(exec_id.clone());
        save_record(self, record)?;
        let next = execs.write(
            &record.room_id,
            &exec_id,
            0,
            &base64::engine::general_purpose::STANDARD.encode(input.as_bytes()),
        )?;
        execs.close_stdin(&record.room_id, &exec_id, next)?;
        self.follow_local_job(
            record,
            output,
            cancelled,
            owned,
            active,
            LocalProgress::default(),
        )
        .await
    }

    pub(super) async fn resume_local_job(
        &self,
        record: &mut Record,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
        active: ActiveLocalJob,
        progress: LocalProgress,
    ) -> Result<JobResult, DaemonError> {
        let owned = LocalExec {
            service: self.owned.room_execs.clone(),
            room: record.room_id.clone(),
            id: record.exec_id.clone().expect("a restored job has an exec"),
        };
        self.follow_local_job(record, output, cancelled, owned, active, progress)
            .await
    }

    async fn follow_local_job(
        &self,
        record: &mut Record,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
        owned: LocalExec,
        mut active: ActiveLocalJob,
        mut progress: LocalProgress,
    ) -> Result<JobResult, DaemonError> {
        let execs = &owned.service;
        let exec_id = &owned.id;
        let mut deadline = record.running_at_ms.map(|start| {
            let elapsed =
                Duration::from_millis(crate::session::unix_epoch_ms().saturating_sub(start));
            tokio::time::Instant::now()
                + Duration::from_secs(record.request.timeout_s).saturating_sub(elapsed)
        });
        loop {
            if progress.stopped.is_none()
                && (*cancelled.borrow()
                    || deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline))
            {
                progress.stopped = Some(if *cancelled.borrow() {
                    JobOutcome::Cancelled
                } else {
                    JobOutcome::TimeLimit
                });
                execs.cancel(&record.room_id, exec_id)?;
            }
            if let Some(summary) = execs
                .list(&record.room_id)
                .into_iter()
                .find(|summary| &summary.exec_id == exec_id)
            {
                let state = match summary.state {
                    crate::room_exec::ExecState::Queued { .. } => "queued",
                    crate::room_exec::ExecState::Starting => "starting",
                    crate::room_exec::ExecState::Running => {
                        deadline.get_or_insert_with(|| {
                            tokio::time::Instant::now()
                                + Duration::from_secs(record.request.timeout_s)
                        });
                        record
                            .running_at_ms
                            .get_or_insert_with(crate::session::unix_epoch_ms);
                        "running"
                    }
                    crate::room_exec::ExecState::Finished => record.state.as_str(),
                };
                if record.state != state {
                    record.state = state.into();
                    save_record(self, record)?;
                }
            }
            let page = tokio::select! {
                biased;
                shutdown = active.shutdown() => {
                    if matches!(shutdown, Shutdown::Reload) {
                        if let Err(error) = self.freeze_local_job(record, output, progress, exec_id).await {
                            tracing::warn!(%error, "local job ownership could not be transferred");
                        }
                    } else {
                        execs.settle_job_for_restart(&record.room_id, exec_id).await;
                    }
                    drop(owned);
                    drop(active);
                    // The wake and job receipt only settle in the successor.
                    return std::future::pending().await;
                }
                page = execs.read(&record.room_id, exec_id, progress.cursor, Duration::from_millis(250)) => page?,
            };
            progress.cursor = page.next_cursor;
            output.page(self, &record.room_id, &page)?;
            if let Some(outcome) = page.outcome {
                execs.close(&record.room_id, exec_id)?;
                if self.job_wake_driver.as_ref().is_some_and(|driver| {
                    driver
                        .remote_moves
                        .lock()
                        .expect("job moves")
                        .contains_key(&record.job_id)
                }) {
                    record.exec_id = None;
                    return Err(DaemonError::MachineRequest {
                        code: "job_move_remote".into(),
                        message: "Queued job moved".into(),
                    });
                }
                note_failed_end(self, record, output, &outcome)?;
                return Ok(result_from_exit(record, outcome, progress.stopped));
            }
        }
    }

    async fn track_local_job(&self) -> ActiveLocalJob {
        let local = &self
            .job_wake_driver
            .as_ref()
            .expect("a local job has a driver")
            .local;
        match local.enter() {
            Some(active) => active,
            None => std::future::pending().await,
        }
    }

    async fn freeze_local_job(
        &self,
        record: &Record,
        output: &mut Output,
        progress: LocalProgress,
        exec_id: &str,
    ) -> Result<(), DaemonError> {
        let frozen = self
            .owned
            .room_execs
            .freeze_job(&record.room_id, exec_id)
            .await?;
        let log = output
            .keep_log()
            .map_err(|failure| ledger::error(failure.to_string()))?;
        let state = zeroize::Zeroizing::new(
            serde_json::to_vec(&LocalJobState {
                reader: output.take_checkpoint(),
                buffer: frozen.buffer,
                progress,
            })
            .map_err(|failure| ledger::error(failure.to_string()))?,
        );
        crate::hot_reload::hand_over_local_job(
            record.job_id.clone(),
            record.wake_id.clone(),
            frozen.handover,
            frozen.pipes,
            frozen.guard,
            log,
            &state,
        )
        .map_err(|failure| ledger::error(failure.to_string()))
    }

    pub(in crate::runtime::state) async fn shutdown_local_jobs(&self, reload: bool) {
        if let Some(driver) = &self.job_wake_driver {
            driver
                .local
                .shutdown(if reload {
                    Shutdown::Reload
                } else {
                    Shutdown::Restart
                })
                .await;
        }
    }
}

/// An output or persistence failure must not abandon the command's physical ownership.
struct LocalExec {
    service: crate::room_exec::RoomExecService,
    room: String,
    id: String,
}
impl Drop for LocalExec {
    fn drop(&mut self) {
        let _ = self.service.close(&self.room, &self.id);
    }
}

#[derive(Clone, Copy, Default)]
enum Shutdown {
    #[default]
    Running,
    Restart,
    Reload,
}

#[derive(Default)]
pub(super) struct LocalJobs {
    active: std::sync::Mutex<usize>,
    shutdown: watch::Sender<Shutdown>,
    drained: Notify,
}

impl LocalJobs {
    pub(super) fn enter(self: &Arc<Self>) -> Option<ActiveLocalJob> {
        let mut active = self.active.lock().expect("active local jobs");
        let shutdown = self.shutdown.subscribe();
        if !matches!(*shutdown.borrow(), Shutdown::Running) {
            return None;
        }
        *active += 1;
        Some(ActiveLocalJob {
            jobs: Arc::clone(self),
            shutdown,
        })
    }

    async fn shutdown(&self, kind: Shutdown) {
        {
            let _active = self.active.lock().expect("active local jobs");
            self.shutdown.send_replace(kind);
        }
        let wait = async {
            loop {
                let drained = self.drained.notified();
                tokio::pin!(drained);
                drained.as_mut().enable();
                if *self.active.lock().expect("active local jobs") == 0 {
                    return;
                }
                drained.await;
            }
        };
        if tokio::time::timeout(Duration::from_secs(2), wait)
            .await
            .is_err()
        {
            tracing::warn!("local job supervisors did not stop before kernel shutdown");
        }
    }
}

pub(super) struct ActiveLocalJob {
    jobs: Arc<LocalJobs>,
    shutdown: watch::Receiver<Shutdown>,
}

impl ActiveLocalJob {
    async fn shutdown(&mut self) -> Shutdown {
        *self
            .shutdown
            .wait_for(|kind| !matches!(kind, Shutdown::Running))
            .await
            .expect("the registered job's driver holds its shutdown channel")
    }
}

impl Drop for ActiveLocalJob {
    fn drop(&mut self) {
        let mut active = self.jobs.active.lock().expect("active local jobs");
        *active -= 1;
        if *active == 0 {
            self.jobs.drained.notify_waiters();
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct LocalProgress {
    cursor: u64,
    stopped: Option<JobOutcome>,
}

#[derive(Serialize, Deserialize)]
struct LocalJobState {
    reader: OutputCheckpoint,
    buffer: JobExecBuffer,
    progress: LocalProgress,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct LocalJobHandover {
    job: String,
    wake: String,
    exec: JobExecHandover,
    state_fd: i32,
    log_fd: i32,
}

impl LocalJobHandover {
    pub(crate) fn new(
        job: String,
        wake: String,
        exec: JobExecHandover,
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
        self.exec.pid()
    }

    pub(crate) fn fds(&self) -> impl Iterator<Item = i32> + '_ {
        [self.state_fd, self.log_fd]
            .into_iter()
            .chain(self.exec.fds())
    }

    pub(crate) fn discard(self) {
        // The previous program named both sets of descriptors for this job alone.
        drop(unsafe { OwnedFd::from_raw_fd(self.state_fd) });
        drop(unsafe { OwnedFd::from_raw_fd(self.log_fd) });
        self.exec.discard();
    }

    pub(super) fn restore(
        self,
        runtime: &KernelRuntimeState,
        record: &Record,
    ) -> Result<(Output, LocalProgress), DaemonError> {
        if !self
            .exec
            .matches(&record.room_id, record.exec_id.as_deref())
        {
            self.discard();
            return Err(ledger::error(
                "the local job receipt does not match its handover",
            ));
        }
        let state = crate::hot_reload::read_local_job_state(self.state_fd)
            .and_then(|bytes| {
                serde_json::from_slice::<LocalJobState>(&bytes).map_err(std::io::Error::other)
            })
            .map_err(|failure| {
                ledger::error(format!("cannot restore local job output: {failure}"))
            });
        // The log descriptor is shared with the outgoing reader at its last write offset.
        let log = unsafe { std::fs::File::from_raw_fd(self.log_fd) };
        let state = match state {
            Ok(state) => state,
            Err(error) => {
                self.exec.discard();
                return Err(error);
            }
        };
        runtime
            .owned
            .room_execs
            .restore_job(self.exec, state.buffer)?;
        Ok((
            Output::reopen(runtime, record, state.reader, log),
            state.progress,
        ))
    }
}
