//! MP-08 / MP-10 / MP-11: workflow trace visibility uses immutable run participants.
use super::RuntimeSession;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::session::WorkflowRun;

impl RuntimeSession {
    pub(crate) fn workflow_run_visible_for_user(
        &self,
        run: &WorkflowRun,
        user_id: &str,
        history: Option<&DurableKernelStateStore>,
    ) -> Result<bool, DaemonError> {
        if self
            .collaboration_level_for_user(user_id)
            .is_some_and(|level| level.can_view_agent_trace())
        {
            return Ok(true);
        }
        let Some(creator) = run.created_by_agent_id() else {
            return Ok(false);
        };
        for id in std::iter::once(creator)
            .chain(run.node_runs().iter().map(|node| node.agent_id()))
            .chain(run.runtime_agent_ids_by_node().values().map(String::as_str))
        {
            let owned = match self.agents().iter().find(|agent| agent.id() == id) {
                Some(agent) => agent.owner_user_id() == user_id,
                None => match history {
                    Some(store) => store.deleted_agent_is_owned(self.id(), id, user_id)?,
                    None => false,
                },
            };
            if !owned {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(crate) fn visible_workflow_runs_for_user(
        &self,
        user_id: &str,
        history: Option<&DurableKernelStateStore>,
    ) -> Result<Vec<WorkflowRun>, DaemonError> {
        let mut visible = Vec::new();
        for run in self.workflow_runs() {
            if self.workflow_run_visible_for_user(run, user_id, history)? {
                visible.push(run.clone());
            }
        }
        Ok(visible)
    }

    pub(crate) fn redacted_for_user_with_workflow_history(
        mut self,
        user_id: &str,
        history: &DurableKernelStateStore,
    ) -> Result<Self, DaemonError> {
        let visible = self.visible_workflow_runs_for_user(user_id, Some(history))?;
        self.workflow_runs.clear();
        let mut redacted = self.redacted_for_user(user_id);
        redacted.workflow_runs = visible;
        Ok(redacted)
    }
}
