//! MP-08 / MP-10 / MP-11: private skill authority commits with the existing prompt journal.
use super::*;
use crate::{
    durable_state::DurableKernelStateStore, session::SessionStateStore,
    skill::turn_authority::SkillTurnAuthority,
};
impl PromptStateOwner {
    /// Lock order matches forwarded resource admission: sessions, then prompts.
    /// The update closure is pure; it must not reenter runtime stores.
    pub(crate) fn update_skill_authority_durably(
        &self,
        sessions: &SessionStateStore,
        durable: &DurableKernelStateStore,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        update: impl FnOnce(&PromptQueueItem) -> Result<SkillTurnAuthority, DaemonError>,
    ) -> Result<PromptQueueItem, DaemonError> {
        let mut sessions = sessions.write();
        let session = sessions.get_session(session_id)?;
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(&session, agent_id);
        let previous = state
            .active_prompt
            .as_ref()
            .filter(|prompt| prompt.id() == prompt_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "skills.v2.turn",
                message: "skill turn is no longer active".into(),
            })?
            .clone();
        let authority = update(&previous)?;
        if authority.project_id != session.project_id() {
            return Err(DaemonError::LocalTransport {
                operation: "skills.v2.turn",
                message: "Project changed before skill delivery; submit a new turn".into(),
            });
        }
        let replacement = previous.clone().with_skill_turn_authority(authority);
        if replacement == previous {
            return Ok(previous);
        }
        let mirrored = sessions.mirror_agent_prompt_state(
            session_id,
            agent_id,
            Some(replacement.clone()),
            state.queued_prompts.clone(),
        )?;
        if let Err(error) = crate::durable_prompt_state::append_durable_prompt_state_event(
            durable, &mirrored, agent_id,
        ) {
            sessions.mirror_agent_prompt_state(
                session_id,
                agent_id,
                Some(previous),
                state.queued_prompts.clone(),
            )?;
            return Err(error);
        }
        state.active_prompt = Some(replacement.clone());
        Ok(replacement)
    }
}
