use std::collections::{BTreeMap, BTreeSet};

use crate::app::ActiveTurnState;

use super::*;

#[derive(serde::Serialize)]
pub(crate) struct AgentInspectionFailure {
    outcome: crate::provider::TurnOutcome,
    finished_at_ms: u64,
}

impl KernelRuntimeState {
    pub(crate) fn durable_state_store(&self) -> &DurableKernelStateStore {
        &self.owned.durable_state_store
    }

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
        let activity_mutation = self.owned.begin_managed_activity_mutation();
        self.owned.active_turns.start(active_turn);
        activity_mutation.record();
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

    pub(crate) fn agent_inspection_status(
        &self,
        agent: &crate::agent::AgentInstance,
        activity: Option<&crate::runtime::projection::AgentRuntimeActivity>,
        deadline: Option<&crate::local::AgentDeadline>,
    ) -> Result<(&'static str, Option<AgentInspectionFailure>), DaemonError> {
        if deadline.is_some_and(|deadline| deadline.stopped_at_ms.is_some()) {
            return Ok(("ended (deadline)", None));
        }

        let state = crate::runtime::projection::agent_state_label(agent, activity);
        if matches!(state, "working" | "waiting") || activity.is_some_and(|activity| activity.busy)
        {
            return Ok((state, None));
        }

        // Read the settlement for both the live and restarted kernel so inspection
        // uses the original timestamp even if a later Git observation was merged in.
        let failure = self
            .owned
            .operational_history_store
            .load_latest_agent_settlement_event(agent.session_id(), agent.id())?
            .and_then(|event| {
                use crate::history::{
                    PROMPT_SETTLED_AT_MS_METADATA_KEY, TURN_OUTCOME_METADATA_KEY,
                };
                let outcome: crate::provider::TurnOutcome =
                    serde_json::from_value(event.metadata.get(TURN_OUTCOME_METADATA_KEY)?.clone())
                        .ok()?;
                let finished_at_ms = event
                    .metadata
                    .get(PROMPT_SETTLED_AT_MS_METADATA_KEY)?
                    .as_u64()?;
                outcome.is_error().then_some(AgentInspectionFailure {
                    outcome,
                    finished_at_ms,
                })
            });
        Ok((if failure.is_some() { "error" } else { state }, failure))
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
        let mut activity = crate::runtime::projection::agent_activity_for_session_projection(
            session,
            // The provider store moves a run's output clock; the projection holds a
            // copy as of its last publish, which is read only while a launch holds
            // the store.
            |agent_id| {
                self.owned
                    .provider_store
                    .try_get_agent_run_for_projection(session.id(), agent_id)
                    .or_else(|| {
                        self.owned
                            .provider_run_projection
                            .get_for_agent(session.id(), agent_id)
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
        );
        // Durable liveness reads must not hold up provider output or settlement.
        drop(prompt_activity);
        self.owned
            .session_projection
            .project_external_observed_activity(session.id(), &mut activity);
        crate::runtime::projection::annotate_waits(
            &mut activity,
            &self.session_processes(session.id()),
            &crate::runtime::projection::YieldedSources::of_room(
                &self.owned.durable_state_store,
                session.id(),
            ),
            &crate::runtime::projection::SourceLabels::of_session(session),
        );
        crate::runtime::projection::annotate_cards(
            &mut activity,
            session,
            &self.decisions_held_by_agents(session.id()),
        );
        if let Err(error) = crate::runtime::projection::annotate_deadline_ends(
            &mut activity,
            &self.owned.durable_state_store,
            session.id(),
        ) {
            tracing::warn!(%error, session_id = session.id(), "could not project stopped agent deadlines");
        }
        activity
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

    pub(crate) fn agent_for_parent(
        &self,
        id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned.agent_store.get_agent(id)
    }

    pub(crate) fn parent_execution_limit(
        &self,
        room: &crate::session::RuntimeSession,
        parent: &crate::agent::AgentInstance,
    ) -> Result<crate::session::EffectiveAgentExecutionConfig, DaemonError> {
        if parent.session_id() == room.id() {
            Ok(self.owned.agent_store.effective_execution_config(
                room,
                parent,
                &self.owned.session_store,
            ))
        } else {
            let parent_room = self.owned.session_store.get_session(parent.session_id())?;
            Ok(self.owned.agent_store.effective_execution_config(
                &parent_room,
                parent,
                &self.owned.session_store,
            ))
        }
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
