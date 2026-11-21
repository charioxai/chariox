use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::{AccountKernelsRequest, LocalDaemonResponse};
use crate::runtime::cloud_api_client::{list_account_kernels, unlink_account_kernel};
use crate::runtime::command::{KernelCallerKind, KernelCommand};

impl CommandRouter {
    pub(super) async fn account_kernels(
        &self,
        command: &KernelCommand,
        request: &AccountKernelsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let actor = self.account_kernel_actor(command).ok_or_else(|| {
            crate::runtime::kernel_access::error(
                "Managing linked kernels requires the owner's kernel access. Request kernel access from the owner or run this from an agent in the owner's Room",
            )
        })?;
        let config = self.config_projection.snapshot();
        let profile = crate::runtime::cloud_relay_control::active_cloud_profile(&config)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "manage linked kernels",
                message:
                    "Sign this kernel in with the owner account before managing linked kernels"
                        .into(),
            })?;
        match request {
            AccountKernelsRequest::List {} => Ok(LocalDaemonResponse::AccountKernelsListed {
                kernels: list_account_kernels(profile).await?,
            }),
            AccountKernelsRequest::Unlink { target_id } => {
                if target_id.is_empty() || target_id.trim() != target_id {
                    return Err(crate::runtime::kernel_access::error(
                        "Choose the exact target ID returned by the account kernel directory",
                    ));
                }
                unlink_account_kernel(profile, target_id.clone(), actor).await?;
                Ok(LocalDaemonResponse::AccountKernelUnlinked {
                    target_id: target_id.clone(),
                })
            }
        }
    }

    fn account_kernel_actor(&self, command: &KernelCommand) -> Option<String> {
        if let Some(grant_id) = command.external_grant_id() {
            let grant = self.runtime_state.access_grant_summary(&grant_id)?;
            return self
                .runtime_state
                .is_gateway_owner(&grant.owner_user_id)
                .then_some(grant.grant_id);
        }
        if command.caller.caller_kind != KernelCallerKind::Metaagent {
            return None;
        }
        let run = command.provider_run_id.as_deref()?;
        let reach = self.runtime_state.reach_for_id(&format!(
            "{}{run}",
            crate::runtime::state::reach::REACH_PREFIX,
        ))?;
        (command.caller.metaagent_id.as_deref() == Some(reach.agent_id.as_str()))
            .then_some(reach.agent_id)
    }
}
