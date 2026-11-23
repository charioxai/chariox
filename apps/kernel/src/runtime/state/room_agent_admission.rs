//! Shared mutation fence below clients/provider bridges. MP-08 / MP-11, A01.
use super::*;
use crate::runtime::room_tool_admission::{denied, in_subtree};

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
            if agent.archived() {
                return Err(denied("unarchive this agent before using Room tools"));
            }
            // MP-08 / MP-10 / MP-11 A10-S02: a home-owned remote agent has
            // a projected worker epoch, not a local provider-store run. The
            // forwarding seam already checks the authenticated current lease.
            let current = if let Some(remote) = agent.remote_execution() {
                remote
                    .active_worker_provider_run_id
                    .as_deref()
                    .is_some_and(|worker| {
                        crate::provider::projected_leased_provider_run_id(
                            &remote.leased_agent_id,
                            worker,
                        ) == run
                    })
            } else {
                self.owned
                    .provider_store
                    .get_tool_run_for_agent(agent.session_id(), agent.id())
                    .is_some_and(|current| {
                        current.id() == run && current.owner_user_id() == agent.owner_user_id()
                    })
            };
            if !current {
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
        if actor.archived() {
            return Err(denied("unarchive this agent before using Room tools"));
        }
        if let LocalDaemonRequest::SpawnAgent(r) = request {
            if r.session_id != actor.session_id() {
                return self.authorize_cross_room_spawn(&actor, r);
            }
            return Ok(());
        }
        // A peer may stop a turn in another room of the same owner, so a cancel is admitted
        // against its target before the caller-room fence below.
        if let LocalDaemonRequest::CancelActivePrompt(r) = request {
            return match r.target_agent_id.as_deref() {
                // An agent may always stop its own turn.
                Some(target) if target == actor.id() && r.session_id == actor.session_id() => {
                    Ok(())
                }
                Some(target) => {
                    let target = self
                        .owned
                        .agent_store
                        .get_agent(target)
                        .ok()
                        .filter(|target| target.session_id() == r.session_id)
                        .ok_or_else(|| denied("agent is unavailable in that room"))?;
                    self.authorize_peer_control(
                        &actor,
                        &target,
                        super::peer_rooms::PeerControl::Cancel,
                    )
                }
                None => Err(denied("cancel names the agent whose turn it stops")),
            };
        }
        let deadline_target = |room: &str, agent: &str| {
            self.owned
                .agent_store
                .get_agent(agent)
                .ok()
                .filter(|target| target.session_id() == room)
                .ok_or_else(|| denied("agent is unavailable in that room"))
        };
        if let LocalDaemonRequest::SetAgentDeadline(r) = request {
            let target = deadline_target(&r.session_id, &r.agent_id)?;
            if target.id() == actor.id() {
                return Err(denied(
                    "only the owner or another authorized agent can change your deadline",
                ));
            }
            return self.authorize_peer_control(
                &actor,
                &target,
                super::peer_rooms::PeerControl::Cancel,
            );
        }
        // PA10: whoever may stop an agent's turn in another room of the owner may also take back
        // or re-arm its usage-limit resume there (`agent resume [cancel] --room`).
        if let LocalDaemonRequest::UpdateAgentConfig(r) = request {
            if r.session_id != actor.session_id() && only_usage_limit(r) {
                let target = deadline_target(&r.session_id, &r.agent_id)?;
                return self.authorize_peer_control(
                    &actor,
                    &target,
                    super::peer_rooms::PeerControl::Cancel,
                );
            }
        }
        // Who may change a deadline in another room may read it there.
        if let LocalDaemonRequest::GetAgentDeadline(r) = request {
            if r.session_id != actor.session_id() {
                let target = deadline_target(&r.session_id, &r.agent_id)?;
                return self.authorize_peer_control(
                    &actor,
                    &target,
                    super::peer_rooms::PeerControl::Cancel,
                );
            }
        }
        // Row 405: a request aimed at one of the owner's agents in another room is judged by
        // the peer check below, like one in the caller's room.
        let cross_room_target = match request {
            LocalDaemonRequest::DestroyAgent(r) => r.session_id != actor.session_id(),
            LocalDaemonRequest::AliasAgent(r) => r.session_id != actor.session_id(),
            LocalDaemonRequest::UpdateAgentConfig(r) => r.session_id != actor.session_id(),
            LocalDaemonRequest::UpdateAgentProfile(r) => r.session_id != actor.session_id(),
            LocalDaemonRequest::UpdateAgentSubstitutes(r) => r.session_id != actor.session_id(),
            _ => false,
        };
        if !cross_room_target
            && rooms_named(request)
                .iter()
                .any(|room| room != actor.session_id())
        {
            return Err(denied("request is outside the caller room"));
        }
        let (room, target_id, deleting) = match request {
            LocalDaemonRequest::GetAgentDeadline(r) => (&r.session_id, &r.agent_id, false),
            LocalDaemonRequest::ListDescendants(r) => (&r.session_id, &r.agent_id, false),
            LocalDaemonRequest::AliasAgent(r) => (&r.session_id, &r.agent_id, false),
            // Organising the owner's agents is an ordinary owner action (row 405).
            LocalDaemonRequest::HideAgent(_) | LocalDaemonRequest::SetAgentParent(_) => {
                return Ok(());
            }
            LocalDaemonRequest::DestroyAgent(r) => (&r.session_id, &r.agent_id, true),
            LocalDaemonRequest::UpdateAgentConfig(r) => (&r.session_id, &r.agent_id, false),
            LocalDaemonRequest::UpdateAgentProfile(r) => (&r.session_id, &r.agent_id, false),
            LocalDaemonRequest::UpdateAgentSubstitutes(r) => (&r.session_id, &r.agent_id, false),
            // Ending, archiving and deleting are the owner's ordinary actions (row 405).
            LocalDaemonRequest::EndSession(_)
            | LocalDaemonRequest::DeleteSession(_)
            | LocalDaemonRequest::ArchiveProject(_)
            | LocalDaemonRequest::DeleteProject(_)
            | LocalDaemonRequest::DeleteKernel(_) => {
                return Ok(());
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
                return in_subtree(&actor, target, &agents);
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
                return in_subtree(&actor, target, &agents);
            }
            // MP-08/MP-11 A05: App bindings only for self or an immutable direct
            // child; the binding path checks request causation and the subset.
            LocalDaemonRequest::GrantAgentExtension(r) => {
                return self.authorize_room_extension_target(&actor, &r.agent_ref);
            }
            LocalDaemonRequest::RevokeAgentExtension(r) => {
                return self.authorize_room_extension_target(&actor, &r.agent_ref);
            }
            LocalDaemonRequest::FocusAgent(_) | LocalDaemonRequest::CycleAgentFocus(_) => {
                return Err(denied("agents cannot change user focus"));
            }
            LocalDaemonRequest::AttachToSession(r) => {
                if r.session_id != actor.session_id() {
                    return Err(denied("attachment is outside the caller room"));
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        let target = self.owned.agent_store.get_agent(target_id)?;
        if target.session_id() != room {
            return Err(denied("agent is unavailable in that room"));
        }
        if room != actor.session_id() {
            let source = self.owned.session_store.get_session(actor.session_id())?;
            let destination = self.owned.session_store.get_session(room)?;
            if self.drives_as_room_guest(&source, &actor)
                || !self.same_owner(source.owner_user_id(), destination.owner_user_id())
            {
                return Err(denied("target is outside the caller room"));
            }
        }
        // An agent changes its own settings within what the person allowed it; the kernel's
        // settings gate holds it to that set. Everything else needs the agent to be above.
        let own_settings = target.id() == actor.id()
            && match request {
                LocalDaemonRequest::UpdateAgentProfile(_) => true,
                LocalDaemonRequest::UpdateAgentConfig(r) => r.usage_limit.is_none(),
                _ => false,
            };
        // Row 405: an agent above it, or another of the owner's agents the peer check admits
        // (its permission comparison keeps the owner's per-agent settings).
        if !own_settings && target.id() == actor.id() {
            return Err(denied("only the agents above an agent can control it"));
        }
        if !own_settings && !self.owned.agent_store.is_ancestor(actor.id(), &target) {
            self.authorize_peer_control(&actor, &target, super::peer_rooms::PeerControl::Cancel)?;
        }
        if let LocalDaemonRequest::UpdateAgentConfig(r) = request {
            self.keep_configured_folders_inside_parent(&target, r)?;
        }
        // Deleting an agent ends its sub-agents with it (A4); only its own unsettled runs refuse.
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

/// Every room a request acts on. Membership scope leaves out the requests whose handler checks the
/// agent's owner itself, or that act on a Room's workflows and Apps, so they are named here.
fn rooms_named(request: &LocalDaemonRequest) -> Vec<String> {
    use crate::runtime::session_membership::scope::{
        request_session_scope, SessionMembershipScope,
    };
    match request_session_scope(request) {
        Some(SessionMembershipScope::SessionId(room)) => return vec![room],
        Some(SessionMembershipScope::SessionIds(rooms)) => return rooms,
        // References and attachment ids resolve to a room inside their handler.
        Some(_) => return Vec::new(),
        None => {}
    }
    let room = match request {
        LocalDaemonRequest::UndoTurn(r) => &r.session_id,
        LocalDaemonRequest::SetWorkspaceLiveSyncMode(r) => &r.session_id,
        LocalDaemonRequest::GenerateWorkspaceCommitMessage(r) => &r.session_id,
        LocalDaemonRequest::RunAgentUtility(r) => &r.session_id,
        LocalDaemonRequest::AcceptAppHostAction(r) => &r.session_id,
        LocalDaemonRequest::GrantAppFile(r) => &r.session_id,
        LocalDaemonRequest::SaveAppFileExport(r) => &r.session_id,
        LocalDaemonRequest::BeginAppPublisherEnrollment(r) => &r.session_id,
        LocalDaemonRequest::BeginAppInstall(r) => &r.session_id,
        LocalDaemonRequest::BeginAppDevPush(r) => &r.session_id,
        LocalDaemonRequest::BeginAppUpdate(r) => &r.session_id,
        LocalDaemonRequest::PrepareDeploymentApps(r) => &r.session_id,
        LocalDaemonRequest::ApplyWorkflowCodeArtifact(r) => &r.session_id,
        LocalDaemonRequest::RunWorkflowCodeArtifact(r) => &r.session_id,
        LocalDaemonRequest::BindWorkflowCodeSource(r) => &r.session_id,
        LocalDaemonRequest::RebuildWorkflowCodeSource(r) => &r.session_id,
        LocalDaemonRequest::UpdateWorkflowCodeSourceFromWorkflow(r) => &r.session_id,
        LocalDaemonRequest::ListWorkflowRegistry(r) => &r.session_id,
        LocalDaemonRequest::GetWorkflowRegistryEntry(r) => &r.session_id,
        LocalDaemonRequest::AddWorkflowRegistryEntry(r) => &r.session_id,
        LocalDaemonRequest::AddWorkflowRegistryEntryFromWorkflow(r) => &r.session_id,
        LocalDaemonRequest::DeleteWorkflowRegistryEntry(r) => &r.session_id,
        LocalDaemonRequest::LoadWorkflowRegistryEntry(r) => &r.session_id,
        LocalDaemonRequest::RunWorkflowRegistryEntry(r) => &r.session_id,
        _ => return Vec::new(),
    };
    vec![room.clone()]
}

impl KernelRuntimeState {
    /// A child may start in another Room of the same owner.
    fn authorize_cross_room_spawn(
        &self,
        actor: &crate::agent::AgentInstance,
        request: &crate::local::SpawnAgentRequest,
    ) -> Result<(), DaemonError> {
        if request.worktree_id.is_some()
            || request.kernel_ref.is_some()
            || request.slice_ref.is_some()
            || request.worktree_placement.is_some()
        {
            return Err(denied(
                "a sub-agent in another Room starts in that Room's own folder",
            ));
        }
        let rooms = &self.owned.session_store;
        let own_room = rooms.get_session(actor.session_id())?;
        if self.drives_as_room_guest(&own_room, actor) {
            return Err(denied("the destination Room does not exist"));
        }
        let (Some(own_owner), Some(room_owner)) = (
            rooms.session_owner_user_id(actor.session_id()),
            rooms.session_owner_user_id(&request.session_id),
        ) else {
            return Err(denied("the destination Room does not exist"));
        };
        if !self.same_owner(&own_owner, &room_owner) {
            return Err(denied(
                "a sub-agent can only be created in a Room of the same owner",
            ));
        }
        Ok(())
    }

    /// A5: moving a sub-agent keeps it inside the folders of its own parent.
    fn keep_configured_folders_inside_parent(
        &self,
        target: &crate::agent::AgentInstance,
        request: &crate::local::UpdateAgentConfigRequest,
    ) -> Result<(), DaemonError> {
        let Some(parent_id) = target.parent_agent_id() else {
            return Ok(());
        };
        let parent = self.owned.agent_store.get_agent(parent_id)?;
        let session = self.owned.session_store.get_session(parent.session_id())?;
        for directory in [&request.workspace_id, &request.worktree_id]
            .into_iter()
            .flatten()
            .map(|directory| directory.trim())
            .filter(|directory| !directory.is_empty())
        {
            crate::agent::authority_limit::check_directory(&parent, &session, directory)?;
        }
        Ok(())
    }

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
        in_subtree(actor, target, &agents)
    }
}

#[cfg(test)]
#[path = "room_agent_admission_tests.rs"]
mod tests;

/// A config change that only takes back or re-arms the usage-limit resume.
fn only_usage_limit(r: &crate::local::UpdateAgentConfigRequest) -> bool {
    r.usage_limit.is_some()
        && r.execution_mode.is_none()
        && !r.clear_execution_mode
        && r.permission_level.is_none()
        && !r.clear_permission_level
        && r.max_context_tokens.is_none()
        && !r.clear_max_context_tokens
        && r.workspace_id.is_none()
        && !r.clear_workspace_id
        && r.worktree_id.is_none()
        && !r.clear_worktree_id
}
