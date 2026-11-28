//! Under Ask me the person decides what a usage-limit stop does. The kernel raises that decision as
//! one interaction in the stopped agent's pane, so every client of the room shows it and answers it
//! the way it answers any other kernel decision.

use super::*;
use crate::agent::{UsageLimitStop, UsageLimitStopState};
use crate::local::UsageLimitStopAction;

pub(super) fn is_usage_limit_decision(interaction_id: &str) -> bool {
    interaction_id.starts_with(UsageLimitStop::DECISION_PREFIX)
}

fn decision_id(agent_id: &str, stop: &UsageLimitStop) -> String {
    format!(
        "{}{agent_id}-{}-{}",
        UsageLimitStop::DECISION_PREFIX,
        stop.stopped_at_ms,
        stop.refused
    )
}

/// The card at the reset of a stop the person chose to decide ("Ask me"): usage is back.
fn back_decision_id(agent_id: &str, stop: &UsageLimitStop) -> String {
    format!("{}-back", decision_id(agent_id, stop))
}

fn is_usage_limit_decision_of(
    interaction: &crate::session::RuntimeInteraction,
    agent_id: &str,
) -> bool {
    is_usage_limit_decision(interaction.id()) && interaction.related_agent_id() == Some(agent_id)
}

impl KernelRuntimeOwnedState {
    pub(super) fn close_usage_limit_decision(
        &self,
        agent: &crate::agent::AgentInstance,
        stop: &UsageLimitStop,
        reason: String,
    ) -> Result<(), DaemonError> {
        if stop.state == UsageLimitStopState::Asking {
            let room = self.session_store.get_session(agent.session_id())?;
            for id in [
                decision_id(agent.id(), stop),
                back_decision_id(agent.id(), stop),
            ] {
                if room
                    .active_interactions()
                    .iter()
                    .any(|interaction| interaction.id() == id)
                {
                    self.settle_card_condition_met(room.id(), &id, reason.clone())?;
                }
            }
        }
        Ok(())
    }

    /// A card for a usage stop is the person's only when they have a real choice (an account of the
    /// provider with room) and no orchestrator above the agent owns the switch. Anything else is
    /// status on the agent's row, never a card.
    pub(super) fn usage_limit_card_wanted(
        &self,
        agent: &crate::agent::AgentInstance,
        account_profile: &str,
    ) -> bool {
        agent.parent_agent_id().is_none()
            && !self
                .usage_limit_alternatives(agent, account_profile)
                .is_empty()
    }

    /// The person set the resume policy to "Ask me" themselves: the resume is theirs to decide.
    pub(super) fn usage_limit_resume_is_asked(&self) -> bool {
        self.config_projection
            .snapshot()
            .user_config
            .agents
            .usage_limit_resume
            == Some(crate::config::UsageLimitResume::Ask)
    }

    /// A stop the person decides is due its card when usage is back: at the reset the provider
    /// named, or at the first retry time when it named none. A refused resume is not "back".
    fn usage_limit_is_back(&self, stop: &UsageLimitStop, now_ms: u64) -> bool {
        stop.refused == 0
            && super::usage_limit_stop_transitions::next_resume_at(
                stop.stopped_at_ms,
                stop.resumes_at_ms,
                0,
            )
            .is_some_and(|at_ms| at_ms <= now_ms)
    }

    /// The accounts of the stopped agent's provider that can go on with its turn right now.
    fn usage_limit_alternatives(
        &self,
        agent: &crate::agent::AgentInstance,
        account_profile: &str,
    ) -> Vec<(String, String)> {
        let owner = crate::account_profile::provider_account_authority_owner_user_id(
            &self.config_projection.snapshot(),
            agent.owner_user_id(),
        );
        let Ok(profiles) = self
            .provider_account_profiles
            .list(&owner, Some(agent.provider()))
        else {
            return Vec::new();
        };
        let now = crate::session::unix_epoch_ms();
        profiles
            .into_iter()
            .filter(|profile| {
                profile.auth_state
                    == crate::account_profile::ProviderAccountAuthState::Authenticated
                    && profile.profile_id != account_profile
                    && !(account_profile == "default" && profile.is_default)
                    && !agent
                        .model()
                        .is_some_and(|model| profile.has_confirmed_exhaustion(model, now))
            })
            .map(|profile| {
                let label = profile.label.trim();
                let label = if label.is_empty() {
                    &profile.profile_id
                } else {
                    label
                };
                (profile.profile_id.clone(), label.to_string())
            })
            .collect()
    }
}

