//! MP-08 / MP-09 / MP-10 / MP-11 reach: the kernel owner's Room agents act on other
//! rooms and kernels. The identity is the agent's own live provider run, never a
//! token the agent holds. Without /sudo a reach caller may do elsewhere what a Room
//! agent without /sudo may do in its own room; a live /sudo window of its own turn
//! adds what /sudo adds at home. Human-only requests are refused on every path.
use super::super::outside_events::is_outside_event_request;
use super::super::sudo::sudo_request_forbidden;
use super::policy::restricted_request;
use super::*;

/// A reach caller's id: this prefix plus its provider run id.
pub(crate) const REACH_PREFIX: &str = "reach:";

/// The kernel owner's own gateway grant, for what a person asks their kernel to do on
/// another kernel without an access grant: this prefix plus the owner's user id.
pub(crate) const OWNER_GRANT_PREFIX: &str = "owner:";

#[derive(Clone, Debug)]
pub(crate) struct ReachCaller {
    pub(crate) id: String,
    pub(crate) run_id: String,
    pub(crate) agent_id: String,
    pub(crate) owner_user_id: String,
    /// What the target's audit and paired-clients list call the agent.
    pub(crate) harness: String,
}

impl ReachCaller {
    /// The gateway origin this agent forwards under.
    pub(crate) fn origin(&self) -> crate::runtime::kernel_gateway::Origin {
        crate::runtime::kernel_gateway::Origin {
            grant_id: self.id.clone(),
            harness: self.harness.clone(),
        }
    }
}

/// What a request needs from a reach caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReachNeed {
    /// A Room agent without /sudo may do this in its own room.
    Room,
    /// A Room agent needs a /sudo window for this in its own room.
    Sudo,
    /// Owner-only: no agent gets it, with or without /sudo.
    HumanOnly,
    /// Excluded for agents even inside a /sudo window.
    Excluded,
}

pub(crate) fn reach_need(request: &LocalDaemonRequest) -> ReachNeed {
    if restricted_request(request) {
        return ReachNeed::HumanOnly;
    }
    // Following other agents and answering the routine decision of another room are part of
    // driving it; both stay outside /sudo's reach for the caller's own room (see `reach_request`).
    if is_outside_event_request(request)
        || matches!(request, LocalDaemonRequest::RespondToInteraction(_))
    {
        return ReachNeed::Room;
    }
    if sudo_request_forbidden(request) {
        return ReachNeed::Excluded;
    }
    // Deleting a saved machine state loses the user's data, so it needs /sudo like every other destructive request.
    if matches!(
        request,
        LocalDaemonRequest::ControlPlane(ControlPlaneRequest::DeleteSavedState { .. })
    ) {
        return ReachNeed::Sudo;
    }
    // The room tools: discover and read sessions, agents and history, create a room, attach, message
    // and steer agents, a room's own terminals and workspace files, and machines under
    // the acquisition setting. Kernel-wide file access and every other kernel mutation need /sudo.
    if matches!(
        request,
        LocalDaemonRequest::ListGatewayKernels(_)
            | LocalDaemonRequest::ListSessions(_)
            | LocalDaemonRequest::ResolveSession(_)
            | LocalDaemonRequest::GetSessionState(_)
            | LocalDaemonRequest::CreateSession(_)
            | LocalDaemonRequest::ListAgents(_)
            | LocalDaemonRequest::QueryRecall(_)
            | LocalDaemonRequest::SearchRecall(_)
            | LocalDaemonRequest::SemanticSearchRecall(_)
            | LocalDaemonRequest::GetSessionHistoryOutline(_)
            | LocalDaemonRequest::GetSessionHistoryBlobContent(_)
            | LocalDaemonRequest::GetSessionHistoryBlobContents(_)
            | LocalDaemonRequest::AttachToSession(_)
            | LocalDaemonRequest::DetachFromSession(_)
            | LocalDaemonRequest::HeartbeatAttachments(_)
            | LocalDaemonRequest::SubmitPrompt(_)
            | LocalDaemonRequest::SubmitPrompts(_)
            | LocalDaemonRequest::SteerQueuedPrompt(_)
            | LocalDaemonRequest::SteerActivePrompt(_)
            | LocalDaemonRequest::RoomTerminal(_)
            | LocalDaemonRequest::RunShellCommand(_)
            | LocalDaemonRequest::ReadDirectoryTree(_)
            | LocalDaemonRequest::ReadFile(_)
            | LocalDaemonRequest::EditFile(_)
            | LocalDaemonRequest::ListRemoteMachines(_)
            | LocalDaemonRequest::ControlPlane(_)
    ) {
        return ReachNeed::Room;
    }
    ReachNeed::Sudo
}

impl KernelRuntimeState {
    /// The live caller of a `reach:<run>` id, for the gateway and the control plane.
    pub(crate) fn reach_for_id(&self, id: &str) -> Option<ReachCaller> {
        self.reach_for_run(id.strip_prefix(REACH_PREFIX)?, true)
            .ok()
    }

