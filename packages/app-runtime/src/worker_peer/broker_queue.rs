use super::*;

pub(super) struct Waiting {
    pub(super) request: BrokerRequest,
    cancel: watch::Sender<bool>,
}
impl Actor {
    pub(super) fn broker_request(
        &mut self,
        id: String,
        method: String,
        params: serde_json::Value,
        deadline_ms: u64,
    ) -> Result<()> {
        self.publishing.retain(|_, pending| {
            !(pending.finished.load(Ordering::Acquire)
                && pending.physically_published.load(Ordering::Relaxed))
        });
        if self.active.contains_key(&id)
            || self.publishing.contains_key(&id)
            || self
                .broker_waiting
                .iter()
                .any(|entry| entry.request.id == id)
        {
            return Err(PeerError::Protocol);
        }
        let remaining = deadline_ms.saturating_sub(wall_ms()?);
        if remaining == 0 {
            return self.failure(id, "DEADLINE_EXCEEDED", "App broker deadline exceeded");
        }
        let running = self.active.len() + self.publishing.len();
        if running >= self.limits.broker_handlers
            && self.broker_waiting.len() >= self.limits.queued_broker_requests
        {
            return self.failure(id, "BUSY", "Kernel App broker queue is full");
        }
        let deadline =
            Instant::now() + Duration::from_millis(remaining).min(self.limits.max_deadline);
        let (cancel, cancellation) = watch::channel(false);
        let cancellation_cause = Arc::new(AtomicU8::new(CancellationCause::Teardown as u8));
        let mut callers = Vec::new();
        for actor in self.pending.values().filter_map(|call| call.actor.as_ref()) {
            if !callers.contains(actor) {
                callers.push(actor.clone());
            }
        }
        // Frames arrive in order, so a request the worker sent before
        // answering a call is attributed to that call.
        let open_calls: std::collections::BTreeSet<_> = self
            .pending
            .values()
            .filter(|pending| pending.live.sent())
            .map(|pending| pending.method.clone())
            .collect();
        let request = BrokerRequest {
            id: id.clone(),
            method,
            params,
            deadline,
            cancellation: BrokerCancellation(cancellation, cancellation_cause),
            callers,
            open_calls: open_calls.into_iter().collect(),
        };
        self.broker_waiting.push_back(Waiting { request, cancel });
        Ok(())
    }
    pub(super) fn dispatch_brokers(&mut self) -> Result<()> {
        while self.active.len() + self.publishing.len() < self.limits.broker_handlers {
            let Some(waiting) = self.broker_waiting.pop_front() else {
                break;
            };
            let Waiting { request, cancel } = waiting;
            let id = request.id.clone();
            if Instant::now() >= request.deadline {
                request
                    .cancellation
                    .1
                    .store(CancellationCause::Deadline as u8, Ordering::Release);
                cancel.send_replace(true);
                self.broker.request_not_dispatched(&request);
                self.failure(
                    id,
                    "DEADLINE_EXCEEDED",
                    "App broker queue deadline exceeded",
                )?;
                continue;
            }
            self.active.insert(
                id.clone(),
                Active {
                    cancel,
                    cancellation_cause: request.cancellation.1.clone(),
                    deadline: request.deadline,
                    replied: false,
                },
            );
            let broker = self.broker.clone();
            self.handlers.spawn(async move {
                if request.cancellation.is_cancelled() || Instant::now() >= request.deadline {
                    broker.request_not_dispatched(&request);
                    return (
                        id,
                        Err(RemoteError {
                            code: "CANCELLED".into(),
                            message: "App broker call cancelled before dispatch".into(),
                            retryable: Some(false),
                        }),
                    );
                }
                (id, broker.handle(request).await)
            });
        }
        Ok(())
    }
    pub(super) fn cancel_waiting_broker(&mut self, id: &str, code: &str) -> Result<bool> {
        let Some(index) = self
            .broker_waiting
            .iter()
            .position(|entry| entry.request.id == id)
        else {
            return Ok(false);
        };
        let waiting = self
            .broker_waiting
            .remove(index)
            .expect("matched queued request");
        waiting.request.cancellation.1.store(
            if code == "DEADLINE_EXCEEDED" {
                CancellationCause::Deadline
            } else {
                CancellationCause::Request
            } as u8,
            Ordering::Release,
        );
        waiting.cancel.send_replace(true);
        self.broker.request_not_dispatched(&waiting.request);
        self.failure(id.into(), code, "App broker call stopped before dispatch")?;
        Ok(true)
    }
    pub(super) fn discard_waiting_brokers(&mut self) {
        for waiting in self.broker_waiting.drain(..) {
            waiting.cancel.send_replace(true);
            self.broker.request_not_dispatched(&waiting.request);
        }
    }
}
