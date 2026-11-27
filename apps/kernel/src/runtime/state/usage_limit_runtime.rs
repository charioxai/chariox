//! A turn that ends on the provider's usage limit goes on when usage is back. The kernel alone
//! does it: the stop lives on the agent, the one-second scheduler fires it at the reset time, and a
//! restart finds it again on the agent. No agent has to be awake, or on another account, for it.

use super::agent_task_owner_resolution::OwnerTaskAction;
use super::usage_limit_stop_transitions::{
    next_resume_at, transition, AfterLimit, LimitHit, StopEvent, StopOrigin,
};
use super::workflow_run_help::is_run_step;
use super::*;
use crate::agent::{UsageLimitStop, UsageLimitStopState};
use crate::config::UsageLimitResume;
use crate::durable_state::agent_lifecycle::ExecutionState;
use crate::local::UsageLimitStopAction as UsageLimitAction;

/// Between two resumes the kernel sends, so a team on one account does not reach the provider at once.
pub(crate) const RESUME_STAGGER_MS: u64 = 20_000;

pub(super) fn clock(at_ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(at_ms as i64)
        .map(|time| {
            let format = if time.date_naive() == chrono::Utc::now().date_naive() {
                "%H:%M UTC"
            } else {
                "%b %-d %H:%M UTC"
            };
            time.format(format).to_string()
        })
        .unwrap_or_default()
}

/// Why a resume is taken, which decides what the stop may be when the claim is made.
pub(super) enum ResumeReason<'a> {
    /// The scheduler found this exact stop armed and due.
    Due(&'a UsageLimitStop),
    Recovered(&'a UsageLimitStop),
    /// A person chose to go on: whatever the stop says now, unless a resume already took it.
    Decided,
}

impl<'a> ResumeReason<'a> {
    fn claim(&self, now: u64) -> StopEvent<'a> {
        match self {
            Self::Due(selected) => StopEvent::ClaimDue { selected, now },
            Self::Recovered(selected) => StopEvent::ClaimRecovered { selected, now },
            Self::Decided => StopEvent::ClaimDecided { now },
        }
    }
}

fn refused(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "usage limit",
        message: message.into(),
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn usage_limit_profile(
        &self,
        agent: &crate::agent::AgentInstance,
        profile: &str,
    ) -> Option<crate::account_profile::ProviderAccountProfile> {
        let owner = crate::account_profile::provider_account_authority_owner_user_id(
            &self.config_projection.snapshot(),
            agent.owner_user_id(),
        );
        self.provider_account_profiles
            .get(&owner, agent.provider(), profile)
            .ok()
    }

    pub(super) fn usage_limit_capacity_is_back(
        &self,
        agent: &crate::agent::AgentInstance,
        stop: &UsageLimitStop,
        now: u64,
    ) -> bool {
        // The observation must follow the most recent refusal, not just the original stop.
        let after = match stop.state {
            UsageLimitStopState::Armed { at_ms } if stop.refused > 0 => {
                next_resume_at(0, stop.resumes_at_ms, stop.refused)
                    .map_or(stop.stopped_at_ms, |delay| at_ms.saturating_sub(delay))
            }
            _ => stop.stopped_at_ms,
        };
        self.usage_limit_profile(agent, &stop.account_profile)
            .is_some_and(|profile| profile.usage_capacity_observed_after(agent.model(), after, now))
    }

    /// A delivery acknowledgement identifies the account that actually took the prompt.
    /// Write the stop's end before completion can make the turn invisible to the scheduler.
    pub(super) fn settle_usage_limit_on_delivery(
        &self,
        agent_id: &str,
        prompt_id: &str,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        let Some(stop) = agent.usage_limit_stop() else {
            return Ok(());
        };
        let account_label = self.account_label(&agent, run.account_profile());
        let event = StopEvent::Continued {
            account_label: &account_label,
        };
        let changed = self.agent_store.change_usage_limit_stop_durably(
            &self.durable_state_store,
            agent_id,
            |current| {
                let held = current
                    .usage_limit_stop()
                    .filter(|held| *held == stop && held.prompt_id != prompt_id)?;
                transition(Some(held), &event)
            },
        )?;
        if let Some(updated) = changed {
            let notice = event_notice(&event, None);
            self.notice_usage_limit_stop(&updated, &notice);
            self.close_usage_limit_decision(&updated, stop, notice)?;
            let _ = self.session_snapshot(agent.session_id());
        }
        Ok(())
    }

    fn notice_usage_limit_stop(&self, agent: &crate::agent::AgentInstance, notice: &str) {
        if !notice.is_empty() {
            self.record_notice_for_agent(
                agent.session_id(),
                None,
                Some(agent.id()),
                self.attachment_store
                    .list_session_attachment_ids(agent.session_id()),
                notice,
            );
        }
    }
}

