//! An agent that changes its own settings stays within what the person allowed it. A value
//! outside that set, or a higher permission level or mode, takes one decision from the person.

use super::agent_settings_change::{SettingsContext, SettingsOrigin};
use super::*;
use crate::agent::SelfSettingKind;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle};

const DECISION_TIMEOUT: Duration = Duration::from_secs(300);
const ALLOW_ONCE: &str = "allow_once";
const ALLOW_ALWAYS: &str = "allow_always";
const DENY: &str = "deny";

/// The settings a call asks for, as the kernel resolved them.
#[derive(Debug, Default, Clone)]
pub(super) struct RequestedSettings {
    pub(super) model: Option<String>,
    /// `Some(None)` clears the effort.
    pub(super) effort: Option<Option<String>>,
    pub(super) account_profile: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) mode: Option<AgentExecutionMode>,
    pub(super) permission_level: Option<AgentPermissionLevel>,
    pub(super) directory: Option<String>,
}

/// One thing the person decides: how it reads on the card, and what "Allow always" remembers.
struct Needed {
    text: String,
    remember: Option<(SelfSettingKind, String)>,
}

fn refused(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "agent settings",
        message: message.into(),
    }
}

impl KernelRuntimeState {
    /// Lets `context`'s call change `target` when the person's set allows it, after one decision
    /// when it does not. Only an agent changing itself is held to the set: the agents above an
    /// agent choose its settings as they do when they start it.
    pub(super) async fn gate_agent_settings_change(
        &self,
        target: &crate::agent::AgentInstance,
        context: &SettingsContext,
        requested: &RequestedSettings,
    ) -> Result<(), DaemonError> {
        let SettingsOrigin::Agent(caller) = &context.origin else {
            return Ok(());
        };
        if caller != target.id() {
            return Ok(());
        }
        let owned = &self.owned;
        let session = owned.session_store.get_session(target.session_id())?;
        let agents = owned.agent_store.get_session_agents(target.session_id());
        let current =
            crate::session::effective_agent_execution_config_in_tree(&session, target, &agents);
        let mut cap_agent = target.clone();
        cap_agent.set_execution_mode_override(None);
        cap_agent.set_permission_level_override(None);
        let cap =
            crate::session::effective_agent_execution_config_in_tree(&session, &cap_agent, &agents);
        let allowed = target.self_settings();
        let mut needed = Vec::new();
        if let Some(provider) = requested
            .provider
            .as_deref()
            .filter(|provider| *provider != target.provider())
        {
            needed.push(Needed {
                text: format!("provider to {provider}"),
                remember: None,
            });
        }
        if let Some(model) = requested
            .model
            .as_deref()
            .filter(|model| Some(*model) != target.model())
            .filter(|model| !allowed.allows(SelfSettingKind::Model, model))
        {
            needed.push(Needed {
                text: format!("model to {model}"),
                remember: Some((SelfSettingKind::Model, model.to_string())),
            });
        }
        if let Some(effort) = requested.effort.as_ref() {
            let value = effort.as_deref().unwrap_or("default");
            if effort.as_deref() != target.effort()
                && !allowed.allows(SelfSettingKind::Effort, value)
            {
                needed.push(Needed {
                    text: format!("effort to {value}"),
                    remember: Some((SelfSettingKind::Effort, value.to_string())),
                });
            }
        }
        if let Some(account) = requested
            .account_profile
            .as_deref()
            .filter(|account| *account != target.provider_account_profile())
            .filter(|account| !allowed.allows(SelfSettingKind::AccountProfile, account))
        {
            let label = owned.account_label(target, account);
            needed.push(Needed {
                text: format!("account to {label}"),
                remember: Some((SelfSettingKind::AccountProfile, account.to_string())),
            });
        }
        if let Some(directory) = requested
            .directory
            .as_deref()
            .filter(|directory| Some(*directory) != target.worktree_id())
            .filter(|directory| !allowed.allows(SelfSettingKind::Directory, directory))
        {
            needed.push(Needed {
                text: format!("directory to {directory}"),
                remember: Some((SelfSettingKind::Directory, directory.to_string())),
            });
        }
        if let Some(mode) = requested.mode {
            if !cap.mode.permits(mode) {
                return Err(refused(
                    "this agent's mode is limited by the agent above it and cannot be raised",
                ));
            }
            if !current.mode.permits(mode) {
                needed.push(Needed {
                    text: format!("mode to {}", format!("{mode:?}").to_lowercase()),
                    remember: None,
                });
            }
        }
        if let Some(level) = requested.permission_level {
            if !cap.permission_level.permits(level) {
                return Err(refused(
                    "this agent's permission level is limited by the agent above it and cannot be raised",
                ));
            }
            if !current.permission_level.permits(level) {
                needed.push(Needed {
                    text: format!(
                        "permission level to {}",
                        format!("{level:?}").to_lowercase()
                    ),
                    remember: None,
                });
            }
        }
        if needed.is_empty() {
            return Ok(());
        }
        // The owner authorized this agent's task with /sudo: that is the standing permission.
        if self
            .sudo_task_authorizes_agent(target.session_id(), target.id())
            .await
        {
            return Ok(());
        }
        let allow_always = needed.iter().any(|need| need.remember.is_some());
        let choice = self
            .ask_for_agent_settings_change(target, &needed, context.reason.as_deref(), allow_always)
            .await?;
        if choice == ALLOW_ALWAYS {
            let mut set = target.self_settings().clone();
            for (kind, value) in needed.iter().filter_map(|need| need.remember.as_ref()) {
                set = set.with_allowed(*kind, value);
            }
            let agent = owned
                .agent_store
                .set_agent_self_settings(target.id(), set)?;
            self.append_agent_durable_event("agent.updated", &agent, None)
                .await?;
        }
        Ok(())
    }

