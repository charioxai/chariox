//! Bounded requests to the retained owner, never an independent SDK dispatcher.
use super::*;
use serde_json::{json, Value};
use std::sync::mpsc;

pub(super) struct Request {
    pub event: &'static str,
    pub data: Value,
    pub reply: mpsc::SyncSender<Result<()>>,
    dispatched: Arc<AtomicBool>,
}
pub(super) struct Receipt {
    response: mpsc::Receiver<Result<()>>,
    dispatched: Arc<AtomicBool>,
}
impl Control {
    pub(super) fn enqueue(&self, event: &'static str, data: Value) -> Result<Receipt> {
        let (reply, response) = mpsc::sync_channel(1);
        let dispatched = Arc::new(AtomicBool::new(false));
        let mut pending = self
            .notification
            .lock()
            .map_err(|_| LifecycleError::Supervisor)?;
        if self.finished() {
            return Err(LifecycleError::NotificationNotDispatched);
        }
        if self.stopped() {
            return Err(LifecycleError::Stopped);
        }
        if pending.is_some() || self.idle_requested.load(Ordering::Acquire) {
            return Err(LifecycleError::Busy);
        }
        if event == "suspend" {
            self.idle_requested.store(true, Ordering::Release);
        }
        *pending = Some(Request {
            event,
            data,
            reply,
            dispatched: dispatched.clone(),
        });
        self.wake.notify_all();
        Ok(Receipt {
            response,
            dispatched,
        })
    }
    pub(super) fn wait_notification(&self, receipt: Receipt) -> Result<()> {
        match receipt.response.recv_timeout(Duration::from_secs(45)) {
            Ok(result) => result,
            Err(error) => {
                if matches!(error, mpsc::RecvTimeoutError::Timeout) {
                    self.cancel(false);
                }
                if receipt.dispatched.load(Ordering::Acquire) {
                    Err(LifecycleError::Notification)
                } else {
                    Err(LifecycleError::NotificationNotDispatched)
                }
            }
        }
    }
    pub(super) fn notify(&self, event: &'static str, data: Value) -> Result<()> {
        self.wait_notification(self.enqueue(event, data)?)
    }
}

pub(super) fn configuration(context: &owner::Context) -> Result<Value> {
    let grants = context
        .store
        .app_connection_grants(&context.owner, &context.installation)
        .map_err(|_| LifecycleError::Storage)?;
    // The effective mutable installation configuration exposed by the SDK is
    // its connection grants. No credentials or host configuration are copied.
    Ok(json!({"connections": grants.into_iter().map(|grant| json!({
        "generatorId": grant.generator_id, "connectionId": grant.connection_id,
    })).collect::<Vec<_>>()}))
}