impl KernelRuntimeState {
    /// Makes the room show the decision its agent's stop asks for and nothing else: a stop that is
    /// asking, with an account to go on with and no orchestrator owning the switch, has one card;
    /// any other state (or no stop) has none.
    /// The answer to the decision applies the stop again, which syncs it: the boxed type ends that cycle.
    pub(super) fn sync_usage_limit_interaction<'a>(
        &'a self,
        agent: &'a crate::agent::AgentInstance,
    ) -> futures_util::future::BoxFuture<'a, Result<(), DaemonError>> {
        Box::pin(async move {
            let current = self.owned.agent_store.get_agent(agent.id())?;
            self.raise_usage_limit_decision(&current).await
        })
    }

    async fn raise_usage_limit_decision(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        let room = agent.session_id();
        let Ok(session) = self.owned.session_store.get_session(room) else {
            return Ok(());
        };
        let now_ms = crate::session::unix_epoch_ms();
        let back = |stop: &UsageLimitStop| {
            self.owned.usage_limit_resume_is_asked() && self.owned.usage_limit_is_back(stop, now_ms)
        };
        // Nothing before the reset unless the person has a real choice (an account with room, no
        // orchestrator owning the switch). At the reset of a stop they chose to decide: one card.
        // A person who set Ask me themselves has no card at the stop, whatever accounts exist: their
        // one card is at the reset.
        let asked = self.owned.usage_limit_resume_is_asked();
        let asking = agent.usage_limit_wait().filter(|stop| {
            stop.state == UsageLimitStopState::Asking
                && (back(stop)
                    || (!asked
                        && self
                            .owned
                            .usage_limit_card_wanted(agent, &stop.account_profile)))
        });
        let wanted = asking.map(|stop| {
            if back(stop) {
                back_decision_id(agent.id(), stop)
            } else {
                decision_id(agent.id(), stop)
            }
        });
        for interaction in session
            .active_interactions()
            .iter()
            .filter(|interaction| is_usage_limit_decision_of(interaction, agent.id()))
        {
            let live = self
                .owned
                .pending_interactions
                .write()
                .get(interaction.id())
                .is_some_and(|pending| {
                    pending
                        .responder
                        .lock()
                        .expect("interaction responder")
                        .as_ref()
                        .is_some_and(|sender| !sender.is_closed())
                });
            if wanted.as_deref() == Some(interaction.id()) && live {
                return Ok(());
            }
            self.owned
                .timeout_runtime_interaction(room, interaction.id())?;
        }
        let (Some(stop), Some(id)) = (asking, wanted) else {
            return Ok(());
        };
        let name = agent.alias().unwrap_or(agent.agent_ref());
        let is_back = back(stop);
        let when = stop
            .resumes_at_ms
            .map(|at_ms| format!(", back at {}", super::usage_limit_runtime::clock(at_ms)))
            .unwrap_or_default();
        let (title, message) = if is_back {
            ("Usage is back", format!("{name}: usage is back."))
        } else {
            (
                "Usage limit reached",
                format!(
                    "{name} stopped: {} reached its usage limit{when}.",
                    stop.account_label
                ),
            )
        };
        let mut choices = vec![
            crate::session::RuntimeInteractionChoice::new(
                "resume_when_back",
                if is_back {
                    "Resume"
                } else {
                    "Resume when it's back"
                },
                "resume_when_back",
                None,
            ),
            crate::session::RuntimeInteractionChoice::new(
                "not_now",
                if is_back { "Leave paused" } else { "Dismiss" },
                "not_now",
                None,
            ),
        ];
        if self
            .owned
            .usage_limit_card_wanted(agent, &stop.account_profile)
        {
            for (profile, label) in self
                .owned
                .usage_limit_alternatives(agent, &stop.account_profile)
            {
                choices.push(crate::session::RuntimeInteractionChoice::new(
                    format!("continue_on:{profile}"),
                    format!("Continue now on {label}"),
                    format!("continue_on:{profile}"),
                    None,
                ));
            }
        }
        let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
            &id, &id, title, &message, choices,
        )
        .without_timeout()
        .with_related_agent_id(agent.id().to_string());
        let rx = self
            .create_kernel_operation_interaction(
                crate::runtime::state::InteractionSurface::Hidden,
                room,
                session.owner_user_id(),
                interaction,
            )
            .await?;
        if self
            .owned
            .agent_store
            .get_agent(agent.id())?
            .usage_limit_wait()
            != Some(stop)
        {
            self.owned.timeout_runtime_interaction(room, &id)?;
            return Ok(());
        }
        let (state, agent_id, owner) = (
            self.as_kernel(),
            agent.id().to_string(),
            agent.owner_user_id().to_string(),
        );
        let room = room.to_string();
        tokio::spawn(async move {
            let Ok(answer) = rx.await else { return };
            let action = match answer.choice_id.as_deref() {
                Some("resume_when_back") => UsageLimitStopAction::ResumeWhenBack,
                Some("not_now") => UsageLimitStopAction::NotNow,
                Some(choice) => match choice.strip_prefix("continue_on:") {
                    Some(profile) => UsageLimitStopAction::ContinueOn {
                        account_profile: profile.to_string(),
                    },
                    None => return,
                },
                None => return,
            };
            if let Err(error) = state
                .resolve_usage_limit_stop(&room, &agent_id, &owner, None, action)
                .await
            {
                tracing::warn!(%error, agent_id, "the usage limit decision could not be applied");
            }
        });
        Ok(())
    }
}
