//! Freezes a job's exec after its only reader has stopped consuming pages.
use std::os::fd::{FromRawFd as _, OwnedFd};

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use super::job_process::{ProcessGuard, StreamsHandover};
use super::*;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct JobExecHandover {
    intent: LaunchIntent,
    streams: Option<StreamsHandover>,
    cancel: Option<CancelReason>,
}

pub(crate) struct FrozenJobExec {
    pub(crate) handover: JobExecHandover,
    pub(crate) buffer: JobExecBuffer,
    pub(crate) pipes: Vec<OwnedFd>,
    pub(crate) guard: Option<JobExecGuard>,
}

pub(crate) struct JobExecGuard {
    _process: ProcessGuard,
}

impl JobExecHandover {
    pub(crate) fn pid(&self) -> Option<u32> {
        self.streams.as_ref().and_then(StreamsHandover::pid)
    }

    pub(crate) fn fds(&self) -> impl Iterator<Item = i32> + '_ {
        self.streams
            .iter()
            .flat_map(|streams| [streams.stdout, streams.stderr])
            .flatten()
    }

    pub(crate) fn matches(&self, room: &str, exec: Option<&str>) -> bool {
        self.intent.session_id == room && Some(self.intent.exec_id.as_str()) == exec
    }

    pub(crate) fn discard(self) {
        // The handover names each descriptor only once.
        for fd in self.fds() {
            drop(unsafe { OwnedFd::from_raw_fd(fd) });
        }
        if let Some(streams) = self.streams {
            streams.discard();
        }
    }
}

impl RoomExecService {
    pub(crate) async fn freeze_job(
        &self,
        room: &str,
        exec: &str,
    ) -> Result<FrozenJobExec, DaemonError> {
        let record = self.record(room, exec)?;
        let transfers = record
            .transfer
            .as_ref()
            .ok_or_else(|| error("job_handover", "the command is not a local job"))?;
        let request = match *record.state.lock().expect("exec state poisoned") {
            ExecState::Running => {
                let (reply, detached) = oneshot::channel();
                let _ = transfers.try_send(reply);
                Some(detached)
            }
            ExecState::Finished => None,
            ExecState::Starting | ExecState::Queued { .. } => {
                return Err(error("job_handover", "the job has not started running"));
            }
        };
        let detached = match request {
            Some(detached) => detached.await.ok(),
            None => None,
        };
        let (streams, pipes, guard) = match detached {
            Some(detached) => (
                Some(detached.handover),
                detached.pipes,
                Some(detached.guard),
            ),
            None => (None, Vec::new(), None),
        };
        let buffer = {
            let mut buffer = record.output.lock().expect("exec output poisoned");
            if streams.is_none() && buffer.outcome.is_none() {
                return Err(error(
                    "job_handover",
                    "the local job supervisor ended without a result",
                ));
            }
            buffer.checkpoint()
        };
        let cancel = *record.cancel.borrow();
        Ok(FrozenJobExec {
            handover: JobExecHandover {
                intent: record.intent.clone(),
                streams,
                cancel,
            },
            buffer,
            pipes,
            guard: guard.map(|guard| JobExecGuard { _process: guard }),
        })
    }

    pub(crate) fn restore_job(
        &self,
        handover: JobExecHandover,
        buffer: JobExecBuffer,
    ) -> Result<(), DaemonError> {
        let JobExecHandover {
            intent,
            streams,
            cancel,
        } = handover;
        let started_at = tokio::time::Instant::now();
        let reader_lease = lease::JOB_READER_LEASE;
        let (reader_deadline, deadline) = watch::channel(started_at + reader_lease);
        let (transfer, requests) = mpsc::channel(1);
        let exec = intent.exec_id.clone();
        let running = streams.is_some();
        let record = Arc::new(Record {
            stdin_file: None,
            scope: intent.session_id.clone(),
            intent,
            launch_command: Vec::new(),
            job: true,
            minimum_memory: 0,
            state: Mutex::new(if running {
                ExecState::Running
            } else {
                ExecState::Finished
            }),
            force: watch::channel(false).0,
            place: watch::channel(None).0,
            created_at: started_at,
            reader_deadline,
            reader_lease,
            input: Mutex::new(Input {
                sender: None,
                offset: 0,
                last: None,
            }),
            output: Mutex::new(buffer::Output::from_checkpoint(
                buffer,
                self.inner.output_pool.clone(),
            )),
            changed: Notify::new(),
            cancel: watch::channel(cancel).0,
            transfer: Some(transfer),
        });
        self.inner
            .records
            .lock()
            .expect("exec records poisoned")
            .insert(exec.clone(), record.clone());
        if let Some(streams) = streams {
            // The previous program handed over just these two read ends for this record.
            let stdout = streams.stdout.map(|fd| unsafe { OwnedFd::from_raw_fd(fd) });
            let stderr = streams.stderr.map(|fd| unsafe { OwnedFd::from_raw_fd(fd) });
            self.inner.running.fetch_add(1, Ordering::AcqRel);
            let result = job_process::resume(
                streams,
                stdout,
                stderr,
                self.clone(),
                record.clone(),
                requests,
            );
            if let Err(failure) = result {
                self.inner.running.fetch_sub(1, Ordering::AcqRel);
                self.inner
                    .records
                    .lock()
                    .expect("exec records poisoned")
                    .remove(&exec);
                return Err(error(
                    "job_handover",
                    format!("cannot resume the local job: {failure}"),
                ));
            }
        }
        lease::start(exec, Arc::downgrade(&self.inner), deadline);
        self.report_work();
        Ok(())
    }
}
