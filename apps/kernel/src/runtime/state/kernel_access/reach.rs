//! Reach: the kernel owner's agents act with the owner's authority on the owner's rooms and
//! kernels (row 405). The identity is the agent's own live provider run, never a token the
//! agent holds. Payments and secrets go through the person; those requests are refused to
//! agents on every path.
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
    /// The owner's authority: an owner's agent does this readily, with no card, passkey or
    /// /sudo, on any of the owner's kernels (row 405).
    Owner,
    /// Owner-only: no agent gets it, with or without /sudo.
    HumanOnly,
    /// Excluded for agents even inside a /sudo window.
    Excluded,
}

/// An owner's agent has the owner's authority on every kernel of the owner. Only payments and
/// secrets go through the person; everything else is the default.
pub(crate) fn reach_need(request: &LocalDaemonRequest) -> ReachNeed {
    if restricted_request(request) {
        return ReachNeed::HumanOnly;
    }
    // Following other agents and answering the routine decision of another room are part of
    // driving it; the person's own cards stay theirs (`answers_a_person_only_card`).
    if is_outside_event_request(request)
        || matches!(request, LocalDaemonRequest::RespondToInteraction(_))
    {
        return ReachNeed::Owner;
    }
    if sudo_request_forbidden(request) {
        return ReachNeed::Excluded;
    }
    ReachNeed::Owner
}

impl KernelRuntimeState {
    /// The live caller of a `reach:<run>` id, for the gateway and the control plane.
    pub(crate) fn reach_for_id(&self, id: &str) -> Option<ReachCaller> {
        self.reach_for_run(id.strip_prefix(REACH_PREFIX)?, true)
            .ok()
    }

    /// The live reach caller of `run_id`, or why the run cannot act for the owner.
    pub(crate) fn reach_caller_for_run(&self, run_id: &str) -> Result<ReachCaller, DaemonError> {
        self.reach_for_run(run_id, true)
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

#[cfg(test)]
mod owner_authority_tests {
    use super::*;
    use crate::local::*;

    fn config(path: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::SetUserConfigValue(SetUserConfigValueRequest {
            path: path.into(),
            value: "true".into(),
        })
    }

    // Row 405: an owner's agent does what the owner does, deletes and machine admin included.
    #[test]
    fn an_owners_agent_has_the_owners_authority() {
        for request in [
            LocalDaemonRequest::ListProjects(ListProjectsRequest {
                include_archived: false,
            }),
            LocalDaemonRequest::KernelTerminal(KernelTerminalRequest {
                directory: None,
                operation: RoomTerminalOperation::List,
            }),
            LocalDaemonRequest::DeleteProject(DeleteProjectRequest {
                project_id: "p".into(),
            }),
            LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
                session_ref: "s".into(),
                workspace_id: None,
            }),
            LocalDaemonRequest::DeleteKernel(DeleteKernelRequest),
            LocalDaemonRequest::ControlPlane(ControlPlaneRequest::DeleteSavedState {
                allocation_ref: "a".into(),
                saved_state_ref: "v".into(),
                capability_revision: "c".into(),
                request_id: "r".into(),
                expected_revision: 1,
            }),
            LocalDaemonRequest::ApproveRemoteMachine(ApproveRemoteMachineRequest {
                machine_ref: "m".into(),
            }),
            LocalDaemonRequest::HideAgent(HideAgentRequest {
                session_id: "s".into(),
                agent_id: "a".into(),
                hidden: true,
            }),
            LocalDaemonRequest::ForgetGatewayKernel(ForgetGatewayKernelRequest {
                name: "k".into(),
            }),
            LocalDaemonRequest::CloudRelayStatus(CloudRelayStatusRequest),
            LocalDaemonRequest::GetProviderAccountProfile(GetProviderAccountProfileRequest {
                provider: "codex".into(),
                account_profile: "default".into(),
            }),
            LocalDaemonRequest::GetUserConfig(GetUserConfigRequest),
            config("agents.usage_limit_resume"),
        ] {
            assert_eq!(reach_need(&request), ReachNeed::Owner, "{request:?}");
        }
    }

    // Only payments and secrets go through the person.
    #[test]
    fn payments_and_secrets_stay_with_the_person() {
        for request in [
            LocalDaemonRequest::OwnerVault(OwnerVaultRequest::Reveal { id: "e".into() }),
            LocalDaemonRequest::ListCredentials(ListCredentialsRequest),
            LocalDaemonRequest::ExportDebugBundle(ExportDebugBundleRequest {
                session_id: "s".into(),
                bundle_label: None,
                limit: None,
            }),
            config("agents.offload_without_asking"),
            config("agents"),
            config("credential_vault.path"),
            config("relay.url"),
        ] {
            assert_ne!(reach_need(&request), ReachNeed::Owner, "{request:?}");
        }
    }
}
