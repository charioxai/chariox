//! Each bounded native lifecycle thread owns its process, single SDK peer and
//! every preparation lease until actual reaping and broker completion.
use super::*;
use chariox_app_runtime::worker_process::WorkerExit;

pub(super) struct Context {
    pub store: DurableKernelStateStore,
    pub publisher: AppWorkerPublisher,
    pub admission: Arc<Semaphore>,
    pub owner: String,
    pub installation: String,
    pub attempt: String,
    pub control: Arc<Control>,
    pub runtime: Handle,
    pub recovery: bool,
    #[cfg(test)]
    pub fixture: Option<start::FixturePlatform>,
    #[cfg(test)]
    pub claim_checkpoint: Option<Arc<dyn Fn() + Send + Sync>>,
}
struct Completion(Arc<Control>);
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.complete();
    }
}
pub(super) fn run(
    context: Context,
    live: OwnedSemaphorePermit,
    preparation: OwnedSemaphorePermit,
    operation: OwnedSemaphorePermit,
) {
    let _completion = Completion(context.control.clone());
    let _live = live;
    let claim_budget = context.control.budget();
    #[cfg(test)]
    let claim_budget = match &context.claim_checkpoint {
        Some(observe) => claim_budget.fixture_observe_checks(observe.clone()),
        None => claim_budget,
    };
    let admission = match context.store.claim_active_app_start(
        &context.owner,
        &context.installation,
        &context.attempt,
        context.recovery,
        claim_budget,
    ) {
        Ok(value) => value,
        Err(_) => {
            // Keep the initial claim's App permit until the requested stop is written.
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
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        serve(&context, &admission, preparation, operation)
    }));
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
    let manual_requested = context.control.manual.load(Ordering::Acquire);
    let recorded = context.store.record_app_worker(
        &admission,
        phase,
        !manual_requested,
        failure.as_deref(),
        AppOperationBudget::from_supervisor(|| false),
    );
    if recorded.is_ok() && manual_requested {
        context.control.confirm_manual_stop();
    }
}
fn serve(
    context: &Context,
    admission: &ActiveStartAdmission,
    preparation: OwnedSemaphorePermit,
    operation: OwnedSemaphorePermit,
) -> Result<WorkerExit> {
    let started = start::activate(context, admission, preparation, operation)?;
    let owner = started.owner;
    context.control.retain_drain(owner.drain_handle());
    let handle = started.handle;
    let mut events = started.events;
    #[cfg(test)]
    let _fixture_release = started.fixture_release;
    let work = (|| {
        if context.control.stopped() {
            return Err(LifecycleError::Stopped);
        }
        owner
            .startup_blocking()
            .map_err(|_| LifecycleError::Startup)?;
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
                    pending_check = None;
                    authority_check = Instant::now() + Duration::from_secs(2);
                }
            }
            context.control.wait(Duration::from_millis(100));
        }
        Ok(())
    })();
    if context.control.stopped() && !owner.is_closed() {
        let _ = owner.drain_blocking();
    }
    let exit = owner
        .finish_blocking()
        .map_err(|_| LifecycleError::Supervisor)?;
    work?;
    Ok(exit)
}
