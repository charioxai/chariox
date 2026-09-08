//! Restart-time package verification and physical worker preparation. All paths
//! derive from kernel/installer state; no client may choose a native program.
use super::*;
use crate::runtime::app_worker::{ActivatedApp, AppWorkerOwner};
use chariox_app_package::{verify, VerificationPolicy, VerifiedPackage};
use chariox_app_runtime::{
    app_catalog::AppCatalog,
    app_outbox::EventCatalog,
    release_store::{ReleaseStore, VerifiedReleaseLease},
    worker_peer::{ControlEvent, PeerLimits},
    worker_process::{WorkerLimits, WorkerProcess},
};

#[cfg(test)]
#[derive(Clone)]
pub(super) struct FixturePlatform {
    pub(super) native: Arc<chariox_app_runtime::worker_process::test_fixture::Fixture>,
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
    context
        .store
        .verify_app_start(admission, budget.fork(|| false))?;
    let releases = ReleaseStore::open_or_create(context.store.path())
        .map_err(|_| LifecycleError::Preparation)?;
    let mut stored = releases
        .open_stored_archive(admission.binding().package_digest())
        .map_err(|_| LifecycleError::Preparation)?;
    let bytes = stored
        .read_bytes()
        .map_err(|_| LifecycleError::Preparation)?;
    budget.check().map_err(|_| LifecycleError::Stopped)?;
    let verified = verify(
        &bytes,
        &VerificationPolicy::new(
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            vec![admission.trust().publisher().clone()],
        ),
    )
    .map_err(|_| LifecycleError::Preparation)?;
    let catalog = Arc::new(
        AppCatalog::compile(&verified, admission.binding(), admission.trust())
            .map_err(|_| LifecycleError::Authority)?,
    );
    let events = Arc::new(
        EventCatalog::compile(&verified, catalog).map_err(|_| LifecycleError::Preparation)?,
    );
    let release = releases
        .lease_verified(&verified, &bytes)
        .map_err(|_| LifecycleError::Preparation)?;
    context
        .store
        .verify_app_start(admission, budget.fork(|| false))?;
    let prepared = spawn(context, admission, &verified, release)?;
    let process = prepared.process;
    if context.control.stopped() {
        return Err(LifecycleError::Stopped);
    }
    let delegate = crate::runtime::app_backend_broker::broker(
        context.store.clone(),
        admission.owner().into(),
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
    let mut registered = starting
        .await_registered_blocking(Duration::from_secs(15))
        .map_err(|_| LifecycleError::Registration)?;
    if context.control.stopped() {
        return Err(LifecycleError::Stopped);
    }
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
        events: controls,
        #[cfg(test)]
        fixture_release: prepared.fixture_release,
    })
}
fn spawn(
    context: &owner::Context,
    admission: &ActiveStartAdmission,
    verified: &VerifiedPackage<'_>,
    release: VerifiedReleaseLease,
) -> Result<PreparedProcess> {
    #[cfg(test)]
    if let Some(fixture) = &context.fixture {
        let (process, observation) = fixture
            .native
            .spawn_blocking(
                chariox_app_runtime::worker_process::test_fixture::Mode::Ready,
                verified,
            )
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
            runtime,
            release,
            admission.binding(),
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
        let _ = (context, admission, verified, release);
        Err(LifecycleError::Preparation)
    }
}