impl KernelRuntimeState {
    /// How the usage limit reached a turn, or `None` for a turn that did not end on it, a step of a
    /// workflow run (the run owns the step, so no stop continues it outside the run, whether or not
    /// a task was kept for it), or an agent whose worker owns the stop.
    fn usage_limit_hit(
        &self,
        agent: &crate::agent::AgentInstance,
        prompt: &crate::session::PromptQueueItem,
        failure: &crate::provider::TurnOutcome,
        now: u64,
    ) -> Option<LimitHit> {
        if failure.kind != crate::provider::TurnOutcomeKind::UsageLimit
            || agent.remote_execution().is_some()
            || agent.archived()
            || is_run_step(prompt)
        {
            return None;
        }
        // The person's own word wins: Automatic goes on by itself, an explicit Ask me never does.
        // Left at its default, the kernel goes on by itself unless the person has a choice to make
        // (an account with room, and no orchestrator above the agent owning the switch): a stop
        // with nothing to decide is status on the row, and it resumes at the reset.
        let automatic = match self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .agents
            .usage_limit_resume
        {
            Some(policy) => policy == UsageLimitResume::Automatic,
            None => !self
                .owned
                .usage_limit_card_wanted(agent, agent.provider_account_profile()),
        };
        Some(LimitHit {
            reset_at_ms: self
                .owned
                .usage_limit_profile(agent, agent.provider_account_profile())
                .and_then(|profile| profile.usage_limit_reset_at_ms(agent.model(), now))
                .or_else(|| crate::provider::usage_limit_reset_at_ms(&failure.reason, now)),
            automatic,
            now,
        })
    }

    /// The failed turn's line when the kernel resumes the usage-limit stop by itself.
    pub(super) fn usage_limit_resume_notice(
        &self,
        agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
        failure: &crate::provider::TurnOutcome,
    ) -> Option<String> {
        let agent = self.owned.agent_store.get_agent(agent_id).ok()?;
        let hit = self.usage_limit_hit(&agent, prompt, failure, crate::session::unix_epoch_ms())?;
        let after = AfterLimit::of(agent.usage_limit_stop(), hit);
        let UsageLimitStopState::Armed { at_ms } = after.state else {
            return None;
        };
        Some(failure.notice_resuming(after.named_reset.map(|_| clock(at_ms)).as_deref()))
    }

    /// The turn of `agent_id` ended on the provider's usage limit: records the stop on the agent
    /// and, when the person lets the kernel resume, arms it. Called while the stopped turn settles,
    /// so a parent that is told about it can read when it goes on.
    pub(super) async fn record_usage_limit_stop(
        &self,
        agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
        failure: &crate::provider::TurnOutcome,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let agent = owned.agent_store.get_agent(agent_id)?;
        let Some(hit) =
            self.usage_limit_hit(&agent, prompt, failure, crate::session::unix_epoch_ms())
        else {
            return Ok(());
        };
        let origin = StopOrigin {
            // The turn that failed now: a resumed task goes on under a new prompt each time.
            prompt_id: prompt.id().to_string(),
            provider: run.provider().to_string(),
            account_profile: run.account_profile().to_string(),
            account_label: owned.account_label(&agent, run.account_profile()),
        };
        self.apply_stop_event(
            agent_id,
            &StopEvent::Hit {
                origin: &origin,
                hit,
            },
        )
        .await?;
        Ok(())
    }

