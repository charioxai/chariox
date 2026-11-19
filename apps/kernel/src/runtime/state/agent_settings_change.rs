//! What changed when a person or an agent changed an agent's settings: the durable record and the
//! line the pane shows. The settings themselves are written by the profile and config updates.

use super::*;
use crate::local::SettingsApplies;

/// Who asked for a settings change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum SettingsOrigin {
    #[default]
    Person,
    Agent(String),
    /// An outside agent acting for the person through an owner grant.
    ExternalAgent,
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

/// The changes held for one agent and who asked last.
type PendingChanges = (Vec<SettingChange>, SettingsOrigin);

/// Changes accepted for a running turn that its process has not taken yet, by agent: the oldest
/// value and the newest of each field, and who asked last, so one swap shows one line.
#[derive(Debug, Clone, Default)]
pub(crate) struct PendingSettingsNotices {
    inner: Arc<std::sync::Mutex<BTreeMap<AgentKey, PendingChanges>>>,
}

impl PendingSettingsNotices {
    pub(crate) fn add(
        &self,
        session_id: &str,
        agent_id: &str,
        changes: Vec<SettingChange>,
        by: &SettingsOrigin,
    ) {
        let mut pending = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (entry, last_by) = pending
            .entry((session_id.to_string(), agent_id.to_string()))
            .or_insert_with(|| (Vec::new(), by.clone()));
        *last_by = by.clone();
        for change in changes {
            match entry.iter_mut().find(|held| held.field == change.field) {
                Some(held) => held.new = change.new,
                None => entry.push(change),
            }
        }
        entry.retain(|change| change.old != change.new);
    }

