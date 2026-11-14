use super::*;
use crate::session::{AgentSchedule, TriggerDelivery, TriggerDeliveryState};

impl SessionService {
    pub fn next_scheduled_runtime_wake_at_ms(&self) -> Option<u64> {
        self.store
            .non_ended_sessions()
            .flat_map(|session| {
                let watchdogs = session
                    .workflow_watchdogs()
                    .iter()
                    .filter(|watchdog| {
                        watchdog.enabled()
                            && watchdog
                                .max_wakeups()
                                .is_none_or(|limit| watchdog.wakeups_executed() < limit)
                    })
                    .map(|watchdog| watchdog.next_run_at_ms());
                let agent_triggers = session
                    .agent_triggers()
                    .iter()
                    .filter(|trigger| {
                        matches!(
                            trigger.source(),
                            crate::session::AgentTriggerSource::Schedule { .. }
                        ) && trigger.enabled()
                            && !trigger.done()
                            && !trigger.dispatch_in_flight()
                    })
                    .map(|trigger| trigger.next_run_at_ms());
                watchdogs.chain(agent_triggers)
            })
            .min()
    }

    pub fn create_agent_schedule_trigger(
        &mut self,
        session_id: &str,
        agent_id: &str,
        created_by_user_id: &str,
        schedule: AgentSchedule,
        busy_policy: AgentTriggerBusyPolicy,
        prompt: Option<String>,
    ) -> Result<AgentTrigger, DaemonError> {
        let refuse = |message: String| DaemonError::LocalTransport {
            operation: "agent trigger create",
            message,
        };
        schedule.trigger.validate().map_err(refuse)?;
        match (schedule.repeat, schedule.max_runs) {
            (_, Some(0)) => {
                return Err(refuse(
                    "Max runs must be a positive integer or null.".to_string(),
                ))
            }
            (AgentTriggerRepeat::Once, Some(_)) => {
                return Err(refuse(
                    "a one-time schedule fires once; it takes no max runs".to_string(),
                ))
            }
            _ => {}
        }
        self.get_session(session_id)?;
        let prompt = prompt
            .map(|prompt| prompt.trim().to_string())
            .filter(|prompt| !prompt.is_empty())
            .unwrap_or_else(|| {
                crate::prompt_assembly::render_configured_prompt(
                    "workflow/schedule-continuation",
                    crate::prompt_assembly::bundled_prompt_template(
                        "workflow/schedule-continuation",
                    )
                    .expect("workflow schedule prompt must be registered"),
                    &[],
                )
            });
        let trigger = AgentTrigger::schedule(
            self.next_agent_trigger_id(),
            agent_id,
            created_by_user_id,
            schedule,
            busy_policy,
            prompt,
            unix_epoch_ms(),
        )
        .map_err(refuse)?;
        Ok(self
            .agent_trigger_session_mut(session_id)?
            .add_agent_trigger(trigger))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "An HTTP trigger is created from distinct request fields and the caller identity."
    )]
    pub(crate) fn create_agent_http_trigger(
        &mut self,
        session_id: &str,
        agent_id: &str,
        owner: &str,
        port: std::num::NonZeroU16,
        options: crate::session::AgentHttpOptions,
        policy: AgentTriggerBusyPolicy,
        prompt: Option<String>,
    ) -> Result<AgentTrigger, DaemonError> {
        self.get_session(session_id)?;
        let prompt = prompt.filter(|p| !p.trim().is_empty()).unwrap_or_else(|| "A request arrived through your local HTTP trigger. Decide what to do with its input.".into());
        let trigger = AgentTrigger::http(
            self.next_agent_trigger_id(),
            agent_id,
            owner,
            port,
            options,
            policy,
            prompt,
            unix_epoch_ms(),
        );
        Ok(self
            .agent_trigger_session_mut(session_id)?
            .add_agent_trigger(trigger))
    }

    /// The notification trigger of an agent: the picker's name for the source is kept for display.
    #[expect(
        clippy::too_many_arguments,
        reason = "A notification trigger is created from distinct request fields and the caller identity."
    )]
    pub(crate) fn create_agent_notification_trigger(
        &mut self,
        session_id: &str,
        agent_id: &str,
        owner: &str,
        source: crate::session::AgentTriggerNotificationSource,
        subscription_id: String,
        display_name: String,
        prompt: Option<String>,
    ) -> Result<AgentTrigger, DaemonError> {
        self.get_session(session_id)?;
        let prompt = prompt
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| {
                format!(
                    "A notification from {display_name} arrived. Decide what to do with its input."
                )
            });
        let trigger = AgentTrigger::notification(
            self.next_agent_trigger_id(),
            agent_id,
            owner,
            source,
            subscription_id,
            display_name,
            prompt,
            unix_epoch_ms(),
        );
        Ok(self
            .agent_trigger_session_mut(session_id)?
            .add_agent_trigger(trigger))
    }

    /// Claims the trigger for one firing of an outside input; `None` while it is off or mid-firing.
    pub(crate) fn claim_input_agent_trigger(
        &mut self,
        session_id: &str,
        trigger_id: &str,
        kind: crate::session::TriggerInputKind,
        payload: serde_json::Value,
    ) -> Result<Option<AgentTriggerDispatch>, DaemonError> {
        let session = self.agent_trigger_session_mut(session_id)?;
        let trigger = session
            .agent_triggers_mut()
            .iter_mut()
            .find(|t| t.id() == trigger_id)
            .ok_or_else(|| agent_trigger_not_found(session_id, trigger_id))?;
        if !trigger.claim_input_dispatch(kind) {
            return Ok(None);
        }
        Ok(Some(AgentTriggerDispatch {
            payload: Some(payload),
            input_kind: kind,
            claim_generation: trigger.dispatch_generation(),
            session_id: session_id.into(),
            trigger_id: trigger.id().into(),
            agent_id: trigger.agent_id().into(),
            prompt: trigger.prompt().into(),
            busy_policy: trigger.busy_policy(),
        }))
    }

    pub fn update_agent_trigger(
        &mut self,
        session_id: &str,
        trigger_id: &str,
        busy_policy: Option<AgentTriggerBusyPolicy>,
        prompt: Option<String>,
        enabled: Option<bool>,
    ) -> Result<AgentTrigger, DaemonError> {
        let prompt = match prompt.map(|prompt| prompt.trim().to_string()) {
            Some(prompt) if prompt.is_empty() => {
                return Err(DaemonError::LocalTransport {
                    operation: "agent trigger update",
                    message: "trigger prompt must not be empty; omit it to keep the current one"
                        .to_string(),
                });
            }
            prompt => prompt,
        };
        let trigger = self
            .agent_trigger_session_mut(session_id)?
            .agent_triggers_mut()
            .iter_mut()
            .find(|trigger| trigger.id() == trigger_id)
            .ok_or_else(|| agent_trigger_not_found(session_id, trigger_id))?;
        if busy_policy.is_some()
            && matches!(
                trigger.source(),
                crate::session::AgentTriggerSource::Notification { .. }
            )
        {
            return Err(DaemonError::LocalTransport {
                operation: "agent trigger update",
                message: "a notification trigger always queues; it has no busy policy".to_string(),
            });
        }
        trigger.update(busy_policy, prompt, enabled, unix_epoch_ms());
        Ok(trigger.clone())
    }

    /// Sets the availability of each listed notification trigger; true when any changed.
    pub(crate) fn set_notification_trigger_availability(
        &mut self,
        session_id: &str,
        availability: &[(String, bool)],
    ) -> Result<bool, DaemonError> {
        let session = self.agent_trigger_session_mut(session_id)?;
        let mut changed = false;
        for trigger in session.agent_triggers_mut() {
            if let Some((_, available)) = availability.iter().find(|(id, _)| id == trigger.id()) {
                changed |= trigger.set_notification_source_available(*available);
            }
        }
        Ok(changed)
    }

    pub fn delete_agent_trigger(
        &mut self,
        session_id: &str,
        trigger_id: &str,
    ) -> Result<AgentTrigger, DaemonError> {
        // Deleting stops future firings. Prompts already accepted by the normal
        // queue remain available to complete or be cancelled explicitly.
        self.agent_trigger_session_mut(session_id)?
            .remove_agent_trigger(trigger_id)
            .ok_or_else(|| agent_trigger_not_found(session_id, trigger_id))
    }

    pub fn remove_agent_triggers_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<AgentTrigger>, DaemonError> {
        Ok(self
            .agent_trigger_session_mut(session_id)?
            .remove_agent_triggers_for_agent(agent_id))
    }

    pub fn claim_due_agent_triggers(&mut self, now_ms: u64) -> Vec<AgentTriggerDispatch> {
        let mut dispatches = Vec::new();
        for session in self.store.non_ended_sessions_mut() {
            let session_id = session.id().to_string();
            for trigger in session.agent_triggers_mut() {
                if trigger.claim_dispatch(now_ms) {
                    dispatches.push(AgentTriggerDispatch {
                        payload: None,
                        input_kind: crate::session::TriggerInputKind::Http,
                        claim_generation: trigger.dispatch_generation(),
                        session_id: session_id.clone(),
                        trigger_id: trigger.id().to_string(),
                        agent_id: trigger.agent_id().to_string(),
                        prompt: trigger.prompt().to_string(),
                        busy_policy: trigger.busy_policy(),
                    });
                }
            }
        }
        dispatches
    }

    pub fn mark_agent_trigger_fired(
        &mut self,
        session_id: &str,
        trigger_id: &str,
        now_ms: u64,
        outcome: AgentTriggerOutcome,
        steer_refusal: Option<String>,
    ) -> Result<Option<AgentTrigger>, DaemonError> {
        self.settle_agent_trigger(session_id, trigger_id, |trigger| {
            trigger.mark_fired(now_ms, outcome, steer_refusal)
        })
    }

    pub(crate) fn note_agent_trigger_http_firing(
        &mut self,
        session_id: &str,
        trigger_id: &str,
        prompt_id: &str,
    ) -> Result<(), DaemonError> {
        if let Some(trigger) = self
            .agent_trigger_session_mut(session_id)?
            .agent_triggers_mut()
            .iter_mut()
            .find(|trigger| trigger.id() == trigger_id)
        {
            trigger.note_http_firing(prompt_id);
        }
        Ok(())
    }

    pub fn mark_agent_trigger_failed(
        &mut self,
        session_id: &str,
        trigger_id: &str,
        now_ms: u64,
        error: String,
    ) -> Result<Option<AgentTrigger>, DaemonError> {
        let session = self.agent_trigger_session_mut(session_id)?;
        let Some(trigger) = session
            .agent_triggers_mut()
            .iter_mut()
            .find(|trigger| trigger.id() == trigger_id)
        else {
            return Ok(None);
        };
        trigger.mark_failed(now_ms, error);
        Ok(Some(trigger.clone()))
    }

    /// Releases a claim whose agent could not take the firing; a one-shot
    /// trigger stays pending.
    pub(crate) fn defer_agent_trigger(
        &mut self,
        session_id: &str,
        trigger_id: &str,
        now_ms: u64,
    ) -> Result<Option<AgentTrigger>, DaemonError> {
        let session = self.agent_trigger_session_mut(session_id)?;
        let Some(trigger) = session
            .agent_triggers_mut()
            .iter_mut()
            .find(|trigger| trigger.id() == trigger_id)
        else {
            return Ok(None);
        };
        trigger.defer_dispatch(now_ms);
        Ok(Some(trigger.clone()))
    }

    pub(crate) fn mark_agent_trigger_coalesced(
        &mut self,
        session_id: &str,
        trigger_id: &str,
        now_ms: u64,
    ) -> Result<Option<AgentTrigger>, DaemonError> {
        self.settle_agent_trigger(session_id, trigger_id, |trigger| {
            trigger.mark_coalesced(now_ms)
        })
    }

    /// Settles a steered firing in one change: the trigger counts as fired and
    /// an unconfirmed delivery is remembered, so a single durable write covers
    /// both.
    pub fn settle_steered_firing(
        &mut self,
        session_id: &str,
        trigger_id: Option<&str>,
        now_ms: u64,
        unconfirmed: Option<TriggerDelivery>,
    ) -> Result<(), DaemonError> {
        if let Some(trigger_id) = trigger_id {
            self.mark_agent_trigger_fired(
                session_id,
                trigger_id,
                now_ms,
                AgentTriggerOutcome::Steered,
                None,
            )?;
        }
        if let Some(delivery) = unconfirmed {
            self.agent_trigger_session_mut(session_id)?
                .note_unconfirmed_trigger_delivery(delivery);
        }
        Ok(())
    }

    pub fn move_trigger_delivery(
        &mut self,
        session_id: &str,
        prompt_id: &str,
        from: TriggerDeliveryState,
        to: TriggerDeliveryState,
    ) -> Result<Option<TriggerDelivery>, DaemonError> {
        Ok(self
            .agent_trigger_session_mut(session_id)?
            .move_trigger_delivery(prompt_id, from, to))
    }

    /// Records a settled firing; a one-shot trigger is done after its first.
    fn settle_agent_trigger(
        &mut self,
        session_id: &str,
        trigger_id: &str,
        settle: impl FnOnce(&mut AgentTrigger),
    ) -> Result<Option<AgentTrigger>, DaemonError> {
        let session = self.agent_trigger_session_mut(session_id)?;
        let Some(trigger) = session
            .agent_triggers_mut()
            .iter_mut()
            .find(|trigger| trigger.id() == trigger_id)
        else {
            return Ok(None);
        };
        settle(trigger);
        if trigger.is_one_shot() {
            return Ok(session.remove_agent_trigger(trigger_id));
        }
        Ok(Some(trigger.clone()))
    }

    fn agent_trigger_session_mut(
        &mut self,
        session_id: &str,
    ) -> Result<&mut RuntimeSession, DaemonError> {
        self.store
            .get_mut(session_id)
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }
}