    /// A stop that cannot be recorded leaves the failed turn as it was: the person still sees it.
    pub(super) async fn note_usage_limit_stop(
        &self,
        agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
        failure: &crate::provider::TurnOutcome,
        run: &crate::provider::RuntimeProviderRun,
    ) {
        if let Err(error) = self
            .record_usage_limit_stop(agent_id, prompt, failure, run)
            .await
        {
            tracing::warn!(agent_id, %error, "could not record a usage limit stop");
        }
    }

    /// The one owner of the stop: `event` moves it when the stop held at this moment accepts it
    /// (`transition`), checked and written under the agent store's lock, and only then is it told
    /// to the session and every client. `None` comes back, with nothing written, when the held
    /// stop does not accept the event, and so does the stop it held before.
    pub(super) async fn apply_stop_event(
        &self,
        agent_id: &str,
        event: &StopEvent<'_>,
    ) -> Result<Option<AppliedStop>, DaemonError> {
        let mut before = None;
        let changed = self.owned.agent_store.change_usage_limit_stop_durably(
            &self.owned.durable_state_store,
            agent_id,
            |current| {
                if let StopEvent::Hit { origin, .. } = event {
                    if !current
                        .matches_usage_limit_profile(&origin.provider, &origin.account_profile)
                    {
                        return None;
                    }
                }
                let held = current.usage_limit_stop();
                let next = transition(held, event);
                if next.is_some() {
                    before = held.cloned();
                }
                next
            },
        )?;
        let Some(agent) = changed else {
            return Ok(None);
        };
        let notice = event_notice(event, agent.usage_limit_stop());
        if let StopEvent::Reset { selected, .. } = event {
            self.owned
                .close_usage_limit_decision(&agent, selected, notice.clone())?;
        }
        self.announce_usage_limit_stop(&agent, &notice).await?;
        Ok(Some(AppliedStop { before }))
    }

    /// Tells the session and every client of a stop that is already written.
    async fn announce_usage_limit_stop(
        &self,
        updated: &crate::agent::AgentInstance,
        notice: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        owned.notice_usage_limit_stop(updated, notice);
        let _ = owned.session_snapshot(updated.session_id());
        self.sync_usage_limit_interaction(updated).await
    }

