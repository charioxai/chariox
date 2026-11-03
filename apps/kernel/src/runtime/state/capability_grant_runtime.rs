//! MP-08/MP-10/MP-11 A05: user-requested capability grants. Prompt causation is
//! typed (human attachment); agents never mint authority or widen a transfer.
use super::kernel_browser_runtime::host_error;
use super::*;
use crate::error::UserDomainRefusalReason;
use crate::local::UserDomainResource;
use crate::runtime::room_tool_admission::{denied, direct_child, resolve_agent};
use crate::transport::runtime_tools::RuntimeToolResult;

/// MP-08/MP-11: the typed refusal clients and providers classify by code.
pub(super) fn refused(reason: UserDomainRefusalReason) -> DaemonError {
    DaemonError::UserDomainRefused { reason }
}

/// MP-11 SB-03: process-local admission witness, never client-supplied authority.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct AppPromptDelivery {
    prompt_id: String,
    phase: Option<crate::session::DurablePromptDeliveryPhase>,
    provider_run_id: Option<String>,
    owner_request: bool,
    source_user_id: Option<String>,
}

impl KernelRuntimeState {
    pub(super) fn app_prompt_delivery(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Option<AppPromptDelivery> {
        let session = self
            .owned
            .session_store
            .get_session(agent.session_id())
            .ok()?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, agent.id())?;
        (prompt.status() == crate::session::PromptStatus::Running)
            .then(|| AppPromptDelivery::from_prompt(&prompt))
    }
}

