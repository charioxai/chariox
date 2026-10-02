//! Bounded requests to the retained owner, never an independent SDK dispatcher.
use super::*;
use serde_json::{json, Value};
use std::sync::mpsc;

pub(super) struct Request {
    pub event: &'static str,
    pub data: Value,
    pub reply: mpsc::SyncSender<Result<()>>,
}
impl Control {
    pub(super) fn notify(&self, event: &'static str, data: Value) -> Result<()> {
        let (reply, response) = mpsc::sync_channel(1);
        {
            let mut pending = self
                .notification
                .lock()
                .map_err(|_| LifecycleError::Supervisor)?;
            if self.stopped() || self.finished() {
                return Err(LifecycleError::Stopped);
            }
            if pending.is_some() {
                return Err(LifecycleError::Busy);
            }
            *pending = Some(Request { event, data, reply });
        }
        self.wake.notify_all();
        // Includes an already admitted startup and the wire's 30 s cap. An
        // abandoned request never survives into a later worker generation.
        match response.recv_timeout(Duration::from_secs(45)) {
            Ok(result) => result,
            Err(_) => {
                // A timed-out caller cannot leave a notification queued to run
                // later. Withdrawal wakes the owner and preempts dispatch.
                self.cancel(false);
                Err(LifecycleError::Notification)
            }
        }
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
                .is_dormant(&context.owner, &context.installation)
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
                    callback_settled = true;
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
                callback_settled = false;
                let result = (|| {
                    if request.event == "suspend" {
                        owner
                            .begin_draining()
                            .map_err(|_| LifecycleError::Notification)?;
                    }
                    owner
                        .notify_blocking(request.event, request.data, || context.control.stopped())
                        .map_err(|_| LifecycleError::Notification)?;
                    callback_settled = true;
                    if request.event == "suspend" {
                        if context.control.stopped() {
                            return Err(LifecycleError::Stopped);
                        }
                        if !context.publisher.retain_dormant(
                            &context.owner,
                            catalog.clone(),
                            configuration.clone(),
                        ) {
                            return Err(LifecycleError::Busy);
                        }
                        if let Err(error) = context.control.cancel_idle() {
                            context
                                .publisher
                                .forget_dormant(&context.owner, &context.installation);
                            return Err(error);
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
                let budget = pending_check.get_or_insert_with(|| context.control.budget());
                budget.check().map_err(|_| LifecycleError::Authority)?;
                if let Ok(_permit) = context.admission.clone().try_acquire_owned() {
                    context
                        .store
                        .verify_app_start(admission, budget.fork(|| false))?;
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
                        callback_settled = true;
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