    async fn ask_for_agent_settings_change(
        &self,
        target: &crate::agent::AgentInstance,
        needed: &[Needed],
        reason: Option<&str>,
        allow_always: bool,
    ) -> Result<&'static str, DaemonError> {
        let owned = &self.owned;
        let session_id = target.session_id();
        let session = owned.session_store.get_session(session_id)?;
        let label = target.alias().unwrap_or_else(|| target.agent_ref());
        let wants = needed
            .iter()
            .map(|need| need.text.as_str())
            .collect::<Vec<_>>()
            .join(" and ");
        let mut description = format!("{label} wants to change its {wants}.");
        if let Some(reason) = reason {
            let reason = reason.chars().take(200).collect::<String>();
            description.push_str(&format!(" Its reason: {reason}"));
        }
        let id = format!("agent-settings-{:032x}", rand::random::<u128>());
        let mut choices = vec![RuntimeInteractionChoice::new(
            ALLOW_ONCE,
            "Allow once",
            ALLOW_ONCE,
            Some(RuntimeInteractionChoiceStyle::Primary),
        )];
        if allow_always {
            choices.push(RuntimeInteractionChoice::new(
                ALLOW_ALWAYS,
                "Allow always",
                ALLOW_ALWAYS,
                Some(RuntimeInteractionChoiceStyle::Secondary),
            ));
        }
        choices.push(RuntimeInteractionChoice::new(
            DENY,
            "Deny",
            DENY,
            Some(RuntimeInteractionChoiceStyle::Danger),
        ));
        let interaction =
            RuntimeInteraction::for_kernel_operation(&id, &id, "", description, choices)
                .with_related_agent_id(target.id().to_string());
        let receiver = self
            .create_kernel_operation_interaction(session_id, session.owner_user_id(), interaction)
            .await?;
        let answer = match tokio::time::timeout(DECISION_TIMEOUT, receiver).await {
            Ok(Ok(answer)) => answer,
            _ => {
                self.timeout_runtime_interaction(session_id, &id).await?;
                return Err(refused(
                    "the person did not answer in time; nothing was changed",
                ));
            }
        };
        if answer.status != "answered" {
            return Err(refused("the person did not allow this change"));
        }
        match answer.choice_id.as_deref() {
            Some(ALLOW_ONCE) => Ok(ALLOW_ONCE),
            Some(ALLOW_ALWAYS) if allow_always => Ok(ALLOW_ALWAYS),
            _ => Err(refused("the person did not allow this change")),
        }
    }
}

impl KernelRuntimeOwnedState {
    /// The account's name as people know it.
    pub(super) fn account_label(
        &self,
        agent: &crate::agent::AgentInstance,
        profile_id: &str,
    ) -> String {
        let owner = crate::account_profile::provider_account_authority_owner_user_id(
            &self.config_projection.snapshot(),
            agent.owner_user_id(),
        );
        self.provider_account_profiles
            .get(&owner, agent.provider(), profile_id)
            .map(|profile| profile.label)
            .unwrap_or_else(|_| profile_id.to_string())
    }
}
