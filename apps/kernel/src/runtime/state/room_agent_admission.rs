//! Shared mutation fence below clients/provider bridges. MP-08 / MP-11, A01.
use super::*;
use crate::runtime::room_tool_admission::{denied, direct_child};

impl KernelRuntimeState {
    /// MP-08 / MP-11: retain the admitted typed operation across asynchronous waits.
    pub(crate) fn with_room_request_origin(
        &self,
        actor: Option<&str>,
        request: &LocalDaemonRequest,
    ) -> Self {
        let mut state = self.clone();
        if self.room_agent_tools_enabled() {
            state.room_request_origin = actor.map(|actor| (actor.to_owned(), request.clone()));
        }
        state
    }

    pub(crate) fn with_room_provider_origin(&self, actor: Option<&str>, run: Option<&str>) -> Self {
        let mut state = self.clone();
        if self.room_agent_tools_enabled() {
            state.room_provider_origin = actor
                .zip(run)
                .map(|(actor, run)| (actor.to_owned(), run.to_owned()));
        }
        state
    }

    pub(crate) fn authorize_room_provider_epoch(
        &self,
        actor: Option<&str>,
        run: Option<&str>,
    ) -> Result<(), DaemonError> {
        if !self.room_agent_tools_enabled() {
            return Ok(());
        }
        if let (Some(actor), Some(run)) = (actor, run) {
            let agent = self.owned.agent_store.get_agent(actor)?;
            let current = self
                .owned
                .provider_store
                .get_run_for_agent(agent.session_id(), agent.id());
            if !current.is_some_and(|current| {
                current.id() == run && current.owner_user_id() == agent.owner_user_id()
            }) {
                return Err(denied("room tool caller provider run is stale"));
            }
        }
        Ok(())
    }

    pub(crate) fn authorize_room_agent_request(
        &self,
        actor_id: &str,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        if !self.room_agent_tools_enabled() {
            return Ok(());
        }
        let actor = self.owned.agent_store.get_agent(actor_id)?;
        if let Some(crate::runtime::session_membership::scope::SessionMembershipScope::SessionId(
            room,
        )) = crate::runtime::session_membership::scope::request_session_scope(request)
        {
            if room != actor.session_id() {
                return Err(denied("request is outside the caller room"));
            }
        }
        let registry_room = match request {
            LocalDaemonRequest::ListWorkflowRegistry(r) => Some(&r.session_id),
            LocalDaemonRequest::GetWorkflowRegistryEntry(r) => Some(&r.session_id),
            LocalDaemonRequest::AddWorkflowRegistryEntry(r) => Some(&r.session_id),
            LocalDaemonRequest::AddWorkflowRegistryEntryFromWorkflow(r) => Some(&r.session_id),
            LocalDaemonRequest::DeleteWorkflowRegistryEntry(r) => Some(&r.session_id),
            LocalDaemonRequest::LoadWorkflowRegistryEntry(r) => Some(&r.session_id),
            LocalDaemonRequest::RunWorkflowRegistryEntry(r) => Some(&r.session_id),
            _ => None,
        };
        if registry_room.is_some_and(|room| room != actor.session_id()) {
            return Err(denied("registry request is outside the caller room"));
        }
        let (room, target_id, deleting) = match request {
            LocalDaemonRequest::AliasAgent(r) => (&r.session_id, &r.agent_id, false),
            LocalDaemonRequest::DestroyAgent(r) => (&r.session_id, &r.agent_id, true),
            LocalDaemonRequest::UpdateAgentConfig(r) => (&r.session_id, &r.agent_id, false),
            LocalDaemonRequest::UpdateAgentProfile(r) => (&r.session_id, &r.agent_id, false),
            LocalDaemonRequest::UpdateAgentSubstitutes(r) => (&r.session_id, &r.agent_id, false),
            LocalDaemonRequest::EndSession(_)
            | LocalDaemonRequest::DeleteSession(_)
            | LocalDaemonRequest::DeleteKernel(_) => {
                return Err(denied("room agents cannot tear down sessions or kernels"));
            }
            LocalDaemonRequest::ForkAgent(_) => {
                return Err(denied(
                    "use room spawn; provider forks are outside the PR1 surface",
                ));
            }
            LocalDaemonRequest::MoveAgentToRemote(r) => {
                let agents = self
                    .owned
                    .agent_store
                    .get_session_agents(actor.session_id());
                let target = crate::runtime::room_tool_admission::resolve_agent(
                    &agents,
                    actor.session_id(),
                    &r.agent_ref,
                )?;
                if r.session_id != actor.session_id() {
                    return Err(denied("placement is outside the caller room"));
                }
                return direct_child(&actor, target);
            }
            LocalDaemonRequest::MoveAgentToLocal(r) => {
                let agents = self
                    .owned
                    .agent_store
                    .get_session_agents(actor.session_id());
                let target = crate::runtime::room_tool_admission::resolve_agent(
                    &agents,
                    actor.session_id(),
                    &r.agent_ref,
                )?;
                if r.session_id != actor.session_id() {
                    return Err(denied("placement is outside the caller room"));
                }
                return direct_child(&actor, target);
            }
            // MP-08/MP-11 A05: App bindings only for self or an immutable direct
            // child; the binding path checks request causation and the subset.
            LocalDaemonRequest::GrantAgentExtension(r) => {
                return self.authorize_room_extension_target(&actor, &r.agent_ref)
            }
            LocalDaemonRequest::RevokeAgentExtension(r) => {
                return self.authorize_room_extension_target(&actor, &r.agent_ref)
            }
            LocalDaemonRequest::FocusAgent(_) | LocalDaemonRequest::CycleAgentFocus(_) => {
                return Err(denied("agents cannot change user focus"))
            }
            LocalDaemonRequest::SpawnAgent(r) => {
                if r.session_id != actor.session_id() {
                    return Err(denied("spawn target is outside the caller room"));
                }
                return Ok(());
            }
            LocalDaemonRequest::AttachToSession(r) => {
                if r.session_id != actor.session_id() {
                    return Err(denied("attachment is outside the caller room"));
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        if room != actor.session_id() {
            return Err(denied("target is outside the caller room"));
        }
        let target = self.owned.agent_store.get_agent(target_id)?;
        direct_child(&actor, &target)?;
        // Deletion never silently cancels or destroys the target's descendants/work.
        // Keep them visible for the owner and require explicit settlement by the child.
        if deleting {
            let session = self.owned.session_store.get_session(room)?;
            if session.workflow_runs().iter().any(|run| {
                !run.status().is_terminal() && run.created_by_agent_id() == Some(target.id())
            }) {
                return Err(denied(
                    "child has unsettled workflow runs; ask it to settle them first",
                ));
            }
        }
        Ok(())
    }
}

impl KernelRuntimeState {
    fn authorize_room_extension_target(
        &self,
        actor: &crate::agent::AgentInstance,
        reference: &str,
    ) -> Result<(), DaemonError> {
        let agents = self
            .owned
            .agent_store
            .get_session_agents(actor.session_id());
        let target = crate::runtime::room_tool_admission::resolve_agent(
            &agents,
            actor.session_id(),
            reference,
        )?;
        if target.id() == actor.id() {
            return Ok(());
        }
        direct_child(actor, target)
    }
}

#[cfg(test)]
#[path = "room_agent_admission_tests.rs"]
mod tests;
