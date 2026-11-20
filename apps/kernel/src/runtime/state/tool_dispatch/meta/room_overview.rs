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
        let visible = session
            .agents()
            .iter()
            .filter(|candidate| candidate.visible_in_freeform())
            .cloned()
            .collect::<Vec<_>>();
        let page = super::super::super::agent_rows::page_of_agents(&visible, &args.page).map_err(
            |message| DaemonError::LocalTransport {
                operation: "runtime_tool_meta.session_overview",
                message,
            },
        )?;
        let visible_ids = visible
            .iter()
            .map(|candidate| candidate.id())
            .collect::<std::collections::BTreeSet<_>>();
        let activity = self
            .agent_activity_for_session(&session)
            .into_iter()
            .filter(|(id, _)| visible_ids.contains(id.as_str()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let aliases = super::super::super::agent_rows::alias_index(&visible);
        let tasks = if args.include_tasks.unwrap_or(false) {
            let page_ids = page
                .agents
                .iter()
                .map(|agent| agent.id())
                .collect::<std::collections::BTreeSet<_>>();
            Some(
                self.owned
                    .durable_state_store
                    .agent_tasks(Some(session.id()), None)?
                    .into_iter()
                    .filter(|task| page_ids.contains(task.agent_id.as_str()))
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        let mut payload = serde_json::json!({
            "session_id": session.id(),
            "total": page.total,
            "next_cursor": page.next_cursor,
            "agents": page.agents.iter().map(|candidate| room_agent_row(&session, candidate, &aliases, &activity)).collect::<Vec<_>>(),
            "workflows": if include_workflows { session.workflows().to_vec() } else { Vec::new() },
            "workflow_runs": workflow_runs,
            "pending_interactions": session.active_interactions().iter().filter(|interaction| {
                !interaction.provider_login_is_human_only()
                    && interaction.agent_id().map_or(agent.owner_user_id() == session.owner_user_id(), |id| visible_ids.contains(id))
            }).map(|interaction| serde_json::json!({ "id": interaction.id(), "agent_id": interaction.agent_id(), "kind": interaction.kind() })).collect::<Vec<_>>(),
        });
        if let Some(tasks) = tasks {
            payload["agent_tasks"] = serde_json::json!(tasks);
        }
        Ok(RuntimeToolResult { ok: true, payload })
    }

    // MP-11: a shrinking roster cannot grant access to retained foreign traces.
    fn room_visible_workflow_runs(
        &self,
        session: &crate::session::RuntimeSession,
        user_id: &str,
    ) -> Result<Vec<crate::session::WorkflowRun>, DaemonError> {
        session.visible_workflow_runs_for_user(user_id, Some(&self.owned.durable_state_store))
    }
}

// MP-08 / MP-10 / MP-11: explicit public fields keep future provider/transport
// additions private without relying on the Room's registered-secret scrubber.
fn room_agent_row(
    session: &crate::session::RuntimeSession,
    agent: &crate::agent::AgentInstance,
    aliases: &std::collections::BTreeMap<&str, &str>,
    activity: &std::collections::BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity>,
) -> serde_json::Value {
    let mut public =
        super::super::super::agent_rows::compact_agent_row(session, agent, aliases, activity);
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
