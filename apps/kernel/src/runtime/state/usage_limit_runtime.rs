//! A turn that ends on the provider's usage limit goes on when usage is back. The kernel alone
//! does it: the stop lives on the agent, the one-second scheduler fires it at the reset time, and a
//! restart finds it again on the agent. No agent has to be awake, or on another account, for it.

use super::agent_task_owner_resolution::OwnerTaskAction;
use super::*;
use crate::agent::{UsageLimitStop, UsageLimitStopState};
use crate::config::UsageLimitResume;
use crate::durable_state::agent_lifecycle::ExecutionState;
use crate::local::UsageLimitStopAction as UsageLimitAction;

/// Between two resumes the kernel sends, so a team on one account does not reach the provider at once.
pub(crate) const RESUME_STAGGER_MS: u64 = 20_000;
/// The provider named a reset time and still refused at it: wait this long after each refusal.
const RETRY_AFTER_NAMED_RESET_MS: [u64; 3] = [60_000, 300_000, 900_000];
/// The provider named no time: wait this long after the stop and after each refusal.
const RETRY_AFTER_UNNAMED_RESET_MS: [u64; 4] = [900_000, 1_800_000, 3_600_000, 7_200_000];
/// A resume that has not stopped again after this long worked.
const RESUME_SETTLE_MS: u64 = 30_000;

/// When the kernel resumes next: at the reset time the provider named, otherwise after a wait
/// that grows with each refusal; `None` once it has tried as often as it will.
fn next_resume_at(now_ms: u64, named_reset_ms: Option<u64>, refused: u32) -> Option<u64> {
    match (named_reset_ms, refused) {
        (Some(reset), 0) => Some(reset.max(now_ms)),
        (Some(_), refused) => RETRY_AFTER_NAMED_RESET_MS
            .get(refused as usize - 1)
            .map(|wait| now_ms + wait),
        (None, refused) => RETRY_AFTER_UNNAMED_RESET_MS
            .get(refused as usize)
            .map(|wait| now_ms + wait),
    }
}

fn clock(at_ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(at_ms as i64)
        .map(|time| time.format("%H:%M UTC").to_string())
        .unwrap_or_default()
}

fn refused(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "usage limit",
        message: message.into(),
    }
}

impl KernelRuntimeState {
    /// The turn of `agent_id` ended on the provider's usage limit: records the stop on the agent
    /// and, when the person lets the kernel resume, arms it. Called while the stopped turn settles,
    /// so a parent that is told about it can read when it goes on.
    pub(super) async fn record_usage_limit_stop(
        &self,
        agent_id: &str,
        prompt_id: &str,
        failure: &crate::provider::TurnOutcome,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        if failure.kind != crate::provider::TurnOutcomeKind::UsageLimit {
            return Ok(());
        }
        let agent = owned.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_some() || agent.archived() {
            return Ok(());
        }
        let now = crate::session::unix_epoch_ms();
        let previous = agent.usage_limit_stop().cloned();
        // A stop while a resume runs is that resume refused.
        let resumed_before = previous
            .as_ref()
            .filter(|stop| matches!(stop.state, UsageLimitStopState::Resuming { .. }));
        let refusals = resumed_before.map_or(0, |stop| stop.refused + 1);
        let named_reset = match resumed_before {
            Some(stop) => stop.resumes_at_ms,
            None => crate::provider::usage_limit_reset_at_ms(&failure.reason, now)
                .filter(|at| *at > now),
        };
        let automatic = owned
            .config_projection
            .snapshot()
            .user_config
            .agents
            .usage_limit_resume
            .unwrap_or_default()
            == UsageLimitResume::Automatic;
        let state = if automatic || resumed_before.is_some() {
            match next_resume_at(now, named_reset, refusals) {
                Some(at_ms) => UsageLimitStopState::Armed { at_ms },
                None => UsageLimitStopState::Asking,
            }
        } else {
            UsageLimitStopState::Asking
        };
        let stop = UsageLimitStop {
            prompt_id: resumed_before
                .map_or_else(|| prompt_id.to_string(), |stop| stop.prompt_id.clone()),
            provider: run.provider().to_string(),
            account_profile: run.account_profile().to_string(),
            account_label: owned.account_label(&agent, run.account_profile()),
            stopped_at_ms: resumed_before.map_or(now, |stop| stop.stopped_at_ms),
            resumes_at_ms: named_reset,
            refused: refusals,
            state,
        };
        let line = match (&stop.state, refusals) {
            (UsageLimitStopState::Armed { at_ms }, 0) => format!(
                "Usage limit reached on {}. The kernel resumes at {}.",
                stop.account_label,
                clock(*at_ms)
            ),
            (UsageLimitStopState::Armed { at_ms }, _) => format!(
                "Still limited on {}. The kernel tries again at {}.",
                stop.account_label,
                clock(*at_ms)
            ),
            (_, 0) => match stop.resumes_at_ms {
                Some(at_ms) => format!(
                    "Usage limit reached on {}, back at {}.",
                    stop.account_label,
                    clock(at_ms)
                ),
                None => format!("Usage limit reached on {}.", stop.account_label),
            },
            (_, _) => format!("Still limited on {}.", stop.account_label),
        };
        self.store_usage_limit_stop(&agent, Some(stop), &line).await
    }

