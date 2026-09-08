//! The actual process FD creates the sole peer and its readiness broker. The
//! report cannot be replayed from a separately supplied peer or registration.

use super::*;
use crate::durable_state::app_activation::CommittedAppActivation;
use crate::runtime::app_operation_budget::AppOperationBudget;
use chariox_app_package::VerifiedPackage;
use chariox_app_runtime::{
    wire::RemoteError,
    worker_peer::{Broker, BrokerFuture, BrokerRequest, ControlEvent, PeerLimits},
    worker_readiness::ReadinessContract,
};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

mod health;
pub(crate) use health::{FirstInstallHealth, HealthyAppWorker};

/// Provisional generation. Only this owner can consume its channel's report.
pub(crate) struct StartingAppWorker {
    owner: AppWorkerOwner,
    registration: oneshot::Receiver<Result<ReadyReport, AppWorkerError>>,
}

/// Readiness precedes the existing durable activation commit; SDK effects remain
/// blocked until activate_blocking consumes that commit's exact private proof.
pub(crate) struct RegisteredAppWorker {
    owner: AppWorkerOwner,
    registration: RegisteredHandlers,
    activation_budget: Option<AppOperationBudget>,
}

struct ReadyReport {
    registration: RegisteredHandlers,
    budget: AppOperationBudget,
}

impl AppWorkerOwner {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_blocking(
        mut process: WorkerProcess,
        package: &VerifiedPackage<'_>,
        catalog: Arc<EventCatalog>,
        delegate: Arc<dyn Broker>,
        limits: PeerLimits,
        runtime: Handle,
    ) -> Result<(StartingAppWorker, mpsc::Receiver<ControlEvent>), AppWorkerError> {
        if process.installation_id() != catalog.installation_id()
            || process.release_digest() != catalog.app_catalog().package_digest()
            || process.release_digest() != package.package_digest()
        {
            return Err(AppWorkerError::Identity);
        }
        let contract = ReadinessContract::from_verified(package, catalog.app_catalog().clone())
            .map_err(|_| AppWorkerError::Identity)?;
        let (changed, _) = watch::channel(Phase::Starting);
        let admission = Arc::new(Admission {
            phase: Mutex::new(Phase::Starting),
            changed,
            cancellation: process.cancellation(),
            broker: Arc::downgrade(&delegate),
            broker_draining: std::sync::atomic::AtomicBool::new(false),
        });
        let (sender, registration) = oneshot::channel();
        let broker = Arc::new(StartupBroker {
            admission: admission.clone(),
            delegate,
            contract: Mutex::new(Some(contract)),
            report: Mutex::new(Some(sender)),
        });
        let (peer, events, peer_task) = {
            let _entered = runtime.enter();
            let channel = process
                .take_sdk_channel()
                .map_err(|_| AppWorkerError::Unavailable)?;
            // Generation is taken from the exact native process's inherited FD.
            if channel.generation() != catalog.generation().to_string() {
                return Err(AppWorkerError::Identity);
            }
            WorkerPeer::start(channel, broker, limits).map_err(|_| AppWorkerError::Unavailable)?
        };
        let owner = AppWorkerOwner {
            process: Some(process),
            peer,
            peer_task: Some(peer_task),
            admission,
            catalog,
            live: None,
            registration: None,
            runtime,
        };
        Ok((
            StartingAppWorker {
                owner,
                registration,
            },
            events,
        ))
    }
}

impl StartingAppWorker {
    pub(crate) fn await_registered_blocking(
        self,
        timeout: Duration,
    ) -> Result<RegisteredAppWorker, AppWorkerError> {
        if timeout.is_zero() || timeout > Duration::from_secs(15) {
            return Err(AppWorkerError::Invalid);
        }
        let Self {
            owner,
            registration,
        } = self;
        let report = owner.runtime.block_on(async {
            tokio::select! {
                biased;
                report = registration => report.map_err(|_| AppWorkerError::Unavailable)?,
                _ = owner.peer.closed() => Err(AppWorkerError::Unavailable),
                _ = tokio::time::sleep(timeout) => Err(AppWorkerError::Deadline),
            }
        })?;
        Ok(RegisteredAppWorker {
            owner,
            registration: report.registration,
            activation_budget: Some(report.budget),
        })
    }
}

impl RegisteredAppWorker {
    pub(crate) fn catalog(&self) -> &Arc<EventCatalog> {
        &self.owner.catalog
    }

    /// The activation writer uses this actual ready request's monotonic deadline
    /// and cancellation, including time already spent waiting for registration.
    pub(crate) fn take_activation_budget(&mut self) -> Result<AppOperationBudget, AppWorkerError> {
        self.activation_budget
            .take()
            .ok_or(AppWorkerError::Unavailable)
    }

