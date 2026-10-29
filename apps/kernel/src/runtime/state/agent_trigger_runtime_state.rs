use super::*;
use crate::session::{
    AgentTrigger, AgentTriggerBusyPolicy, AgentTriggerDispatch, AgentTriggerOutcome,
    AgentTriggerRepeat,
};

enum AgentTriggerFiring {
    Fired {
        outcome: AgentTriggerOutcome,
        steer_refusal: Option<String>,
    },
    Coalesced,
    AgentGone,
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

    pub(crate) async fn create_agent_prompt_schedule(
        &self,
        request: crate::local::CreateAgentPromptScheduleRequest,
        caller_user_id: &str,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let repeat = match request.kind {
            crate::session::AgentPromptScheduleKind::Once => AgentTriggerRepeat::Once,
            crate::session::AgentPromptScheduleKind::Recurring => AgentTriggerRepeat::Every,
        };
        let (trigger, session) = self.create_agent_schedule_trigger(
            &request.session_id,
            &request.agent_id,
            caller_user_id,
            request.interval_seconds,
            repeat,
            AgentTriggerBusyPolicy::Queue,
            request.prompt,
        )?;
        let schedule = trigger
            .legacy_schedule()
            .expect("a new schedule trigger is enabled");
        Ok(crate::local::LocalDaemonResponse::AgentPromptScheduleCreated { schedule, session })
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

    pub(crate) async fn cancel_agent_prompt_schedule(
        &self,
        request: crate::local::CancelAgentPromptScheduleRequest,
        caller_user_id: &str,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let schedule = self
            .owned
            .session_store
            .get_session(&request.session_id)?
            .agent_triggers()
            .iter()
            .find(|trigger| trigger.id() == request.schedule_id)
            .and_then(AgentTrigger::legacy_schedule)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "agent prompt schedule cancel",
                message: format!(
                    "no /wait schedule `{}` in session `{}`; a disabled trigger is deleted with DeleteAgentTrigger",
                    request.schedule_id, request.session_id
                ),
            })?;
        let (_, session) = self.delete_agent_trigger_by_id(
            &request.session_id,
            &request.schedule_id,
            caller_user_id,
        )?;
        Ok(crate::local::LocalDaemonResponse::AgentPromptScheduleCancelled { schedule, session })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Both the trigger and legacy /wait requests pass the same distinct fields."
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
            // A /wait schedule stored before triggers were deleted with their
            // agent can outlive it; any session member may remove it.
            Err(DaemonError::AgentNotFound { .. }) => Ok(()),
            result => result.map(|_| ()),
        }
    }

    pub(crate) async fn dispatch_due_agent_triggers(&self, now_ms: u64) {
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
                Ok(AgentTriggerFiring::Skipped) => continue,
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
            // before that must not start a turn.
            Ok(agent) if agent.archived() => return Ok(AgentTriggerFiring::Skipped),
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
        .with_hidden_system_context(crate::prompt_assembly::attach_prompt_manifest_entry(
            scheduled_context,
            &scheduled_manifest_entry,
        ));
        let submission = self
            .submit_prepared_agent_trigger_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: dispatch.session_id.clone(),
                    prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                dispatch,
            )
            .await;
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
        if dispatch.busy_policy == AgentTriggerBusyPolicy::Queue {
            return Ok(AgentTriggerFiring::Fired {
                outcome: AgentTriggerOutcome::Queued,
                steer_refusal: None,
            });
        }
        // Steer the queued firing exactly as a person steers a queued prompt,
        // so local and leased agents share one path and its receipts. A
        // refused steer leaves the firing queued and says why.
        match self
            .steer_agent_trigger_prompt(
                &dispatch.session_id,
                &dispatch.agent_id,
                &source_attachment_id,
                &queued_prompt_id,
            )
            .await
        {
            Ok(_) => Ok(AgentTriggerFiring::Fired {
                outcome: AgentTriggerOutcome::Steered,
                steer_refusal: None,
            }),
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
mod tests {
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
