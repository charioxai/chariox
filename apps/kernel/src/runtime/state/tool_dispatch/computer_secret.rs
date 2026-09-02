use crate::error::DaemonError;

use super::*;

impl KernelRuntimeState {
    pub(super) async fn dispatch_computer_secret_input_tool(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        agent_id: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args = serde_json::from_value::<
            crate::transport::runtime_tools::PasteSecretToComputerArgs,
        >(arguments)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool_paste_secret_to_computer",
            message: format!("invalid tool arguments: {error}"),
        })?;
        let secret = match self
            .resolve_remote_home_credential_secret(
                provider_run,
                &args.credential_id,
                crate::transport::relay_peer::RemoteCredentialSecretInjection::Computer,
            )
            .await?
        {
            Some(secret) => secret,
            None => {
                let user_config = self.owned.config_projection.snapshot().user_config;
                let credentials = crate::credential::load_user_credentials()?;
                let service = crate::secret::RuntimeSecretService::with_vault_config(
                    credentials,
                    &user_config.credential_vault,
                )?;
                service.validate_computer_secret_input(&args.credential_id)?;
                self.ensure_computer_secret_input_approved(
                    provider_run.session_id(),
                    agent_id,
                    &args.credential_id,
                )
                .await?;
                let _vault_unlock = self
                    .ensure_vault_unlocked_for_provider_run(
                        provider_run,
                        "runtime_tool_paste_secret_to_computer",
                    )
                    .await?;
                zeroize::Zeroizing::new(service.computer_secret_input(&args.credential_id)?)
            }
        };
        let execution = self
            .execute_computer_input_as_agent(
                provider_run.session_id(),
                agent_id,
                "secret_input",
                crate::transport::room_browser_controller::RoomComputerInputAction::SecretText {
                    input:
                        crate::transport::room_browser_controller::RoomComputerSecretInput::from_zeroizing(
                            secret,
                        ),
                },
            )
            .await?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "submitted": true,
                "credential_id": args.credential_id,
                "target": "desktop_focus",
                "action_id": execution.action_id,
                "actor_id": execution.actor_id,
            }),
        })
    }
}
