//! What changed when a person or an agent changed an agent's settings: the durable record and the
//! line the pane shows. The settings themselves are written by the profile and config updates.

use super::*;
use crate::local::SettingsApplies;

/// Who asked for a settings change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SettingsOrigin {
    Person,
    Agent(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettingChange {
    pub(crate) field: &'static str,
    pub(crate) old: String,
    pub(crate) new: String,
}

/// An agent's settings as people read them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSettingsView {
    provider: String,
    account: String,
    model: String,
    effort: String,
    mode: String,
    permission: String,
    directory: String,
}

impl AgentSettingsView {
    pub(crate) fn changes_to(&self, after: &Self) -> Vec<SettingChange> {
        [
            ("provider", &self.provider, &after.provider),
            ("account", &self.account, &after.account),
            ("model", &self.model, &after.model),
            ("effort", &self.effort, &after.effort),
            ("mode", &self.mode, &after.mode),
            ("permission", &self.permission, &after.permission),
            ("directory", &self.directory, &after.directory),
        ]
        .into_iter()
        .filter(|(_, old, new)| old != new)
        .map(|(field, old, new)| SettingChange {
            field,
            old: old.clone(),
            new: new.clone(),
        })
        .collect()
    }
}

/// One agent of one session.
type AgentKey = (String, String);

/// Changes accepted for a running turn that its process has not taken yet, by agent: the oldest
/// value and the newest of each field, so one swap shows one line.
#[derive(Debug, Clone, Default)]
pub(crate) struct PendingSettingsNotices {
    inner: Arc<std::sync::Mutex<BTreeMap<AgentKey, Vec<SettingChange>>>>,
}

impl PendingSettingsNotices {
    pub(crate) fn add(&self, session_id: &str, agent_id: &str, changes: Vec<SettingChange>) {
        let mut pending = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = pending
            .entry((session_id.to_string(), agent_id.to_string()))
            .or_default();
        for change in changes {
            match entry.iter_mut().find(|held| held.field == change.field) {
                Some(held) => held.new = change.new,
                None => entry.push(change),
            }
        }
        entry.retain(|change| change.old != change.new);
    }

    pub(crate) fn take(&self, session_id: &str, agent_id: &str) -> Vec<SettingChange> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(session_id.to_string(), agent_id.to_string()))
            .unwrap_or_default()
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn agent_settings_view(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<AgentSettingsView, DaemonError> {
        let session = self.session_store.get_session(agent.session_id())?;
        let agents = self.agent_store.get_session_agents(agent.session_id());
        let limit =
            crate::session::effective_agent_execution_config_in_tree(&session, agent, &agents);
        let owner = crate::account_profile::provider_account_authority_owner_user_id(
            &self.config_projection.snapshot(),
            agent.owner_user_id(),
        );
        let account = self
            .provider_account_profiles
            .get(&owner, agent.provider(), agent.provider_account_profile())
            .map(|profile| profile.label)
            .unwrap_or_else(|_| agent.provider_account_profile().to_string());
        Ok(AgentSettingsView {
            provider: agent.provider().to_string(),
            account,
            model: agent.model().unwrap_or("default").to_string(),
            effort: agent.effort().unwrap_or("default").to_string(),
            mode: format!("{:?}", limit.mode).to_lowercase(),
            permission: format!("{:?}", limit.permission_level).to_lowercase(),
            directory: agent
                .worktree_id()
                .unwrap_or_else(|| session.worktree_id())
                .to_string(),
        })
    }

    /// The durable record of an accepted change: old -> new, who asked, why, and when it applies.
    pub(super) fn record_settings_change(
        &self,
        agent_id: &str,
        changes: &[SettingChange],
        origin: &SettingsOrigin,
        reason: Option<&str>,
        applies: SettingsApplies,
    ) {
        let who = match origin {
            SettingsOrigin::Person => serde_json::json!({"kind": "person"}),
            SettingsOrigin::Agent(id) => serde_json::json!({"kind": "agent", "agent_id": id}),
        };
        let payload = serde_json::json!({
            "agent_id": agent_id,
            "changes": changes.iter().map(|change| serde_json::json!({
                "field": change.field, "old": change.old, "new": change.new,
            })).collect::<Vec<_>>(),
            "by": who,
            "reason": reason,
            "applies": match applies {
                SettingsApplies::Now => "now",
                SettingsApplies::AtStepBoundary => "at_step_boundary",
                SettingsApplies::AtTurnEnd => "at_turn_end",
            },
        });
        if let Err(error) = self.durable_state_store.append_event(
            "agent.settings_changed",
            Some(agent_id.to_string()),
            payload,
        ) {
            crate::logging::warn_with_fields(
                "daemon.agent_settings",
                "could not record a settings change",
                serde_json::json!({"agent_id": agent_id, "error": error.to_string()}),
            );
        }
    }

    /// The pane line of a change: "Model changed to X, continuing." once the process took it,
    /// "Model changes to X when this turn ends." while it waits for the turn to end.
    pub(super) fn announce_settings_change(
        &self,
        session_id: &str,
        agent_id: &str,
        changes: &[SettingChange],
        waiting_for_turn_end: bool,
        continuing: bool,
    ) {
        if changes.is_empty() {
            return;
        }
        let parts = changes
            .iter()
            .map(|change| {
                let noun = change.field;
                if waiting_for_turn_end {
                    format!("{noun} changes to {}", change.new)
                } else {
                    format!("{noun} changed to {}", change.new)
                }
            })
            .collect::<Vec<_>>();
        let mut line = parts.join(", ");
        if waiting_for_turn_end {
            line.push_str(" when this turn ends");
        } else if continuing {
            line.push_str(", continuing");
        }
        line.push('.');
        let mut chars = line.chars();
        let line = chars
            .next()
            .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_default();
        self.record_notice_for_agent(
            session_id,
            None,
            Some(agent_id),
            self.attachment_store
                .list_session_attachment_ids(session_id),
            line,
        );
    }
}

