//! MP-08 / MP-09 / MP-10 / MP-11 reach: the kernel owner's Room agents act on other
//! rooms and kernels. The identity is the agent's own live provider run, never a
//! token the agent holds. Without /sudo a reach caller may do elsewhere what a Room
//! agent without /sudo may do in its own room; a live /sudo window of its own turn
//! adds what /sudo adds at home. Human-only requests are refused on every path.
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
    if sudo_request_forbidden(request) {
        return ReachNeed::Excluded;
    }
    // The room tools: discover and read sessions, agents and history, attach and
    // message agents, and use machines under the rental rule.
    if matches!(
        request,
        LocalDaemonRequest::ListGatewayKernels(_)
            | LocalDaemonRequest::ListSessions(_)
            | LocalDaemonRequest::ResolveSession(_)
            | LocalDaemonRequest::GetSessionState(_)
            | LocalDaemonRequest::ListAgents(_)
            | LocalDaemonRequest::GetSessionHistoryOutline(_)
            | LocalDaemonRequest::GetSessionHistoryBlobContent(_)
            | LocalDaemonRequest::GetSessionHistoryBlobContents(_)
            | LocalDaemonRequest::AttachToSession(_)
            | LocalDaemonRequest::DetachFromSession(_)
            | LocalDaemonRequest::HeartbeatAttachments(_)
            | LocalDaemonRequest::SubmitPrompt(_)
            | LocalDaemonRequest::ListRemoteMachines(_)
            | LocalDaemonRequest::ControlPlane(_)
    ) {
        return ReachNeed::Room;
    }
    ReachNeed::Sudo
}

/// Renting is the only machine request that spends money.
pub(crate) fn rents_machine(request: &LocalDaemonRequest) -> bool {
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
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(reach_error("reach needs one active provider run"));
        };
        self.reach_for_run(run.id())
    }

    /// The live caller of a `reach:<run>` id, for the gateway and the control plane.
    pub(crate) fn reach_for_id(&self, id: &str) -> Option<ReachCaller> {
        self.reach_for_run(id.strip_prefix(REACH_PREFIX)?).ok()
    }

    fn reach_for_run(&self, run_id: &str) -> Result<ReachCaller, DaemonError> {
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
        if !current {
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

    /// `<room>/<agent> (Chariox agent)`: how another kernel's audit and paired
    /// clients name an agent of this kernel.
    pub(crate) fn agent_harness_name(&self, session_id: &str, agent_id: &str) -> Option<String> {
        let session = self.owned.session_store.get_session(session_id).ok()?;
        let agent = self.owned.agent_store.get_agent(agent_id).ok()?;
        let room = session.alias().unwrap_or_else(|| session.id());
        let name = agent.alias().unwrap_or_else(|| agent.agent_ref());
        Some(format!("{room}/{name} (Chariox agent)"))
    }

    /// Whether the owner accepts this rental. Always yes unless "Ask before agents
    /// rent machines" is on; then one decision in the agent's room, to every client.
    pub(crate) async fn confirm_machine_rental(
        &self,
        caller: &ReachCaller,
    ) -> Result<bool, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        if !config
            .user_config
            .agents
            .ask_before_renting_machines
            .unwrap_or(false)
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
                    message: format!("Rent a machine for {}?", caller.harness),
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
