//! Internal entry points until the shared authenticated terminal route lands.
use super::*;
type Result<T> = std::result::Result<T, PublisherOperationError>;
enum Request {
    Begin(PublisherEnrollmentInput),
    Status,
    Cancel,
}
impl AppPublisherControl {
    pub(crate) async fn begin(
        &self,
        runtime: &KernelRuntimeState,
        owner: &str,
        request: &str,
        input: PublisherEnrollmentInput,
    ) -> Result<PublisherOperation> {
        if !runtime.app_install_session_member(&input.session_id, owner) {
            return Err(PublisherOperationError::Invalid);
        }
        self.submit((owner.into(), request.into()), Request::Begin(input))
            .await
    }
    pub(crate) async fn status(&self, owner: &str, request: &str) -> Result<PublisherOperation> {
        self.submit((owner.into(), request.into()), Request::Status)
            .await
    }
    pub(crate) async fn cancel(&self, owner: &str, request: &str) -> Result<PublisherOperation> {
        let key = (owner.into(), request.into());
        self.cancel_admission(&key);
        self.submit(key, Request::Cancel).await
    }
    async fn submit(&self, key: Key, request: Request) -> Result<PublisherOperation> {
        for value in [&key.0, &key.1] {
            if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
                return Err(PublisherOperationError::Invalid);
            }
        }
        let (response, receive) = oneshot::channel();
        {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.0.stopped.load(Ordering::Acquire) {
                return Err(PublisherOperationError::Stopped);
            }
            // Reap completed request tasks before applying capacity. They never
            // own an entry; durable recovery covers a caller losing its ACK.
            while state.requests.try_join_next().is_some() {}
            if state.requests.len() + state.jobs.len() >= JOBS {
                return Err(PublisherOperationError::Limit);
            }
            let permit = self
                .0
                .shared
                .admission
                .clone()
                .try_acquire_owned()
                .map_err(|_| PublisherOperationError::Limit)?;
            let store = self.0.shared.store.clone();
            let stopped = self.0.stopped.clone();
            let run_key = key.clone();
            let negative = matches!(&request, Request::Cancel);
            // The deadline starts at admission, before either task queue. An
            // explicit negative write survives shutdown while its owner joins.
            let budget = AppOperationBudget::from_supervisor(move || {
                !negative && stopped.load(Ordering::Acquire)
            });
            #[cfg(test)]
            let budget = match self.0.request_checkpoint.lock().unwrap().clone() {
                Some(observe) => budget.fixture_observe_checks(observe),
                None => budget,
            };
            state.requests.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    match request {
                        Request::Begin(input) => {
                            store.begin_publisher_enrollment(&run_key.0, &run_key.1, input, budget)
                        }
                        Request::Status => {
                            store.publisher_enrollment_status(&run_key.0, &run_key.1, budget)
                        }
                        Request::Cancel => {
                            store.cancel_publisher_enrollment(&run_key.0, &run_key.1, budget)
                        }
                    }
                })
                .await
                .unwrap_or(Err(PublisherOperationError::CommitUnknown));
                let _ = response.send(result);
            });
        }
        let operation = receive
            .await
            .map_err(|_| PublisherOperationError::CommitUnknown)??;
        if operation.phase == PublisherOperationPhase::Pending {
            self.notify(key, false)
        } else if operation.phase == PublisherOperationPhase::Cancelled {
            self.notify(key, true)
        }
        Ok(operation)
    }
}
