//! MP-08 / MP-09 / MP-10 / MP-11: reconnect and clean each kernel part independently.
use super::*;
impl KernelGateway {
    pub(crate) fn resume_event_parts(self: &Arc<Self>) {
        match self.runtime_state.outside_store().outside_parts() {
            Ok(parts) => {
                for part in parts.into_iter().filter(|part| part.home) {
                    let harness = self
                        .runtime_state
                        .access_grant_summary(&part.id)
                        .map_or_else(|| "external agent".into(), |grant| harness_name(&grant));
                    self.start_event_parts(&Origin {
                        grant_id: part.id,
                        harness,
                    });
                }
            }
            Err(_) => crate::logging::error_with_fields(
                "kernel_gateway.events",
                "event parts could not be resumed",
                serde_json::json!({}),
            ),
        }
    }
    pub(in crate::runtime::kernel_gateway) fn start_event_parts(self: &Arc<Self>, origin: &Origin) {
        let Ok(Some(part)) = self
            .runtime_state
            .outside_store()
            .outside_part(&origin.grant_id)
        else {
            return;
        };
        let local = self.config.snapshot().daemon_id;
        let kernels: HashSet<_> = part
            .targets
            .iter()
            .chain(&part.cleanup_targets)
            .filter(|t| t.kernel != local)
            .map(|t| t.kernel.clone())
            .collect();
        for kernel in kernels {
            let key = (origin.grant_id.clone(), kernel.clone());
            if !self
                .event_parts
                .running
                .lock()
                .expect("event pumps poisoned")
                .insert(key.clone())
            {
                continue;
            }
            let weak = Arc::downgrade(self);
            let origin = Origin {
                grant_id: origin.grant_id.clone(),
                harness: origin.harness.clone(),
            };
            tokio::spawn(async move {
                let mut subscribed = Vec::new();
                loop {
                    let Some(gateway) = weak.upgrade() else {
                        return;
                    };
                    let keep = gateway
                        .pump_event_part(&origin, &kernel, &mut subscribed)
                        .await;
                    if !keep && gateway.finish_event_part(&origin, &kernel, &key) {
                        return;
                    }
                    drop(gateway);
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            });
        }
    }
    /// Whether the pump of `key` ended. A target followed while the pump was busy found it
    /// running and started nothing, so the pump looks again, under the lock that starting takes,
    /// before it leaves; the target is either seen here or starts the next pump.
    pub(super) fn finish_event_part(
        &self,
        origin: &Origin,
        kernel: &str,
        key: &(String, String),
    ) -> bool {
        let mut running = self
            .event_parts
            .running
            .lock()
            .expect("event pumps poisoned");
        // A store that cannot be read says nothing about the targets: keep following.
        let wanted = self
            .runtime_state
            .outside_store()
            .outside_part(&origin.grant_id)
            .map_or(true, |part| {
                part.is_some_and(|part| {
                    part.ended.is_none() && part.targets.iter().any(|t| t.kernel == kernel)
                })
            });
        if !wanted {
            running.remove(key);
        }
        !wanted
    }
    async fn pump_event_part(
        &self,
        origin: &Origin,
        kernel: &str,
        subscribed: &mut Vec<EventTarget>,
    ) -> bool {
        let store = self.runtime_state.outside_store();
        let part = match store.outside_part(&origin.grant_id) {
            Ok(Some(part)) => part,
            _ => return false,
        };
        if part.ended.is_none()
            && self
                .runtime_state
                .access_grant_summary(&origin.grant_id)
                .is_none()
        {
            if store
                .outside_mutate(Mutation::Unsubscribe {
                    id: origin.grant_id.clone(),
                    targets: vec![],
                    all: true,
                    reason: "grant_ended".into(),
                })
                .is_err()
            {
                return true;
            }
            return true;
        }
        let removed: Vec<_> = part
            .cleanup_targets
            .iter()
            .filter(|t| t.kernel == kernel)
            .cloned()
            .collect();
        if !removed.is_empty() {
            let all = part.ended.is_some();
            let request = LocalDaemonRequest::UnsubscribeEvents(UnsubscribeEventsRequest {
                targets: if all { vec![] } else { removed.clone() },
                all,
            });
            let command_id = event_command_id();
            if let Err(error) = self
                .forward_grant_ended(kernel, origin, &command_id, &request)
                .await
            {
                self.record_event_health(origin, kernel, Err(error));
                return true;
            }
            if store
                .outside_mutate(Mutation::CleanupAck {
                    id: origin.grant_id.clone(),
                    targets: removed.clone(),
                })
                .is_err()
            {
                return true;
            }
            subscribed.retain(|t| !removed.contains(t));
        }
        let targets: Vec<_> = part
            .targets
            .iter()
            .filter(|t| t.kernel == kernel)
            .cloned()
            .collect();
        if part.ended.is_some() || targets.is_empty() {
            return false;
        }
        let result = self
            .pull_event_part(origin, kernel, &targets, subscribed, &part)
            .await;
        self.record_event_health(origin, kernel, result);
        true
    }
    pub(super) fn event_health(
        &self,
        origin: &Origin,
        kernel: &str,
        status: Result<(), GatewayError>,
    ) -> Result<(), crate::error::DaemonError> {
        self.event_health_at(origin, kernel, status, Instant::now())
    }
    pub(super) fn event_health_at(
        &self,
        origin: &Origin,
        kernel: &str,
        status: Result<(), GatewayError>,
        now: Instant,
    ) -> Result<(), crate::error::DaemonError> {
        let key = (origin.grant_id.clone(), kernel.to_string());
        {
            let mut timing_out = self
                .event_parts
                .timing_out_since
                .lock()
                .expect("event timeouts poisoned");
            if matches!(status, Err(GatewayError::RequestTimeout { .. })) {
                let since = *timing_out.entry(key.clone()).or_insert(now);
                if now.saturating_duration_since(since) < BUSY_GRACE {
                    return Ok(());
                }
            } else {
                timing_out.remove(&key);
            }
        }
        let mut health = self
            .event_parts
            .health
            .lock()
            .expect("event health poisoned");
        // Following a kernel again clears its durable baseline. The status cache
        // alone cannot tell whether this subscription still observed it.
        let previous = self
            .runtime_state
            .outside_store()
            .outside_part(&origin.grant_id)?
            .and_then(|part| part.kernel_health.get(kernel).copied());
        if previous != Some(status.is_ok()) {
            self.runtime_state
                .outside_store()
                .outside_mutate(Mutation::KernelHealth {
                    id: origin.grant_id.clone(),
                    kernel: kernel.into(),
                    online: status.is_ok(),
                    reason: match &status {
                        Ok(()) => "connected",
                        // A target's error code is data, not a gateway reason class.
                        Err(GatewayError::Remote { .. }) => "events_part_failed",
                        Err(error) => error.code(),
                    }
                    .into(),
                })?;
        }
        health.insert(key, status);
        Ok(())
    }
    pub(super) fn record_event_health(
        &self,
        origin: &Origin,
        kernel: &str,
        status: Result<(), GatewayError>,
    ) {
        if self.event_health(origin, kernel, status).is_err() {
            // Do not cache an uncommitted transition: the next pump retries it.
            crate::logging::error_with_fields(
                "kernel_gateway.events",
                "event health could not be committed; retrying",
                serde_json::json!({"kernel": kernel, "subscription": origin.grant_id}),
            );
        }
    }
    async fn pull_event_part(
        &self,
        origin: &Origin,
        kernel: &str,
        targets: &[EventTarget],
        subscribed: &mut Vec<EventTarget>,
        part: &Part,
    ) -> Result<(), GatewayError> {
        if subscribed != targets {
            self.forward(
                kernel,
                origin,
                &event_command_id(),
                &LocalDaemonRequest::SubscribeEvents(SubscribeEventsRequest {
                    targets: targets.to_vec(),
                }),
                PULL_DEADLINE,
            )
            .await?;
            *subscribed = targets.to_vec();
        }
        let since = part.remote_cursors.get(kernel).copied().unwrap_or(0);
        let mut response = self.read_event_part(origin, kernel, since).await?;
        if let LocalDaemonResponse::EventsCursorExpired { oldest_cursor } = response {
            response = self
                .read_event_part(origin, kernel, oldest_cursor.saturating_sub(1))
                .await?;
        }
        if let LocalDaemonResponse::EventsListed {
            events,
            next_cursor,
            ..
        } = response
        {
            self.runtime_state
                .outside_store()
                .outside_mutate(Mutation::Import {
                    id: origin.grant_id.clone(),
                    kernel: kernel.into(),
                    events,
                })
                .map_err(|_| part_error(kernel, "could not commit the merged event inbox"))?;
            // Only a cursor that moved needs saying: an ack is a write on the target.
            if next_cursor > since {
                self.forward(
                    kernel,
                    origin,
                    &event_command_id(),
                    &LocalDaemonRequest::AckEvents(AckEventsRequest {
                        cursor: next_cursor,
                    }),
                    PULL_DEADLINE,
                )
                .await?;
            }
            return Ok(());
        }
        Err(part_error(
            kernel,
            "target event cursor could not be resumed",
        ))
    }
    async fn read_event_part(
        &self,
        origin: &Origin,
        kernel: &str,
        since: u64,
    ) -> Result<LocalDaemonResponse, GatewayError> {
        let response = self
            .forward(
                kernel,
                origin,
                &event_command_id(),
                &LocalDaemonRequest::ListEvents(ListEventsRequest {
                    since: Some(since),
                    limit: 1000,
                }),
                PULL_DEADLINE,
            )
            .await?;
        serde_json::from_value(response)
            .map_err(|_| part_error(kernel, "kernel event response is invalid"))
    }
}
