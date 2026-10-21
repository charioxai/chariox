//! MP-08 / MP-09 / MP-10 / MP-11 A12: explicit, fenced legacy adoption.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, Operation, Outcome};

impl KernelRuntimeState {
    /// Old snapshots cannot prove whether the registry template was invoked
    /// through `/goal` or manually. Offer adoption without changing its work.
    pub(super) fn offer_legacy_goal_adoption(&self) -> Result<(), DaemonError> {
        for session in self.owned.session_store.list_all_sessions() {
            for run in session
                .workflow_runs()
                .iter()
                .filter(|r| !r.status().is_terminal())
            {
                let Some(workflow) = session
                    .workflows()
                    .iter()
                    .find(|w| w.id() == run.workflow_id())
                else {
                    continue;
                };
                if !workflow.alias().is_some_and(|a| {
                    a == "planner-worker-reviewer" || a.starts_with("planner-worker-reviewer-")
                }) {
                    continue;
                }
                if self
                    .owned
                    .durable_state_store
                    .agent_tasks(Some(session.id()), None)?
                    .iter()
                    .any(|t| {
                        t.goal
                            .as_ref()
                            .is_some_and(|g| g.legacy_run_id.as_deref() == Some(run.id()))
                    })
                {
                    continue;
                }
                if !self
                    .owned
                    .durable_state_store
                    .load_subject_events_by_kind(run.id(), "goal.legacy_adoption_offered", 1)?
                    .is_empty()
                {
                    continue;
                }
                self.owned.durable_state_store.append_event("goal.legacy_adoption_offered", Some(run.id().into()),
                    serde_json::json!({"room_id":session.id(),"owner_user_id":session.owner_user_id(),"legacy_run_id":run.id(),"requires_owner_objective":true}))?;
                self.owned.record_notice_for_agent(session.id(), None, None,
                    self.owned.attachment_store.list_session_attachment_ids(session.id()),
                    format!("Legacy planner/worker/reviewer invocation {} keeps draining. To adopt it as a goal, focus its entry agent and enter /goal adopt {} <owner-confirmed objective>. No node is relaunched; completed/cancelled runs remain terminal.",run.id(),run.id()));
            }
        }
        Ok(())
    }

    pub(super) fn adopt_legacy_goal(
        &self,
        request: &crate::local::SubmitPromptRequest,
        owner: &str,
    ) -> Result<Option<LocalDaemonResponse>, DaemonError> {
        let Some(rest) = request.prompt.trim().strip_prefix("/goal adopt ") else {
            return Ok(None);
        };
        let (run_id, objective) = rest.split_once(char::is_whitespace).ok_or_else(|| {
            ledger::error("usage: /goal adopt <legacy-run-id> <owner-confirmed objective>")
        })?;
        if !self.owned.config_projection.snapshot().room_agent_tools {
            return Err(ledger::error("hybrid goals require room agent tools"));
        }
        let session = self.owned.session_store.get_session(&request.session_id)?;
        let attachment = self
            .owned
            .ensure_attachment_in_session(session.id(), &request.attachment_id)?;
        if owner != session.owner_user_id()
            || attachment.owner_user_id() != owner
            || !matches!(
                attachment.capability_level(),
                crate::attachment::ClientCapabilityLevel::FullTerminal
                    | crate::attachment::ClientCapabilityLevel::InteractiveStructured
            )
        {
            return Err(ledger::error(
                "only the room owner can adopt a legacy invocation",
            ));
        }
        let run = session
            .workflow_runs()
            .iter()
            .find(|r| r.id() == run_id)
            .ok_or_else(|| ledger::error("legacy invocation is unavailable in this room"))?;
        if run.status().is_terminal() {
            return Err(ledger::error(
                "completed/cancelled legacy invocations remain terminal",
            ));
        }
        let workflow = session
            .workflows()
            .iter()
            .find(|w| w.id() == run.workflow_id())
            .ok_or_else(|| {
                ledger::error("legacy definition unavailable; let it drain and restore its binding")
            })?;
        let endpoint = workflow
            .endpoints()
            .iter()
            .find(|e| e.id() == run.endpoint_id())
            .ok_or_else(|| ledger::error("legacy endpoint binding unavailable"))?;
        let entry = workflow
            .nodes()
            .iter()
            .find(|n| n.id() == run.entry_node_id())
            .ok_or_else(|| ledger::error("legacy entry binding unavailable"))?;
        let agent_id = run
            .runtime_agent_id_for_node(entry.id())
            .unwrap_or(entry.agent_id());
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if endpoint.owner_user_id() != owner
            || entry.owner_user_id() != owner
            || agent.owner_user_id() != owner
            || agent.session_id() != session.id()
            || agent.is_metaagent()
            || request
                .target_agent_id
                .as_deref()
                .is_some_and(|a| a != agent_id)
        {
            return Err(ledger::error("adoption requires the owner and authoritative ordinary entry agent; focus that agent"));
        }
        let id = format!("goal-adopt-{run_id}");
        if self
            .owned
            .durable_state_store
            .agent_tasks(Some(session.id()), Some(agent_id))?
            .iter()
            .any(|t| {
                t.goal.is_some()
                    && t.task_id != id
                    && !matches!(
                        t.state,
                        ledger::ExecutionState::Done | ledger::ExecutionState::Cancelled
                    )
            })
        {
            return Err(ledger::error("entry agent already has an unfinished goal"));
        }
        // The objective is the owner's own text; Room protection does not scrub it.
        let objective = objective.trim().to_string();
        let Outcome::Task(_) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::AdoptGoal {
                    owner: owner.into(),
                    room: session.id().into(),
                    agent: agent_id.into(),
                    run: run.id().into(),
                    nodes: workflow.nodes().iter().map(|n| n.id().into()).collect(),
                    objective,
                    now: crate::session::unix_epoch_ms(),
                })?
        else {
            unreachable!()
        };
        // Keep the legacy dispatcher as the sole owner of its existing nodes.
        // Only shared source/check-in receipts can admit a later goal turn.
        self.owned
            .terminal_stream
            .notify_terminal_projection_change(session.id());
        let snapshot = self.owned.session_snapshot(session.id())?;
        Ok(Some(LocalDaemonResponse::SessionState {
            room_workflows: crate::runtime::projection::RoomWorkflowInventory::project(
                &snapshot,
                crate::session::DEFAULT_LOCAL_USER_ID,
            ),
            session: snapshot,
            agent_activity: self.agent_activity_for_session(&session),
            agent_activity_revision: self.owned.session_projection.change_sequence(),
        }))
    }
}