/// Who asked for a change and why.
#[derive(Debug, Clone)]
pub(crate) struct SettingsContext {
    pub(crate) origin: SettingsOrigin,
    pub(crate) reason: Option<String>,
}

impl SettingsContext {
    pub(crate) fn person() -> Self {
        Self {
            origin: SettingsOrigin::Person,
            reason: None,
        }
    }

    pub(crate) fn agent(agent_id: &str, reason: Option<String>) -> Self {
        Self {
            origin: SettingsOrigin::Agent(agent_id.to_string()),
            reason: reason
                .map(|reason| reason.trim().to_string())
                .filter(|reason| !reason.is_empty()),
        }
    }
}

impl KernelRuntimeState {
    /// Records a change that was written to `agent`, and says when its provider takes it. For a
    /// running turn the provider's process is marked stale when it cannot take the change in
    /// place: the step-boundary swap or the next dispatch then replaces it.
    pub(super) fn settle_settings_change(
        &self,
        before: &AgentSettingsView,
        agent: &crate::agent::AgentInstance,
        context: &SettingsContext,
        active_turn: bool,
    ) -> Result<SettingsApplies, DaemonError> {
        let owned = &self.owned;
        let changes = before.changes_to(&owned.agent_settings_view(agent)?);
        if changes.is_empty() {
            return Ok(SettingsApplies::Now);
        }
        let applies = if active_turn {
            self.mark_running_process_stale(agent, &changes)
        } else {
            SettingsApplies::Now
        };
        owned.record_settings_change(
            agent.id(),
            &changes,
            &context.origin,
            context.reason.as_deref(),
            applies,
        );
        match applies {
            SettingsApplies::Now => owned.announce_settings_change(
                agent.session_id(),
                agent.id(),
                &changes,
                false,
                false,
            ),
            SettingsApplies::AtStepBoundary => {
                owned
                    .pending_settings_notices
                    .add(agent.session_id(), agent.id(), changes);
            }
            SettingsApplies::AtTurnEnd => owned.announce_settings_change(
                agent.session_id(),
                agent.id(),
                &changes,
                true,
                false,
            ),
        }
        Ok(applies)
    }

    fn mark_running_process_stale(
        &self,
        agent: &crate::agent::AgentInstance,
        changes: &[SettingChange],
    ) -> SettingsApplies {
        let owned = &self.owned;
        let Some(run) = owned
            .provider_store
            .get_run_for_agent(agent.session_id(), agent.id())
            .filter(|run| run.state() != crate::provider::ProviderRunState::Ended)
        else {
            return SettingsApplies::Now;
        };
        let in_place = owned
            .provider_store
            .adapter_supports_turn_scoped_execution_config(run.adapter_key());
        let process_bound = changes
            .iter()
            .any(|change| !(in_place && matches!(change.field, "mode" | "permission")));
        if !process_bound || !run.client_interface().is_chariox() {
            return SettingsApplies::Now;
        }
        if let Err(error) = owned.provider_store.mark_run_settings_stale(run.id()) {
            crate::logging::warn_with_fields(
                "daemon.agent_settings",
                "could not mark a provider run for a settings change",
                serde_json::json!({"provider_run_id": run.id(), "error": error.to_string()}),
            );
        }
        let canonical = |provider: &str| {
            crate::provider::canonical_provider_family(provider)
                .unwrap_or(provider)
                .to_string()
        };
        if owned
            .provider_store
            .adapter_resumes_conversation_in_new_process(run.adapter_key())
            && canonical(run.provider()) == canonical(agent.provider())
        {
            SettingsApplies::AtStepBoundary
        } else {
            SettingsApplies::AtTurnEnd
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(field: &'static str, old: &str, new: &str) -> SettingChange {
        SettingChange {
            field,
            old: old.into(),
            new: new.into(),
        }
    }

    #[test]
    fn several_changes_of_a_step_keep_the_first_old_and_the_last_new_value() {
        let pending = PendingSettingsNotices::default();
        pending.add("s", "a", vec![change("model", "haiku", "sonnet")]);
        pending.add("s", "a", vec![change("model", "sonnet", "opus")]);
        pending.add("s", "a", vec![change("effort", "low", "high")]);
        assert_eq!(
            pending.take("s", "a"),
            vec![
                change("model", "haiku", "opus"),
                change("effort", "low", "high")
            ]
        );
        assert!(pending.take("s", "a").is_empty());
    }

    #[test]
    fn a_change_taken_back_before_the_step_ends_shows_nothing() {
        let pending = PendingSettingsNotices::default();
        pending.add("s", "a", vec![change("model", "haiku", "sonnet")]);
        pending.add("s", "a", vec![change("model", "sonnet", "haiku")]);
        assert!(pending.take("s", "a").is_empty());
    }
}
