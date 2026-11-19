//! MP-08 / MP-10 / MP-11: reserve body budget on the existing prompt owner and durable queue.
use super::*;
use crate::skill::turn_authority::SkillTurnAuthority;
fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "skills.v2.turn",
        message: message.into(),
    }
}
#[derive(Clone)]
pub(super) struct SkillToolOrigin {
    pub session_id: String,
    pub agent_id: String,
    pub prompt_id: String,
}
impl KernelRuntimeState {
    pub(super) fn with_skill_tool_origin<R>(
        &self,
        origin: Option<&SkillToolOrigin>,
        operation: impl FnOnce() -> Result<R, DaemonError>,
    ) -> Result<R, DaemonError> {
        if let Some(origin) = origin {
            self.current_skill_tool_prompt(
                &origin.session_id,
                &origin.agent_id,
                &origin.prompt_id,
            )?;
            if self.forwarded_peer_binding.is_some() {
                return self.with_forwarded_binding_operation(operation);
            }
            let session = self.owned.session_store.get_session(&origin.session_id)?;
            return self.owned.prompt_state_owner.with_current_prompt(
                &session,
                &origin.agent_id,
                Some(&origin.prompt_id),
                operation,
            );
        }
        self.with_forwarded_binding_operation(operation)
    }
}
impl KernelRuntimeOwnedState {
    pub(super) fn update_skill_turn_authority(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        update: impl Fn(&mut SkillTurnAuthority) -> Result<(), DaemonError>,
    ) -> Result<(), DaemonError> {
        self.prompt_state_owner
            .update_skill_authority_durably(
                &self.session_store,
                &self.durable_state_store,
                session_id,
                agent_id,
                prompt_id,
                |prompt| {
                    let mut authority = prompt
                        .skill_turn_authority()
                        .cloned()
                        .ok_or_else(|| failure("this turn has no admitted skill catalog"))?;
                    update(&mut authority)?;
                    Ok(authority)
                },
            )
            .map(|_| ())
    }

    pub(super) fn reserve_skill_body(
        &self,
        expected: &SkillTurnAuthority,
        tokens: u64,
        record: &crate::skill::v2::SkillLoadRecord,
        remaining_context: Option<u64>,
    ) -> Result<(), DaemonError> {
        self.update_skill_turn_authority(
            &record.session_id,
            &record.agent_id,
            &record.prompt_id,
            |current| {
                if current.catalog_revision != expected.catalog_revision
                    || current.provider_run_id != expected.provider_run_id
                    || current.context_generation != expected.context_generation
                {
                    return Err(failure("skill turn binding changed; start a new turn"));
                }
                // A real provider usage update can replace the initial supported
                // floor. Keep all prior reservations in this ceiling, conservatively
                // counting already delivered bodies again rather than oversubscribing.
                if let Some(remaining) = remaining_context {
                    current.body_budget_tokens = remaining;
                }
                current
                    .reserve_body(&record.skill_id, tokens)
                    .map_err(failure)?;
                if !current.pending_loads.iter().any(|pending| {
                    pending.skill_id == record.skill_id && pending.digest == record.digest
                }) {
                    current.pending_loads.push(record.clone());
                }
                Ok(())
            },
        )
    }
}
