use super::*;

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
                    .filter(|trigger| trigger.enabled() && !trigger.dispatch_in_flight())
                    .map(|trigger| trigger.next_run_at_ms());
                watchdogs.chain(agent_triggers)
            })
            .min()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "A schedule trigger is created from distinct request fields and the caller identity."
    )]
    pub fn create_agent_schedule_trigger(
        &mut self,
        session_id: &str,
        agent_id: &str,
        created_by_user_id: &str,
        interval_seconds: u64,
        repeat: AgentTriggerRepeat,
        busy_policy: AgentTriggerBusyPolicy,
        prompt: Option<String>,
    ) -> Result<AgentTrigger, DaemonError> {
        if interval_seconds == 0 {
            return Err(DaemonError::LocalTransport {
                operation: "agent trigger create",
                message: "wait duration must be greater than zero".to_string(),
            });
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
            interval_seconds,
            repeat,
            busy_policy,
            prompt,
            unix_epoch_ms(),
        );
        Ok(self
            .agent_trigger_session_mut(session_id)?
            .add_agent_trigger(trigger))
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
        trigger.update(busy_policy, prompt, enabled, unix_epoch_ms());
        Ok(trigger.clone())
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

    pub(crate) fn apply_agent_trigger_settlement(
        &mut self,
        session_id: &str,
        settlement: crate::session::AgentTriggerSettlement,
    ) -> Result<(), DaemonError> {
        self.agent_trigger_session_mut(session_id)?
            .apply_agent_trigger_settlement(settlement);
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
                interval_seconds,
                repeat,
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
                5,
                AgentTriggerRepeat::Every,
                AgentTriggerBusyPolicy::Steer,
                None,
            )
            .unwrap();
        let other = service
            .create_agent_schedule_trigger(
                &session_id,
                "agent-2",
                "user-1",
                5,
                AgentTriggerRepeat::Every,
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