    /// The one-second scheduler: resumes every agent whose armed time has come, one per stagger.
    /// Each send runs on its own task, since a provider that cannot start (a locked Vault) waits
    /// for the person and the scheduler must not wait with it; the handles are for tests.
    pub(super) async fn tick_usage_limit_stops(
        &self,
        now: u64,
    ) -> Result<Vec<tokio::task::JoinHandle<()>>, DaemonError> {
        let mut sends = Vec::new();
        for agent in self.owned.agent_store.list_agents() {
            let Some(stop) = agent.usage_limit_stop().cloned() else {
                continue;
            };
            // An explicit "Ask me" stays the person's to decide at the reset: the stop and its
            // "Usage is back" decision stay until they choose Resume or Leave paused.
            if stop.state == UsageLimitStopState::Asking
                && stop.reset_reached(now)
                && !self.owned.usage_limit_resume_is_asked()
            {
                self.apply_stop_event(
                    agent.id(),
                    &StopEvent::Reset {
                        selected: &stop,
                        now,
                    },
                )
                .await?;
                continue;
            }
            // A decision a restart left without its reply channel is raised again. A board that
            // cannot take it (eight decisions pending) must not hold back another agent's resume.
            if let Err(error) = self.sync_usage_limit_interaction(&agent).await {
                crate::logging::warn_with_fields(
                    "daemon.usage_limit",
                    "could not raise the usage limit decision",
                    serde_json::json!({"agent_id": agent.id(), "error": error.to_string()}),
                );
            }
            if self.drop_stop_of_cancelled_task(&agent, &stop).await? {
                continue;
            }
            match stop.state {
                UsageLimitStopState::Armed { at_ms }
                    if at_ms <= now
                        || self.owned.usage_limit_capacity_is_back(&agent, &stop, now) =>
                {
                    let reason = if at_ms <= now {
                        ResumeReason::Due(&stop)
                    } else {
                        ResumeReason::Recovered(&stop)
                    };
                    if let Some(taken) = self.claim_resume(&agent, now, reason).await? {
                        sends.push(self.spawn_resume(agent, taken, now));
                    }
                }
                UsageLimitStopState::Resuming {
                    since_ms,
                    sent: false,
                } if !self.resume_is_being_sent(&agent) => {
                    // A kernel that went away took the resume. When it admitted the continuation
                    // first, that turn runs now or already ended, and sending again would repeat
                    // finished work; otherwise the resume is due now. The agent's own record of
                    // when its last turn started is the receipt: it is saved with the turn.
                    let admitted = self.agent_has_turn(&agent)
                        || agent
                            .last_prompt_sent_at_ms()
                            .is_some_and(|started_ms| started_ms >= since_ms);
                    if admitted {
                        self.mark_resume_sent(&agent, since_ms).await;
                    } else {
                        self.apply_stop_event(
                            agent.id(),
                            &StopEvent::Lost {
                                selected: &stop,
                                now,
                            },
                        )
                        .await?;
                    }
                }
                UsageLimitStopState::Resuming { sent: true, .. }
                    if !self.agent_has_turn(&agent) =>
                {
                    // The turn it started ended without the limit, once it has run a while: the
                    // agent is back.
                    self.apply_stop_event(
                        agent.id(),
                        &StopEvent::Settle {
                            selected: &stop,
                            now,
                        },
                    )
                    .await?;
                }
                _ => {}
            }
        }
        Ok(sends)
    }

    /// The owner or an agent cancelled the task the stop belongs to (or its workflow ended it): the
    /// work is not theirs to go on with, so no resume starts it again. A task the failure itself
    /// cancelled, or one that never existed, still resumes.
    fn stopped_task_was_cancelled(
        &self,
        agent: &crate::agent::AgentInstance,
        stop: &UsageLimitStop,
    ) -> Result<bool, DaemonError> {
        Ok(self
            .owned
            .durable_state_store
            .agent_turn_task(agent.session_id(), agent.id(), &stop.prompt_id)?
            .is_some_and(|task| task.cancelled_on_purpose()))
    }

    /// Ends the stop when its task was cancelled on purpose, so nothing resumes it. Returns whether
    /// it did.
    async fn drop_stop_of_cancelled_task(
        &self,
        agent: &crate::agent::AgentInstance,
        stop: &UsageLimitStop,
    ) -> Result<bool, DaemonError> {
        if !self.stopped_task_was_cancelled(agent, stop)? {
            return Ok(false);
        }
        self.apply_stop_event(
            agent.id(),
            &StopEvent::Drop {
                prompt_id: &stop.prompt_id,
            },
        )
        .await?;
        Ok(true)
    }

