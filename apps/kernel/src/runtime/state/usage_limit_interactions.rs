//! Under Ask me the person decides what a usage-limit stop does. The kernel raises that decision as
//! one interaction in the stopped agent's pane, so every client of the room shows it and answers it
//! the way it answers any other kernel decision.

use super::*;
use crate::agent::{UsageLimitStop, UsageLimitStopState};
use crate::local::UsageLimitStopAction;

const USAGE_LIMIT_DECISION_PREFIX: &str = "usage-limit-";

pub(super) fn is_usage_limit_decision(interaction_id: &str) -> bool {
    interaction_id.starts_with(USAGE_LIMIT_DECISION_PREFIX)
}

fn decision_id(agent_id: &str, stop: &UsageLimitStop) -> String {
    format!(
        "{USAGE_LIMIT_DECISION_PREFIX}{agent_id}-{}-{}",
        stop.stopped_at_ms, stop.refused
    )
}

fn is_usage_limit_decision_of(
    interaction: &crate::session::RuntimeInteraction,
    agent_id: &str,
) -> bool {
    is_usage_limit_decision(interaction.id()) && interaction.related_agent_id() == Some(agent_id)
}

impl KernelRuntimeOwnedState {
    /// The accounts of the stopped agent's provider that can go on with its turn right now.
    fn usage_limit_alternatives(
        &self,
        agent: &crate::agent::AgentInstance,
        stop: &UsageLimitStop,
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
                    && profile.profile_id != stop.account_profile
                    && !(stop.account_profile == "default" && profile.is_default)
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
    /// asking has one card, any other state (or no stop) has none.
    /// The answer to the decision applies the stop again, which syncs it: the boxed type ends that cycle.
    pub(super) fn sync_usage_limit_interaction<'a>(
        &'a self,
        agent: &'a crate::agent::AgentInstance,
    ) -> futures_util::future::BoxFuture<'a, Result<(), DaemonError>> {
        Box::pin(self.raise_usage_limit_decision(agent))
    }

    async fn raise_usage_limit_decision(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        let room = agent.session_id();
        let Ok(session) = self.owned.session_store.get_session(room) else {
            return Ok(());
        };
        let asking = agent
            .usage_limit_stop()
            .filter(|stop| stop.state == UsageLimitStopState::Asking);
        let wanted = asking.map(|stop| decision_id(agent.id(), stop));
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
        let when = stop
            .resumes_at_ms
            .map(|at_ms| format!(", back at {}", super::usage_limit_runtime::clock(at_ms)))
            .unwrap_or_default();
        let message = format!(
            "{name} stopped: {} reached its usage limit{when}.",
            stop.account_label
        );
        let mut choices = vec![
            crate::session::RuntimeInteractionChoice::new(
                "resume_when_back",
                "Resume when it's back",
                "resume_when_back",
                None,
            ),
            crate::session::RuntimeInteractionChoice::new("not_now", "Not now", "not_now", None),
        ];
        for (profile, label) in self.owned.usage_limit_alternatives(agent, stop) {
            choices.push(crate::session::RuntimeInteractionChoice::new(
                format!("continue_on:{profile}"),
                format!("Continue now on {label}"),
                format!("continue_on:{profile}"),
                None,
            ));
        }
        let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            "Usage limit reached",
            &message,
            choices,
        )
        .without_timeout()
        .with_related_agent_id(agent.id().to_string());
        let rx = self
            .create_kernel_operation_interaction(room, session.owner_user_id(), interaction)
            .await?;
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
