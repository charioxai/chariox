//! MP-08 / MP-10 / MP-11 S02: a live sudo window covers every typed kernel
//! request except those `sudo_request_forbidden` reserves for the owner. It
//! never asks for another approval, and it cannot claim another agent's work.
use super::*;

impl KernelRuntimeState {
    pub(super) fn check_sudo_creator_scope(
        &self,
        turn: &KernelSudoTurn,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        // Sudo cannot bypass the immutable creator rule or claim another
        // agent's work. Reuse the ordinary shared fences for these operations.
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "sudo-scope",
            None,
            None,
            request,
        );
        let workflow = crate::runtime::workflow_actor::is_workflow_command(request);
        if command.command_type.starts_with("agent.") || workflow {
            let target = match request {
                LocalDaemonRequest::AliasAgent(r) => Some(&r.agent_id),
                LocalDaemonRequest::DestroyAgent(r) => Some(&r.agent_id),
                LocalDaemonRequest::UpdateAgentConfig(r) => Some(&r.agent_id),
                LocalDaemonRequest::UpdateAgentProfile(r) => Some(&r.agent_id),
                LocalDaemonRequest::UpdateAgentSubstitutes(r) => Some(&r.agent_id),
                _ => None,
            };
            if let Some(target) = target {
                let actor = self.owned.agent_store.get_agent(&turn.agent_id)?;
                let child = self.owned.agent_store.get_agent(target)?;
                crate::runtime::room_tool_admission::direct_child(&actor, &child)?;
            }
            self.authorize_room_agent_request(&turn.agent_id, request)?;
            if workflow {
                self.owned
                    .ensure_workflow_request_controlled_by_metaagent(request, &turn.agent_id)?;
            }
        }
        Ok(())
    }
}