    fn resume_is_being_sent(&self, agent: &crate::agent::AgentInstance) -> bool {
        self.owned
            .usage_limit_sends
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(agent.id())
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

    /// Takes the resume of a stopped agent: marks it as going on, so no other tick or decision
    /// sends it again, and returns the stop it took. A scheduled resume is held back until the
    /// kernel has not sent another for a while. The scheduler and a person's decision can arrive
    /// together, so there is one claim per agent and whoever finds it taken sends nothing.
    pub(super) async fn claim_resume(
        &self,
        agent: &crate::agent::AgentInstance,
        now: u64,
        reason: ResumeReason<'_>,
    ) -> Result<Option<UsageLimitStop>, DaemonError> {
        let claimed = self
            .owned
            .usage_limit_sends
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(agent.id().to_string());
        if !claimed {
            return Ok(None);
        }
        let taken = self.take_claimed_resume(agent, now, reason).await;
        if !matches!(taken, Ok(Some(_))) {
            self.end_resume_send(agent);
        }
        taken
    }

    async fn take_claimed_resume(
        &self,
        agent: &crate::agent::AgentInstance,
        now: u64,
        reason: ResumeReason<'_>,
    ) -> Result<Option<UsageLimitStop>, DaemonError> {
        // A person's own prompt is running: usage is back for it, or it ends the stop itself.
        if self.agent_has_turn(agent) {
            return Ok(None);
        }
        let current = self.owned.agent_store.get_agent(agent.id())?;
        let Some(current) = current
            .usage_limit_stop()
            .filter(|current| !matches!(current.state, UsageLimitStopState::Resuming { .. }))
            .cloned()
        else {
            return Ok(None);
        };
        if self.drop_stop_of_cancelled_task(agent, &current).await? {
            return Ok(None);
        }
        if matches!(reason, ResumeReason::Due(_) | ResumeReason::Recovered(_)) {
            let last = *self
                .owned
                .usage_limit_resumes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if now < last.saturating_add(RESUME_STAGGER_MS) {
                return Ok(None);
            }
        }
        // The caller read the stop before it got here, and a person can cancel, or another claim
        // can move it on, until the write: the claim is an event the held stop accepts or refuses,
        // so a decision or a resume sent meanwhile is never overwritten and sent again.
        let Some(applied) = self
            .apply_stop_event(agent.id(), &reason.claim(now))
            .await?
        else {
            return Ok(None);
        };
        *self
            .owned
            .usage_limit_resumes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = now;
        crate::logging::info_with_fields(
            "daemon.usage_limit",
            "resuming a stopped agent",
            serde_json::json!({"agent_id": agent.id(), "now_ms": now}),
        );
        Ok(applied.before)
    }

    pub(super) fn end_resume_send(&self, agent: &crate::agent::AgentInstance) {
        self.owned
            .usage_limit_sends
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(agent.id());
    }

    /// Sends the continuation of the stop a claim at `since_ms` took.
    fn spawn_resume(
        &self,
        agent: crate::agent::AgentInstance,
        stop: UsageLimitStop,
        since_ms: u64,
    ) -> tokio::task::JoinHandle<()> {
        let state = self.clone();
        tokio::spawn(async move { state.finish_resume(&agent, &stop, since_ms).await })
    }

    /// Sends the continuation. When it cannot be sent the stop the claim took is back for the next
    /// tick, and the pane says why once.
    async fn finish_resume(
        &self,
        agent: &crate::agent::AgentInstance,
        stop: &UsageLimitStop,
        since_ms: u64,
    ) {
        let sent = self.send_resume(agent, stop).await;
        match sent {
            Ok(()) => {
                self.mark_resume_sent(agent, since_ms).await;
                self.owned
                    .usage_limit_resume_failures
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(agent.id());
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.usage_limit",
                    "resume retained for the next tick",
                    serde_json::json!({"agent_id": agent.id(), "error": error.to_string()}),
                );
                let event = StopEvent::Retain {
                    since_ms,
                    restore: stop,
                };
                if let Err(store_error) = self.apply_stop_event(agent.id(), &event).await {
                    crate::logging::warn_with_fields(
                        "daemon.usage_limit",
                        "could not restore the stop of a refused resume",
                        serde_json::json!({"agent_id": agent.id(), "error": store_error.to_string()}),
                    );
                }
                self.note_resume_failure(agent, &error);
            }
        }
        self.end_resume_send(agent);
    }

