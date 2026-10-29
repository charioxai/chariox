use super::prompt_dispatch::TriggerSteerDelivery;
use super::*;
use crate::session::{
    AgentTrigger, AgentTriggerBusyPolicy, AgentTriggerDispatch, AgentTriggerOutcome,
    AgentTriggerRepeat,
};

type TriggerResendKey = (String, String);
pub(super) type TriggerResendClaims = Arc<std::sync::Mutex<BTreeSet<TriggerResendKey>>>;

struct TriggerResendClaim {
    key: TriggerResendKey,
    claims: TriggerResendClaims,
}

impl Drop for TriggerResendClaim {
    fn drop(&mut self) {
        self.claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

/// Keep a queued replacement from racing promotion until its admission is
/// durable. Cancellation and failed writes discard only this uncommitted copy.
struct TriggerResendAdmissionHold {
    owned: KernelRuntimeOwnedState,
    session_id: String,
    agent_id: String,
    original_prompt_id: String,
    prompt: crate::session::PromptQueueItem,
    reservation: u64,
}

impl Drop for TriggerResendAdmissionHold {
    fn drop(&mut self) {
        let rollback = match self
            .owned
            .durable_state_store
            .trigger_resend_admitted(&self.session_id, &self.original_prompt_id)
        {
            Ok(true) => self
                .owned
                .session_store
                .write()
                .move_trigger_delivery(
                    &self.session_id,
                    &self.original_prompt_id,
                    crate::session::TriggerDeliveryState::NotConfirmed,
                    crate::session::TriggerDeliveryState::Resent,
                )
                .map(|_| ()),
            Ok(false) => (|| {
                self.owned.discard_unadmitted_trigger_resend(
                    &self.session_id,
                    &self.agent_id,
                    &self.original_prompt_id,
                )?;
                let tasks = self
                    .owned
                    .durable_state_store
                    .agent_tasks(Some(&self.session_id), Some(&self.agent_id))?;
                if tasks.iter().any(|task| {
                    task.task_id == self.prompt.id() && task.prompt_id == self.prompt.id()
                }) {
                    self.owned.withdraw_agent_task(self.prompt.id())?;
                }
                Ok(())
            })(),
            Err(error) => Err(error),
        };
        if let Err(error) = rollback {
            crate::logging::error_with_fields(
                "daemon.agent_trigger",
                "failed to discard an unadmitted resend",
                serde_json::json!({"session_id": self.session_id, "error": error.to_string()}),
            );
        }
        self.prompt.release_remote_steer(self.reservation);
    }
}

impl KernelRuntimeOwnedState {
    fn discard_unadmitted_trigger_resend(
        &self,
        session_id: &str,
        agent_id: &str,
        original_prompt_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let (active, queued) = self.prompt_state_owner.state_parts(&session, agent_id);
        if let Some(prompt) =
            active.filter(|prompt| prompt.trigger_resend_of() == Some(original_prompt_id))
        {
            self.prompt_state_owner.complete_active_prompt_if_matches(
                &session,
                agent_id,
                Some(prompt.id()),
            );
        }
        for prompt in queued
            .iter()
            .filter(|prompt| prompt.trigger_resend_of() == Some(original_prompt_id))
        {
            self.prompt_state_owner
                .remove_queued_prompt(&session, agent_id, prompt.id());
        }
        let (active, queued) = self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store
            .mirror_agent_prompt_state(session_id, agent_id, active, queued)?;
        Ok(())
    }
}

enum AgentTriggerFiring {
    Fired {
        outcome: AgentTriggerOutcome,
        steer_refusal: Option<String>,
    },
    /// The steer path already settled the trigger ahead of the queue removal.
    Settled,
    Coalesced,
    AgentGone,
    /// The agent is archived: nothing fires, and the trigger waits for its next run.
    Deferred,
    Skipped,
}

impl KernelRuntimeState {
    pub(crate) async fn create_agent_trigger(
        &self,
        request: crate::local::CreateAgentTriggerRequest,
        caller_user_id: &str,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let crate::local::AgentTriggerSourceRequest::Schedule {
            interval_seconds,
            repeat,
        } = request.source;
        let (trigger, session) = self.create_agent_schedule_trigger(
            &request.session_id,
            &request.agent_id,
            caller_user_id,
            interval_seconds,
            repeat,
            request.busy_policy,
            request.prompt,
        )?;
        Ok(crate::local::LocalDaemonResponse::AgentTriggerCreated { trigger, session })
    }

    pub(crate) async fn update_agent_trigger(
        &self,
        request: crate::local::UpdateAgentTriggerRequest,
        caller_user_id: &str,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        self.ensure_agent_trigger_access(
            &request.session_id,
            &request.trigger_id,
            caller_user_id,
            "update agent trigger",
        )?;
        let trigger = self.owned.session_store.write().update_agent_trigger(
            &request.session_id,
            &request.trigger_id,
            request.busy_policy,
            request.prompt,
            request.enabled,
        )?;
        let session = self
            .owned
            .persist_agent_trigger_session(&request.session_id, "agent_trigger_updated")?;
        Ok(crate::local::LocalDaemonResponse::AgentTriggerUpdated { trigger, session })
    }

    pub(crate) async fn delete_agent_trigger(
        &self,
        request: crate::local::DeleteAgentTriggerRequest,
        caller_user_id: &str,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let (trigger, session) = self.delete_agent_trigger_by_id(
            &request.session_id,
            &request.trigger_id,
            caller_user_id,
        )?;
        Ok(crate::local::LocalDaemonResponse::AgentTriggerDeleted { trigger, session })
    }

    /// Sends an unconfirmed firing once more, following its trigger's busy
    /// policy. A transient claim excludes concurrent clicks; the replacement's
    /// durable admission consumes the resend, including after a restart.
    pub(crate) async fn resend_trigger_delivery(
        &self,
        request: crate::local::ResendTriggerDeliveryRequest,
        caller_user_id: &str,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        use crate::session::TriggerDeliveryState::NotConfirmed;
        let delivery = self
            .owned
            .session_store
            .get_session(&request.session_id)?
            .trigger_deliveries()
            .iter()
            .find(|delivery| delivery.prompt_id() == request.prompt_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "resend trigger delivery",
                message: format!(
                    "no unconfirmed trigger firing `{}` in session `{}`",
                    request.prompt_id, request.session_id
                ),
            })?;
        self.owned.ensure_agent_prompt_access(
            delivery.agent_id(),
            caller_user_id,
            "resend trigger delivery",
        )?;
        let key = (request.session_id.clone(), request.prompt_id.clone());
        let claims = Arc::clone(&self.owned.trigger_resend_claims);
        let reserved = claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.clone());
        if !reserved {
            return Err(DaemonError::LocalTransport {
                operation: "resend trigger delivery",
                message: format!(
                    "trigger firing `{}` is already being sent again",
                    request.prompt_id
                ),
            });
        }
        let _claim = TriggerResendClaim { key, claims };
        // Re-read after reserving: another request may have admitted its
        // replacement between the first snapshot and acquiring this claim.
        if self
            .owned
            .durable_state_store
            .trigger_resend_admitted(&request.session_id, &request.prompt_id)?
        {
            self.owned.session_store.write().move_trigger_delivery(
                &request.session_id,
                &request.prompt_id,
                crate::session::TriggerDeliveryState::NotConfirmed,
                crate::session::TriggerDeliveryState::Resent,
            )?;
        }
        let current = self.owned.session_store.get_session(&request.session_id)?;
        if !current
            .trigger_deliveries()
            .iter()
            .any(|known| known.prompt_id() == request.prompt_id && known.state() == NotConfirmed)
        {
            return Err(DaemonError::LocalTransport {
                operation: "resend trigger delivery",
                message: format!(
                    "trigger firing `{}` was already sent again",
                    request.prompt_id
                ),
            });
        }
        let dispatch = AgentTriggerDispatch {
            claim_generation: 0,
            session_id: request.session_id.clone(),
            trigger_id: delivery.trigger_id().to_string(),
            agent_id: delivery.agent_id().to_string(),
            prompt: delivery.prompt().to_string(),
            busy_policy: delivery.busy_policy(),
        };
        self.deliver_agent_trigger_prompt(
            &dispatch,
            None,
            delivery.origin(),
            Some(&request.prompt_id),
        )
        .await?;
        let session = self.owned.session_snapshot(&request.session_id)?;
        Ok(crate::local::LocalDaemonResponse::TriggerDeliveryResent { session })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "A schedule trigger is created from distinct request fields and the caller identity."
    )]
    fn create_agent_schedule_trigger(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        interval_seconds: u64,
        repeat: AgentTriggerRepeat,
        busy_policy: AgentTriggerBusyPolicy,
        prompt: Option<String>,
    ) -> Result<(AgentTrigger, crate::session::RuntimeSession), DaemonError> {
        let agent = self.owned.ensure_agent_prompt_access(
            agent_id,
            caller_user_id,
            "create agent trigger",
        )?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let trigger = self
            .owned
            .session_store
            .write()
            .create_agent_schedule_trigger(
                session_id,
                agent_id,
                caller_user_id,
                interval_seconds,
                repeat,
                busy_policy,
                prompt,
            )?;
        let session = self
            .owned
            .persist_agent_trigger_session(session_id, "agent_trigger_created")?;
        Ok((trigger, session))
    }

    fn delete_agent_trigger_by_id(
        &self,
        session_id: &str,
        trigger_id: &str,
        caller_user_id: &str,
    ) -> Result<(AgentTrigger, crate::session::RuntimeSession), DaemonError> {
        self.ensure_agent_trigger_access(
            session_id,
            trigger_id,
            caller_user_id,
            "delete agent trigger",
        )?;
        let trigger = self
            .owned
            .session_store
            .write()
            .delete_agent_trigger(session_id, trigger_id)?;
        let session = self
            .owned
            .persist_agent_trigger_session(session_id, "agent_trigger_deleted")?;
        Ok((trigger, session))
    }

    /// Whoever may prompt an agent may arm, change and remove its triggers.
    fn ensure_agent_trigger_access(
        &self,
        session_id: &str,
        trigger_id: &str,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(trigger) = session
            .agent_triggers()
            .iter()
            .find(|trigger| trigger.id() == trigger_id)
        else {
            return Err(DaemonError::LocalTransport {
                operation: "agent trigger",
                message: format!(
                    "agent trigger `{trigger_id}` was not found in session `{session_id}`"
                ),
            });
        };
        match self
            .owned
            .ensure_agent_prompt_access(trigger.agent_id(), caller_user_id, operation)
        {
            // A trigger whose agent is already gone has no prompt access to
            // check; any session member may remove it.
            Err(DaemonError::AgentNotFound { .. }) => Ok(()),
            result => result.map(|_| ()),
        }
    }

    /// The window between an archive's state change and its trigger cleanup.
    #[cfg(test)]
    pub(crate) fn set_agent_archived_without_cleanup_for_test(
        &self,
        agent_id: &str,
        archived: bool,
    ) {
        self.owned
            .agent_store
            .archive_agent(agent_id, archived, &mut self.owned.session_store.write())
            .expect("agent archive state should change");
    }

    pub(super) fn resume_idle_agent_trigger_queues(
        &self,
        agents: impl IntoIterator<Item = (String, String)>,
    ) {
        for (session_id, agent_id) in agents.into_iter().collect::<BTreeSet<_>>() {
            if let Err(error) = self.resume_idle_agent_trigger_queue(&session_id, &agent_id) {
                crate::logging::error_with_fields(
                    "daemon.agent_trigger",
                    "failed to resume an idle agent queue",
                    serde_json::json!({"session_id":session_id,"agent_id":agent_id,"error":error.to_string()}),
                );
            }
        }
    }

    pub(crate) async fn dispatch_due_agent_triggers(&self, now_ms: u64) {
        let recovered_steers = self.owned.reconcile_inactive_local_steers();
        self.resume_idle_agent_trigger_queues(
            recovered_steers
                .into_iter()
                .map(|(session_id, agent_id, _)| (session_id, agent_id)),
        );
        let dispatches = self
            .owned
            .session_store
            .write()
            .claim_due_agent_triggers(now_ms);
        for dispatch in dispatches {
            let result = self.fire_agent_trigger(&dispatch).await;
            let update = match result {
                Ok(AgentTriggerFiring::Fired {
                    outcome,
                    steer_refusal,
                }) => self.owned.session_store.write().mark_agent_trigger_fired(
                    &dispatch.session_id,
                    &dispatch.trigger_id,
                    now_ms,
                    outcome,
                    steer_refusal,
                ),
                Ok(AgentTriggerFiring::Coalesced) => self
                    .owned
                    .session_store
                    .write()
                    .mark_agent_trigger_coalesced(
                        &dispatch.session_id,
                        &dispatch.trigger_id,
                        now_ms,
                    ),
                Ok(AgentTriggerFiring::Deferred) => self
                    .owned
                    .session_store
                    .write()
                    .defer_agent_trigger(&dispatch.session_id, &dispatch.trigger_id, now_ms),
                Ok(AgentTriggerFiring::Skipped | AgentTriggerFiring::Settled) => continue,
                Ok(AgentTriggerFiring::AgentGone) => {
                    self.record_agent_trigger_notice(
                        &dispatch,
                        format!(
                            "Trigger `{}` was removed: its agent `{}` no longer exists",
                            dispatch.trigger_id, dispatch.agent_id
                        ),
                    );
                    self.owned
                        .session_store
                        .write()
                        .delete_agent_trigger(&dispatch.session_id, &dispatch.trigger_id)
                        .map(Some)
                }
                Err(error) => {
                    self.record_agent_trigger_notice(
                        &dispatch,
                        format!(
                            "Trigger `{}` could not deliver its prompt: {error}",
                            dispatch.trigger_id
                        ),
                    );
                    self.owned.session_store.write().mark_agent_trigger_failed(
                        &dispatch.session_id,
                        &dispatch.trigger_id,
                        now_ms,
                        error.to_string(),
                    )
                }
            };
            if let Err(error) = update.and_then(|_| {
                self.owned
                    .persist_agent_trigger_session(&dispatch.session_id, "agent_trigger_fired")
                    .map(|_| ())
            }) {
                crate::logging::warn_with_fields(
                    "daemon.agent_trigger",
                    "agent trigger state update failed",
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "trigger_id": dispatch.trigger_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }

    async fn fire_agent_trigger(
        &self,
        dispatch: &AgentTriggerDispatch,
    ) -> Result<AgentTriggerFiring, DaemonError> {
        match self.owned.agent_store.get_agent(&dispatch.agent_id) {
            Err(DaemonError::AgentNotFound { .. }) => return Ok(AgentTriggerFiring::AgentGone),
            // Archive removes the agent's triggers; a firing claimed just
            // before that, or during an archive that is rolled back, must not
            // start a turn and must not keep its claim.
            Ok(agent) if agent.archived() => return Ok(AgentTriggerFiring::Deferred),
            result => result.map(|_| ())?,
        }
        let session = self.owned.session_store.get_session(&dispatch.session_id)?;
        if !session.agent_triggers().iter().any(|trigger| {
            trigger.id() == dispatch.trigger_id
                && trigger.matches_dispatch_claim(dispatch.claim_generation)
        }) {
            return Ok(AgentTriggerFiring::Skipped);
        }
        let (active, queued) = self
            .owned
            .prompt_state_owner
            .state_parts(&session, &dispatch.agent_id);
        // A schedule represents one outstanding request, whether queued or
        // already running. The dispatch claim prevents overlapping ticks from
        // admitting another occurrence before this submission settles.
        if active
            .iter()
            .chain(queued.iter())
            .any(|prompt| prompt.agent_trigger_id() == Some(dispatch.trigger_id.as_str()))
        {
            return Ok(AgentTriggerFiring::Coalesced);
        }
        self.deliver_agent_trigger_prompt(
            dispatch,
            Some(dispatch),
            crate::session::PromptOrigin::Schedule,
            None,
        )
        .await
    }

    /// Admits the trigger's message as a labelled prompt of its agent. A
    /// resend has no claim: its trigger may be gone or not due.
    async fn deliver_agent_trigger_prompt(
        &self,
        dispatch: &AgentTriggerDispatch,
        claim: Option<&AgentTriggerDispatch>,
        origin: crate::session::PromptOrigin,
        resend_of: Option<&str>,
    ) -> Result<AgentTriggerFiring, DaemonError> {
        let source_attachment_id = self
            .owned
            .ensure_agent_trigger_attachment(&dispatch.session_id)?;
        let prompt_id = self.owned.session_store.reserve_prompt_id();
        let (scheduled_context, scheduled_manifest_entry) =
            crate::prompt_assembly::render_configured_prompt_with_manifest(
                "runtime/scheduled-prompt",
                crate::prompt_assembly::bundled_prompt_template("runtime/scheduled-prompt")
                    .expect("scheduled prompt context must be registered"),
                &[("SCHEDULE_ID", &dispatch.trigger_id)],
            );
        let prompt = crate::session::PromptQueueItem::new(
            prompt_id,
            &source_attachment_id,
            &dispatch.agent_id,
            &dispatch.prompt,
            crate::session::PromptStatus::Queued,
        )
        .with_agent_trigger(&dispatch.trigger_id)
        .with_prompt_origin(origin)
        .with_hidden_system_context(crate::prompt_assembly::attach_prompt_manifest_entry(
            scheduled_context,
            &scheduled_manifest_entry,
        ));
        let prompt = if let Some(original_prompt_id) = resend_of {
            prompt.with_trigger_resend(original_prompt_id)
        } else {
            prompt
        };
        let admission_hold = if let Some(original_prompt_id) = resend_of {
            let reservation =
                prompt
                    .reserve_remote_steer()
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "resend trigger delivery",
                        message: "replacement prompt admission is already reserved".into(),
                    })?;
            Some(TriggerResendAdmissionHold {
                owned: self.owned.clone(),
                session_id: dispatch.session_id.clone(),
                agent_id: dispatch.agent_id.clone(),
                original_prompt_id: original_prompt_id.to_string(),
                prompt: prompt.clone(),
                reservation,
            })
        } else {
            None
        };
        let submission = self
            .submit_prepared_agent_trigger_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: dispatch.session_id.clone(),
                    prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                claim,
            )
            .await;
        drop(admission_hold);
        let mut submission = match submission {
            Err(DaemonError::LocalTransport {
                operation: "agent trigger claim",
                ..
            }) => return Ok(AgentTriggerFiring::Skipped),
            result => result?,
        };
        if let (
            crate::session::PromptSubmissionOutcome::Started { prompt },
            Some(runtime_dispatch),
        ) = (&submission.outcome, submission.dispatch.as_ref())
        {
            self.start_active_turn_with_trace_id(
                &runtime_dispatch.session_id,
                &runtime_dispatch.agent_id,
                prompt.id(),
                &runtime_dispatch.provider_run_id,
                "scheduled-prompt",
            );
        }
        if let Some(runtime_dispatch) = submission.dispatch.take() {
            self.spawn_prompt_dispatch(runtime_dispatch, self.provider_runtime_lanes.clone());
        }
        if let Some(remote_dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(remote_dispatch);
        }
        let queued_prompt_id = match &submission.outcome {
            crate::session::PromptSubmissionOutcome::Started { .. } => {
                return Ok(AgentTriggerFiring::Fired {
                    outcome: AgentTriggerOutcome::Started,
                    steer_refusal: None,
                });
            }
            crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
        };
        if resend_of.is_some() {
            self.resume_idle_agent_trigger_queue(&dispatch.session_id, &dispatch.agent_id)?;
        }
        if dispatch.busy_policy == AgentTriggerBusyPolicy::Queue {
            return Ok(AgentTriggerFiring::Fired {
                outcome: AgentTriggerOutcome::Queued,
                steer_refusal: None,
            });
        }
        // Steer the queued firing exactly as a person steers a queued prompt,
        // so local and leased agents share one path and its receipts. A
        // refused steer leaves the firing queued and says why.
        let settlement_started = std::sync::atomic::AtomicBool::new(false);
        let steer = self
            .steer_agent_trigger_prompt(
                &dispatch.session_id,
                &dispatch.agent_id,
                &source_attachment_id,
                &queued_prompt_id,
                |delivery| {
                    let unconfirmed = (delivery == TriggerSteerDelivery::Unconfirmed).then(|| {
                        crate::session::TriggerDelivery::not_confirmed(
                            &queued_prompt_id,
                            &dispatch.trigger_id,
                            &dispatch.agent_id,
                            origin,
                            &dispatch.prompt,
                            dispatch.busy_policy,
                            crate::session::unix_epoch_ms(),
                        )
                    });
                    settlement_started.store(true, std::sync::atomic::Ordering::SeqCst);
                    self.owned.settle_steered_firing(
                        &dispatch.session_id,
                        claim.map(|_| dispatch.trigger_id.as_str()),
                        &queued_prompt_id,
                        unconfirmed,
                    )?;
                    Ok(())
                },
            )
            .await;
        match steer {
            Ok(TriggerSteerDelivery::Confirmed)
                if settlement_started.load(std::sync::atomic::Ordering::SeqCst) =>
            {
                Ok(AgentTriggerFiring::Settled)
            }
            Ok(TriggerSteerDelivery::Confirmed) => Ok(AgentTriggerFiring::Fired {
                outcome: AgentTriggerOutcome::Steered,
                steer_refusal: None,
            }),
            Ok(TriggerSteerDelivery::Unconfirmed) => {
                self.record_agent_trigger_notice(
                    dispatch,
                    format!(
                        "Trigger `{}`: the provider did not confirm the steer. It is not sent again by itself; resend it from the prompt if it did not arrive.",
                        dispatch.trigger_id
                    ),
                );
                Ok(AgentTriggerFiring::Settled)
            }
            Err(error) if settlement_started.load(std::sync::atomic::Ordering::SeqCst) => {
                Err(error)
            }
            Err(error) => {
                let refusal = format!("steer refused, the firing stays queued: {error}");
                self.record_agent_trigger_notice(
                    dispatch,
                    format!("Trigger `{}`: {refusal}", dispatch.trigger_id),
                );
                Ok(AgentTriggerFiring::Fired {
                    outcome: AgentTriggerOutcome::Queued,
                    steer_refusal: Some(refusal),
                })
            }
        }
    }

    fn resume_idle_agent_trigger_queue(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let (active, queued) = self
            .owned
            .prompt_state_owner
            .state_parts(&session, agent_id);
        if active.is_some() || queued.is_empty() {
            return Ok(());
        }
        if self
            .owned
            .agent_store
            .get_agent(agent_id)?
            .remote_execution()
            .is_none()
        {
            return self.spawn_idle_local_prompt_queue_promotion(session_id, agent_id);
        }
        let runtime = self.clone();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        tokio::spawn(async move {
            if let Err(error) = runtime
                .spawn_next_queued_remote_prompt(&session_id, &agent_id)
                .await
            {
                crate::logging::error_with_fields(
                    "daemon.agent_trigger",
                    "failed to promote queued work after trigger delivery",
                    serde_json::json!({"session_id":session_id,"agent_id":agent_id,"error":error.to_string()}),
                );
            }
        });
        Ok(())
    }

    fn record_agent_trigger_notice(&self, dispatch: &AgentTriggerDispatch, message: String) {
        self.owned.record_notice_for_agent(
            &dispatch.session_id,
            None,
            Some(&dispatch.agent_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id),
            message,
        );
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn ensure_current_agent_trigger_claim(
        &self,
        claim: &AgentTriggerDispatch,
    ) -> Result<(), DaemonError> {
        let session = self.session_store.get_session(&claim.session_id)?;
        if session.agent_triggers().iter().any(|trigger| {
            trigger.id() == claim.trigger_id
                && trigger.matches_dispatch_claim(claim.claim_generation)
        }) {
            return Ok(());
        }
        Err(DaemonError::LocalTransport {
            operation: "agent trigger claim",
            message: "trigger was disabled, deleted, or its firing claim was invalidated before prompt admission".into(),
        })
    }

    fn ensure_agent_trigger_attachment(&self, session_id: &str) -> Result<String, DaemonError> {
        // The client id predates triggers; queued firings already reference it.
        let client_id = format!("agent-schedules:{session_id}");
        if let Some(attachment) = self
            .attachment_store
            .list_client_attachments(&client_id)
            .into_iter()
            .find(|attachment| attachment.session_id() == session_id)
        {
            return Ok(attachment.id().to_string());
        }
        let session = self.session_store.get_session(session_id)?;
        let attachment = self.attach(crate::attachment::AttachRequest::for_user(
            session_id,
            client_id,
            crate::attachment::ClientCapabilityLevel::AutomationOnly,
            session.owner_user_id(),
        ))?;
        Ok(attachment.id().to_string())
    }

    pub(super) fn persist_agent_trigger_session(
        &self,
        session_id: &str,
        reason: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let session = self.session_snapshot(session_id)?;
        let durable_session = session.durable_runtime_snapshot();
        self.durable_state_store.append_event(
            "session.updated",
            Some(session_id.to_string()),
            serde_json::json!({
                "session": &durable_session,
                "reason": reason,
            }),
        )?;
        Ok(session)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt;
    use super::*;

    async fn schedule(
        runtime: &KernelRuntimeState,
        session_id: &str,
        agent_id: &str,
        repeat: AgentTriggerRepeat,
    ) -> AgentTrigger {
        let owner = runtime.owned.agent_store.get_agent(agent_id).unwrap();
        let response = runtime
            .create_agent_trigger(
                crate::local::CreateAgentTriggerRequest {
                    session_id: session_id.into(),
                    agent_id: agent_id.into(),
                    source: crate::local::AgentTriggerSourceRequest::Schedule {
                        interval_seconds: 60,
                        repeat,
                    },
                    busy_policy: AgentTriggerBusyPolicy::Steer,
                    prompt: Some("Keep this firing".into()),
                },
                owner.owner_user_id(),
            )
            .await
            .unwrap();
        let crate::local::LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = response
        else {
            panic!("expected trigger");
        };
        trigger
    }

    async fn wait_for_queued_firing(
        runtime: &KernelRuntimeState,
        session_id: &str,
        agent_id: &str,
    ) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let session = runtime.owned.session_store.get_session(session_id).unwrap();
                let (_, queued) = runtime
                    .owned
                    .prompt_state_owner
                    .state_parts(&session, agent_id);
                if !queued.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("firing must stay queued while the provider lane is held");
    }

    #[tokio::test]
    async fn trigger_steer_keeps_once_and_recurring_firings_until_local_delivery_succeeds() {
        for repeat in [AgentTriggerRepeat::Once, AgentTriggerRepeat::Every] {
            let (_worktree, runtime, session_id, agent_id, attachment_id, _, run_id) =
                runtime_with_active_prompt().await;
            let trigger = schedule(&runtime, &session_id, &agent_id, repeat).await;
            let held = runtime.provider_runtime_lanes.acquire(&run_id).await;
            let firing_runtime = runtime.clone();
            let due = trigger.next_run_at_ms();
            let firing =
                tokio::spawn(async move { firing_runtime.dispatch_due_agent_triggers(due).await });
            wait_for_queued_firing(&runtime, &session_id, &agent_id).await;
            assert!(
                !firing.is_finished(),
                "must not report success before local acceptance"
            );
            let session = runtime
                .owned
                .session_store
                .get_session(&session_id)
                .unwrap();
            let (_, pending) = runtime
                .owned
                .prompt_state_owner
                .state_parts(&session, &agent_id);
            assert!(
                runtime
                    .owned
                    .steer_queued_prompt(&session_id, &agent_id, &attachment_id, pending[0].id())
                    .is_err(),
                "a second steer cannot consume an in-flight trigger firing"
            );
            runtime
                .owned
                .prompt_state_owner
                .complete_active_prompt_only(&session, &agent_id)
                .unwrap();
            drop(held);
            tokio::time::timeout(std::time::Duration::from_secs(5), firing)
                .await
                .unwrap()
                .unwrap();
            let snapshot = runtime.owned.session_snapshot(&session_id).unwrap();
            let (active, queued) = runtime
                .owned
                .prompt_state_owner
                .state_parts(&snapshot, &agent_id);
            let retained = active
                .iter()
                .chain(queued.iter())
                .filter(|prompt| prompt.agent_trigger_id() == Some(trigger.id()))
                .count();
            assert_eq!(
                retained, 1,
                "a rejected steer must retain or resume the firing exactly once"
            );
            let durable = snapshot.durable_runtime_snapshot();
            let durable_count = usize::from(durable.active_prompt_for_agent(&agent_id).is_some())
                + durable
                    .queued_prompts_for_agent(&agent_id)
                    .map_or(0, |prompts| prompts.len());
            assert_eq!(
                durable_count, 1,
                "retained or resumed firing must be durable"
            );
            if repeat == AgentTriggerRepeat::Every {
                let trigger = &snapshot.agent_triggers()[0];
                assert_eq!(trigger.last_outcome(), Some(AgentTriggerOutcome::Queued));
                assert!(trigger.last_error().unwrap().contains("steer refused"));
            }
        }
    }

    #[tokio::test]
    async fn trigger_steer_consumes_only_after_ack_even_if_the_original_turn_finishes() {
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            runtime_with_active_prompt().await;
        let request = crate::provider::LaunchProviderRequest::new(
            &session_id,
            "dev-stub",
            "slow-structured",
            "default",
            "model",
        )
        .with_agent_id(&agent_id);
        let mut run = crate::provider::RuntimeProviderRun::new(
            &run_id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "slow-ack".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: Some("test".into()),
            },
        );
        run.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        runtime.owned.provider_run_projection.update(run);
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        let state = runtime.clone();
        let due = trigger.next_run_at_ms();
        let firing = tokio::spawn(async move { state.dispatch_due_agent_triggers(due).await });
        wait_for_queued_firing(&runtime, &session_id, &agent_id).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !runtime
                .owned
                .provider_store
                .structured_prompt_io_in_flight(&run_id)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the Submit command must reach the mailbox before completion");
        assert!(
            !firing.is_finished(),
            "a mailbox enqueue is not yet an acknowledgement"
        );
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        runtime
            .owned
            .prompt_state_owner
            .complete_active_prompt_only(&session, &agent_id)
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), firing)
            .await
            .unwrap()
            .unwrap();
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let (active, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert!(active.is_none());
        assert!(
            queued.is_empty(),
            "accepted steering must not run again after completion"
        );
        assert!(session.agent_triggers().is_empty());
    }

    // MP-08/MP-09/MP-10: the steering intent is durable before the provider
    // write, so a crash before the acknowledgement cannot replay the firing.
    #[tokio::test]
    async fn trigger_steer_intent_is_durable_before_acknowledgement_and_not_replayed_after_restart()
    {
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
                durable_state_config("trigger-intent-restore"), &[],
            ).await;
        let request = crate::provider::LaunchProviderRequest::new(
            &session_id,
            "dev-stub",
            "slow-structured",
            "default",
            "model",
        )
        .with_agent_id(&agent_id);
        let mut run = crate::provider::RuntimeProviderRun::new(
            &run_id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "slow-ack".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: Some("test".into()),
            },
        );
        run.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        runtime.owned.provider_run_projection.update(run);
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        let state = runtime.clone();
        let due = trigger.next_run_at_ms();
        let firing = tokio::spawn(async move { state.dispatch_due_agent_triggers(due).await });
        wait_for_queued_firing(&runtime, &session_id, &agent_id).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !runtime
                .owned
                .provider_store
                .structured_prompt_io_in_flight(&run_id)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the Submit command must reach the mailbox");

        // The kernel dies here: the provider holds the message, nothing
        // acknowledged it, and the queue entry is still on disk.
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let crashed = serde_json::to_value(
            crate::durable_prompt_state::DurablePromptStateEventPayload::capture(
                &session, &agent_id,
            ),
        )
        .unwrap();
        let mut restored: crate::durable_prompt_state::DurablePromptStateEventPayload =
            serde_json::from_value(crashed).unwrap();
        restored.restore_private_states();
        assert!(
            restored
                .queued_prompts
                .front()
                .is_some_and(|prompt| prompt.local_steer_submitted()),
            "the durable queue must remember the steer was handed to the provider"
        );

        let events = runtime
            .owned
            .durable_state_store
            .load_events_after(0)
            .unwrap();
        firing.abort();
        let _ = firing.await;
        let (runtime, _) =
            restart_from_log_cut(&events, events.len(), "trigger-intent-restored").await;
        let dropped = runtime.owned.reconcile_inactive_local_steers();
        assert_eq!(dropped.len(), 1, "the held firing must leave the queue");
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let (_, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert!(queued.is_empty(), "a held steer must never be replayed");
        assert!(
            session.agent_triggers().is_empty(),
            "the once-trigger counts as sent and cannot fire the message again"
        );
        assert!(session
            .durable_runtime_snapshot()
            .queued_prompts_for_agent(&agent_id)
            .is_none_or(|prompts| prompts.is_empty()));
    }

    fn install_slow_structured_run(
        runtime: &KernelRuntimeState,
        session_id: &str,
        agent_id: &str,
        run_id: &str,
        process_label: &str,
    ) {
        let request = crate::provider::LaunchProviderRequest::new(
            session_id,
            "dev-stub",
            "slow-structured",
            "default",
            "model",
        )
        .with_agent_id(agent_id);
        let mut run = crate::provider::RuntimeProviderRun::new(
            run_id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: process_label.into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: Some("test".into()),
            },
        );
        run.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        runtime.owned.provider_run_projection.update(run);
    }

    pub(in crate::runtime::state) fn durable_state_config(
        tag: &str,
    ) -> crate::config::DaemonConfig {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.state.path = Some(
            std::env::temp_dir()
                .join("chariox-tests")
                .join(format!("{tag}-{}.db", crate::session::unix_epoch_ms()))
                .display()
                .to_string(),
        );
        config
    }

    /// Starts a kernel from a durable log that stops right before `events[cut]`,
    /// which is what a stop between two durable writes leaves on disk.
    async fn restart_from_log_cut(
        events: &[crate::durable_state::DurableStateEvent],
        cut: usize,
        tag: &str,
    ) -> (KernelRuntimeState, crate::config::DaemonConfig) {
        let config = durable_state_config(tag);
        let path = std::path::PathBuf::from(config.user_config.state.path.clone().unwrap());
        {
            let store = crate::durable_state::DurableKernelStateStore::open(path).unwrap();
            for event in &events[..cut] {
                store
                    .append_event(&event.kind, event.subject_id.clone(), event.payload.clone())
                    .unwrap();
            }
        }
        let app = crate::app::DaemonApp::bootstrap(config.clone()).expect("restart boots");
        let runtime = super::super::local_prompt_dispatch_runtime::tests::owned_runtime_state(
            &std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        )
        .await;
        (runtime, config)
    }

    fn reject_trigger_settlement(runtime: &KernelRuntimeState) -> rusqlite::Connection {
        let connection =
            rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_trigger_settlement BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'session.updated'
               AND json_extract(NEW.payload_json, '$.reason') = 'agent_trigger_fired'
             BEGIN SELECT RAISE(FAIL, 'injected trigger settlement failure'); END;",
            )
            .unwrap();
        connection
    }

    async fn settlement_failure_retains_held_firing(restart_recovery: bool) {
        let config = durable_state_config("trigger-settlement-failure");
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
                config, &[],
            ).await;
        install_slow_structured_run(&runtime, &session_id, &agent_id, &run_id, "slow-ack");
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        let state = runtime.clone();
        let due = trigger.next_run_at_ms();
        let connection = (!restart_recovery).then(|| reject_trigger_settlement(&runtime));
        let firing = tokio::spawn(async move { state.dispatch_due_agent_triggers(due).await });
        wait_for_queued_firing(&runtime, &session_id, &agent_id).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !runtime
                .owned
                .provider_store
                .structured_prompt_io_in_flight(&run_id)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let runtime = if restart_recovery {
            let events = runtime
                .owned
                .durable_state_store
                .load_events_after(0)
                .unwrap();
            firing.abort();
            let (restarted, _) =
                restart_from_log_cut(&events, events.len(), "settlement-failure-restart").await;
            let _connection = reject_trigger_settlement(&restarted);
            assert!(
                restarted.owned.reconcile_inactive_local_steers().is_empty(),
                "failed settlement must not remove the held firing"
            );
            restarted
        } else {
            tokio::time::timeout(std::time::Duration::from_secs(5), firing)
                .await
                .unwrap()
                .unwrap();
            drop(connection);
            runtime
        };
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let (_, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert!(
            queued.iter().any(|prompt| prompt.local_steer_submitted()),
            "failed settlement must retain the hold in memory"
        );
        let events = runtime
            .owned
            .durable_state_store
            .load_events_after(0)
            .unwrap();
        let (restarted, _) =
            restart_from_log_cut(&events, events.len(), "settlement-failure-restored").await;
        let session = restarted.owned.session_snapshot(&session_id).unwrap();
        let (_, queued) = restarted
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert!(
            queued.iter().any(|prompt| prompt.local_steer_submitted()),
            "failed settlement must retain the durable hold across restart"
        );
        assert_restart_cannot_refire(
            &restarted,
            &agent_id,
            "settlement append failed then recovered",
        );
    }

    #[tokio::test]
    async fn trigger_steer_settlement_recovers_on_live_maintenance_without_replay() {
        let config = durable_state_config("trigger-live-settlement-retry");
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
                config, &[],
            ).await;
        install_slow_structured_run(&runtime, &session_id, &agent_id, &run_id, "slow-ack");
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        let connection = reject_trigger_settlement(&runtime);
        let due = trigger.next_run_at_ms();
        runtime.dispatch_due_agent_triggers(due).await;
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let (_, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        let held = queued
            .iter()
            .find(|prompt| prompt.local_steer_submitted())
            .expect("failed settlement must retain the delivery hold")
            .id()
            .to_string();
        let completed = runtime
            .owned
            .provider_store
            .drain_finished_structured_prompt_submit_jobs();
        assert_eq!(
            completed
                .iter()
                .filter(|job| job.provider_run_id == run_id && job.prompt_id == held)
                .count(),
            1
        );
        connection
            .execute_batch("DROP TRIGGER reject_trigger_settlement;")
            .unwrap();
        for tick in 0..3 {
            runtime
                .dispatch_due_agent_triggers(due + tick * 1_000)
                .await;
        }
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let (_, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert!(
            !queued.iter().any(|prompt| prompt.id() == held),
            "ordinary live maintenance must clear a settled hold without restarting"
        );
        let delivery = session
            .trigger_deliveries()
            .iter()
            .find(|delivery| delivery.prompt_id() == held)
            .expect("uncertain settlement must expose the person's resend action");
        assert_eq!(
            delivery.state(),
            crate::session::TriggerDeliveryState::NotConfirmed
        );
        assert_eq!(delivery.busy_policy(), AgentTriggerBusyPolicy::Steer);
        assert!(session
            .agent_triggers()
            .iter()
            .all(|known| known.id() != trigger.id()));
        let completed = runtime
            .owned
            .provider_store
            .drain_finished_structured_prompt_submit_jobs();
        assert_eq!(
            completed
                .iter()
                .filter(|job| job.provider_run_id == run_id && job.prompt_id == held)
                .count(),
            0,
            "recovery must never send the provider another message"
        );
    }

    #[derive(Clone, Copy)]
    enum HoldRecoveryPath {
        Live,
        Restart,
        // The hold is removed, then the kernel stops before the next prompt
        // is promoted: the identities that would have woken it are lost.
        LiveThenRestart,
        RestartThenRestart,
    }

    // MP-08/MP-10/MP-11: clearing a hold must wake ordinary queued work.
    #[tokio::test]
    async fn live_hold_recovery_dispatches_remaining_user_prompt_once() {
        hold_recovery_dispatches_remaining_user_prompt_once(HoldRecoveryPath::Live).await;
    }

    #[tokio::test]
    async fn restart_hold_recovery_dispatches_remaining_user_prompt_once() {
        hold_recovery_dispatches_remaining_user_prompt_once(HoldRecoveryPath::Restart).await;
    }

    #[tokio::test]
    async fn restart_after_live_hold_removal_dispatches_remaining_user_prompt_once() {
        hold_recovery_dispatches_remaining_user_prompt_once(HoldRecoveryPath::LiveThenRestart)
            .await;
    }

    #[tokio::test]
    async fn second_restart_after_hold_removal_dispatches_remaining_user_prompt_once() {
        hold_recovery_dispatches_remaining_user_prompt_once(HoldRecoveryPath::RestartThenRestart)
            .await;
    }

    async fn hold_recovery_dispatches_remaining_user_prompt_once(path: HoldRecoveryPath) {
        let mut config = durable_state_config("trigger-live-queue-recovery");
        config.room_agent_tools = false;
        let (_worktree, runtime, session_id, agent_id, attachment_id, _, run_id) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
                config, &[],
            ).await;
        install_slow_structured_run(&runtime, &session_id, &agent_id, &run_id, "slow-ack");
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        let connection = reject_trigger_settlement(&runtime);
        runtime
            .dispatch_due_agent_triggers(trigger.next_run_at_ms())
            .await;
        runtime
            .owned
            .provider_store
            .drain_finished_structured_prompt_submit_jobs();
        let prompt = crate::session::PromptQueueItem::new(
            "user-behind-hold",
            &attachment_id,
            &agent_id,
            "Run the accepted user work",
            crate::session::PromptStatus::Queued,
        );
        let submission = runtime
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            })
            .await
            .unwrap();
        let crate::session::PromptSubmissionOutcome::Queued { prompt } = submission.outcome else {
            panic!("user work must queue behind the held steer");
        };
        let admitted_prompt_id = prompt.id().to_string();
        runtime
            .complete_agent_prompt_with_external_authority(&session_id, &agent_id, None, None)
            .await
            .unwrap();
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let (active, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert!(active.is_none());
        assert_eq!(
            queued.len(),
            2,
            "completion cannot cross the held queue head"
        );
        let (runtime, _recovery) = match path {
            HoldRecoveryPath::Live => {
                connection
                    .execute_batch("DROP TRIGGER reject_trigger_settlement;")
                    .unwrap();
                runtime
                    .dispatch_due_agent_triggers(trigger.next_run_at_ms())
                    .await;
                (runtime, None)
            }
            HoldRecoveryPath::Restart => {
                let events = runtime
                    .owned
                    .durable_state_store
                    .load_events_after(0)
                    .unwrap();
                let (restarted, _) =
                    restart_from_log_cut(&events, events.len(), "held-user-queue-restart").await;
                install_slow_structured_run(
                    &restarted,
                    &session_id,
                    &agent_id,
                    &run_id,
                    "slow-ack",
                );
                let recovery = restarted.spawn_durable_restart_recovery();
                (restarted, Some(recovery))
            }
            HoldRecoveryPath::LiveThenRestart | HoldRecoveryPath::RestartThenRestart => {
                let mut stopped = runtime;
                if matches!(path, HoldRecoveryPath::RestartThenRestart) {
                    let events = stopped
                        .owned
                        .durable_state_store
                        .load_events_after(0)
                        .unwrap();
                    let (first, _) =
                        restart_from_log_cut(&events, events.len(), "held-user-queue-first").await;
                    stopped = first;
                }
                connection
                    .execute_batch("DROP TRIGGER reject_trigger_settlement;")
                    .unwrap();
                // Hold removal is durable; the promotion behind it never ran.
                assert_eq!(stopped.owned.reconcile_inactive_local_steers().len(), 1);
                let session = stopped.owned.session_snapshot(&session_id).unwrap();
                let (active, queued) = stopped
                    .owned
                    .prompt_state_owner
                    .state_parts(&session, &agent_id);
                assert!(active.is_none());
                assert_eq!(queued.len(), 1);
                let events = stopped
                    .owned
                    .durable_state_store
                    .load_events_after(0)
                    .unwrap();
                let (restarted, _) =
                    restart_from_log_cut(&events, events.len(), "held-user-queue-second").await;
                install_slow_structured_run(
                    &restarted,
                    &session_id,
                    &agent_id,
                    &run_id,
                    "slow-ack",
                );
                let recovery = restarted.spawn_durable_restart_recovery();
                (restarted, Some(recovery))
            }
        };
        let jobs = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let jobs = runtime
                    .owned
                    .provider_store
                    .drain_finished_structured_prompt_submit_jobs();
                if !jobs.is_empty() {
                    break jobs;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("maintenance must deliver the user prompt to the provider");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].prompt_id, admitted_prompt_id);
        assert_eq!(jobs[0].provider_run_id, run_id);
        assert!(jobs[0].result.is_ok());
        runtime
            .dispatch_due_agent_triggers(trigger.next_run_at_ms())
            .await;
        assert!(runtime
            .owned
            .provider_store
            .drain_finished_structured_prompt_submit_jobs()
            .is_empty());
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let (active, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert_eq!(active.unwrap().id(), admitted_prompt_id);
        assert!(queued.is_empty());
    }

    // MP-08/MP-10/MP-11: failed appends must preserve the replay barrier.
    #[tokio::test]
    async fn live_settlement_failure_retains_held_firing() {
        settlement_failure_retains_held_firing(false).await;
    }

    #[tokio::test]
    async fn restart_settlement_failure_retains_held_firing() {
        settlement_failure_retains_held_firing(true).await;
    }

    fn assert_restart_cannot_refire(runtime: &KernelRuntimeState, agent_id: &str, at: &str) {
        runtime.owned.reconcile_inactive_local_steers();
        assert!(
            runtime
                .owned
                .session_store
                .write()
                .claim_due_agent_triggers(u64::MAX)
                .is_empty(),
            "{at}: an overdue trigger would fire the steered message again"
        );
        for session in runtime.owned.session_store.list_all_sessions() {
            let (_, queued) = runtime
                .owned
                .prompt_state_owner
                .state_parts(&session, agent_id);
            assert!(queued.is_empty(), "{at}: a held steer must not stay queued");
        }
    }

    fn final_trigger_steer_writes(
        events: &[crate::durable_state::DurableStateEvent],
    ) -> [usize; 2] {
        let removal = events
            .iter()
            .rposition(|event| {
                event.kind == crate::durable_prompt_state::DURABLE_PROMPT_STATE_EVENT_KIND
            })
            .expect("the queue removal must be durable");
        let settlement = events
            .iter()
            .rposition(|event| {
                event.kind == "session.updated" && event.payload["reason"] == "agent_trigger_fired"
            })
            .expect("the trigger settlement must be durable");
        let mut cuts = [removal, settlement];
        cuts.sort_unstable();
        cuts
    }

    // MP-08/MP-09/MP-10: a stop between the two durable writes of a steered
    // firing (queue removal, trigger settlement) must not fire it again.
    #[tokio::test]
    async fn trigger_steer_stop_between_removal_and_settlement_does_not_refire() {
        let config = durable_state_config("trigger-steer-cut");
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
                config, &[],
            )
            .await;
        install_slow_structured_run(&runtime, &session_id, &agent_id, &run_id, "slow-ack");
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        let state = runtime.clone();
        let due = trigger.next_run_at_ms();
        let firing = tokio::spawn(async move { state.dispatch_due_agent_triggers(due).await });
        wait_for_queued_firing(&runtime, &session_id, &agent_id).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !runtime
                .owned
                .provider_store
                .structured_prompt_io_in_flight(&run_id)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the Submit command must reach the mailbox");
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        runtime
            .owned
            .prompt_state_owner
            .complete_active_prompt_only(&session, &agent_id)
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), firing)
            .await
            .unwrap()
            .unwrap();
        let events = runtime
            .owned
            .durable_state_store
            .load_events_after(0)
            .unwrap();
        for cut in final_trigger_steer_writes(&events) {
            let (restarted, _) = restart_from_log_cut(&events, cut, "trigger-steer-restart").await;
            assert_restart_cannot_refire(&restarted, &agent_id, &format!("log cut at {cut}"));
            let recovered = restarted.owned.session_snapshot(&session_id).unwrap();
            let delivery = recovered
                .trigger_deliveries()
                .iter()
                .find(|delivery| delivery.trigger_id() == trigger.id())
                .expect("the firing must remain recorded after recovery");
            assert_eq!(
                delivery.busy_policy(),
                AgentTriggerBusyPolicy::Steer,
                "restart must preserve steering after the once trigger retired"
            );
        }
    }

    #[tokio::test]
    async fn recurring_resend_recovery_preserves_schedule_after_restart() {
        recurring_resend_recovery_preserves_schedule(true).await;
    }

    #[tokio::test]
    async fn recurring_resend_recovery_preserves_schedule_after_append_failure() {
        recurring_resend_recovery_preserves_schedule(false).await;
    }

    // MP-08/MP-10/MP-11: a resend is another delivery, not another scheduled occurrence.
    async fn recurring_resend_recovery_preserves_schedule(restart: bool) {
        let config = durable_state_config("recurring-resend-recovery");
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
                config, &[],
            ).await;
        install_slow_structured_run(&runtime, &session_id, &agent_id, &run_id, "ack-lost");
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Every).await;
        runtime
            .dispatch_due_agent_triggers(trigger.next_run_at_ms())
            .await;
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let original = session.trigger_deliveries()[0].clone();
        let expected = session
            .agent_triggers()
            .iter()
            .find(|t| t.id() == trigger.id())
            .unwrap()
            .clone();
        assert_eq!(expected.fired(), 1);
        let connection = reject_trigger_settlement(&runtime);
        let owner = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        assert!(
            runtime
                .resend_trigger_delivery(
                    crate::local::ResendTriggerDeliveryRequest {
                        session_id: session_id.clone(),
                        prompt_id: original.prompt_id().into(),
                    },
                    owner.owner_user_id()
                )
                .await
                .is_err(),
            "the resend settlement must fail"
        );
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let (_, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        let held = queued
            .iter()
            .find(|prompt| prompt.local_steer_submitted())
            .unwrap();
        let replacement_id = held.id().to_string();
        assert_eq!(held.trigger_resend_of(), Some(original.prompt_id()));
        let recovered = if restart {
            let events = runtime
                .owned
                .durable_state_store
                .load_events_after(0)
                .unwrap();
            restart_from_log_cut(&events, events.len(), "recurring-resend-restarted")
                .await
                .0
        } else {
            connection
                .execute_batch("DROP TRIGGER reject_trigger_settlement;")
                .unwrap();
            runtime.clone()
        };
        recovered
            .dispatch_due_agent_triggers(trigger.next_run_at_ms())
            .await;
        let session = recovered.owned.session_snapshot(&session_id).unwrap();
        let actual = session
            .agent_triggers()
            .iter()
            .find(|t| t.id() == trigger.id())
            .unwrap();
        assert_eq!(
            actual.fired(),
            expected.fired(),
            "resend recovery must not count a new occurrence"
        );
        assert_eq!(
            actual.next_run_at_ms(),
            expected.next_run_at_ms(),
            "resend recovery must preserve the original schedule"
        );
        assert_eq!(
            session
                .trigger_deliveries()
                .iter()
                .find(|d| d.prompt_id() == original.prompt_id())
                .unwrap()
                .state(),
            crate::session::TriggerDeliveryState::Resent
        );
        let replacement = session
            .trigger_deliveries()
            .iter()
            .find(|d| d.prompt_id() == replacement_id)
            .unwrap();
        assert_eq!(
            replacement.state(),
            crate::session::TriggerDeliveryState::NotConfirmed
        );
        let (_, queued) = recovered
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert!(!queued.iter().any(|p| p.id() == replacement_id));
        assert!(recovered
            .owned
            .durable_state_store
            .trigger_firing_settled(&session_id, &replacement_id)
            .unwrap());
    }

    #[tokio::test]
    async fn recurring_steer_settlement_is_idempotent_after_restart() {
        recurring_steer_settlement_is_idempotent(false).await;
    }

    #[tokio::test]
    async fn confirmed_recurring_steer_settlement_is_idempotent_after_restart() {
        recurring_steer_settlement_is_idempotent(true).await;
    }

    async fn recurring_steer_settlement_is_idempotent(confirmed: bool) {
        let config = durable_state_config("recurring-trigger-steer-cut");
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
                config, &[],
            )
            .await;
        install_slow_structured_run(&runtime, &session_id, &agent_id, &run_id, "slow-ack");
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Every).await;
        let state = runtime.clone();
        let due = trigger.next_run_at_ms();
        let firing = tokio::spawn(async move { state.dispatch_due_agent_triggers(due).await });
        wait_for_queued_firing(&runtime, &session_id, &agent_id).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !runtime
                .owned
                .provider_store
                .structured_prompt_io_in_flight(&run_id)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the Submit command must reach the mailbox");
        if !confirmed {
            let session = runtime.owned.session_snapshot(&session_id).unwrap();
            runtime
                .owned
                .prompt_state_owner
                .complete_active_prompt_only(&session, &agent_id)
                .unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), firing)
            .await
            .unwrap()
            .unwrap();
        let events = runtime
            .owned
            .durable_state_store
            .load_events_after(0)
            .unwrap();
        let settled = runtime.owned.session_snapshot(&session_id).unwrap();
        let expected = settled
            .agent_triggers()
            .iter()
            .find(|known| known.id() == trigger.id())
            .unwrap()
            .clone();
        assert_eq!(expected.fired(), 1);
        let cut = events
            .iter()
            .rposition(|event| {
                event.kind == "session.updated" && event.payload["reason"] == "agent_trigger_fired"
            })
            .unwrap()
            + 1;
        let (restarted, _) =
            restart_from_log_cut(&events, cut, "recurring-trigger-steer-restart").await;
        restarted.owned.reconcile_inactive_local_steers();
        let recovered = restarted.owned.session_snapshot(&session_id).unwrap();
        let actual = recovered
            .agent_triggers()
            .iter()
            .find(|known| known.id() == trigger.id())
            .unwrap();
        assert_eq!(
            actual.fired(),
            expected.fired(),
            "a settled recurring firing must not count twice"
        );
        assert_eq!(actual.next_run_at_ms(), expected.next_run_at_ms());
    }

    // MP-08/MP-09/MP-10: the same stop while restart recovery drops a held
    // steer; recovery must be safe to run again after every partial write.
    #[tokio::test]
    async fn restart_recovery_of_a_held_steer_survives_a_stop_between_its_writes() {
        let config = durable_state_config("trigger-steer-held");
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
                config, &[],
            )
            .await;
        install_slow_structured_run(&runtime, &session_id, &agent_id, &run_id, "slow-ack");
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        let state = runtime.clone();
        let due = trigger.next_run_at_ms();
        let firing = tokio::spawn(async move { state.dispatch_due_agent_triggers(due).await });
        wait_for_queued_firing(&runtime, &session_id, &agent_id).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !runtime
                .owned
                .provider_store
                .structured_prompt_io_in_flight(&run_id)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the Submit command must reach the mailbox");
        // The kernel stops here, with the steer handed over and unacknowledged.
        let crashed = runtime
            .owned
            .durable_state_store
            .load_events_after(0)
            .unwrap();
        firing.abort();
        let (first_restart, _) =
            restart_from_log_cut(&crashed, crashed.len(), "trigger-steer-held-1").await;
        first_restart.owned.reconcile_inactive_local_steers();
        let remembered = first_restart
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        assert_eq!(
            remembered
                .trigger_deliveries()
                .iter()
                .map(|delivery| (delivery.state(), delivery.origin(), delivery.agent_id()))
                .collect::<Vec<_>>(),
            vec![(
                crate::session::TriggerDeliveryState::NotConfirmed,
                crate::session::PromptOrigin::Schedule,
                agent_id.as_str()
            )],
            "a firing the restart could not confirm is remembered for the person to resend"
        );
        let recovery = first_restart
            .owned
            .durable_state_store
            .load_events_after(0)
            .unwrap();
        for cut in final_trigger_steer_writes(&recovery) {
            let (second, _) = restart_from_log_cut(&recovery, cut, "trigger-steer-held-2").await;
            assert_restart_cannot_refire(&second, &agent_id, &format!("recovery log cut at {cut}"));
        }
    }

    fn trigger_prompt_history(
        runtime: &KernelRuntimeState,
        session_id: &str,
    ) -> Vec<crate::history::HistoryEvent> {
        runtime
            .owned
            .operational_history_store
            .query_events(crate::history::HistoryEventQuery {
                session_id: Some(session_id.to_string()),
                kind: Some("user_prompt".to_string()),
                ..Default::default()
            })
            .unwrap()
            .into_iter()
            .filter(|event| {
                event.metadata.get("prompt_origin") == Some(&serde_json::json!("schedule"))
            })
            .collect()
    }

    pub(in crate::runtime::state) async fn durable_unconfirmed_trigger_delivery(
        tag: &str,
    ) -> (
        crate::test_support::TestWorktree,
        KernelRuntimeState,
        crate::config::DaemonConfig,
        String,
        String,
        String,
    ) {
        durable_unconfirmed_trigger_delivery_with_config(durable_state_config(tag)).await
    }

    pub(in crate::runtime::state) async fn durable_unconfirmed_trigger_delivery_with_config(
        config: crate::config::DaemonConfig,
    ) -> (
        crate::test_support::TestWorktree,
        KernelRuntimeState,
        crate::config::DaemonConfig,
        String,
        String,
        String,
    ) {
        use crate::session::TriggerDelivery;
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
                config.clone(), &[],
            ).await;
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        runtime
            .owned
            .session_store
            .write()
            .settle_steered_firing(
                &session_id,
                Some(trigger.id()),
                crate::session::unix_epoch_ms(),
                Some(TriggerDelivery::not_confirmed(
                    "original-firing",
                    trigger.id(),
                    &agent_id,
                    crate::session::PromptOrigin::Schedule,
                    "Resend this firing",
                    AgentTriggerBusyPolicy::Queue,
                    crate::session::unix_epoch_ms(),
                )),
            )
            .unwrap();
        runtime
            .owned
            .persist_agent_trigger_session(&session_id, "test_unconfirmed_firing")
            .unwrap();
        (_worktree, runtime, config, session_id, agent_id, run_id)
    }

    // MP-08/MP-10/MP-11: each log prefix must distinguish a reserved resend
    // from a replacement prompt that has actually been durably admitted.
    #[tokio::test]
    async fn resend_log_cut_keeps_unadmitted_firings_resendable() {
        use crate::session::TriggerDeliveryState;
        let (_worktree, runtime, _, session_id, agent_id, _) =
            durable_unconfirmed_trigger_delivery("trigger-resend-cut").await;
        let before = runtime
            .owned
            .durable_state_store
            .load_events_after(0)
            .unwrap()
            .len();
        let owner = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        runtime
            .resend_trigger_delivery(
                crate::local::ResendTriggerDeliveryRequest {
                    session_id: session_id.clone(),
                    prompt_id: "original-firing".into(),
                },
                owner.owner_user_id(),
            )
            .await
            .unwrap();
        let events = runtime
            .owned
            .durable_state_store
            .load_events_after(0)
            .unwrap();
        let admission = events
            .iter()
            .enumerate()
            .skip(before)
            .find(|(_, event)| {
                event.kind == crate::durable_prompt_state::DURABLE_PROMPT_STATE_EVENT_KIND
            })
            .map(|(index, _)| index)
            .expect("resend must admit a replacement prompt");
        for cut in before..=events.len() {
            let (restarted, _) = restart_from_log_cut(&events, cut, "resend-cut-restart").await;
            let session = restarted.owned.session_snapshot(&session_id).unwrap();
            let expected = if cut > admission {
                TriggerDeliveryState::Resent
            } else {
                TriggerDeliveryState::NotConfirmed
            };
            assert_eq!(
                session.trigger_deliveries()[0].state(),
                expected,
                "log cut at {cut}: only durable prompt admission may consume a resend"
            );
            let result = restarted
                .resend_trigger_delivery(
                    crate::local::ResendTriggerDeliveryRequest {
                        session_id: session_id.clone(),
                        prompt_id: "original-firing".into(),
                    },
                    owner.owner_user_id(),
                )
                .await;
            assert_eq!(
                result.is_ok(),
                cut <= admission,
                "log cut at {cut}: only unadmitted firings may be resent"
            );
            let session = restarted.owned.session_snapshot(&session_id).unwrap();
            let (_, queued) = restarted
                .owned
                .prompt_state_owner
                .state_parts(&session, &agent_id);
            assert_eq!(
                queued.len(),
                1,
                "each prefix admits exactly one replacement"
            );
        }
    }

    #[tokio::test]
    async fn resend_admission_survives_stale_sessions_completed_prompts_and_snapshots() {
        use crate::session::TriggerDeliveryState::{NotConfirmed, Resent};
        for (stale_event, snapshot) in [(false, false), (true, false), (false, true), (true, true)]
        {
            let (_worktree, runtime, config, session_id, agent_id, _) =
                durable_unconfirmed_trigger_delivery("resend-stale-state").await;
            let owner = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
            let request = crate::local::ResendTriggerDeliveryRequest {
                session_id: session_id.clone(),
                prompt_id: "original-firing".into(),
            };
            runtime
                .resend_trigger_delivery(request.clone(), owner.owner_user_id())
                .await
                .unwrap();
            let session = runtime.owned.session_snapshot(&session_id).unwrap();
            let (_, queued) = runtime
                .owned
                .prompt_state_owner
                .state_parts(&session, &agent_id);
            runtime
                .owned
                .prompt_state_owner
                .remove_queued_prompt(&session, &agent_id, queued.front().unwrap().id())
                .unwrap();
            let (active, queued) = runtime
                .owned
                .prompt_state_owner
                .state_parts(&session, &agent_id);
            runtime
                .owned
                .mirror_prompt_owner_agent_state(&session_id, &agent_id, active, queued)
                .unwrap();
            // A projection captured before the admission update must not
            // override the durable receipt, even without a queued replacement.
            runtime
                .owned
                .session_store
                .write()
                .move_trigger_delivery(&session_id, "original-firing", Resent, NotConfirmed)
                .unwrap();
            if stale_event {
                runtime
                    .owned
                    .persist_agent_trigger_session(&session_id, "stale_resend_projection")
                    .unwrap();
            }
            if snapshot {
                runtime
                    .with_app_side_effect(|app| app.save_durable_state_snapshot())
                    .await
                    .unwrap();
            }
            // Copy the committed SQLite state (including its checkpoint) to
            // simulate a restart without the original fixture's live lease.
            let mut restart_config = config.clone();
            let restart_path = durable_state_config("resend-snapshot-copy")
                .user_config
                .state
                .path
                .unwrap();
            rusqlite::Connection::open(runtime.owned.durable_state_store.path())
                .unwrap()
                .execute("VACUUM INTO ?1", rusqlite::params![restart_path])
                .unwrap();
            restart_config.user_config.state.path = Some(restart_path);
            let app = crate::app::DaemonApp::bootstrap(restart_config).unwrap();
            let restarted =
                super::super::local_prompt_dispatch_runtime::tests::owned_runtime_state(&Arc::new(
                    tokio::sync::Mutex::new(app),
                ))
                .await;
            assert_eq!(
                restarted
                    .owned
                    .session_snapshot(&session_id)
                    .unwrap()
                    .trigger_deliveries()[0]
                    .state(),
                Resent
            );
            assert!(restarted.resend_trigger_delivery(request, owner.owner_user_id()).await.is_err(),
                "completed admission must remain consumed (stale={stale_event}, snapshot={snapshot})");
        }
    }

    #[tokio::test]
    async fn resend_admission_failure_rolls_back_receipt_and_allows_one_retry() {
        use crate::session::TriggerDeliveryState::{NotConfirmed, Resent};
        for idle in [false, true] {
            let (_worktree, runtime, _, session_id, agent_id, run_id) =
                durable_unconfirmed_trigger_delivery("resend-admission-failure").await;
            let owner = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
            if idle {
                let session = runtime.owned.session_snapshot(&session_id).unwrap();
                runtime
                    .owned
                    .prompt_state_owner
                    .complete_active_prompt_only(&session, &agent_id)
                    .unwrap();
                let (active, queued) = runtime
                    .owned
                    .prompt_state_owner
                    .state_parts(&session, &agent_id);
                runtime
                    .owned
                    .mirror_prompt_owner_agent_state(&session_id, &agent_id, active, queued)
                    .unwrap();
                install_slow_structured_run(&runtime, &session_id, &agent_id, &run_id, "slow-ack");
            }

            let request = crate::local::ResendTriggerDeliveryRequest {
                session_id: session_id.clone(),
                prompt_id: "original-firing".into(),
            };
            let before = runtime
                .owned
                .durable_state_store
                .load_events_after(0)
                .unwrap()
                .len();
            let connection =
                rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
            connection
                .execute_batch(
                    "CREATE TRIGGER reject_resend_receipt BEFORE INSERT ON durable_trigger_resends
            BEGIN SELECT RAISE(FAIL, 'injected resend admission failure'); END;",
                )
                .unwrap();
            let error = runtime
                .resend_trigger_delivery(request.clone(), owner.owner_user_id())
                .await
                .unwrap_err();
            assert!(error
                .to_string()
                .contains("injected resend admission failure"));
            assert!(!runtime
                .owned
                .durable_state_store
                .trigger_resend_admitted(&session_id, "original-firing")
                .unwrap());
            assert_eq!(
                runtime
                    .owned
                    .session_snapshot(&session_id)
                    .unwrap()
                    .trigger_deliveries()[0]
                    .state(),
                NotConfirmed
            );
            let events = runtime
                .owned
                .durable_state_store
                .load_events_after(0)
                .unwrap();
            assert!(
                !events.iter().skip(before).any(|event| event.kind
                    == crate::durable_prompt_state::DURABLE_PROMPT_STATE_EVENT_KIND),
                "failed receipt must roll back its prompt admission event too"
            );
            let session = runtime.owned.session_snapshot(&session_id).unwrap();
            let (active, queued) = runtime
                .owned
                .prompt_state_owner
                .state_parts(&session, &agent_id);
            assert!(
                queued.is_empty(),
                "failed admission must discard the phantom queue entry"
            );
            assert_eq!(
                active.is_none(),
                idle,
                "idle admission must roll back its phantom active prompt"
            );
            connection
                .execute_batch("DROP TRIGGER reject_resend_receipt;")
                .unwrap();
            runtime
                .resend_trigger_delivery(request.clone(), owner.owner_user_id())
                .await
                .unwrap();
            assert!(runtime
                .owned
                .durable_state_store
                .trigger_resend_admitted(&session_id, "original-firing")
                .unwrap());
            let session = runtime.owned.session_snapshot(&session_id).unwrap();
            assert_eq!(session.trigger_deliveries()[0].state(), Resent);
            let (active, queued) = runtime
                .owned
                .prompt_state_owner
                .state_parts(&session, &agent_id);
            if idle {
                assert!(
                    active.is_some(),
                    "idle retry must start through normal admission"
                );
                assert!(queued.is_empty());
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let finished = runtime
                            .owned
                            .provider_store
                            .drain_finished_structured_prompt_submit_jobs();
                        if !finished.is_empty() {
                            assert_eq!(finished.len(), 1, "retry must dispatch exactly once");
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                assert_eq!(trigger_prompt_history(&runtime, &session_id).len(), 1);
            } else {
                assert_eq!(queued.len(), 1, "retry must admit one queued replacement");
            }
            assert!(runtime
                .resend_trigger_delivery(request, owner.owner_user_id())
                .await
                .is_err());
        }
    }

    // The firing is in the transcript, labelled with its origin, before the
    // provider has the message, so it exists even if the kernel stops first.
    #[tokio::test]
    async fn trigger_steer_is_in_the_transcript_before_the_provider_write() {
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            runtime_with_active_prompt().await;
        install_slow_structured_run(&runtime, &session_id, &agent_id, &run_id, "slow-ack");
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        let state = runtime.clone();
        let due = trigger.next_run_at_ms();
        let firing = tokio::spawn(async move { state.dispatch_due_agent_triggers(due).await });
        wait_for_queued_firing(&runtime, &session_id, &agent_id).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !runtime
                .owned
                .provider_store
                .structured_prompt_io_in_flight(&run_id)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the Submit command must reach the mailbox");
        assert_eq!(
            trigger_prompt_history(&runtime, &session_id).len(),
            1,
            "the labelled firing must be recorded while the provider still holds the write"
        );
        firing.abort();
    }

    async fn unconfirmed_trigger_steer() -> (
        crate::test_support::TestWorktree,
        KernelRuntimeState,
        String,
        String,
        crate::session::TriggerDelivery,
    ) {
        let (worktree, runtime, session_id, agent_id, _, _, run_id) =
            runtime_with_active_prompt().await;
        install_slow_structured_run(&runtime, &session_id, &agent_id, &run_id, "ack-lost");
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            runtime.dispatch_due_agent_triggers(trigger.next_run_at_ms()),
        )
        .await
        .expect("the firing must settle");
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let delivery = session
            .trigger_deliveries()
            .first()
            .cloned()
            .expect("an unconfirmed steer must be remembered for the person to resend");
        (worktree, runtime, session_id, agent_id, delivery)
    }

    // An unconfirmed firing says so and can be resent exactly once, as one
    // more turn that follows the trigger's busy policy.
    #[tokio::test]
    async fn unconfirmed_trigger_steer_is_resent_exactly_once() {
        let (_worktree, runtime, session_id, agent_id, delivery) =
            unconfirmed_trigger_steer().await;
        assert_eq!(
            delivery.state(),
            crate::session::TriggerDeliveryState::NotConfirmed
        );
        assert_eq!(delivery.origin(), crate::session::PromptOrigin::Schedule);
        assert_eq!(delivery.busy_policy(), AgentTriggerBusyPolicy::Steer);
        let owner = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        let resend = |runtime: &KernelRuntimeState| {
            let request = crate::local::ResendTriggerDeliveryRequest {
                session_id: session_id.clone(),
                prompt_id: delivery.prompt_id().to_string(),
            };
            let runtime = runtime.clone();
            let owner_user_id = owner.owner_user_id().to_string();
            async move {
                runtime
                    .resend_trigger_delivery(request, &owner_user_id)
                    .await
            }
        };
        let submits = || {
            runtime
                .owned
                .provider_store
                .drain_finished_structured_prompt_submit_jobs()
                .len()
        };
        submits();
        resend(&runtime).await.expect("the first resend succeeds");
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        assert_eq!(
            session.trigger_deliveries()[0].state(),
            crate::session::TriggerDeliveryState::Resent
        );
        assert!(
            resend(&runtime).await.is_err(),
            "a second resend must not send it again"
        );
        let (active, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        let resent_firings = active
            .iter()
            .chain(queued.iter())
            .filter(|prompt| prompt.prompt() == delivery.prompt())
            .count()
            + session
                .trigger_deliveries()
                .iter()
                .filter(|known| known.prompt_id() != delivery.prompt_id())
                .count();
        assert_eq!(resent_firings, 1, "exactly one more delivery");
        assert_eq!(
            trigger_prompt_history(&runtime, &session_id).len(),
            2,
            "the resend shows as one more labelled prompt"
        );
    }

    // MP-08/MP-09/MP-10: a steer whose acknowledgement is lost may already be
    // with the provider, so the agent must see the firing once, never twice.
    #[tokio::test]
    async fn trigger_steer_with_lost_acknowledgement_is_not_replayed() {
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            runtime_with_active_prompt().await;
        let request = crate::provider::LaunchProviderRequest::new(
            &session_id,
            "dev-stub",
            "slow-structured",
            "default",
            "model",
        )
        .with_agent_id(&agent_id);
        let mut run = crate::provider::RuntimeProviderRun::new(
            &run_id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "ack-lost".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: Some("test".into()),
            },
        );
        run.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        runtime.owned.provider_run_projection.update(run);
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            runtime.dispatch_due_agent_triggers(trigger.next_run_at_ms()),
        )
        .await
        .expect("the firing must settle");
        let submits = || {
            runtime
                .owned
                .provider_store
                .drain_finished_structured_prompt_submit_jobs()
                .len()
        };
        assert_eq!(submits(), 1, "the provider is shown the steer once");
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let (_, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert!(
            queued.is_empty(),
            "an unconfirmed steer must not stay queued for a second delivery"
        );
        assert!(session
            .durable_runtime_snapshot()
            .queued_prompts_for_agent(&agent_id)
            .is_none_or(|prompts| prompts.is_empty()));
        assert!(session.agent_triggers().is_empty());
        runtime
            .owned
            .prompt_state_owner
            .complete_active_prompt_only(&session, &agent_id)
            .unwrap();
        runtime
            .spawn_idle_local_prompt_queue_promotion(&session_id, &agent_id)
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(submits(), 0, "the next turn must not send the firing again");
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let (active, _) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert!(
            active.is_none(),
            "the next turn must not replay the steered firing"
        );
    }

    // MP-08/MP-09/MP-10: mailbox admission is not provider acceptance.
    #[tokio::test]
    async fn trigger_steer_rejected_after_mailbox_admission_runs_on_next_turn() {
        let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
            runtime_with_active_prompt().await;
        let original_run = runtime.owned.provider_store.get_run(&run_id).unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            &session_id,
            "codex",
            "codex",
            "default",
            "gpt-6-luna",
        )
        .with_agent_id(&agent_id);
        let mut run = crate::provider::RuntimeProviderRun::new(
            &run_id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "rejected-after-mailbox".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: Some("test".into()),
            },
        );
        run.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        runtime.owned.provider_run_projection.update(run);
        let trigger = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
        runtime
            .dispatch_due_agent_triggers(trigger.next_run_at_ms())
            .await;
        // The actor accepts Submit in its mailbox, then its Codex adapter
        // rejects it because there is no live native turn/runtime.
        let finished = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let mut jobs = runtime
                    .owned
                    .provider_store
                    .drain_finished_structured_prompt_submit_jobs();
                if let Some(job) = jobs.pop() {
                    break job;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            finished.result.is_err(),
            "the actor must reject after enqueue succeeds"
        );
        runtime
            .owned
            .provider_store
            .schedule_finished_structured_prompt_submit_retry(finished);
        runtime.reap_structured_prompt_jobs_and_dispatch();
        let session = runtime.owned.session_snapshot(&session_id).unwrap();
        let (_, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert_eq!(
            queued.len(),
            1,
            "the rejected one-shot firing must remain pending"
        );
        assert_eq!(queued[0].agent_trigger_id(), Some(trigger.id()));
        assert_eq!(
            session
                .durable_runtime_snapshot()
                .queued_prompts_for_agent(&agent_id)
                .unwrap()
                .len(),
            1
        );
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(original_run.clone());
        runtime.owned.provider_run_projection.update(original_run);
        runtime
            .owned
            .prompt_state_owner
            .complete_active_prompt_only(&session, &agent_id)
            .unwrap();
        runtime
            .spawn_idle_local_prompt_queue_promotion(&session_id, &agent_id)
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let session = runtime.owned.session_snapshot(&session_id).unwrap();
                let (active, queued) = runtime
                    .owned
                    .prompt_state_owner
                    .state_parts(&session, &agent_id);
                if let Some(active) = active {
                    assert_eq!(active.agent_trigger_id(), Some(trigger.id()));
                    assert!(queued.is_empty());
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("retained firing must start on the next turn");
    }

    #[tokio::test]
    async fn trigger_disabled_while_an_earlier_claim_is_blocked_is_not_admitted() {
        for reenable in [false, true] {
            let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
                runtime_with_active_prompt().await;
            let first = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Every).await;
            let second = schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
            let held = runtime.provider_runtime_lanes.acquire(&run_id).await;
            let firing_runtime = runtime.clone();
            let due = first.next_run_at_ms().max(second.next_run_at_ms());
            let firing =
                tokio::spawn(async move { firing_runtime.dispatch_due_agent_triggers(due).await });
            wait_for_queued_firing(&runtime, &session_id, &agent_id).await;
            let owner = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
            for enabled in [Some(false), reenable.then_some(true)]
                .into_iter()
                .flatten()
            {
                runtime
                    .update_agent_trigger(
                        crate::local::UpdateAgentTriggerRequest {
                            session_id: session_id.clone(),
                            trigger_id: second.id().into(),
                            busy_policy: None,
                            prompt: None,
                            enabled: Some(enabled),
                        },
                        owner.owner_user_id(),
                    )
                    .await
                    .unwrap();
            }
            drop(held);
            tokio::time::timeout(std::time::Duration::from_secs(5), firing)
                .await
                .unwrap()
                .unwrap();
            let snapshot = runtime.owned.session_snapshot(&session_id).unwrap();
            let second = snapshot
                .agent_triggers()
                .iter()
                .find(|t| t.id() == second.id())
                .expect("an invalidated one-shot claim must not delete its trigger");
            assert_eq!(second.enabled(), reenable);
            assert_eq!(second.fired(), 0);
            assert_eq!(second.coalesced(), 0);
            let (_, queued) = runtime
                .owned
                .prompt_state_owner
                .state_parts(&snapshot, &agent_id);
            assert!(queued
                .iter()
                .all(|p| p.agent_trigger_id() != Some(second.id())));
        }
    }
    #[tokio::test]
    async fn trigger_stale_claim_cannot_queue_after_disable_or_disable_then_enable() {
        for reenable in [false, true] {
            let (_worktree, runtime, session_id, agent_id, _, _, _) =
                runtime_with_active_prompt().await;
            let trigger =
                schedule(&runtime, &session_id, &agent_id, AgentTriggerRepeat::Once).await;
            let owner = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
            runtime
                .update_agent_trigger(
                    crate::local::UpdateAgentTriggerRequest {
                        session_id: session_id.clone(),
                        trigger_id: trigger.id().into(),
                        busy_policy: Some(AgentTriggerBusyPolicy::Queue),
                        prompt: None,
                        enabled: None,
                    },
                    owner.owner_user_id(),
                )
                .await
                .unwrap();
            let claims = runtime
                .owned
                .session_store
                .write()
                .claim_due_agent_triggers(trigger.next_run_at_ms());
            assert_eq!(claims.len(), 1);
            for enabled in [Some(false), reenable.then_some(true)]
                .into_iter()
                .flatten()
            {
                runtime
                    .update_agent_trigger(
                        crate::local::UpdateAgentTriggerRequest {
                            session_id: session_id.clone(),
                            trigger_id: trigger.id().into(),
                            busy_policy: None,
                            prompt: None,
                            enabled: Some(enabled),
                        },
                        owner.owner_user_id(),
                    )
                    .await
                    .unwrap();
            }
            runtime.fire_agent_trigger(&claims[0]).await.unwrap();
            let session = runtime
                .owned
                .session_store
                .get_session(&session_id)
                .unwrap();
            let (_, queued) = runtime
                .owned
                .prompt_state_owner
                .state_parts(&session, &agent_id);
            assert!(
                queued.is_empty(),
                "a disabled or invalidated claim cannot admit a prompt"
            );
        }
    }
}
