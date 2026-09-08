//! Restart-time package verification and physical worker preparation. All paths
//! derive from kernel/installer state; no client may choose a native program.
use super::*;
use crate::runtime::app_worker::{ActivatedApp, AppWorkerOwner, RegisteredAppWorker};
use chariox_app_package::{verify, VerificationPolicy, VerifiedPackage};
use chariox_app_runtime::{
    app_catalog::AppCatalog,
    app_outbox::EventCatalog,
    installation::StageTrustBinding,
    publisher_trust::TrustedPublisherSnapshot,
    release_store::{ReleaseStore, VerifiedReleaseLease},
    worker_peer::{ControlEvent, PeerLimits},
    worker_process::{WorkerLimits, WorkerProcess},
};

#[cfg(test)]
#[derive(Clone)]
pub(super) struct FixturePlatform {
    pub(super) native: Arc<chariox_app_runtime::worker_process::test_fixture::Fixture>,
    pub(super) fail_health: bool,
    pub(super) observations:
        Arc<Mutex<Vec<chariox_app_runtime::worker_process::test_fixture::Observation>>>,
}

pub(super) struct Started {
    pub owner: AppWorkerOwner,
    pub handle: ActivatedApp,
    pub events: tokio::sync::mpsc::Receiver<ControlEvent>,
    #[cfg(test)]
    pub fixture_release: Option<VerifiedReleaseLease>,
}
pub(super) struct RegisteredStart {
    pub registered: RegisteredAppWorker,
    pub events: tokio::sync::mpsc::Receiver<ControlEvent>,
    #[cfg(test)]
    pub fixture_release: Option<VerifiedReleaseLease>,
}
struct PreparedProcess {
    process: WorkerProcess,
    #[cfg(test)]
    fixture_release: Option<VerifiedReleaseLease>,
}
pub(super) fn activate(
    context: &owner::Context,
    admission: &ActiveStartAdmission,
    preparation: OwnedSemaphorePermit,
    operation: OwnedSemaphorePermit,
) -> Result<Started> {
    let _preparation = preparation;
    let _operation = operation;
    let budget = context.control.budget();
    let started = register(
        context,
        admission.binding(),
        admission.trust(),
        &budget,
        || {
            context
                .store
                .verify_app_start(admission, budget.fork(|| false))
                .map_err(Into::into)
        },
    )?;
    let mut registered = started.registered;
    let actual_ready_budget = registered
        .take_activation_budget()
        .map_err(|_| LifecycleError::Registration)?;
    let cancellation = context.control.clone();
    let proof = context
        .store
        .confirm_app_activation(
            admission.owner(),
            registered.catalog().clone(),
            actual_ready_budget.fork(move || cancellation.stopped()),
        )
        .map_err(|_| LifecycleError::Authority)?;
    let (owner, handle) = registered
        .activate_blocking(proof)
        .map_err(|_| LifecycleError::Registration)?;
    Ok(Started {
        owner,
        handle,
        events: started.events,
        #[cfg(test)]
        fixture_release: started.fixture_release,
    })
}
pub(super) fn register(
    context: &owner::Context,
    binding: &StageTrustBinding,
    trust: &TrustedPublisherSnapshot,
    budget: &AppOperationBudget,
    mut check: impl FnMut() -> Result<()>,
) -> Result<RegisteredStart> {
    check()?;
    let releases = ReleaseStore::open_or_create(context.store.path())
        .map_err(|_| LifecycleError::Preparation)?;
    let mut stored = releases
        .open_stored_archive(binding.package_digest())
        .map_err(|_| LifecycleError::Preparation)?;
    let bytes = stored
        .read_bytes()
        .map_err(|_| LifecycleError::Preparation)?;
    budget.check().map_err(|_| LifecycleError::Stopped)?;
    let verified = verify(
        &bytes,
        &VerificationPolicy::new(
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            vec![trust.publisher().clone()],
        ),
    )
    .map_err(|_| LifecycleError::Preparation)?;
    let catalog = Arc::new(
        AppCatalog::compile(&verified, binding, trust).map_err(|_| LifecycleError::Authority)?,
    );
    let events = Arc::new(
        EventCatalog::compile(&verified, catalog).map_err(|_| LifecycleError::Preparation)?,
    );
    let release = releases
        .lease_verified(&verified, &bytes)
        .map_err(|_| LifecycleError::Preparation)?;
    check()?;
    let prepared = spawn(context, binding, &verified, release)?;
    let process = prepared.process;
    if context.control.stopped() {
        return Err(LifecycleError::Stopped);
    }
    let delegate = crate::runtime::app_backend_broker::broker(
        context.store.clone(),
        binding.owner_id().into(),
        events.clone(),
        context.admission.clone(),
        &process,
    )
    .map_err(|_| LifecycleError::Preparation)?;
    let (starting, controls) = AppWorkerOwner::start_blocking(
        process,
        &verified,
        events,
        delegate,
        PeerLimits::default(),
        context.runtime.clone(),
    )
    .map_err(|_| LifecycleError::Registration)?;
    let registered = starting
        .await_registered_blocking(Duration::from_secs(15))
        .map_err(|_| LifecycleError::Registration)?;
    if context.control.stopped() {
        return Err(LifecycleError::Stopped);
    }
    Ok(RegisteredStart {
        registered,
        events: controls,
        #[cfg(test)]
        fixture_release: prepared.fixture_release,
    })
}

fn spawn(
    context: &owner::Context,
    binding: &StageTrustBinding,
    verified: &VerifiedPackage<'_>,
    release: VerifiedReleaseLease,
) -> Result<PreparedProcess> {
    #[cfg(test)]
    if let Some(fixture) = &context.fixture {
        use chariox_app_runtime::worker_process::test_fixture::Mode;
        let mode = match context.kind {
            StartKind::First { .. } if fixture.fail_health => Mode::BadHealth,
            StartKind::First { .. } => Mode::Health,
            StartKind::Active { .. } => Mode::Ready,
        };
        let (process, observation) = fixture
            .native
            .spawn_for_installation_blocking(mode, verified, &binding.token().installation_id)
            .map_err(|_| LifecycleError::Preparation)?;
        fixture
            .observations
            .lock()
            .map_err(|_| LifecycleError::Supervisor)?
            .push(observation);
        // Fixture bytes are fixed libc, never package code. Keep the verified
        // release lease separately through this test owner's actual reap/drain.
        return Ok(PreparedProcess {
            process,
            fixture_release: Some(release),
        });
    }
    #[cfg(target_os = "linux")]
    {
        let _ = (context, verified);
        let runtime = chariox_app_runtime::runtime_enrollment::EnrolledRuntime::open_installed()
            .map_err(|_| LifecycleError::Preparation)?;
        let prepared = chariox_app_runtime::worker_process::PreparedWorker::prepare_linux(
            runtime, release, binding,
        )
        .map_err(|_| LifecycleError::Preparation)?;
        if context.control.stopped() {
            return Err(LifecycleError::Stopped);
        }
        let process = WorkerProcess::spawn_blocking(prepared, WorkerLimits::default())
            .map_err(|_| LifecycleError::Preparation)?;
        Ok(PreparedProcess {
            process,
            #[cfg(test)]
            fixture_release: None,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (context, binding, verified, release);
        Err(LifecycleError::Preparation)
    }
}
