//! Typed work admitted by the one durable writer; no arbitrary App callbacks.
use super::*;
use crate::runtime::app_http::{decode::Command, encode, policy::AppHttpPolicy};
use chariox_app_runtime::worker_peer::{BrokerCancellation, ResponsePublication};
use serde_json::Value;
use std::sync::atomic::Ordering;
use tokio::sync::{oneshot, OwnedSemaphorePermit};

pub(crate) struct HttpJob {
    group: HttpStreams,
    operation: Operation,
    budget: AppOperationBudget,
    deadline: Instant,
    cancellation: BrokerCancellation,
    permit: OwnedSemaphorePermit,
    response: oneshot::Sender<Result<HttpReply>>,
}
pub(crate) struct HttpReply {
    value: Value,
    cleanup: Option<Cleanup>,
}
struct Cleanup {
    group: Weak<Inner>,
    id: String,
    stop: Option<watch::Sender<bool>>,
    gate: Option<PortGate>,
    permit: Option<OwnedSemaphorePermit>,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(stop) = &self.stop {
            retire(&self.group, &self.id, stop);
        }
    }
}
impl ResponsePublication for Cleanup {
    fn published(&mut self) {
        self.stop = None;
        self.permit.take();
        self.gate.take();
    }
}
impl HttpReply {
    pub(crate) fn into_publication(self) -> (Value, Option<Box<dyn ResponsePublication>>) {
        (
            self.value,
            self.cleanup
                .map(|cleanup| Box::new(cleanup) as Box<dyn ResponsePublication>),
        )
    }
}
struct PortGate(Arc<OperationGate>);
impl PortGate {
    fn acquire(flag: Arc<OperationGate>) -> Result<Self> {
        flag.phase
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| HttpError::Busy)?;
        Ok(Self(flag))
    }
    fn publishing(&self) {
        self.0.phase.store(2, Ordering::Release);
    }
}
impl Drop for PortGate {
    fn drop(&mut self) {
        self.0.phase.store(0, Ordering::Release);
        self.0.wake.notify_waiters();
    }
}
enum Operation {
    Open(PendingStart, PortGate),
    Port(StreamPort, PortOperation, PortGate),
}
enum PortOperation {
    Write(bytes::Bytes, bool),
    Headers,
    Read,
}
impl std::fmt::Debug for HttpJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppHttpJob")
            .field(
                "installation_id",
                &self.group.0.scope.catalog.installation_id(),
            )
            .field("generation", &self.group.0.scope.catalog.generation())
            .finish_non_exhaustive()
    }
}
impl HttpStreams {
    /// Called on bounded blocking admission, since opening reads host resolver
    /// configuration. No DNS query/socket starts before the writer fence.
    pub(in crate::runtime::app_http) fn job(
        &self,
        policy: &AppHttpPolicy,
        command: Command,
        budget: AppOperationBudget,
        deadline: Instant,
        cancellation: BrokerCancellation,
        permit: OwnedSemaphorePermit,
    ) -> Result<(HttpJob, oneshot::Receiver<Result<HttpReply>>)> {
        budget.check().map_err(|_| HttpError::Cancelled)?;
        let is_headers = matches!(&command, Command::Headers(_));
        let operation = match command {
            Command::Open(open) => {
                let pending = self.prepare(
                    policy.anonymous_target(
                        &open.url,
                        &open.method,
                        &open.headers,
                        open.connection.as_deref(),
                        open.operation.as_deref(),
                    )?,
                    open.has_body,
                )?;
                let gate = PortGate::acquire(pending.entry.opening.clone())?;
                Operation::Open(pending, gate)
            }
            Command::Write { id, bytes, end } => {
                let port = self.port(&id)?;
                let gate = PortGate::acquire(port.entry.writing.clone())?;
                Operation::Port(port, PortOperation::Write(bytes, end), gate)
            }
            Command::Headers(id) | Command::Read(id) => {
                let port = self.port(&id)?;
                let gate = PortGate::acquire(port.entry.reading.clone())?;
                Operation::Port(
                    port,
                    if is_headers {
                        PortOperation::Headers
                    } else {
                        PortOperation::Read
                    },
                    gate,
                )
            }
            Command::Cancel(_) => return Err(HttpError::Invalid),
        };
        let (response, receiver) = oneshot::channel();
        Ok((
            HttpJob {
                group: self.clone(),
                operation,
                budget,
                deadline,
                cancellation,
                permit,
                response,
            },
            receiver,
        ))
    }
}
impl HttpJob {
    pub(crate) fn reject(self, error: HttpError) {
        if let Operation::Port(port, _, _) = &self.operation {
            // A stale grant cannot keep transmitting an already-open stream.
            if !matches!(error, HttpError::Busy) {
                port.cancel();
            }
        }
        let _ = self.response.send(Err(error));
    }
    pub(crate) fn submit(self, transaction: &rusqlite::Transaction<'_>) {
        let admitted = self
            .budget
            .check()
            .map_err(|_| HttpError::Cancelled)
            .and_then(|()| {
                self.group
                    .0
                    .scope
                    .catalog
                    .require_current(transaction, &self.group.0.scope.owner)
                    .map_err(|_| HttpError::Provenance)
            });
        if let Err(error) = admitted {
            self.reject(error);
            return;
        }
        let Self {
            group,
            operation,
            budget,
            deadline,
            cancellation,
            permit,
            response,
        } = self;
        match operation {
            Operation::Open(pending, gate) => {
                // The first task poll checks the original request budget before
                // acknowledging a new stream. Normal request completion may
                // close that cancellation watch, so it cannot govern the later
                // lifetime of an already acknowledged streaming operation.
                let id = pending.id.clone();
                let cleanup = Cleanup {
                    group: Arc::downgrade(&group.0),
                    id: id.clone(),
                    stop: Some(pending.entry.stop.clone()),
                    gate: Some(gate),
                    permit: Some(permit),
                };
                let reply = Arc::new(Mutex::new(Some(response)));
                let task_reply = reply.clone();
                let execution = budget.fork(|| false);
                #[cfg(test)]
                let network = group.0.fixture.lock().unwrap().clone();
                let outcome = pending.start_with(
                    transaction,
                    &budget,
                    move |transport, target, exchange, stopped, lease, admitted| async move {
                        let ready = execution.check().map_err(|_| HttpError::Cancelled);
                        let sender = task_reply
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        let Some(sender) = sender else {
                            return Err(HttpError::Cancelled);
                        };
                        if let Some(gate) = &cleanup.gate {
                            gate.publishing();
                        }
                        let value = ready.map(|()| HttpReply {
                            value: encode::open(&id),
                            cleanup: Some(cleanup),
                        });
                        if sender.send(value).is_err() {
                            return Err(HttpError::Cancelled);
                        }
                        ready?;
                        #[cfg(test)]
                        if let Some(network) = network {
                            let _lease = lease;
                            return network.run(exchange, stopped).await;
                        }
                        transport
                            .run(target, exchange, stopped, lease, admitted)
                            .await
                    },
                );
                if let Err(error) = outcome {
                    if let Some(sender) = reply
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                    {
                        let _ = sender.send(Err(error));
                    }
                }
            }
            Operation::Port(port, operation, gate) => {
                let mut tasks = match group.0.tasks.try_lock() {
                    Ok(tasks) => tasks,
                    Err(_) => {
                        let _ = response.send(Err(HttpError::Busy));
                        return;
                    }
                };
                while tasks.try_join_next().is_some() {}
                let current = group
                    .0
                    .state
                    .lock()
                    .map(|state| {
                        !state.closed
                            && state
                                .entries
                                .get(&port.id)
                                .is_some_and(|entry| Arc::ptr_eq(entry, &port.entry))
                    })
                    .unwrap_or(false);
                if !current || budget.check().is_err() || response.is_closed() {
                    port.cancel();
                    let _ = response.send(Err(HttpError::Cancelled));
                    return;
                }
                // No strong group is captured: its JoinSet cannot own itself.
                // Entry+permit survive cancelled callers until actual I/O ends.
                let mut delivery = Delivery {
                    port,
                    completed: false,
                };
                tasks.spawn_on(
                    async move {
                        let result = if budget.check().is_err() {
                            Err(HttpError::Cancelled)
                        } else {
                            execute_port(&delivery.port, operation, deadline, cancellation).await
                        };
                        // Busy/pending are safely completed operations. A missing
                        // receiver after consuming/enqueueing bytes retires the
                        // handle; neither side automatically replays body chunks.
                        if result
                            .as_ref()
                            .is_err_and(|error| *error != HttpError::Busy)
                        {
                            delivery.port.cancel();
                        }
                        gate.publishing();
                        let result = result.map(|value| HttpReply {
                            value,
                            cleanup: Some(Cleanup {
                                group: delivery.port.group.clone(),
                                id: delivery.port.id.clone(),
                                stop: Some(delivery.port.entry.stop.clone()),
                                gate: Some(gate),
                                permit: Some(permit),
                            }),
                        });
                        delivery.completed = response.send(result).is_ok();
                    },
                    &group.0.runtime,
                );
            }
        }
    }
}
struct Delivery {
    port: StreamPort,
    completed: bool,
}
impl Drop for Delivery {
    fn drop(&mut self) {
        if !self.completed {
            self.port.cancel();
        }
    }
}
async fn execute_port(
    port: &StreamPort,
    operation: PortOperation,
    deadline: Instant,
    cancellation: BrokerCancellation,
) -> Result<Value> {
    let pull = (Instant::now() + std::time::Duration::from_secs(1)).min(deadline);
    match operation {
        PortOperation::Write(bytes, end) => {
            let written = bytes.len();
            port.write(bytes, end, deadline, cancellation).await?;
            Ok(encode::written(written))
        }
        PortOperation::Headers => Ok(encode::headers(port.headers(pull, cancellation).await?)),
        PortOperation::Read => Ok(encode::read(port.read(pull, cancellation).await?)),
    }
}
