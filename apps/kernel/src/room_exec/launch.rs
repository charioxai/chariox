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
    #[cfg(unix)]
    pub(super) transfers: Option<mpsc::Receiver<super::job_process::TransferRequest>>,
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
        #[cfg(unix)]
        transfers,
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
            if let Some(outcome) = record.expired_before_start() {
                service.finish(&record, outcome);
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
            let permit = service.inner.admission.reserve_with_memory(
                &intent.exec_id,
                &intent.command,
                &intent.cwd,
                *forced.borrow_and_update(),
                record.minimum_memory,
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
                        let environment = if launch.job {
                            None
                        } else {
                            authority
                                .as_ref()
                                .filter(|_| launch.intent.session_id != super::KERNEL_SCOPE)
                                .map(|state| {
                                    state.project_environment_for_shell(&launch.intent.session_id)
                                })
                                .transpose()?
                        };
                        if launch.expired_before_start().is_some() {
                            return Ok(None);
                        }
                        let stdin_file = launch
                            .stdin_file
                            .as_ref()
                            .map(|path| {
                                let mut options = std::fs::OpenOptions::new();
                                options.read(true);
                                // The checked pathname may change while admission waits.
                                // Never block on a replacement FIFO before validating its FD.
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::OpenOptionsExt;
                                    options.custom_flags(libc::O_NONBLOCK);
                                }
                                let file = options.open(path).map_err(|failure| {
                                    super::error(
                                        "exec_stdin_unreadable",
                                        format!(
                                            "stdin file `{}` cannot be read: {failure}",
                                            path.display()
                                        ),
                                    )
                                })?;
                                if !file
                                    .metadata()
                                    .map_err(|failure| {
                                        super::error("exec_stdin_unreadable", failure.to_string())
                                    })?
                                    .is_file()
                                {
                                    return Err(super::error(
                                        "exec_stdin_unreadable",
                                        "stdin must name a regular file",
                                    ));
                                }
                                Ok(file)
                            })
                            .transpose()?;
                        if launch.expired_before_start().is_some() {
                            return Ok(None);
                        }
                        let intent = &launch.intent;
                        if launch.job {
                            process::spawn_job(&intent.cwd, &launch.launch_command, &permit, intent)
                        } else {
                            process::spawn_command(
                                &intent.cwd,
                                &launch.launch_command,
                                &permit,
                                intent,
                                environment.as_ref(),
                            )
                        }
                        .map(|mut child| {
                            child.stdin_file = stdin_file;
                            Some((child, permit))
                        })
                    })
                    .await;
                    match spawned {
                        Ok(Ok(Some((child, permit)))) => {
                            *record.state.lock().expect("exec state poisoned") = ExecState::Running;
                            record.changed.notify_waiters();
                            #[cfg(unix)]
                            if let Some(transfers) = transfers {
                                super::job_process::start(
                                    child, input, cancel, service, record, permit, transfers,
                                );
                                return;
                            }
                            process::run(child, record, input, cancel, service, permit);
                        }
                        Ok(Ok(None)) => {
                            service.finish(
                                &record,
                                record
                                    .expired_before_start()
                                    .expect("launch deadline elapsed"),
                            );
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
