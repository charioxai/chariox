use super::execute::{command_input, note_failed_end, result_from_exit, Output};
use super::handover::{ActiveJob, JobProgress, Shutdown};
use super::*;
use zeroize::Zeroizing;

impl KernelRuntimeState {
    pub(super) async fn run_local_job(
        &self,
        record: &mut Record,
        environment: &BTreeMap<String, Zeroizing<String>>,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
    ) -> Result<JobResult, DaemonError> {
        let active = self.track_job().await;
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
            JobProgress::default(),
        )
        .await
    }

    pub(super) async fn resume_local_job(
        &self,
        record: &mut Record,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
        active: ActiveJob,
        progress: JobProgress,
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
        mut active: ActiveJob,
        mut progress: JobProgress,
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
                        let result = async {
                            let exec = execs.freeze_job(&record.room_id, exec_id).await?;
                            self.freeze_job(record, output, progress, Some(exec))
                        }.await;
                        if let Err(error) = result {
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
                        .remote_requested
                        .lock()
                        .expect("job move")
                        .remove(&record.job_id)
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