impl AppPromptDelivery {
    pub(super) fn from_prompt(prompt: &crate::session::PromptQueueItem) -> Self {
        Self {
            prompt_id: prompt.id().into(),
            phase: prompt.durable_delivery_phase(),
            provider_run_id: prompt
                .durable_delivery_provider_run_id()
                .map(str::to_string),
            owner_request: prompt.owner_request(),
            source_user_id: prompt.source_user_id().map(str::to_string),
        }
    }
    // MP-08/MP-11 D1: home durable delivery stores the raw worker run;
    // authenticated forwarding carries its projected home run identity.
    pub(super) fn belongs_to_agent_run(
        &self,
        agent: &crate::agent::AgentInstance,
        run: &str,
    ) -> bool {
        let Some(binding) = agent.remote_execution() else {
            return self.belongs_to(run);
        };
        let Some(worker_run) = binding.active_worker_provider_run_id.as_deref() else {
            return false;
        };
        crate::provider::projected_leased_provider_run_id(&binding.leased_agent_id, worker_run)
            == run
            && self.belongs_to(worker_run)
    }
    pub(super) fn belongs_to(&self, run: &str) -> bool {
        use crate::session::DurablePromptDeliveryPhase::{Accepted, Delivered, Dispatching};
        // Native/legacy turns may lack a delivery run ID. Preserve their exact
        // witness while separately fencing the authenticated current run.
        match self.phase {
            Some(Delivered) => self.provider_run_id.as_deref().is_none_or(|id| id == run),
            Some(Dispatching) => false,
            Some(Accepted) | None => self.provider_run_id.is_none(),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareArguments {
    agent: String,
    resources: Vec<UserDomainResource>,
}

impl KernelRuntimeState {
    /// MP-08/MP-11: prompts establish causation, not arbitrary resource scope.
    /// Only the owner's resource-specific interaction reply authorizes acquisition.
    pub(super) async fn confirm_capability_request(
        &self,
        agent: &crate::agent::AgentInstance,
        resource: &str,
        scope_live: impl Fn() -> bool + Send + Sync,
    ) -> Result<String, DaemonError> {
        if agent.remote_execution().is_some() {
            return Err(refused(UserDomainRefusalReason::NotGranted));
        }
        let prompt = self
            .owner_requested_prompt(agent)
            .ok_or_else(|| refused(UserDomainRefusalReason::NotRequested))?;
        let interaction_id = format!("capability-request-{:032x}", rand::random::<u128>());
        let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
            &interaction_id,
            &interaction_id,
            "Chariox resource access",
            format!(
                "Allow agent `{}` to use {resource} for this request?",
                agent.agent_ref()
            ),
            vec![
                crate::session::RuntimeInteractionChoice::new("allow", "Allow", "allow", None),
                crate::session::RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
            ],
        );
        let receiver = self
            .create_kernel_operation_interaction(
                agent.session_id(),
                agent.owner_user_id(),
                interaction,
            )
            .await?;
        // MP-11: the owner decision itself also has a live expiry wake. A turn
        // disappearing while the owner is deciding withdraws the stale popup.
        let resolution = tokio::select! {
            result = receiver => result.map_err(|_| refused(UserDomainRefusalReason::NotGranted))?,
            _ = tokio::time::sleep(Duration::from_secs(300)) => {
                let _ = self.owned.timeout_runtime_interaction(agent.session_id(), &interaction_id);
                return Err(refused(UserDomainRefusalReason::NotGranted));
            }
            _ = async {
                while self.owner_requested_prompt(agent).as_deref() == Some(prompt.as_str())
                    && self.authorize_current_external_command().is_ok() && scope_live()
                {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            } => {
                let _ = self.owned.timeout_runtime_interaction(agent.session_id(), &interaction_id);
                return Err(refused(UserDomainRefusalReason::NotGranted));
            }
        };
        self.authorize_current_external_command()?;
        let current = self.owned.agent_store.get_agent(agent.id())?;
        if current.owner_user_id() != agent.owner_user_id()
            || current.session_id() != agent.session_id()
            || current.remote_execution() != agent.remote_execution()
            || self.owner_requested_prompt(&current).as_deref() != Some(prompt.as_str())
            || !scope_live()
        {
            return Err(refused(UserDomainRefusalReason::NotGranted));
        }
        if resolution.choice_id.as_deref() != Some("allow") {
            return Err(refused(UserDomainRefusalReason::NotRequested));
        }
        Ok(prompt)
    }

    /// MP-08/MP-11: the owner's prompt that is running this agent's turn, if any.
    /// Agent messages, workflows, schedules and automatic wakes are never owner requests.
    /// A human prompt keeps this provenance when its agent executes on a lease.
    pub(crate) fn owner_requested_prompt(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Option<String> {
        if !self.room_agent_tools_enabled() {
            return None;
        }
        let session = self.owned.session_snapshot(agent.session_id()).ok()?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, agent.id())?;
        let owner = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        (prompt.status() == crate::session::PromptStatus::Running
            && prompt.owner_request()
            && prompt
                .source_user_id()
                .is_some_and(|user| self.provider_account_authority_owner_user_id(user) == owner))
        .then(|| prompt.id().to_string())
    }

    /// MP-08/MP-11: hand a direct local child an explicit subset of the caller's
    /// granted user-domain resources. The child cannot widen or outlive it.
    pub(super) fn kernel_browser_share(
        &self,
        actor: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
        admission: &crate::runtime::kernel_browser_host::KernelBrowserAdmission,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if !self.room_agent_tools_enabled() {
            return Err(denied("grant transfer requires room agent tools"));
        }
        let request: ShareArguments = serde_json::from_value(arguments)
            .map_err(|_| host_error("MP-08: invalid grant transfer".into()))?;
        let agents = self
            .owned
            .agent_store
            .get_session_agents(actor.session_id());
        let child = resolve_agent(&agents, actor.session_id(), &request.agent)?;
        direct_child(actor, child)?;
        if child.remote_execution().is_some() || child.owner_user_id() != actor.owner_user_id() {
            return Err(denied(
                "grant transfer requires a direct child executing on this kernel; leased agents use their Room Browser/Computer route",
            ));
        }
        self.owned
            .kernel_browser_host
            .transfer_grant(admission, child.id(), &request.resources)
            .map_err(host_error)?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"shared": request.resources.len(), "agent_id": child.id()}),
        })
    }
}
