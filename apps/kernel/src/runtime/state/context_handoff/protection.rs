//! Final handoff replay admission under the current Room observation policy.
use super::*;

impl super::super::KernelRuntimeOwnedState {
    /// Called under the shared Room barrier immediately before enqueueing Submit.
    /// Rebuild derived history before clipping; scrubbing an old rendered packet
    /// cannot remove a newly registered value that was already truncated.
    pub(in crate::runtime::state) fn revalidate_context_handoff(
        &self,
        mut handoff: PendingAgentContextHandoff,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<PendingAgentContextHandoff, crate::error::DaemonError> {
        let protection = &self.room_secret_observations;
        protection.require(session_id, false)?;
        if handoff.derived {
            handoff.conversation = load_agent_conversation(
                &self.operational_history_store,
                session_id,
                agent_id,
                Some(prompt_id),
                protection,
            )?;
            self.add_session_facts(session_id, agent_id, prompt_id, &mut handoff.conversation);
            handoff.conversation.brief = self
                .protected_stored_handoff_brief(session_id, agent_id)?
                .map(|stored| stored.brief);
        } else {
            // Fork/substitute snapshots have an intentional historical boundary.
            // Replaying them under a changed or recovered policy is unsafe.
            protection.scrub_cached_result(session_id, ())?;
            if handoff.room_revision != Some(protection.revision(session_id)?) {
                return Err(crate::error::DaemonError::LocalTransport {
                    operation: "provider.context_handoff",
                    message: "Room protection changed while preparing this handoff. Request a fresh invocation.".into(),
                });
            }
        }
        Ok(handoff)
    }
}