    /// A stop that cannot be recorded leaves the failed turn as it was: the person still sees it.
    pub(super) async fn note_usage_limit_stop(
        &self,
        agent_id: &str,
        prompt_id: &str,
        failure: &crate::provider::TurnOutcome,
        run: &crate::provider::RuntimeProviderRun,
    ) {
        if let Err(error) = self
            .record_usage_limit_stop(agent_id, prompt_id, failure, run)
            .await
        {
            tracing::warn!(agent_id, %error, "could not record a usage limit stop");
        }
    }

    async fn store_usage_limit_stop(
        &self,
        agent: &crate::agent::AgentInstance,
        stop: Option<UsageLimitStop>,
        notice: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let updated = owned
            .agent_store
            .set_agent_usage_limit_stop(agent.id(), stop)?;
        if !notice.is_empty() {
            owned.record_notice_for_agent(
                agent.session_id(),
                None,
                Some(agent.id()),
                owned
                    .attachment_store
                    .list_session_attachment_ids(agent.session_id()),
                notice,
            );
        }
        self.append_agent_durable_event("agent.updated", &updated, None)
            .await?;
        let _ = owned.session_snapshot(agent.session_id());
        Ok(())
    }

    /// The one-second scheduler: resumes every agent whose armed time has come, one per stagger.
    pub(super) async fn tick_usage_limit_stops(&self, now: u64) -> Result<(), DaemonError> {
        for agent in self.owned.agent_store.list_agents() {
            let Some(stop) = agent.usage_limit_stop().cloned() else {
                continue;
            };
            match stop.state {
                UsageLimitStopState::Armed { at_ms } if at_ms <= now => {
                    if let Err(error) = self.resume_stopped_agent(&agent, &stop, now).await {
                        tracing::warn!(agent_id = agent.id(), %error, "usage limit resume retained for the next tick");
                    }
                }
                UsageLimitStopState::Resuming { since_ms }
                    if now.saturating_sub(since_ms) > RESUME_SETTLE_MS
                        && !self.agent_has_turn(&agent) =>
                {
                    // The turn it started ended without the limit: the agent is back.
                    self.store_usage_limit_stop(&agent, None, "").await?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn agent_has_turn(&self, agent: &crate::agent::AgentInstance) -> bool {
        let Ok(session) = self.owned.session_store.get_session(agent.session_id()) else {
            return false;
        };
        self.owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent.id())
            .is_some()
            || self
                .owned
                .prompt_state_owner
                .queued_prompt_count_for_agent(&session, agent.id())
                > 0
    }

    /// Sends the continuation of a stopped agent, if the kernel has not just sent another.
    async fn resume_stopped_agent(
        &self,
        agent: &crate::agent::AgentInstance,
        stop: &UsageLimitStop,
        now: u64,
    ) -> Result<(), DaemonError> {
        // A person's own prompt is running: usage is back for it, or it ends the stop itself.
        if self.agent_has_turn(agent) {
            return Ok(());
        }
        {
            let mut last = self
                .owned
                .usage_limit_resumes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if now < last.saturating_add(RESUME_STAGGER_MS) {
                return Ok(());
            }
            *last = now;
        }
        // Marked before the send, so the next tick cannot send it again.
        let resuming = UsageLimitStop {
            state: UsageLimitStopState::Resuming { since_ms: now },
            ..stop.clone()
        };
        self.store_usage_limit_stop(agent, Some(resuming), "")
            .await?;
        if let Err(error) = self.send_resume(agent, stop).await {
            // Not sent: it stays armed for the next tick.
            self.store_usage_limit_stop(agent, Some(stop.clone()), "")
                .await?;
            return Err(error);
        }
        Ok(())
    }

    /// The same task goes on when it has one that is waiting on the stop; otherwise the agent gets a
    /// kernel prompt in the same conversation.
    async fn send_resume(
        &self,
        agent: &crate::agent::AgentInstance,
        stop: &UsageLimitStop,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let task = owned.durable_state_store.agent_turn_task(
            agent.session_id(),
            agent.id(),
            &stop.prompt_id,
        )?;
        if let Some(task) = task.filter(|task| task.state == ExecutionState::Blocked) {
            self.resolve_agent_task_owner_action(&task, OwnerTaskAction::Resume)
                .await;
            return Ok(());
        }
        let attachment = self.ensure_agent_message_attachment(agent.session_id(), agent)?;
        let prompt = crate::session::PromptQueueItem::new(
            format!("usage-resume-{}-{}", agent.id(), crate::session::unix_epoch_ms()),
            &attachment,
            agent.id(),
            "[Chariox usage resume] The provider usage limit has reset. Continue the work you were doing when it stopped. Do not repeat completed work.",
            crate::session::PromptStatus::Queued,
        );
        let mut submission = self
            .submit_prepared_prompt_with_queue_policy(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: agent.session_id().to_string(),
                    prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                true,
            )
            .await?;
        if let Some(dispatch) = submission.dispatch.take() {
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        if let Some(dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
        Ok(())
    }

    /// The person's choice on the stripe, or an agent's call for an agent below it.
    pub(crate) async fn resolve_usage_limit_stop(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        caller_agent_id: Option<&str>,
        action: UsageLimitAction,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let owned = &self.owned;
        let agent = owned.ensure_agent_owner(agent_id, caller_user_id, "resolve usage limit")?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let stop = agent
            .usage_limit_stop()
            .cloned()
            .ok_or_else(|| refused("this agent is not stopped by a usage limit"))?;
        let automatic = owned
            .config_projection
            .snapshot()
            .user_config
            .agents
            .usage_limit_resume
            .unwrap_or_default()
            == UsageLimitResume::Automatic;
        let arms = matches!(
            action,
            UsageLimitAction::ResumeWhenBack | UsageLimitAction::ContinueOn { .. }
        );
        // The decision is the person's under Ask me; an agent may only take back an armed resume.
        if caller_agent_id.is_some() && arms && !automatic {
            return Err(refused("the person chose Ask me for usage limits"));
        }
        let now = crate::session::unix_epoch_ms();
        match action {
            UsageLimitAction::ResumeWhenBack => {
                if !matches!(
                    stop.state,
                    UsageLimitStopState::Asking | UsageLimitStopState::Declined
                ) {
                    return Err(refused("this stop is already armed"));
                }
                let at_ms = next_resume_at(now, stop.resumes_at_ms, 0)
                    .ok_or_else(|| refused("the kernel has no time to resume at"))?;
                let line = format!(
                    "The kernel resumes {} at {}.",
                    stop.account_label,
                    clock(at_ms)
                );
                let armed = UsageLimitStop {
                    state: UsageLimitStopState::Armed { at_ms },
                    ..stop
                };
                self.store_usage_limit_stop(&agent, Some(armed), &line)
                    .await?;
            }
            UsageLimitAction::NotNow | UsageLimitAction::Cancel => {
                if !matches!(
                    stop.state,
                    UsageLimitStopState::Asking | UsageLimitStopState::Armed { .. }
                ) {
                    return Err(refused("this stop is not waiting for a decision"));
                }
                let left = UsageLimitStop {
                    state: UsageLimitStopState::Declined,
                    ..stop
                };
                self.store_usage_limit_stop(&agent, Some(left), "").await?;
            }
            UsageLimitAction::ContinueOn { account_profile } => {
                if !matches!(
                    stop.state,
                    UsageLimitStopState::Asking
                        | UsageLimitStopState::Armed { .. }
                        | UsageLimitStopState::Declined
                ) {
                    return Err(refused("this stop is already going on"));
                }
                self.update_agent_profile(
                    session_id,
                    agent_id,
                    caller_user_id,
                    None,
                    Some(account_profile),
                    None,
                    None,
                )
                .await?;
                let agent = owned.agent_store.get_agent(agent_id)?;
                self.resume_stopped_agent_now(&agent, &stop, now).await?;
            }
        }
        owned.agent_store.get_agent(agent_id)
    }

    /// Resumes without the stagger: a person asked for it.
    async fn resume_stopped_agent_now(
        &self,
        agent: &crate::agent::AgentInstance,
        stop: &UsageLimitStop,
        now: u64,
    ) -> Result<(), DaemonError> {
        let resuming = UsageLimitStop {
            state: UsageLimitStopState::Resuming { since_ms: now },
            ..stop.clone()
        };
        self.store_usage_limit_stop(agent, Some(resuming), "")
            .await?;
        if let Err(error) = self.send_resume(agent, stop).await {
            self.store_usage_limit_stop(agent, Some(stop.clone()), "")
                .await?;
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_reset_is_waited_for_and_then_retried_a_few_times() {
        assert_eq!(next_resume_at(1_000, Some(5_000), 0), Some(5_000));
        assert_eq!(next_resume_at(9_000, Some(5_000), 0), Some(9_000));
        assert_eq!(next_resume_at(10_000, Some(5_000), 1), Some(70_000));
        assert_eq!(next_resume_at(10_000, Some(5_000), 2), Some(310_000));
        assert_eq!(next_resume_at(10_000, Some(5_000), 3), Some(910_000));
        assert_eq!(next_resume_at(10_000, Some(5_000), 4), None);
    }

    #[test]
    fn an_unnamed_reset_backs_off_and_then_asks() {
        assert_eq!(next_resume_at(0, None, 0), Some(900_000));
        assert_eq!(next_resume_at(0, None, 1), Some(1_800_000));
        assert_eq!(next_resume_at(0, None, 3), Some(7_200_000));
        assert_eq!(next_resume_at(0, None, 4), None);
    }
}