fn agent_trigger_not_found(session_id: &str, trigger_id: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "agent trigger",
        message: format!("agent trigger `{trigger_id}` was not found in session `{session_id}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service_with_schedule(
        interval_seconds: u64,
        repeat: AgentTriggerRepeat,
    ) -> (SessionService, String, AgentTrigger) {
        let config = crate::config::DaemonConfig::for_tests();
        let mut service = SessionService::new(&config);
        let session = service
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let trigger = service
            .create_agent_schedule_trigger(
                session.id(),
                "agent-1",
                "user-1",
                crate::session::AgentSchedule::interval(interval_seconds, repeat),
                AgentTriggerBusyPolicy::Queue,
                Some("Check once.".to_string()),
            )
            .expect("trigger should create");
        (service, session.id().to_string(), trigger)
    }

    #[test]
    fn due_agent_trigger_is_claimed_once_until_dispatch_settles() {
        let (mut service, session_id, trigger) =
            service_with_schedule(1, AgentTriggerRepeat::Every);
        let due_at_ms = trigger.next_run_at_ms();

        let first = service.claim_due_agent_triggers(due_at_ms);
        let overlapping = service.claim_due_agent_triggers(due_at_ms);

        assert_eq!(first.len(), 1);
        assert!(overlapping.is_empty());
        assert_eq!(service.next_scheduled_runtime_wake_at_ms(), None);

        service
            .mark_agent_trigger_failed(&session_id, trigger.id(), due_at_ms, "retry".to_string())
            .expect("failed dispatch should release the claim");
        assert_eq!(
            service.next_scheduled_runtime_wake_at_ms(),
            Some(due_at_ms.saturating_add(1_000))
        );
    }

    #[test]
    fn interval_agent_trigger_stays_anchored_over_ten_delayed_firings() {
        let (mut service, session_id, trigger) =
            service_with_schedule(3, AgentTriggerRepeat::Every);
        let first_due = trigger.next_run_at_ms();
        for beat in 0..10 {
            let due = first_due + beat * 3_000;
            assert_eq!(service.claim_due_agent_triggers(due).len(), 1);
            service
                .mark_agent_trigger_fired(
                    &session_id,
                    trigger.id(),
                    due + 2_000,
                    AgentTriggerOutcome::Queued,
                    None,
                )
                .unwrap();
            assert_eq!(
                service.next_scheduled_runtime_wake_at_ms(),
                Some(due + 3_000),
                "delivery delay must not move beat {beat}"
            );
        }
    }

    #[test]
    fn interval_agent_trigger_skips_missed_beats_without_changing_phase() {
        let (mut service, session_id, trigger) =
            service_with_schedule(3, AgentTriggerRepeat::Every);
        let due = trigger.next_run_at_ms();
        assert_eq!(service.claim_due_agent_triggers(due + 10_000).len(), 1);
        service
            .mark_agent_trigger_coalesced(&session_id, trigger.id(), due + 10_000)
            .unwrap();
        assert_eq!(
            service.next_scheduled_runtime_wake_at_ms(),
            Some(due + 12_000)
        );
        assert!(service.claim_due_agent_triggers(due + 11_999).is_empty());
        assert_eq!(service.claim_due_agent_triggers(due + 12_000).len(), 1);
        service
            .mark_agent_trigger_failed(&session_id, trigger.id(), due + 14_500, "retry".into())
            .unwrap();
        // A failure keeps the phase and leaves at least a second before retrying.
        assert_eq!(
            service.next_scheduled_runtime_wake_at_ms(),
            Some(due + 18_000)
        );
    }

    #[test]
    fn coalesced_agent_trigger_advances_wake_without_counting_a_firing() {
        let (mut service, session_id, trigger) =
            service_with_schedule(600, AgentTriggerRepeat::Every);
        let due_at_ms = trigger.next_run_at_ms();
        assert_eq!(service.claim_due_agent_triggers(due_at_ms).len(), 1);
        service
            .mark_agent_trigger_fired(
                &session_id,
                trigger.id(),
                due_at_ms,
                AgentTriggerOutcome::Started,
                None,
            )
            .unwrap();
        let next_due_at_ms = due_at_ms + 600_000;
        assert_eq!(service.claim_due_agent_triggers(next_due_at_ms).len(), 1);
        let coalesced = service
            .mark_agent_trigger_coalesced(&session_id, trigger.id(), next_due_at_ms)
            .unwrap()
            .unwrap();
        assert_eq!(coalesced.fired(), 1);
        assert_eq!(coalesced.coalesced(), 1);
        assert_eq!(coalesced.last_fired_at_ms(), Some(due_at_ms));
        assert_eq!(
            service.next_scheduled_runtime_wake_at_ms(),
            Some(next_due_at_ms + 600_000)
        );
        assert!(service.claim_due_agent_triggers(next_due_at_ms).is_empty());
        assert_eq!(
            service
                .claim_due_agent_triggers(next_due_at_ms + 600_000)
                .len(),
            1
        );
    }

    #[test]
    fn disabled_agent_trigger_never_fires_and_restarts_a_full_interval_when_enabled() {
        let (mut service, session_id, trigger) =
            service_with_schedule(60, AgentTriggerRepeat::Every);
        let due_at_ms = trigger.next_run_at_ms();
        service
            .update_agent_trigger(&session_id, trigger.id(), None, None, Some(false))
            .unwrap();
        assert!(service
            .claim_due_agent_triggers(due_at_ms + 3_600_000)
            .is_empty());
        assert_eq!(service.next_scheduled_runtime_wake_at_ms(), None);

        let enabled = service
            .update_agent_trigger(&session_id, trigger.id(), None, None, Some(true))
            .unwrap();
        assert!(enabled.next_run_at_ms() >= due_at_ms);
        assert!(service
            .claim_due_agent_triggers(enabled.next_run_at_ms() - 1)
            .is_empty());
        assert_eq!(
            service
                .claim_due_agent_triggers(enabled.next_run_at_ms())
                .len(),
            1
        );
    }

    #[test]
    fn one_shot_trigger_is_removed_after_it_fires_and_agent_removal_takes_its_triggers() {
        let (mut service, session_id, once) = service_with_schedule(5, AgentTriggerRepeat::Once);
        let every = service
            .create_agent_schedule_trigger(
                &session_id,
                "agent-1",
                "user-1",
                crate::session::AgentSchedule::interval(5, AgentTriggerRepeat::Every),
                AgentTriggerBusyPolicy::Steer,
                None,
            )
            .unwrap();
        let other = service
            .create_agent_schedule_trigger(
                &session_id,
                "agent-2",
                "user-1",
                crate::session::AgentSchedule::interval(5, AgentTriggerRepeat::Every),
                AgentTriggerBusyPolicy::Queue,
                None,
            )
            .unwrap();
        service.claim_due_agent_triggers(once.next_run_at_ms());
        service
            .mark_agent_trigger_fired(
                &session_id,
                once.id(),
                once.next_run_at_ms(),
                AgentTriggerOutcome::Queued,
                None,
            )
            .unwrap();
        let ids = |service: &SessionService| {
            service
                .get_session(&session_id)
                .unwrap()
                .agent_triggers()
                .iter()
                .map(|trigger| trigger.id().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&service), vec![every.id(), other.id()]);

        let removed = service
            .remove_agent_triggers_for_agent(&session_id, "agent-1")
            .unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].id(), every.id());
        assert_eq!(ids(&service), vec![other.id()]);
    }
}