    pub(crate) fn take(&self, session_id: &str, agent_id: &str) -> Option<PendingChanges> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(session_id.to_string(), agent_id.to_string()))
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn agent_settings_view(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<AgentSettingsView, DaemonError> {
        let session = self.session_store.get_session(agent.session_id())?;
        let limit =
            self.agent_store
                .effective_execution_config(&session, agent, &self.session_store);
        let account = self.account_label(agent, agent.provider_account_profile());
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

    /// What the agent above allows this agent's mode and permission level, apart from any value
    /// the agent holds itself. Only these two settings have a limit from above.
    fn parent_cap(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<crate::session::EffectiveAgentExecutionConfig, DaemonError> {
        let session = self.session_store.get_session(agent.session_id())?;
        let mut capped = agent.clone();
        capped.set_execution_mode_override(None);
        capped.set_permission_level_override(None);
        capped.set_parent_limit_waived(false);
        Ok(self
            .agent_store
            .effective_execution_config(&session, &capped, &self.session_store))
    }

    /// Refuses a mode or permission level an agent asks for above what the agent above allows,
    /// and says so on the pane. The person's own request is never refused: it is applied and
    /// holds against that limit.
    pub(super) fn refuse_above_parent_cap(
        &self,
        agent: &crate::agent::AgentInstance,
        origin: &SettingsOrigin,
        mode: Option<crate::provider::AgentExecutionMode>,
        level: Option<crate::provider::AgentPermissionLevel>,
    ) -> Result<(), DaemonError> {
        if !matches!(origin, SettingsOrigin::Agent(_)) {
            return Ok(());
        }
        let cap = self.parent_cap(agent)?;
        let agents = self.agent_store.list_agents();
        let limiting =
            |holds_lower: &dyn Fn(&crate::session::EffectiveAgentExecutionConfig) -> bool| {
                if agent.peer_spawn_limit().as_ref().is_some_and(holds_lower) {
                    return "the spawning agent".to_string();
                }
                for parent in crate::agent::authority_limit::ancestors(agent, &agents) {
                    let Ok(room) = self.session_store.get_session(parent.session_id()) else {
                        break;
                    };
                    let own = crate::session::effective_agent_execution_config(&room, Some(parent));
                    if holds_lower(&own) {
                        return parent
                            .alias()
                            .unwrap_or_else(|| parent.agent_ref())
                            .to_string();
                    }
                    if parent.parent_limit_waived() {
                        break;
                    }
                }
                "the room".to_string()
            };
        let mut refused = Vec::new();
        if let Some(mode) = mode.filter(|mode| !cap.mode.permits(*mode)) {
            let name = limiting(&|own| !own.mode.permits(mode));
            refused.push(format!(
                "mode {mode:?} refused: limited by {name} to {:?}",
                cap.mode
            ));
        }
        if let Some(level) = level.filter(|level| !cap.permission_level.permits(*level)) {
            let name = limiting(&|own| !own.permission_level.permits(level));
            refused.push(format!(
                "permission {level:?} refused: limited by {name} to {:?}",
                cap.permission_level
            ));
        }
        if refused.is_empty() {
            return Ok(());
        }
        let message = format!(
            "{}; the person can raise it",
            refused.join("; ").to_lowercase()
        );
        self.record_notice_for_agent(
            agent.session_id(),
            None,
            Some(agent.id()),
            self.attachment_store
                .list_session_attachment_ids(agent.session_id()),
            message.clone(),
        );
        Err(DaemonError::LocalTransport {
            operation: "agent settings",
            message,
        })
    }

    /// Whether the person's mode or permission level for this agent sits above what the agent
    /// above allows, so it must hold against that limit.
    pub(super) fn person_raised_above_parent_cap(
        &self,
        agent: &crate::agent::AgentInstance,
        mode: Option<Option<crate::provider::AgentExecutionMode>>,
        level: Option<Option<crate::provider::AgentPermissionLevel>>,
    ) -> Result<bool, DaemonError> {
        let session = self.session_store.get_session(agent.session_id())?;
        let mut next = agent.clone();
        if let Some(mode) = mode {
            next.set_execution_mode_override(mode);
        }
        if let Some(level) = level {
            next.set_permission_level_override(level);
        }
        let own = crate::session::effective_agent_execution_config(&session, Some(&next));
        let cap = self.parent_cap(agent)?;
        Ok(!cap.mode.permits(own.mode) || !cap.permission_level.permits(own.permission_level))
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
            SettingsOrigin::ExternalAgent => serde_json::json!({"kind": "external_agent"}),
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

    /// Who asked, as the pane line names them.
    fn settings_origin_name(&self, origin: &SettingsOrigin) -> String {
        match origin {
            SettingsOrigin::Person => "you".to_string(),
            SettingsOrigin::ExternalAgent => "an outside agent, for you".to_string(),
            SettingsOrigin::Agent(id) => self
                .agent_store
                .get_agent(id)
                .map(|agent| {
                    agent
                        .alias()
                        .unwrap_or_else(|| agent.agent_ref())
                        .to_string()
                })
                .unwrap_or_else(|_| id.clone()),
        }
    }

    /// The pane line of a change: "Model changed to X by you, continuing." once the process took it,
    /// "Model changes to X by you when this turn ends." while it waits for the turn to end.
    pub(super) fn announce_settings_change(
        &self,
        session_id: &str,
        agent_id: &str,
        changes: &[SettingChange],
        by: &SettingsOrigin,
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
        line.push_str(&format!(" by {}", self.settings_origin_name(by)));
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

    /// Whether the process can be relaunched on the agent's profile now. The worker relaunches a
    /// leased Claude process with the credential the home launched it with; another account's
    /// has none there until the next prompt brings it, so a change to it waits for the turn end.
    pub(super) fn can_relaunch_with_held_credential(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        agent: &crate::agent::AgentInstance,
    ) -> bool {
        !self
            .provider_run_projection
            .is_leased_provider_run(run.id())
            || crate::provider::canonical_provider_family(agent.provider())
                .unwrap_or(agent.provider())
                != "claude"
            || self
                .leased_launch_credentials
                .for_profile(
                    agent.id(),
                    agent.provider(),
                    agent.provider_account_profile(),
                )
                .is_some()
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

    pub(crate) fn external_agent(reason: Option<String>) -> Self {
        Self {
            origin: SettingsOrigin::ExternalAgent,
            reason: reason
                .map(|reason| reason.trim().to_string())
                .filter(|reason| !reason.is_empty()),
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
        // A leased agent's process runs on the worker, which settles the same change against its
        // own backing agent and tells the agent at its step boundary; the home only records it.
        let worker_applies = active_turn && agent.remote_execution().is_some();
        let applies = if worker_applies {
            SettingsApplies::AtStepBoundary
        } else if active_turn {
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
        if worker_applies {
            return Ok(applies);
        }
        match applies {
            SettingsApplies::Now => owned.announce_settings_change(
                agent.session_id(),
                agent.id(),
                &changes,
                &context.origin,
                false,
                false,
            ),
            SettingsApplies::AtStepBoundary => {
                owned.pending_settings_notices.add(
                    agent.session_id(),
                    agent.id(),
                    changes,
                    &context.origin,
                );
            }
            SettingsApplies::AtTurnEnd => owned.announce_settings_change(
                agent.session_id(),
                agent.id(),
                &changes,
                &context.origin,
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
        let relaunchable = owned.can_relaunch_with_held_credential(&run, agent);
        if relaunchable
            && owned
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
        pending.add(
            "s",
            "a",
            vec![change("model", "haiku", "sonnet")],
            &SettingsOrigin::Person,
        );
        pending.add(
            "s",
            "a",
            vec![change("model", "sonnet", "opus")],
            &SettingsOrigin::Person,
        );
        pending.add(
            "s",
            "a",
            vec![change("effort", "low", "high")],
            &SettingsOrigin::Person,
        );
        let (changes, by) = pending.take("s", "a").expect("changes are pending");
        assert_eq!(
            changes,
            vec![
                change("model", "haiku", "opus"),
                change("effort", "low", "high")
            ]
        );
        assert_eq!(by, SettingsOrigin::Person);
        assert!(pending.take("s", "a").is_none());
    }

    #[test]
    fn a_change_taken_back_before_the_step_ends_shows_nothing() {
        let pending = PendingSettingsNotices::default();
        pending.add(
            "s",
            "a",
            vec![change("model", "haiku", "sonnet")],
            &SettingsOrigin::Person,
        );
        pending.add(
            "s",
            "a",
            vec![change("model", "sonnet", "haiku")],
            &SettingsOrigin::Person,
        );
        let (changes, _) = pending.take("s", "a").unwrap_or_default();
        assert!(changes.is_empty());
    }
}
