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

#[derive(Clone, Debug)]
pub(crate) struct ReachCaller {
    pub(crate) id: String,
    pub(crate) run_id: String,
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) owner_user_id: String,
    /// What the target's audit and paired-clients list call the agent.
    pub(crate) harness: String,
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
            | LocalDaemonRequest::ListArchivedAgents(_)
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

/// The machine requests an owner may want to confirm: acquiring or starting a machine.
pub(crate) fn acquires_machine(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        LocalDaemonRequest::ControlPlane(
            ControlPlaneRequest::Acquire { .. }
                | ControlPlaneRequest::Start { .. }
                | ControlPlaneRequest::StartSavedState { .. }
        )
    )
}

impl KernelRuntimeState {
    /// The reach caller behind a runtime MCP bearer: one live run of an agent
    /// the kernel owner owns. A guest's agent in a shared room is refused.
    pub(crate) fn reach_for_auth_token(&self, token: &str) -> Result<ReachCaller, DaemonError> {
        self.reach_for_token(token, true)
    }

    /// Who asks for the tools of a bearer. OpenCode lists its MCP tools while its run is
    /// still starting and keeps the answer, so listing needs the owner's agent, not a live run.
    pub(crate) fn reach_listing_for_auth_token(
        &self,
        token: &str,
    ) -> Result<ReachCaller, DaemonError> {
        self.reach_for_token(token, false)
    }

    fn reach_for_token(&self, token: &str, live: bool) -> Result<ReachCaller, DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(reach_error("reach needs one active provider run"));
        };
        self.reach_for_run(run.id(), live)
    }

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
            session_id: run.session_id().to_owned(),
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

    /// Whether the owner accepts this acquisition. One decision in the agent's room, shown to every
    /// client, unless this turn already has a live /sudo window or the owner turned
    /// `agents.ask_before_acquiring_machines` off.
    pub(crate) async fn confirm_machine_acquisition(
        &self,
        caller: &ReachCaller,
    ) -> Result<bool, DaemonError> {
        if self.reach_sudo_turn(caller).is_some() {
            return Ok(true);
        }
        let config = self.owned.config_projection.snapshot();
        if !config
            .user_config
            .agents
            .ask_before_acquiring_machines
            .unwrap_or(true)
        {
            return Ok(true);
        }
        let run = self.owned.provider_store.get_run(&caller.run_id)?;
        let choice =
            |id: &str, label: &str| crate::transport::runtime_tools::RequestPopupChoiceArgs {
                id: id.into(),
                label: label.into(),
                reply: id.into(),
                style: None,
            };
        let answer = self
            .dispatch_popup_for_agent(
                &caller.session_id,
                &caller.agent_id,
                Some(&run),
                crate::transport::runtime_tools::RequestPopupArgs {
                    wait: None,
                    message: format!("Let {} acquire a machine?", caller.harness),
                    choices: vec![choice("accept", "Allow"), choice("decline", "Decline")],
                    custom_choice: None,
                    title: None,
                    level: None,
                    timeout_sec: Some(900),
                    default_on_timeout: None,
                },
            )
            .await?;
        Ok(answer.payload["choice_id"] == "accept")
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

    /// What a /sudo window of this caller's own turn adds; `None` without one.
    pub(crate) fn reach_sudo_turn(&self, caller: &ReachCaller) -> Option<KernelSudoTurn> {
        self.sudo_for_run(&caller.run_id).ok()
    }
}

fn reach_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "agent reach",
        message: message.into(),
    }
}