    /// Records that the resume claimed at `since_ms` was admitted, so a restart finds it sent.
    async fn mark_resume_sent(&self, agent: &crate::agent::AgentInstance, since_ms: u64) {
        if let Err(error) = self
            .apply_stop_event(agent.id(), &StopEvent::Sent { since_ms })
            .await
        {
            crate::logging::warn_with_fields(
                "daemon.usage_limit",
                "could not record a sent resume",
                serde_json::json!({"agent_id": agent.id(), "error": error.to_string()}),
            );
        }
    }

    /// Says once, in the agent's pane, why the kernel could not resume it, such as a locked Vault.
    fn note_resume_failure(&self, agent: &crate::agent::AgentInstance, error: &DaemonError) {
        let first = self
            .owned
            .usage_limit_resume_failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(agent.id().to_string());
        if first {
            let reason = crate::provider::sanitize_provider_diagnostic(&error.to_string());
            self.owned.record_notice_for_agent(
                agent.session_id(),
                None,
                Some(agent.id()),
                self.owned
                    .attachment_store
                    .list_session_attachment_ids(agent.session_id()),
                format!("The kernel could not resume this agent yet: {reason}. It tries again."),
            );
        }
    }

    /// The same task goes on when it has one that is waiting on the stop; otherwise the agent gets a
    /// kernel prompt in the same conversation. A cancel by the owner or an agent can land after the
    /// resume was claimed, so it is checked again here and once more right before the prompt is
    /// admitted; the claim stays held until this returns.
    async fn send_resume(
        &self,
        agent: &crate::agent::AgentInstance,
        stop: &UsageLimitStop,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        if self.drop_stop_of_cancelled_task(agent, stop).await? {
            return Ok(());
        }
        let task = owned.durable_state_store.agent_turn_task(
            agent.session_id(),
            agent.id(),
            &stop.prompt_id,
        )?;
        if let Some(task) = task.filter(|task| task.state == ExecutionState::Blocked) {
            return self
                .try_resolve_agent_task_owner_action(&task, OwnerTaskAction::Resume)
                .await;
        }
        let attachment = self.ensure_agent_message_attachment(agent.session_id(), agent)?;
        let prompt = crate::session::PromptQueueItem::new(
            format!("usage-resume-{}-{}", agent.id(), crate::session::unix_epoch_ms()),
            &attachment,
            agent.id(),
            "[Chariox usage resume] The provider usage limit has reset. Continue the work you were doing when it stopped. Do not repeat completed work.",
            crate::session::PromptStatus::Queued,
        );
        if self.drop_stop_of_cancelled_task(agent, stop).await? {
            return Ok(());
        }
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
        // An agent no usage limit stopped may have a blocked task: the same words settle it.
        let Some(stop) = agent.usage_limit_stop().cloned() else {
            Box::pin(self.resolve_blocked_task_command(&agent, caller_agent_id, action)).await?;
            return owned.agent_store.get_agent(agent_id);
        };
        // An agent above may arm unless the person explicitly chose Ask me.
        let automatic = owned
            .config_projection
            .snapshot()
            .user_config
            .agents
            .usage_limit_resume
            != Some(UsageLimitResume::Ask);
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
                if self.owned.usage_limit_capacity_is_back(&agent, &stop, now) {
                    self.resume_stopped_agent_now(&agent, now).await?;
                } else if matches!(stop.state, UsageLimitStopState::Armed { .. }) {
                    // Repeating Resume preserves the already scheduled time.
                } else if matches!(
                    stop.state,
                    UsageLimitStopState::Asking | UsageLimitStopState::Declined
                ) {
                    let at_ms =
                        next_resume_at(stop.stopped_at_ms, stop.resumes_at_ms, stop.refused)
                            .ok_or_else(|| refused("the kernel has no time to resume at"))?
                            .max(now);
                    self.decide_usage_limit_stop(
                        agent_id,
                        &StopEvent::Arm {
                            shown: &stop,
                            at_ms,
                        },
                    )
                    .await?;
                } else {
                    return Err(refused("this stop is already going on"));
                }
            }
            UsageLimitAction::NotNow | UsageLimitAction::Cancel => {
                if !matches!(
                    stop.state,
                    UsageLimitStopState::Asking | UsageLimitStopState::Armed { .. }
                ) {
                    return Err(refused("this stop is not waiting for a decision"));
                }
                self.decide_usage_limit_stop(agent_id, &StopEvent::Decline { shown: &stop })
                    .await?;
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
                self.update_agent_profile_with_selections(
                    session_id,
                    agent_id,
                    caller_user_id,
                    super::agent_profile_owned_state::AgentProfileSelections {
                        provider: None,
                        account_profile: Some(account_profile),
                        model: None,
                        effort: None,
                        continue_limit: Some(stop),
                    },
                    super::SettingsContext::person(),
                )
                .await?;
                let agent = owned.agent_store.get_agent(agent_id)?;
                self.resume_stopped_agent_now(&agent, now).await?;
            }
        }
        owned.agent_store.get_agent(agent_id)
    }

    /// Stores a person's decision on the stop they were shown. A resume the scheduler claimed since
    /// then, or a stop that changed some other way, has moved the stop on, and the decision is
    /// refused rather than written over it.
    pub(super) async fn decide_usage_limit_stop(
        &self,
        agent_id: &str,
        decision: &StopEvent<'_>,
    ) -> Result<(), DaemonError> {
        self.apply_stop_event(agent_id, decision)
            .await?
            .map(|_| ())
            .ok_or_else(|| refused("the stop changed while the decision was made; read it again"))
    }

    /// Resumes without the stagger: a person asked for it.
    pub(super) async fn resume_stopped_agent_now(
        &self,
        agent: &crate::agent::AgentInstance,
        now: u64,
    ) -> Result<(), DaemonError> {
        if let Some(taken) = self.claim_resume(agent, now, ResumeReason::Decided).await? {
            self.spawn_resume(agent.clone(), taken, now);
        }
        Ok(())
    }
}

