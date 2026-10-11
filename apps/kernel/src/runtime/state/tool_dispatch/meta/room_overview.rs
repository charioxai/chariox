//! MP-08 / MP-10 / MP-11: public Room overview and collaboration visibility.
use super::*;

impl KernelRuntimeState {
    pub(super) fn room_session_overview(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaSessionOverviewArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let session = self.owned.session_snapshot(session.id())?;
        let include_workflows = args.include_workflows.unwrap_or(true);
        let workflow_runs = if include_workflows {
            self.room_visible_workflow_runs(&session, agent.owner_user_id())?
        } else {
            Vec::new()
        };
        let session = session.redacted_for_user(agent.owner_user_id());
        let visible_agents = session
            .agents()
            .iter()
            .filter(|candidate| candidate.visible_in_freeform())
            .collect::<Vec<_>>();
        let visible_ids = visible_agents
            .iter()
            .map(|candidate| candidate.id())
            .collect::<std::collections::BTreeSet<_>>();
        let activity = self
            .agent_activity_for_session(&session)
            .into_iter()
            .filter(|(id, _)| visible_ids.contains(id.as_str()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let tasks = self
            .owned
            .durable_state_store
            .agent_tasks(Some(session.id()), None)?
            .into_iter()
            .filter(|task| visible_ids.contains(task.agent_id.as_str()))
            .collect::<Vec<_>>();
        return Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "session_id": session.id(),
                "agents": visible_agents.iter().map(|candidate| room_public_agent_ref_json(candidate)).collect::<Vec<_>>(),
                "workflows": if include_workflows { session.workflows().to_vec() } else { Vec::new() },
                "workflow_runs": workflow_runs,
                "activity": activity,
                "agent_tasks": tasks,
                "pending_interactions": session.active_interactions().iter().filter(|interaction| {
                    !interaction.provider_login_is_human_only()
                        && interaction.agent_id().map_or(agent.owner_user_id() == session.owner_user_id(), |id| visible_ids.contains(id))
                }).map(|interaction| serde_json::json!({ "id": interaction.id(), "agent_id": interaction.agent_id(), "kind": interaction.kind() })).collect::<Vec<_>>(),
            }),
        });
    }

    // MP-11: a shrinking roster cannot grant access to retained foreign traces.
    fn room_visible_workflow_runs(
        &self,
        session: &crate::session::RuntimeSession,
        user_id: &str,
    ) -> Result<Vec<crate::session::WorkflowRun>, DaemonError> {
        if session
            .collaboration_level_for_user(user_id)
            .is_some_and(|level| level.can_view_agent_trace())
        {
            return Ok(session.workflow_runs().to_vec());
        }
        let mut ownership = session
            .agents()
            .iter()
            .map(|agent| (agent.id().to_string(), agent.owner_user_id() == user_id))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut visible = Vec::new();
        for run in session.workflow_runs() {
            let Some(creator) = run.created_by_agent_id() else {
                continue;
            };
            let participants = std::iter::once(creator)
                .chain(run.node_runs().iter().map(|node| node.agent_id()))
                .chain(run.runtime_agent_ids_by_node().values().map(String::as_str));
            let mut authorized = true;
            for id in participants {
                if !ownership.contains_key(id) {
                    // Deletion retains a kernel-owned provenance record. Read only
                    // its ownership fields; never return the deleted AgentInstance.
                    let events = self.owned.durable_state_store.load_subject_events_by_kind(
                        id,
                        "agent.deleted",
                        1,
                    )?;
                    let owned = events.last().is_some_and(|event| {
                        let agent = &event.payload["agent"];
                        agent["id"].as_str() == Some(id)
                            && agent["session_id"].as_str() == Some(session.id())
                            && agent["owner_user_id"].as_str() == Some(user_id)
                    });
                    ownership.insert(id.to_string(), owned);
                }
                if !ownership[id] {
                    authorized = false;
                    break;
                }
            }
            if authorized {
                visible.push(run.clone());
            }
        }
        Ok(visible)
    }
}

// MP-08 / MP-10 / MP-11: explicit public fields keep future provider/transport
// additions private without relying on the Room's registered-secret scrubber.
fn room_public_agent_ref_json(agent: &crate::agent::AgentInstance) -> serde_json::Value {
    let mut public = meta_agent_ref_json(agent);
    if let Some(binding) = agent.remote_execution() {
        public["remote_execution"] = serde_json::json!({
            "worker_kernel_id": binding.worker_kernel_id,
            "worker_machine_id": binding.worker_machine_id,
            "execution_lease_id": binding.execution_lease_id,
            "leased_agent_id": binding.leased_agent_id,
            "active_worker_provider_run_id": binding.active_worker_provider_run_id,
            "relay_url": binding.relay_url,
            "relay_peer_protocol_version": binding.relay_peer_protocol_version,
        });
    }
    public
}
