//! MP-08 / MP-10 / MP-11: workflow trace visibility uses immutable run participants.
use super::RuntimeSession;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::session::WorkflowRun;

impl RuntimeSession {
    #[cfg(test)]
    pub(crate) fn workflow_run_visible_for_user(
        &self,
        run: &WorkflowRun,
        user_id: &str,
        history: Option<&DurableKernelStateStore>,
    ) -> Result<bool, DaemonError> {
        let kernel_owner = history.is_some_and(|store| {
            store.workflow_history_is_kernel_owner(self.host_daemon_id(), user_id)
        });
        self.workflow_run_access(run, user_id, history, kernel_owner)
            .map(|(visible, _)| visible)
    }

    fn workflow_run_access(
        &self,
        run: &WorkflowRun,
        user_id: &str,
        history: Option<&DurableKernelStateStore>,
        kernel_owner: bool,
    ) -> Result<(bool, bool), DaemonError> {
        let invoker = match (run.created_by_agent_id(), history, run.queue_item_id()) {
            (None, Some(store), Some(queue)) => {
                store.workflow_invocation_owner(self.host_daemon_id(), self.id(), queue)?
            }
            _ => None,
        };
        if run.created_by_agent_id().is_none() && history.is_some() && invoker.is_none() {
            return Ok((kernel_owner, true));
        }
        // A projection without the provenance store cannot establish an autonomous
        // run's invoker or the kernel-owner historical exception. Fail closed.
        if history.is_none() && run.created_by_agent_id().is_none() {
            return Ok((false, false));
        }
        if self
            .collaboration_level_for_user(user_id)
            .is_some_and(|level| level.can_view_agent_trace())
        {
            return Ok((true, false));
        }
        let Some(creator) = run.created_by_agent_id() else {
            let owned = invoker.as_deref().is_some_and(|invoker| {
                history.is_some_and(|store| {
                    store.workflow_invoker_matches(
                        self.host_daemon_id(),
                        self.owner_user_id(),
                        user_id,
                        invoker,
                    )
                })
            });
            return Ok((owned, false));
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
                return Ok((false, false));
            }
        }
        Ok((true, false))
    }

    pub(crate) fn project_workflow_run_for_user(
        &self,
        mut run: WorkflowRun,
        user_id: &str,
        history: Option<&DurableKernelStateStore>,
    ) -> Result<Option<WorkflowRun>, DaemonError> {
        let kernel_owner = history.is_some_and(|store| {
            store.workflow_history_is_kernel_owner(self.host_daemon_id(), user_id)
        });
        let (visible, historical) =
            self.workflow_run_access(&run, user_id, history, kernel_owner)?;
        if !visible {
            return Ok(None);
        }
        if historical {
            run.mark_created_before_ownership_tracking();
        }
        Ok(Some(run))
    }

    pub(crate) fn visible_workflow_runs_for_user(
        &self,
        user_id: &str,
        history: Option<&DurableKernelStateStore>,
    ) -> Result<Vec<WorkflowRun>, DaemonError> {
        let mut visible = Vec::new();
        let kernel_owner = history.is_some_and(|store| {
            store.workflow_history_is_kernel_owner(self.host_daemon_id(), user_id)
        });
        for run in self.workflow_runs() {
            let (allowed, historical) =
                self.workflow_run_access(run, user_id, history, kernel_owner)?;
            if allowed {
                let mut run = run.clone();
                if historical {
                    run.mark_created_before_ownership_tracking();
                }
                visible.push(run);
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