/// Runs on the one retained owner thread. A reply or cancellation does not
/// prove that an SDK callback settled; a failed dispatch ends this generation.
pub(super) fn serve(
    context: &owner::Context,
    admission: &ActiveStartAdmission,
    owner: &crate::runtime::app_worker::AppWorkerOwner,
    handle: crate::runtime::app_worker::ActivatedApp,
    events: &mut tokio::sync::mpsc::Receiver<chariox_app_runtime::worker_peer::ControlEvent>,
    catalog: Arc<chariox_app_runtime::app_outbox::EventCatalog>,
) -> (Result<()>, bool) {
    let mut callback_settled = true;
    let work = (|| {
        if context.control.stopped() {
            return Err(LifecycleError::Stopped);
        }
        let mut configuration = notifications::configuration(context)?;
        callback_settled = false;
        owner
            .startup_blocking(|| context.control.stopped())
            .map_err(|_| LifecycleError::Startup)?;
        callback_settled = true;
        if matches!(context.kind, StartKind::Active { .. })
            && context
                .publisher
                .is_suspended(&context.owner, &context.installation)
        {
            if let Some(previous) = context
                .publisher
                .dormant_configuration(&context.owner, &context.installation)
            {
                if previous != configuration {
                    callback_settled = false;
                    owner
                        .notify_blocking(
                            "configuration_change",
                            serde_json::json!({"previous":previous,"current":configuration}),
                            || context.control.stopped(),
                        )
                        .map_err(|_| LifecycleError::Notification)?;
                }
            }
            callback_settled = false;
            owner
                .notify_blocking(
                    "resume",
                    serde_json::json!({"reason":"idle", "configuration":configuration}),
                    || context.control.stopped(),
                )
                .map_err(|_| LifecycleError::Notification)?;
            callback_settled = true;
        }
        context.store.record_app_worker(
            admission,
            WorkerPhase::Running,
            true,
            None,
            context.control.budget(),
        )?;
        if context.control.stopped() {
            return Err(LifecycleError::Stopped);
        }
        #[cfg(test)]
        if let Some(checkpoint) = &context.start_checkpoint {
            checkpoint(StartCheckpoint::BeforePublication);
        }
        context
            .publisher
            .publish(admission.owner(), handle)
            .map_err(|_| LifecycleError::Startup)?;
        let mut authority_check = Instant::now();
        let mut pending_check = None;
        loop {
            if context.control.stopped() {
                break;
            }
            if owner.is_closed() {
                break;
            }
            let request = context
                .control
                .notification
                .lock()
                .map_err(|_| LifecycleError::Supervisor)?
                .take();
            if let Some(request) = request {
                let reservation = if request.event == "suspend" {
                    let Some(reservation) = context.publisher.reserve_dormant(
                        &context.owner,
                        catalog.clone(),
                        configuration.clone(),
                    ) else {
                        // Capacity refusal never drains or fails a live worker.
                        context
                            .control
                            .idle_requested
                            .store(false, Ordering::Release);
                        let _ = request.reply.send(Err(LifecycleError::Busy));
                        continue;
                    };
                    // A wake admitted before suspension keeps the worker live.
                    if !context.control.begin_idle_drain() {
                        // Roll back capacity and visibility before another
                        // idle request can enter or the caller observes Busy.
                        drop(reservation);
                        context
                            .control
                            .idle_requested
                            .store(false, Ordering::Release);
                        let _ = request.reply.send(Err(LifecycleError::Busy));
                        #[cfg(test)]
                        if let Some(checkpoint) = context
                            .control
                            .idle_refusal_checkpoint
                            .lock()
                            .unwrap()
                            .clone()
                        {
                            checkpoint();
                        }
                        continue;
                    }
                    Some(reservation)
                } else {
                    None
                };
                request.dispatched.store(true, Ordering::Release);
                callback_settled = false;
                let result = (|| {
                    if reservation.is_some() {
                        owner
                            .begin_draining()
                            .map_err(|_| LifecycleError::Notification)?;
                    }
                    owner
                        .notify_blocking(request.event, request.data, || context.control.stopped())
                        .map_err(|_| LifecycleError::Notification)?;
                    if let Some(reservation) = reservation {
                        // Commit before stopping: a crash after suspension must
                        // leave this App dormant, rather than booting it again.
                        let _operation = owner::queue(&context.control, &context.admission)?;
                        match context
                            .store
                            .suspend_app_worker(admission, context.control.budget())
                        {
                            Ok(()) => {}
                            // Host storage failure is not an App failure. Keep
                            // the prior in-memory suspension; after a reboot
                            // this App may start once if the flag did not commit.
                            Err(LifecycleStoreError::Storage) => {}
                            Err(error) => return Err(error.into()),
                        }
                        context.control.cancel_idle()?;
                        if !reservation.commit() {
                            return Err(LifecycleError::Stopped);
                        }
                    }
                    Ok(())
                })();
                let _ = request.reply.send(result);
                result?;
            }
            // Control frames are bounded by the existing peer. They are not a
            // second log sink, user transcript or an App-provided health proof.
            for _ in 0..16 {
                if events.try_recv().is_err() {
                    break;
                }
            }
            if Instant::now() >= authority_check {
                // Contention cannot renew a check's deadline or immediately
                // kill a healthy worker. Keep one budget until admission wins.
                let check = pending_check.get_or_insert_with(|| {
                    super::authority_check::AuthorityCheck::new(
                        context.control.budget(),
                        context.admission.clone(),
                    )
                });
                if check.verify(&context.store, admission)? {
                    let current = notifications::configuration(context)?;
                    if current != configuration {
                        callback_settled = false;
                        owner
                            .notify_blocking(
                                "configuration_change",
                                serde_json::json!({
                                    "previous":configuration, "current":current,
                                }),
                                || context.control.stopped(),
                            )
                            .map_err(|_| LifecycleError::Notification)?;
                        configuration = current;
                    }
                    pending_check = None;
                    authority_check = Instant::now() + Duration::from_secs(2);
                }
            }
            context.control.wait(Duration::from_millis(100));
        }
        Ok(())
    })();
    (work, callback_settled)
}