    pub(crate) fn activate_blocking(
        mut self,
        committed: CommittedAppActivation,
    ) -> Result<(AppWorkerOwner, ActivatedApp), AppWorkerError> {
        let catalog = self.owner.catalog.app_catalog();
        if self.activation_budget.is_some()
            || !Arc::ptr_eq(self.registration.catalog(), catalog)
            || committed.installation_id() != catalog.installation_id()
            || committed.generation() != catalog.generation()
            || committed.package_digest() != catalog.package_digest()
            || committed.catalog_digest() != catalog.catalog_digest()
            || self.owner.peer.is_closed()
        {
            return Err(AppWorkerError::Identity);
        }
        let live = Arc::new(LiveWorker {
            owner: committed.owner().into(),
            catalog: self.owner.catalog.clone(),
            peer: self.owner.peer.clone(),
            admission: self.owner.admission.clone(),
        });
        let handle = ActivatedApp(Arc::downgrade(&live));
        {
            let mut phase = self
                .owner
                .admission
                .phase
                .lock()
                .map_err(|_| AppWorkerError::Unavailable)?;
            if *phase != Phase::Starting {
                return Err(AppWorkerError::Unavailable);
            }
            self.owner.live = Some(live);
            self.owner.registration = Some(self.registration);
            *phase = Phase::Active;
            self.owner.admission.changed.send_replace(Phase::Active);
        }
        Ok((self.owner, handle))
    }
}

struct StartupBroker {
    admission: Arc<Admission>,
    delegate: Arc<dyn Broker>,
    contract: Mutex<Option<ReadinessContract>>,
    report: Mutex<Option<oneshot::Sender<Result<ReadyReport, AppWorkerError>>>>,
}
impl Broker for StartupBroker {
    fn begin_draining(&self) {
        // A peer closing before the owner observes it also withdraws this exact
        // worker immediately. Admission forwards the delegate hook only once.
        self.admission.stop();
    }
    fn drain(&self) -> chariox_app_runtime::worker_peer::BrokerDrainFuture {
        self.delegate.drain()
    }
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        if request.method != "worker.ready" {
            if self.admission.broker_open(&request.method) {
                return self.delegate.handle(request);
            }
            return Box::pin(async { Err(remote("APP_NOT_READY")) });
        }
        let budget = AppOperationBudget::from_broker(&request);
        let report = self.report.lock().ok().and_then(|mut report| report.take());
        let registration = self
            .contract
            .lock()
            .ok()
            .and_then(|mut contract| contract.take())
            .ok_or(AppWorkerError::Invalid)
            .and_then(|mut contract| {
                contract
                    .accept(request.params)
                    .map_err(|_| AppWorkerError::Invalid)
            });
        let admission = self.admission.clone();
        Box::pin(async move {
            let Some(report) = report else {
                admission.stop();
                return Err(remote("APP_READY_INVALID"));
            };
            if request.cancellation.is_cancelled()
                || tokio::time::Instant::now() >= request.deadline
            {
                admission.stop();
                let _ = report.send(Err(AppWorkerError::Deadline));
                return Err(remote("APP_READY_EXPIRED"));
            }
            match registration {
                Ok(registration) => {
                    if report
                        .send(Ok(ReadyReport {
                            registration,
                            budget,
                        }))
                        .is_err()
                    {
                        admission.stop();
                        return Err(remote("APP_NOT_READY"));
                    }
                }
                Err(error) => {
                    admission.stop();
                    let _ = report.send(Err(error));
                    return Err(remote("APP_READY_INVALID"));
                }
            }
            let mut changed = admission.changed.subscribe();
            let mut cancellation = request.cancellation;
            loop {
                if cancellation.is_cancelled() || tokio::time::Instant::now() >= request.deadline {
                    admission.stop();
                    return Err(remote("APP_READY_EXPIRED"));
                }
                match *changed.borrow_and_update() {
                    Phase::Active | Phase::Draining => return Ok(Value::Null),
                    Phase::Stopped => return Err(remote("APP_NOT_READY")),
                    Phase::Starting => {}
                }
                tokio::select! {
                    _ = cancellation.cancelled() => { admission.stop(); return Err(remote("APP_READY_EXPIRED")); },
                    _ = tokio::time::sleep_until(request.deadline) => { admission.stop(); return Err(remote("APP_READY_EXPIRED")); },
                    result = changed.changed() => if result.is_err() { return Err(remote("APP_NOT_READY")); },
                }
            }
        })
    }
}
fn remote(code: &str) -> RemoteError {
    RemoteError {
        code: code.into(),
        message: "App worker is not available".into(),
        retryable: Some(false),
    }
}
