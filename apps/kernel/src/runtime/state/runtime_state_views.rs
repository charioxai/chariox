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
        self.owned.active_turns.start(
            ActiveTurnState::new(
                session_id.to_string(),
                agent_id.to_string(),
                prompt_id.to_string(),
                provider_run_id.to_string(),
            )
            .with_trace_id(trace_id),
        );
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
        let active_prompt = session.active_prompt_for_agent(agent_id);
        let prompt_summary = active_prompt
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
            source_attachment_id: active_prompt.map(|prompt| prompt.source_attachment_id().to_string()),
            prompt_origin: active_prompt.map(|prompt| prompt.prompt_origin()),
            external_provider: active_prompt
                .and_then(|prompt| prompt.external_provider().map(str::to_string)),
            external_provider_session_id: active_prompt
                .and_then(|prompt| prompt.external_provider_session_id().map(str::to_string)),
            external_provider_turn_id: active_prompt
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
