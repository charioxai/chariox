use std::collections::{BTreeMap, BTreeSet};

use crate::app::ActiveTurnState;

use super::*;

impl KernelRuntimeState {
    pub(crate) fn start_active_turn_with_trace_id(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        trace_id: &str,
    ) {
        let active_prompt = self
            .owned
            .session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                self.owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, agent_id)
                    .filter(|prompt| prompt_matches_active_turn_id(prompt, prompt_id))
            });
        let mut active_turn = ActiveTurnState::new(
            session_id.to_string(),
            agent_id.to_string(),
            prompt_id.to_string(),
            provider_run_id.to_string(),
        )
        .with_trace_id(trace_id);
        if let Some(prompt) = active_prompt.as_ref() {
            active_turn = active_turn.with_prompt_metadata(prompt);
        }
        self.owned.active_turns.start(active_turn);
        if self
            .owned
            .git_turn_snapshots
            .get(provider_run_id, prompt_id)
            .is_some()
        {
            return;
        }
        let Ok(provider_run) = self.owned.provider_store.get_run(provider_run_id) else {
            return;
        };
        let Ok(session) = self.owned.session_store.get_session(session_id) else {
            return;
        };
        let worktree_path = provider_run
            .working_directory()
            .cloned()
            .unwrap_or_else(|| std::path::PathBuf::from(session.worktree_id()));
        let prompt_summary = active_prompt
            .as_ref()
            .map(|prompt| {
                crate::prompt_transcript::render_prompt_transcript(
                    prompt.prompt(),
                    prompt.attachments(),
                )
            })
            .unwrap_or_default();
        let context = crate::git_observer::GitTurnContext {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            provider: provider_run.provider().to_string(),
            model: provider_run.model().to_string(),
            provider_run_id: provider_run_id.to_string(),
            provider_session_id: provider_run.provider_session_id().map(str::to_string),
            prompt_id: prompt_id.to_string(),
            turn_id: prompt_id.to_string(),
            source_attachment_id: active_prompt
                .as_ref()
                .map(|prompt| prompt.source_attachment_id().to_string()),
            prompt_origin: active_prompt.as_ref().map(|prompt| prompt.prompt_origin()),
            external_provider: active_prompt
                .as_ref()
                .and_then(|prompt| prompt.external_provider().map(str::to_string)),
            external_provider_session_id: active_prompt
                .as_ref()
                .and_then(|prompt| prompt.external_provider_session_id().map(str::to_string)),
            external_provider_turn_id: active_prompt
                .as_ref()
                .and_then(|prompt| prompt.external_provider_turn_id().map(str::to_string)),
            started_at_ms: self
                .owned
                .active_turns
                .snapshot()
                .get(provider_run_id)
                .map(|turn| turn.started_at_ms),
            worktree_path,
            workspace_live_sync_tracked: provider_run.tracks_workspace_live_sync(),
            machine_id: None,
            prompt_summary,
        };
        if let Some(snapshot) = crate::git_observer::capture_turn_snapshot(context) {
            self.owned.git_turn_snapshots.insert(snapshot);
        }
    }

    pub(crate) fn agent_activity_for_session(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity> {
        self.agent_activity_for_session_with_unread(session, None)
    }

    pub(crate) fn active_turn_snapshot(&self) -> BTreeMap<String, ActiveTurnState> {
        self.owned.active_turns.snapshot()
    }

    pub(crate) fn agent_activity_for_session_with_unread(
        &self,
        session: &crate::session::RuntimeSession,
        unread_for_user_id: Option<&str>,
    ) -> BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity> {
        let prompt_activity = self.owned.prompt_activity.read();
        let active_turns = self.owned.active_turns.snapshot();
        crate::runtime::projection::agent_activity_for_session_projection(
            session,
            |agent_id| {
                self.owned
                    .provider_run_projection
                    .get_for_agent(session.id(), agent_id)
                    .or_else(|| {
                        self.owned
                            .provider_store
                            .get_run_for_agent(session.id(), agent_id)
                    })
            },
            &prompt_activity,
            &active_turns,
            unread_for_user_id,
            |agent_id| {
                self.owned
                    .completed_git_turn_snapshots
                    .latest_projection_for_agent(session.id(), agent_id)
            },
        )
    }

    pub(crate) async fn config_snapshot(&self) -> crate::config::DaemonConfig {
        self.owned.config_projection.snapshot()
    }

    pub(crate) fn list_agents(&self) -> Vec<crate::agent::AgentInstance> {
        self.owned.agent_store.list_agents()
    }

    pub(crate) fn session_agent_snapshot(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(crate::session::RuntimeSession, crate::agent::AgentInstance), DaemonError> {
        let session = self
            .owned
            .session_snapshot_without_projection_update(session_id)?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        Ok((session, agent))
    }

    pub(crate) fn session_agents(&self, session_id: &str) -> Vec<crate::agent::AgentInstance> {
        self.owned.agent_store.get_session_agents(session_id)
    }

    pub(crate) fn client_attachment_for_session(
        &self,
        client_id: &str,
        session_id: &str,
    ) -> Option<crate::attachment::RuntimeAttachment> {
        self.owned
            .attachment_store
            .list_client_attachments(client_id)
            .into_iter()
            .find(|attachment| attachment.session_id() == session_id)
    }

    pub(crate) fn append_metaagent_command_audit_event(
        &self,
        metaagent_id: &str,
        payload: serde_json::Value,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            "metaagent.command.executed",
            Some(metaagent_id.to_string()),
            payload,
        )?;
        Ok(())
    }

    pub(crate) async fn workspace_live_sync_health_snapshot(
        &self,
    ) -> crate::runtime::projection::WorkspaceLiveSyncHealthSnapshot {
        let reservations = self
            .owned
            .workspace_live_sync_coordinator
            .lock()
            .await
            .active_reservation_snapshots();
        let active_reservation_artifacts = reservations
            .iter()
            .map(|reservation| reservation.artifact_id.clone())
            .collect::<BTreeSet<_>>()
            .len();
        crate::runtime::projection::WorkspaceLiveSyncHealthSnapshot {
            active_reservations: reservations.len(),
            active_reservation_artifacts,
            managed_mode:
                crate::runtime::projection::WorkspaceLiveSyncManagedModeHealthSnapshot::current(),
            workspace_identity: self.owned.workspace_identity_monitor.health_snapshot(),
            external_changes: self
                .owned
                .workspace_live_sync_external_changes
                .health_snapshot(),
        }
    }
}

fn prompt_matches_active_turn_id(
    prompt: &crate::session::PromptQueueItem,
    prompt_id: &str,
) -> bool {
    prompt.id() == prompt_id || prompt.pending_prompt_id() == Some(prompt_id)
}
