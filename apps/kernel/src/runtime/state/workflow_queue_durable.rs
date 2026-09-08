//! Queue admission reuses SessionService's selector and the existing writer.
//! The claim is a bounded marker, not a held coordinator mutex: provisioning and
//! provider preparation run after the short transition/session guards release.
use super::*;
use crate::durable_state::workflow_dispatch_intents::WorkflowDispatchIntent;
use crate::session::WorkflowQueueRun;

#[derive(Clone, Default)]
pub(super) struct WorkflowEntryClaims(Arc<std::sync::Mutex<BTreeSet<(String, String)>>>);
pub(super) struct WorkflowEntryClaim {
    claims: WorkflowEntryClaims,
    key: (String, String),
}
impl WorkflowEntryClaims {
    pub(super) fn try_claim(&self, session: &str, run: &str) -> Option<WorkflowEntryClaim> {
        let mut claims = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = (session.to_owned(), run.to_owned());
        if claims.len() >= 64 || !claims.insert(key.clone()) {
            return None;
        }
        Some(WorkflowEntryClaim {
            claims: self.clone(),
            key,
        })
    }
}
impl Drop for WorkflowEntryClaim {
    fn drop(&mut self) {
        self.claims
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

impl KernelRuntimeOwnedState {
    /// Workspace release retries use the identical claimed entry path. None
    /// means this is a downstream node (or legacy run) without an entry intent.
    pub(super) fn workflow_retry_durable_entry(
        &self,
        session: &str,
        run_id: &str,
        node_id: &str,
    ) -> Result<Option<WorkflowPromptDispatches>, DaemonError> {
        let Some(intent) = self.workflow_entry_intent(session, run_id)? else {
            return Ok(None);
        };
        if intent.node_id != node_id {
            return Ok(None);
        }
        if intent.submitted {
            return Ok(Some(WorkflowPromptDispatches::default()));
        }
        let Some(_claim) = self.workflow_entry_claims.try_claim(session, run_id) else {
            return Ok(Some(WorkflowPromptDispatches::default()));
        };
        let (run, workflow, endpoint) = {
            let sessions = self.session_store.read();
            let run = sessions.resolve_workflow_run_ref(session, run_id)?;
            if run.status().is_terminal() {
                return Ok(Some(WorkflowPromptDispatches::default()));
            }
            let workflow = sessions.resolve_workflow_ref(session, run.workflow_id())?;
            let endpoint = sessions.resolve_workflow_endpoint_ref(
                session,
                run.workflow_id(),
                run.endpoint_id(),
            )?;
            (run, workflow, endpoint)
        };
        let mut dispatches = WorkflowPromptDispatches::default();
        let (_, admitted) = self.workflow_schedule_queued_prompt_run(
            session,
            intent.queued_prompt,
            run,
            workflow,
            endpoint,
            &mut dispatches,
        )?;
        dispatches.extend(admitted);
        Ok(Some(dispatches))
    }

    pub(super) fn workflow_pending_entry(
        &self,
        session: &str,
    ) -> Result<Option<WorkflowDispatchIntent>, DaemonError> {
        self.durable_state_store.require_writer_healthy()?;
        self.durable_state_store
            .pending_workflow_dispatch_intent(&self.config_projection.snapshot().daemon_id, session)
    }

    pub(super) fn workflow_entry_intent(
        &self,
        session: &str,
        run: &str,
    ) -> Result<Option<WorkflowDispatchIntent>, DaemonError> {
        self.durable_state_store.require_writer_healthy()?;
        self.durable_state_store.workflow_dispatch_intent(
            &self.config_projection.snapshot().daemon_id,
            session,
            run,
        )
    }

    /// Recover the exact Ready run before selecting any new queue item. A crash
    /// cannot make a Delivered App receipt hide an unsubmitted workflow entry.
    pub(super) fn workflow_recover_pending_entry(
        &self,
        session: &str,
    ) -> Result<Option<WorkflowQueueRun>, DaemonError> {
        let Some(intent) = self.workflow_pending_entry(session)? else {
            return Ok(None);
        };
        let sessions = self.session_store.read();
        let run = sessions.resolve_workflow_run_ref(session, &intent.run_id)?;
        let node = run.node_runs().first().ok_or_else(entry_conflict)?;
        if run.status().is_terminal()
            || node.id() != intent.node_id
            || node.agent_id() != intent.agent_id
            || run.queue_item_id() != Some(intent.queued_prompt.id())
            || run.workflow_id() != intent.queued_prompt.workflow_id()
            || run.endpoint_id() != intent.queued_prompt.endpoint_id()
        {
            return Err(entry_conflict());
        }
        let workflow = sessions.resolve_workflow_ref(session, run.workflow_id())?;
        let endpoint = sessions.resolve_workflow_endpoint_ref(
            session,
            run.workflow_id(),
            run.endpoint_id(),
        )?;
        Ok(Some((intent.queued_prompt, run, workflow, endpoint)))
    }

    /// Blocking queue admission. Only the short existing transition
    /// mutex and SessionService guard span the durable queue/run transaction.
    pub(super) fn workflow_dequeue_durably(
        &self,
        session: &str,
    ) -> Result<Option<WorkflowQueueRun>, DaemonError> {
        self.durable_state_store.require_writer_healthy()?;
        self.durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                let mut sessions = self.session_store.write();
                let before = sessions.get_session(session)?;
                if before.workflow_queued_prompts().is_empty() {
                    return Ok(None);
                }
                // Existing manual/watchdog callers may have just enqueued in memory.
                // Make their accepted queue durable before preparing the CAS snapshot.
                self.durable_state_store
                    .persist_workflow_runtime_transition(&before, "workflow_queue_admitted")?;
                let prepared = Arc::new(sessions.prepare_durable_workflow_queue_run(session)?);
                self.durable_state_store
                    .commit_workflow_queue_start(prepared.clone())
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "workflow durable queue start",
                        message: error.to_string(),
                    })?;
                sessions.restore_session(prepared.after().clone());
                Ok(prepared.next().cloned())
            })
    }

    /// Before admission the exact Ready intent remains retryable. Once the
    /// prompt-state event commits, existing prompt recovery owns it; a later
    /// scheduling failure requires a fenced restart, never a second entry.
    pub(super) fn workflow_preserve_entry_after_failure(&self, session: &str, run: &str) -> bool {
        match self.workflow_entry_intent(session, run) {
            Ok(Some(intent)) => {
                if intent.submitted {
                    let _ = self.durable_state_store.fence_writer();
                } else {
                    self.release_workflow_node_workspace_claim(session, run, &intent.node_id);
                }
                true
            }
            Ok(None) => false,
            Err(_) => {
                let _ = self.durable_state_store.fence_writer();
                true
            }
        }
    }
}
fn entry_conflict() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "recover durable workflow entry",
        message: "workflow entry no longer matches its durable run".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claims_are_bounded_shared_and_released_without_holding_mutex() {
        let claims = WorkflowEntryClaims::default();
        let same = claims.clone();
        let held: Vec<_> = (0..64)
            .map(|n| claims.try_claim("session", &n.to_string()).unwrap())
            .collect();
        assert!(same.try_claim("session", "0").is_none());
        assert!(same.try_claim("other", "overflow").is_none());
        drop(held);
        assert!(same.try_claim("session", "0").is_some());
    }
}
