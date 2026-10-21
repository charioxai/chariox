//! MP-08 / MP-10 / MP-11: final handoff replay admission under the current Room observation policy.
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
        let revision = protection.revision(session_id)?;
        let policy_changed = handoff.room_revision != Some(revision);
        // Preserve preparation's chosen fold (or deliberate omission). Disk caches
        // are neither the selected envelope nor trusted fresh folds after recovery.
        let brief = if policy_changed {
            None
        } else {
            handoff
                .conversation
                .brief
                .take()
                .map(|brief| protection.scrub(session_id, brief))
                .transpose()?
        };
        if handoff.derived {
            handoff.conversation = load_agent_conversation(
                &self.operational_history_store,
                session_id,
                agent_id,
                Some(prompt_id),
                protection,
            )?;
            self.add_session_facts(session_id, agent_id, prompt_id, &mut handoff.conversation);
        } else if policy_changed {
            // Fork/substitute snapshots keep their source agent and capture boundary.
            // Re-read unrendered history so newly protected values are removed before
            // clipping; a retirement can also withhold the earlier source records.
            handoff.conversation = handoff
                .conversation
                .rebuild_under_current_policy(
                    &self.operational_history_store,
                    session_id,
                    protection,
                )?
                .before_prompt(prompt_id);
        }
        handoff.conversation.brief = brief;
        handoff.room_revision = Some(revision);
        Ok(handoff)
    }
}
