//! MP-08 / MP-10 / MP-11: actual harness ACKs travel over the authenticated lease tool path.
use super::*;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RemoteWorkspaceLiveSyncContext,
};

pub(in crate::runtime::state) const LEASED_SKILL_DELIVERY_ACK: &str = "_chariox_skill_delivery_ack";

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "delivery", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum LeasedSkillAcknowledgement {
    Prompt,
    Tool { skill_id: String, digest: String },
}
impl LeasedSkillAcknowledgement {
    fn admits(&self, authority: &SkillTurnAuthority, record: &SkillLoadRecord) -> bool {
        match self {
            Self::Prompt => authority
                .prompt_delivery_skill_ids
                .contains(&record.skill_id),
            Self::Tool { skill_id, digest } => {
                &record.skill_id == skill_id && &record.digest == digest
            }
        }
    }
}

pub(in crate::runtime::state) fn acknowledge_leased_delivery(
    state: &KernelRuntimeState,
    context: &RemoteWorkspaceLiveSyncContext,
    arguments: serde_json::Value,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    // The caller holds the app mutation lane while checking the authenticated binding
    // and committing receipts; record_pending_loads separately holds prompt ownership.
    {
        state.authorize_forwarded_workspace_context(context)?;
        let prompt_id = context
            .home_prompt_id
            .as_deref()
            .ok_or_else(|| failure("skill delivery requires the exact home turn"))?;
        let acknowledgement: LeasedSkillAcknowledgement = serde_json::from_value(arguments)
            .map_err(|_| failure("invalid leased skill delivery acknowledgement"))?;
        service(state).record_pending_loads(
            &leased_run_id(&context.leased_agent_id),
            &context.home_session_id,
            &context.home_agent_id,
            prompt_id,
            |authority, record| acknowledgement.admits(authority, record),
        )?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({ "acknowledged": true }),
        })
    }
}

impl SkillDispatchService {
    pub(super) fn forward_leased_delivery(
        &self,
        run: &RuntimeProviderRun,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        acknowledgement: LeasedSkillAcknowledgement,
    ) -> Result<(), DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let Some(prompt) = self
            .prompts
            .active_prompt_for_agent(&session, agent_id)
            .filter(|prompt| {
                prompt.id() == prompt_id
                    && matches!(
                        prompt.status(),
                        crate::session::PromptStatus::Running
                            | crate::session::PromptStatus::Dispatching
                    )
            })
        else {
            return Ok(());
        };
        let Some(context) = prompt
            .leased_skill_context()
            .filter(|context| context.worker_provider_run_id == run.id())
        else {
            return Ok(());
        };
        let config = self.config.snapshot();
        let request = RelayPeerRequest::ForwardCapabilityRuntimeTool {
            context: context.clone(),
            tool_name: LEASED_SKILL_DELIVERY_ACK.into(),
            arguments: serde_json::to_value(acknowledgement)
                .map_err(|_| failure("cannot encode skill delivery"))?,
        };
        // ACK settlement is synchronous and retryable in the existing provider
        // completion queue. The home handler never calls this worker back.
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| failure("skill delivery runtime unavailable"))?;
        let response = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    handle.block_on(super::super::tool_dispatch::send_worker_home_request(
                        &config,
                        &self.relay_state,
                        &context.home_kernel_id,
                        request,
                        std::time::Duration::from_millis(config.relay_request_timeout_ms),
                    ))
                })
                .join()
        })
        .map_err(|_| failure("skill delivery acknowledgement interrupted"))??;
        match response {
            RelayPeerResponse::CapabilityRuntimeToolHandled { result, .. } if result.ok => Ok(()),
            _ => Err(failure("home kernel did not acknowledge skill delivery")),
        }
    }
}
