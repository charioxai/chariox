//! A queued command owns its original pipes and reader lease without a PID.
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};

use super::placement::Placed;
use super::{process, remote, CancelReason, ExecOutcome, ExecState, Record, RoomExecService};

/// What can change a held launch's course while it waits.
pub(super) struct Signals {
    pub(super) cancel: watch::Receiver<Option<CancelReason>>,
    pub(super) forced: watch::Receiver<bool>,
    pub(super) placed: watch::Receiver<Option<Arc<Placed>>>,
}

pub(super) fn start(
    record: Arc<Record>,
    input: mpsc::Receiver<Option<Vec<u8>>>,
    signals: Signals,
    service: RoomExecService,
    authority: Option<crate::runtime::state::KernelRuntimeState>,
) {
    let Signals {
        mut cancel,
        mut forced,
        mut placed,
    } = signals;
    tokio::spawn(async move {
        loop {
            if cancel.borrow().is_some() {
                service.finish(
                    &record,
                    ExecOutcome::Failed {
                        message: "command canceled before start".into(),
                    },
                );
                return;
            }
            let intent = &record.intent;
            if let Some(authority) = &authority {
                if let Err(failure) = authority.authorize_queued_exec(intent) {
                    service.finish(
                        &record,
                        ExecOutcome::Failed {
                            message: failure.to_string(),
                        },
                    );
                    return;
                }
            }
            let elsewhere = placed.borrow().clone();
            if let Some(placed) = elsewhere {
                remote::run(record, input, cancel, service, placed).await;
                return;
            }
            let permit = service.inner.admission.reserve(
                &intent.exec_id,
                &intent.command,
                &intent.cwd,
                *forced.borrow_and_update(),
            );
            match permit {
                Ok(permit) => {
                    {
                        let mut state = record.state.lock().expect("exec state poisoned");
                        if cancel.borrow().is_some() {
                            drop(state);
                            drop(permit);
                            service.finish(
                                &record,
                                ExecOutcome::Failed {
                                    message: "command canceled before start".into(),
                                },
                            );
                            return;
                        }
                        if placed.borrow().is_some() {
                            // Placed while the permit was granted: it runs elsewhere.
                            continue;
                        }
                        *state = ExecState::Starting;
                    }
                    let launch = record.clone();
                    let spawned = tokio::task::spawn_blocking(move || {
                        if let Some(authority) = &authority {
                            authority.authorize_queued_exec(&launch.intent)?;
                        }
                        if launch.cancel.borrow().is_some() {
                            return Err(super::error(
                                "exec_canceled",
                                "command canceled before start",
                            ));
                        }
                        let spawn = if launch.job {
                            process::spawn_job
                        } else {
                            process::spawn
                        };
                        spawn(
                            &launch.intent.cwd,
                            &launch.launch_command,
                            &permit,
                            &launch.intent,
                        )
                        .map(|child| (child, permit))
                    })
                    .await;
                    match spawned {
                        Ok(Ok((child, permit))) => {
                            *record.state.lock().expect("exec state poisoned") = ExecState::Running;
                            record.changed.notify_waiters();
                            process::run(child, record, input, cancel, service, permit);
                        }
                        failure => {
                            let message = match failure {
                                Ok(Err(error)) => error.to_string(),
                                Err(error) => error.to_string(),
                                _ => unreachable!(),
                            };
                            service.finish(&record, ExecOutcome::Failed { message });
                        }
                    }
                    return;
                }
                Err(wait) => {
                    let mut state = record.state.lock().expect("exec state poisoned");
                    if placed.borrow().is_some() {
                        continue;
                    }
                    *state = ExecState::Queued { wait };
                    drop(state);
                    record.changed.notify_waiters();
                }
            }
            tokio::select! {
                _ = cancel.changed() => {},
                _ = forced.changed() => {},
                _ = placed.changed() => {},
                _ = tokio::time::sleep(Duration::from_millis(250)) => {},
            }
        }
    });
}
