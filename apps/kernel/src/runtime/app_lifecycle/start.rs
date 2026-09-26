//! Restart-time package verification and physical worker preparation. All paths
//! derive from kernel/installer state; no client may choose a native program.
use super::*;
use crate::durable_state::app_active_release::{ActiveRelease, ActiveReleaseError};
use crate::runtime::app_worker::{ActivatedApp, AppWorkerOwner, RegisteredAppWorker};
use chariox_app_package::VerifiedPackage;
use chariox_app_runtime::{
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
    pub(super) fail_migration: bool,
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
    let active = ActiveRelease::load(context.store.path(), binding.clone(), trust.clone())
        .map_err(|_| LifecycleError::Preparation)?;
    budget.check().map_err(|_| LifecycleError::Stopped)?;
    let verified = active.verify().map_err(|_| LifecycleError::Preparation)?;
    let events = active
        .event_catalog(&verified)
        .map_err(|error| match error {
            ActiveReleaseError::Untrusted => LifecycleError::Authority,
            _ => LifecycleError::Preparation,
        })?;
    let release = ReleaseStore::open_or_create(context.store.path())
        .and_then(|releases| releases.lease_verified(&verified, active.bytes()))
        .map_err(|_| LifecycleError::Preparation)?;
    check()?;
    // A staged worker whose update opened a migration runs its steps first.
    let migrate_from = context
        .store
        .app_migration_from(&binding.token().installation_id, binding.token().generation)
        .map_err(|_| LifecycleError::Preparation)?;
    let prepared = spawn(context, binding, &verified, release, migrate_from)?;
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
        &verified,
        crate::runtime::app_http::HttpContext {
            limits: context.http_limits.clone(),
            runtime: context.runtime.clone(),
        },
    )
    .map_err(|_| LifecycleError::Preparation)?;
    let started = if migrate_from.is_some() {
        AppWorkerOwner::start_migrating_blocking(
            process,
            &verified,
            events,
            delegate,
            PeerLimits::default(),
            context.runtime.clone(),
        )
    } else {
        AppWorkerOwner::start_blocking(
            process,
            &verified,
            events,
            delegate,
            PeerLimits::default(),
            context.runtime.clone(),
        )
    };
    let (starting, controls) = started.map_err(|_| LifecycleError::Registration)?;
    let registration = Duration::from_secs(15)
        + migrate_from.map_or(Duration::ZERO, |_| {
            Duration::from_millis(chariox_app_runtime::worker_process::MIGRATION_TIMEOUT_MS)
        });
    let registered = starting
        .await_registered_blocking(registration)
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
    migrate_from: Option<u32>,
) -> Result<PreparedProcess> {
    #[cfg(test)]
    if let Some(fixture) = &context.fixture {
        use chariox_app_runtime::worker_process::test_fixture::Mode;
        let mode = match context.kind {
            StartKind::First { .. } if migrate_from.is_some() && fixture.fail_migration => {
                Mode::BadMigration
            }
            StartKind::First { .. } if migrate_from.is_some() => Mode::Migrate,
            StartKind::First { .. } if fixture.fail_health => Mode::BadHealth,
            StartKind::First { .. } => Mode::Health,
            StartKind::Active { .. } => Mode::Ready,
        };
        let (process, observation) = fixture
            .native
            .spawn_for_generation_blocking(
                mode,
                verified,
                &binding.token().installation_id,
                binding.token().generation,
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
            binding,
            migrate_from,
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
    #[cfg(target_os = "macos")]
    {
        let _ = verified;
        // A refused preparation (no enrolled runtime, low disk, ...) keeps its
        // stable code visible; it names no App path or package content.
        let failed = |step: &str, code: String| {
            crate::logging::warn_with_fields(
                "app.worker",
                "App worker preparation failed",
                serde_json::json!({
                    "installation_id": binding.token().installation_id,
                    "generation": binding.token().generation,
                    "step": step,
                    "code": code,
                }),
            );
            LifecycleError::Preparation
        };
        let runtime = chariox_app_runtime::runtime_enrollment::EnrolledRuntime::open_installed()
            .map_err(|error| failed("enrolled_runtime", error.to_string()))?;
        let storage_root = macos_storage_root(&context.store)?;
        // After a failed update the committed generation starts again on
        // storage its uncommitted successor last prepared.
        let committed = context
            .store
            .get_app_installation(binding.owner_id(), &binding.token().installation_id)
            .map(|installation| installation.generation)
            .map_err(|_| failed("installation", "app_installation_unavailable".into()))?;
        let prepared = chariox_app_runtime::worker_process::PreparedWorker::prepare_macos(
            runtime,
            release,
            binding,
            &storage_root,
            committed,
        )
        .map_err(|error| failed("prepare", error.to_string()))?;
        if context.control.stopped() {
            return Err(LifecycleError::Stopped);
        }
        let process = WorkerProcess::spawn_blocking(prepared, WorkerLimits::default())
            .map_err(|error| failed("spawn", error.to_string()))?;
        Ok(PreparedProcess {
            process,
            #[cfg(test)]
            fixture_release: None,
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (context, binding, verified, release, migrate_from);
        Err(LifecycleError::Preparation)
    }
}

/// Kernel-owned private APFS image root beside the kernel database. It is
/// never derived from an App, client or package value.
#[cfg(target_os = "macos")]
fn macos_storage_root(
    store: &crate::durable_state::DurableKernelStateStore,
) -> Result<std::path::PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let root = store
        .path()
        .parent()
        .ok_or(LifecycleError::Preparation)?
        .join("app-storage");
    match std::fs::DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(LifecycleError::Preparation),
    }
    // Recover once per kernel storage root before its first preparation, while
    // no worker of this kernel can hold a volume. Failure retries next start.
    static RECOVERED: std::sync::Mutex<std::collections::BTreeSet<std::path::PathBuf>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    let mut recovered = RECOVERED.lock().map_err(|_| LifecycleError::Supervisor)?;
    if !recovered.contains(&root) {
        chariox_app_runtime::worker_process::PreparedWorker::recover_macos_storage(&root).map_err(
            |code| {
                // Fail closed, but keep the cause visible (no App-supplied paths).
                crate::logging::warn_with_fields(
                    "app.lifecycle",
                    "App storage recovery failed; macOS App starts are paused",
                    serde_json::json!({ "code": code }),
                );
                LifecycleError::Preparation
            },
        )?;
        recovered.insert(root.clone());
    }
    Ok(root)
}