    fn reach_for_run(&self, run_id: &str, live: bool) -> Result<ReachCaller, DaemonError> {
        if self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .revoked_reach_runs
            .contains(run_id)
        {
            return Err(reach_error(
                "reach revoked: the owner ended this run's gateway access",
            ));
        }
        let run = self
            .owned
            .provider_store
            .get_run(run_id)
            .map_err(|_| reach_error("this provider run has ended"))?;
        let agent_id = run
            .agent_instance_id()
            .ok_or_else(|| reach_error("this provider run has no agent"))?;
        let current = run.state() == crate::provider::ProviderRunState::Running
            && self
                .owned
                .provider_store
                .get_run_for_agent(run.session_id(), agent_id)
                .is_some_and(|current| current.id() == run_id);
        if live && !current {
            return Err(reach_error("this provider run has ended"));
        }
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let owner = agent.owner_user_id();
        let daemon_id = self.owned.config_projection.snapshot().daemon_id;
        if agent.archived()
            || agent.remote_execution().is_some()
            || !self
                .owned
                .config_projection
                .is_kernel_owner(&daemon_id, owner)
        {
            return Err(reach_error(
                "only agents of this kernel's owner reach other rooms and kernels",
            ));
        }
        Ok(ReachCaller {
            id: format!("{REACH_PREFIX}{run_id}"),
            run_id: run_id.to_owned(),
            agent_id: agent_id.to_owned(),
            owner_user_id: owner.to_owned(),
            harness: self
                .agent_harness_name(run.session_id(), agent_id)
                .ok_or_else(|| reach_error("this agent's room is unavailable"))?,
        })
    }

    /// A gateway revoke ends existing reach identities, not the agents' work in their rooms.
    /// Runs belong to this kernel process; a later provider run has a new identity.
    pub(super) fn revoke_reach_access(&self, owner: Option<&str>, id: Option<&str>) -> usize {
        let runs = self.owned.provider_store.list_runs();
        let active: std::collections::HashSet<_> = runs
            .iter()
            .filter(|run| run.state() != crate::provider::ProviderRunState::Ended)
            .map(|run| run.id().to_owned())
            .collect();
        let callers: Vec<_> = runs
            .iter()
            .filter(|run| active.contains(run.id()))
            .filter_map(|run| self.reach_for_run(run.id(), false).ok())
            .filter(|caller| {
                id.is_none_or(|id| id == caller.id)
                    && owner.is_none_or(|owner| self.access_owned_by(owner, &caller.owner_user_id))
            })
            .collect();
        let mut access = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned");
        access.revoked_reach_runs.retain(|run| active.contains(run));
        let mut count = 0;
        for caller in callers {
            count += usize::from(access.revoked_reach_runs.insert(caller.run_id));
        }
        count
    }

    /// Handed to the next program of a hot reload, whose adopted runs keep their ids.
    pub(crate) fn hand_over_reach_revocations(&self) {
        let revoked = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .revoked_reach_runs
            .clone();
        crate::hot_reload::hand_over_revoked_reach_runs(revoked);
    }

    /// `<room>/<agent> (Chariox agent)`: how another kernel's audit and paired
    /// clients name an agent of this kernel.
    pub(crate) fn agent_harness_name(&self, session_id: &str, agent_id: &str) -> Option<String> {
        let session = self.owned.session_store.get_session(session_id).ok()?;
        let agent = self.owned.agent_store.get_agent(agent_id).ok()?;
        let room = session.alias().unwrap_or_else(|| session.id());
        let name = agent.alias().unwrap_or_else(|| agent.agent_ref());
        Some(format!("{room}/{name} (Chariox agent)"))
    }

    /// Who owns an event inbox: the access grant or the live reach caller that subscribed.
    pub(crate) fn event_source_owner(&self, id: &str) -> Option<String> {
        if id.starts_with(REACH_PREFIX) {
            return Some(self.reach_for_id(id)?.owner_user_id);
        }
        Some(self.access_grant_summary(id)?.owner_user_id)
    }

    /// Whether the grant or reach run an inbox belongs to still lives; an ended one ends its subscription.
    pub(crate) fn event_source_alive(&self, id: &str) -> bool {
        self.event_source_owner(id).is_some()
    }

    /// Ends the subscriptions of reach runs that are gone, so a finished agent leaves no inbox filling up.
    pub(crate) fn end_dead_reach_subscriptions(&self) -> Result<(), DaemonError> {
        use crate::durable_state::agent_lifecycle::outside::Mutation;
        let store = self.outside_store();
        for part in store.outside_parts()? {
            if part.home
                && part.ended.is_none()
                && part.id.starts_with(REACH_PREFIX)
                && !self.event_source_alive(&part.id)
            {
                store.outside_mutate(Mutation::Unsubscribe {
                    id: part.id,
                    targets: Vec::new(),
                    all: true,
                    reason: "run_ended".into(),
                })?;
            }
        }
        Ok(())
    }
}

fn reach_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "agent reach",
        message: message.into(),
    }
}
