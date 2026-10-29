//! MP-08 / MP-09 / MP-10 / MP-11: outside principals never join a Room.
use super::*;
use crate::durable_state::agent_lifecycle::outside::Mutation;
use crate::local::*;
mod activity;
mod open_decisions;

pub(crate) fn is_outside_event_request(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        LocalDaemonRequest::SubscribeEvents(_)
            | LocalDaemonRequest::UnsubscribeEvents(_)
            | LocalDaemonRequest::GetEventSubscription(_)
            | LocalDaemonRequest::ListEvents(_)
            | LocalDaemonRequest::WaitEvents(_)
            | LocalDaemonRequest::AckEvents(_)
    )
}
impl KernelRuntimeState {
    pub(crate) fn subscribe_outside_part(
        &self,
        id: &str,
        home: bool,
        targets: Vec<EventTarget>,
    ) -> Result<(), DaemonError> {
        let _mutation = self.owned.begin_managed_activity_mutation();
        let baseline = self
            .owned
            .outside_run_state_snapshot(&targets)
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::outside::refused(
                    "event targets are changing; retry subscribe",
                )
            })?;
        let open_decisions = self.owned.outside_open_decisions(&targets);
        self.outside_store().outside_mutate(Mutation::Subscribe {
            id: id.into(),
            home,
            targets,
            baseline,
            open_decisions,
        })
    }
    pub(crate) fn outside_store(&self) -> &DurableKernelStateStore {
        &self.owned.durable_state_store
    }

    pub(crate) fn validate_event_targets(
        &self,
        targets: &[EventTarget],
        owner: Option<&str>,
    ) -> Result<(), DaemonError> {
        if targets.is_empty() || targets.len() > 1024 {
            return Err(crate::durable_state::agent_lifecycle::outside::refused(
                "follow 1–1024 event targets",
            ));
        }
        let config = self.owned.config_projection.snapshot();
        for target in targets {
            if target.kernel != config.daemon_id {
                return Err(crate::durable_state::agent_lifecycle::outside::refused(
                    "event parts accept only targets on this kernel",
                ));
            }
            let session = self.owned.session_store.get_session(&target.session_id).map_err(|_| crate::durable_state::agent_lifecycle::outside::refused("event target session was not found; use ListSessions to choose a session ID"))?;
            if !owner.is_some_and(|owner| session.has_member(owner)) {
                return Err(crate::durable_state::agent_lifecycle::outside::refused(
                    "event target session is not owned by this user",
                ));
            }
            if session.status() == crate::session::SessionStatus::Ended {
                return Err(crate::durable_state::agent_lifecycle::outside::refused(
                    "session ended; follow another session",
                ));
            }
            if let Some(agent) = &target.agent_id {
                let agent = self.owned.agent_store.get_agent(agent).map_err(|_| crate::durable_state::agent_lifecycle::outside::refused("event target agent was not found; use GetSessionState to choose an agent ID"))?;
                if agent.session_id() != session.id() {
                    return Err(crate::durable_state::agent_lifecycle::outside::refused(
                        "event target agent is not in that session",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn outside_part_request(
        &self,
        id: &str,
        home: bool,
        owner: Option<&str>,
        request: &LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let store = self.outside_store();
        match request {
            LocalDaemonRequest::SubscribeEvents(request) => {
                self.validate_event_targets(&request.targets, owner)?;
                self.subscribe_outside_part(id, home, request.targets.clone())?;
                self.outside_subscription_response(id)
            }
            LocalDaemonRequest::UnsubscribeEvents(request) => {
                if request.all != request.targets.is_empty() {
                    return Err(crate::durable_state::agent_lifecycle::outside::refused(
                        "unsubscribe needs targets or --all",
                    ));
                }
                store.outside_mutate(Mutation::Unsubscribe {
                    id: id.into(),
                    targets: request.targets.clone(),
                    all: request.all,
                    reason: "cancelled".into(),
                })?;
                self.outside_subscription_response(id)
            }
            LocalDaemonRequest::GetEventSubscription(_) => self.outside_subscription_response(id),
            LocalDaemonRequest::ListEvents(request) => {
                self.outside_list_response(id, request.since, request.limit)
            }
            LocalDaemonRequest::WaitEvents(request) => {
                self.outside_list_response(id, request.since, request.limit)
            }
            LocalDaemonRequest::AckEvents(request) => {
                store.outside_mutate(Mutation::Ack {
                    id: id.into(),
                    cursor: request.cursor,
                })?;
                Ok(LocalDaemonResponse::EventsAcknowledged {
                    cursor: request.cursor,
                })
            }
            _ => Err(crate::durable_state::agent_lifecycle::outside::refused(
                "not an outside event request",
            )),
        }
    }
    pub(crate) fn outside_subscription_response(
        &self,
        id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let part = self.outside_store().outside_part(id)?.ok_or_else(|| {
            crate::durable_state::agent_lifecycle::outside::refused(
                "no event subscription; run chariox events subscribe first",
            )
        })?;
        Ok(LocalDaemonResponse::EventSubscription {
            subscription: part.subscription(),
        })
    }
    fn outside_list_response(
        &self,
        id: &str,
        since: Option<u64>,
        limit: usize,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match self.outside_store().outside_list(id, since, limit)? {
            Ok(batch) => Ok(LocalDaemonResponse::EventsListed {
                events: batch.events,
                next_cursor: batch.next_cursor,
                oldest_cursor: batch.oldest_cursor,
                timed_out: false,
            }),
            Err(oldest_cursor) => Ok(LocalDaemonResponse::EventsCursorExpired { oldest_cursor }),
        }
    }
    pub(crate) async fn outside_wait(
        &self,
        id: &str,
        request: &WaitEventsRequest,
        grant: Option<&str>,
        lifetime: Option<&crate::runtime::command::TerminalLifetime>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(request.timeout_ms.min(900_000));
        loop {
            if lifetime.is_some_and(|lifetime| !lifetime.is_live()) {
                return Err(crate::durable_state::agent_lifecycle::outside::refused(
                    "event wait disconnected",
                ));
            }
            if let Some(grant) = grant {
                if self.access_grant_summary(grant).is_none() {
                    self.outside_store().outside_mutate(Mutation::Unsubscribe {
                        id: id.into(),
                        targets: vec![],
                        all: true,
                        reason: "grant_ended".into(),
                    })?;
                    return self.outside_ended_response(id)?.ok_or_else(|| {
                        crate::durable_state::agent_lifecycle::outside::refused(
                            "subscription ending receipt is missing",
                        )
                    });
                }
            }
            if let Some(ending) = self.outside_ended_response(id)? {
                return Ok(ending);
            }
            let response = self.outside_list_response(id, request.since, request.limit)?;
            // Shutdown interrupts this wait, while its retained subscription
            // remains live. An early interruption is not a timeout.
            if self.kernel_access_stopping() {
                return match response {
                    LocalDaemonResponse::EventsListed { ref events, .. } if events.is_empty() => {
                        Err(crate::durable_state::agent_lifecycle::outside::refused(
                            "kernel is shutting down; retry event wait after restart",
                        ))
                    }
                    other => Ok(other),
                };
            }
            match response {
                LocalDaemonResponse::EventsListed {
                    events,
                    next_cursor,
                    oldest_cursor,
                    ..
                } if events.is_empty() => {
                    if tokio::time::Instant::now() >= deadline {
                        return Ok(LocalDaemonResponse::EventsListed {
                            events,
                            next_cursor,
                            oldest_cursor,
                            timed_out: true,
                        });
                    }
                }
                _ => return Ok(response),
            }
            tokio::time::sleep_until(
                deadline.min(tokio::time::Instant::now() + Duration::from_millis(100)),
            )
            .await;
        }
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn publish_outside_steer(
        &self,
        session_id: &str,
        agent_id: &str,
        steer_id: &str,
    ) -> Result<(), DaemonError> {
        self.durable_state_store.outside_publish(
            session_id,
            Some(agent_id),
            OutsideEventValue::Steer {
                steer_id: steer_id.into(),
            },
            "steer delivered",
            EventRefs::default(),
        )
    }

    pub(super) fn publish_outside_interaction(
        &self,
        session_id: &str,
        interaction: &crate::session::RuntimeInteraction,
        state: EventInteractionState,
        passkey: Option<&PasskeyPrompt>,
    ) -> Result<(), DaemonError> {
        let (value, summary) = self.outside_interaction_value(interaction, state, passkey);
        self.durable_state_store.outside_publish(
            session_id,
            interaction.agent_id(),
            value,
            &summary,
            EventRefs {
                interaction_id: Some(interaction.id().into()),
                ..Default::default()
            },
        )
    }

    /// The event body and summary an outside caller gets for a decision.
    pub(super) fn outside_interaction_value(
        &self,
        interaction: &crate::session::RuntimeInteraction,
        state: EventInteractionState,
        passkey: Option<&PasskeyPrompt>,
    ) -> (OutsideEventValue, String) {
        let human = interaction.provider_login_is_human_only()
            || interaction
                .choices()
                .iter()
                .any(crate::session::RuntimeInteractionChoice::requires_passkey);
        let pointer=(human && state == EventInteractionState::Pending).then(|| {
            let kernel=self.config_projection.snapshot().daemon_id;
            EventApprovalPointer { approval_id:interaction.id().into(),kind:passkey.map_or_else(||if interaction.provider_login_is_human_only() { "provider_login" } else { "critical_approval" },|prompt|match prompt.kind { PasskeyPromptKind::CriticalApproval=>"critical_approval",PasskeyPromptKind::AccessGrant=>"access_grant",PasskeyPromptKind::AccessExtension=>"access_extension",PasskeyPromptKind::Sudo=>"sudo" }).into(),
                one_line:format!("Approval pending on kernel {kernel}. Open Chariox there and answer its approval popup."),kernel,link:None,
                expires_at:passkey.map_or_else(||crate::session::unix_epoch_ms().saturating_add(interaction.timeout_sec().unwrap_or(600).saturating_mul(1000)),|prompt|prompt.expires_at_ms) }
        });
        let summary = if human {
            "owner approval"
        } else {
            interaction.title().unwrap_or("interaction")
        };
        (
            OutsideEventValue::Interaction { state, pointer },
            summary.into(),
        )
    }
}

impl KernelRuntimeState {
    pub(crate) async fn finish_outside_waits_on_shutdown(&self) {
        let ended = self.end_kernel_access_for_shutdown().unwrap_or_else(|_| {
            crate::logging::error_with_fields(
                "kernel.events",
                "could not commit grant endings during shutdown",
                serde_json::json!({}),
            );
            0
        });
        // The socket boundary keeps admitted waits alive through their final write.
        if ended > 0 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    pub(crate) fn outside_ended_response(
        &self,
        id: &str,
    ) -> Result<Option<LocalDaemonResponse>, DaemonError> {
        let Some(part) = self.outside_store().outside_part(id)? else {
            return Ok(None);
        };
        let Some(reason) = part.ended else {
            return Ok(None);
        };
        let at_ms = part.ended_at_ms.ok_or_else(|| {
            crate::durable_state::agent_lifecycle::outside::refused(
                "subscription ending receipt is corrupt",
            )
        })?;
        let config = self.owned.config_projection.snapshot();
        let event = OutsideEvent {
            cursor: part.latest,
            kernel: config.daemon_id,
            machine: config.host_machine_id,
            session_id: String::new(),
            agent_id: None,
            value: OutsideEventValue::SubscriptionEnded {
                reason: reason.clone(),
            },
            summary: reason,
            at_ms,
            refs: Default::default(),
        };
        Ok(Some(LocalDaemonResponse::EventsListed {
            events: vec![event],
            next_cursor: part.latest,
            oldest_cursor: part.floor + 1,
            timed_out: false,
        }))
    }
}

#[cfg(test)]
mod activity_tests;
