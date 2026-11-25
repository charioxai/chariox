use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::{AccountKernelsRequest, LocalDaemonResponse};
use crate::runtime::cloud_api_client::{list_account_kernels, unlink_account_kernel};
use crate::runtime::command::KernelCommand;

impl CommandRouter {
    pub(super) async fn account_kernels(
        &self,
        command: &KernelCommand,
        request: &AccountKernelsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let actor = self.runtime_state.account_kernel_actor(command).ok_or_else(|| {
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
}