/// What a stop event changed: the stop the agent held before it.
pub(super) struct AppliedStop {
    pub before: Option<UsageLimitStop>,
}

/// The line the session gets when `event` moved the stop to `stop`; empty for a move that is not news.
fn event_notice(event: &StopEvent<'_>, stop: Option<&UsageLimitStop>) -> String {
    match event {
        StopEvent::Continued { account_label, .. } => {
            return format!("Continued on {account_label}.")
        }
        StopEvent::Reset { selected, .. } => {
            return format!("Usage is back on {}.", selected.account_label)
        }
        _ => {}
    }
    let Some(stop) = stop else {
        return String::new();
    };
    match (event, &stop.state) {
        (StopEvent::Arm { .. }, UsageLimitStopState::Armed { at_ms }) => {
            format!(
                "The kernel resumes {} at {}.",
                stop.account_label,
                clock(*at_ms)
            )
        }
        (StopEvent::Hit { .. }, UsageLimitStopState::Armed { at_ms }) if stop.refused == 0 => {
            format!(
                "Usage limit reached on {}. The kernel resumes at {}.",
                stop.account_label,
                clock(*at_ms)
            )
        }
        (StopEvent::Hit { .. }, UsageLimitStopState::Armed { at_ms }) => format!(
            "Still limited on {}. The kernel tries again at {}.",
            stop.account_label,
            clock(*at_ms)
        ),
        (StopEvent::Hit { .. }, _) if stop.refused == 0 => match stop.resumes_at_ms {
            Some(at_ms) => format!(
                "Usage limit reached on {}, back at {}.",
                stop.account_label,
                clock(at_ms)
            ),
            None => format!("Usage limit reached on {}.", stop.account_label),
        },
        (StopEvent::Hit { .. }, _) => format!("Still limited on {}.", stop.account_label),
        _ => String::new(),
    }
}
