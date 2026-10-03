//! Each bounded native lifecycle thread owns its process, single SDK peer and
//! every preparation lease until actual reaping and broker completion.
use super::*;
use chariox_app_runtime::worker_process::WorkerExit;

pub(super) struct Context {
    pub http_limits: Arc<crate::runtime::app_http::HttpLimits>,
    pub event_config: super::EventConfig,
    pub store: DurableKernelStateStore,
    pub publisher: AppWorkerPublisher,
    pub admission: Arc<Semaphore>,
    pub preparation: Arc<Semaphore>,
    pub owner: String,
    pub installation: String,
    pub attempt: String,
    pub control: Arc<Control>,
    pub runtime: Handle,
    pub kind: StartKind,
    #[cfg(test)]
    pub fixture: Option<start::FixturePlatform>,
    #[cfg(test)]
    pub start_checkpoint: Option<StartObserver>,
    #[cfg(test)]
    pub claim_checkpoint: Option<Arc<dyn Fn() + Send + Sync>>,
}
enum Admitted {
    Restart(ActiveStartAdmission),
    First {
        pending: Arc<ApprovedFirstInstall>,
        active: Option<ActiveStartAdmission>,
    },
}
impl Admitted {
    fn active(&self) -> Option<&ActiveStartAdmission> {
        match self {
            Self::Restart(value) => Some(value),
            Self::First { active, .. } => active.as_ref(),
        }
    }
}
struct Completion(Arc<Control>);
impl Drop for Completion {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(checkpoint) = self.0.completion_checkpoint.lock().unwrap().clone() {
            checkpoint();
        }
        self.0.complete();
    }
}
/// An accepted start waits its turn for one of `slots` instead of refusing
/// Busy. A stop or kernel shutdown wakes and ends the wait.
fn queue(control: &Control, slots: &Arc<Semaphore>) -> Result<OwnedSemaphorePermit> {
    loop {
        if control.stopped() {
            return Err(LifecycleError::Stopped);
        }
        match slots.clone().try_acquire_owned() {
            Ok(permit) => return Ok(permit),
            Err(tokio::sync::TryAcquireError::Closed) => return Err(LifecycleError::Supervisor),
            Err(tokio::sync::TryAcquireError::NoPermits) => control.wait(Duration::from_millis(20)),
        }
    }
}
pub(super) fn run(context: Context, live: OwnedSemaphorePermit) {
    let _completion = Completion(context.control.clone());
    let _live = live;
    // The claim is a writer operation: it holds a shared App slot through the
    // claim and, if it fails, through the deferred stop write below.
    let operation = queue(&context.control, &context.admission);
    let claim_budget = context.control.budget();
    #[cfg(test)]
    let claim_budget = match &context.claim_checkpoint {
        Some(observe) => claim_budget.fixture_observe_checks(observe.clone()),
        None => claim_budget,
    };
    let mut first_authority_withdrawn = false;
    #[cfg(test)]
    if let Some(checkpoint) = &context.start_checkpoint {
        checkpoint(StartCheckpoint::BeforeClaim);
    }
    let claim = match &context.kind {
        _ if operation.is_err() => Err(LifecycleError::Stopped),
        StartKind::Active { recovery } => context
            .store
            .claim_active_app_start(
                &context.owner,
                &context.installation,
                &context.attempt,
                *recovery,
                claim_budget,
            )
            .map(Admitted::Restart)
            .map_err(LifecycleError::from),
        StartKind::First { request_id, .. } => context
            .store
            .claim_first_app_install(&context.owner, request_id, &context.attempt, claim_budget)
            .map(|value| Admitted::First {
                pending: Arc::new(value),
                active: None,
            })
            .map_err(|error| {
                first_authority_withdrawn = error == InstallOperationError::Stale;
                LifecycleError::from(error)
            }),
    };
    let mut admission = match claim {
        Ok(value) => value,
        Err(LifecycleError::CommitUnknown) => {
            let _ = context.store.fence_writer();
            return;
        }
        Err(_) => {
            if first_authority_withdrawn {
                if let StartKind::First { request_id, .. } = &context.kind {
                    // A revoked/re-enrolled signer or changed stage cannot be
                    // retried into authority. Keep a terminal receipt; a new
                    // install request needs fresh verification and approval.
                    let result = context.store.cancel_first_app_install(
                        &context.owner,
                        request_id,
                        AppOperationBudget::from_supervisor(|| false),
                    );
                    if matches!(result, Err(InstallOperationError::CommitUnknown)) {
                        let _ = context.store.fence_writer();
                        return;
                    }
                }
            }
            // The claim failed or was cancelled; no ActiveStartAdmission or
            // native process exists yet. A start stopped while queued for its
            // claim holds no App slot: it writes only if one is free now, and
            // never waits for one. Otherwise its entry keeps the pending stop,
            // which maintenance or shutdown persists under its own slot.
            let _slot = match operation {
                Ok(permit) => permit,
                Err(_) => match context.admission.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => return,
                },
            };
            let _ = manual_stop::persist(
                &context.store,
                &context.owner,
                &context.installation,
                &context.control,
                AppOperationBudget::from_supervisor(|| false),
            );
            return;
        }
    };
    drop(operation);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        serve(&context, &mut admission)
    }));
    if let Ok(Err(LifecycleError::DiskSpace(space))) = &outcome {
        disk_space::notify(
            &context.store,
            &context.owner,
            &context.installation,
            *space,
        );
    }
    let uncertain = matches!(&outcome, Ok(Err(LifecycleError::CommitUnknown)));
    if uncertain {
        // Native owner/peer already dropped and reaped during error unwinding.
        // Do not let an unconfirmed install trigger abort or a fresh admission.
        let _ = context.store.fence_writer();
        return;
    }
    let (phase, failure) = match outcome {
        Ok(Ok(exit)) if context.control.stopped() => (
            WorkerPhase::Stopped,
            exit.failure
                .filter(|failure| {
                    *failure != chariox_app_runtime::worker_process::WorkerError::Cancelled
                })
                .map(|failure| failure.to_string()),
        ),
        Ok(Ok(exit)) => (
            WorkerPhase::Failed,
            Some(
                exit.failure
                    .map(|failure| failure.to_string())
                    .unwrap_or_else(|| "app_worker_exited".into()),
            ),
        ),
        Ok(Err(_)) if context.control.stopped() => (WorkerPhase::Stopped, None),
        Ok(Err(error)) => (WorkerPhase::Failed, Some(error.to_string())),
        Err(_) => (WorkerPhase::Failed, Some("app_lifecycle_supervisor".into())),
    };
    // Cleanup records must still be writable after the stop flag/revocation;
    // exact attempt CAS prevents a late old owner overwriting a replacement.
    if let Some(active) = admission.active() {
        let manual_requested = context.control.manual.load(Ordering::Acquire);
        let recorded = context.store.record_app_worker(
            active,
            phase,
            !manual_requested,
            failure.as_deref(),
            AppOperationBudget::from_supervisor(|| false),
        );
        if recorded.is_ok() && manual_requested {
            context.control.confirm_manual_stop();
        }
    } else if let Admitted::First { pending, .. } = admission {
        let result = context.store.finish_first_app_install(
            pending,
            context.control.stopped(),
            failure.as_deref().unwrap_or("app_install_cancelled"),
            AppOperationBudget::from_supervisor(|| false),
        );
        if matches!(result, Err(InstallOperationError::CommitUnknown)) {
            let _ = context.store.fence_writer();
        }
    }
}
fn serve(context: &Context, admission: &mut Admitted) -> Result<WorkerExit> {
    // Claimed (the row shows Starting); one App prepares at a time. A stop
    // while queued records Stopped like a stop during preparation.
    let preparation = queue(&context.control, &context.preparation)?;
    let operation = queue(&context.control, &context.admission)?;
    let started = match admission {
        Admitted::Restart(active) => start::activate(context, active, preparation, operation)?,
        Admitted::First { pending, active } => {
            let (started, committed) =
                first_install::activate(context, pending.clone(), preparation, operation)?;
            *active = Some(committed);
            started
        }
    };
    let admission = admission.active().ok_or(LifecycleError::Authority)?;
    let owner = started.owner;
    context.control.retain_drain(owner.drain_handle());
    let handle = started.handle;
    let catalog = handle
        .lease(&context.owner)
        .map_err(|_| LifecycleError::Authority)?
        .catalog()
        .clone();
    let mut events = started.events;
    #[cfg(test)]
    let _fixture_release = started.fixture_release;
    let (work, callback_settled) =
        notifications::serve(context, admission, &owner, handle, &mut events, catalog);
    if context.control.stopped()
        && !context.control.idle.load(Ordering::Acquire)
        && callback_settled
        && !owner.is_closed()
    {
        let _ = owner.drain_blocking();
    }
    let exit = owner
        .finish_blocking()
        .map_err(|_| LifecycleError::Supervisor)?;
    work?;
    Ok(exit)
}
