use crate::error::DaemonError;

use super::*;

impl KernelRuntimeState {
    pub(super) async fn dispatch_computer_secret_input_tool(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        agent_id: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.dispatch_computer_secret_input_for_authority(
            provider_run.session_id(),
            agent_id,
            arguments,
        )
        .await
    }

    pub(super) async fn dispatch_forwarded_home_computer_secret_input_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        agent: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.dispatch_computer_secret_input_for_authority(
            &context.home_session_id,
            agent.id(),
            arguments,
        )
        .await
    }

    async fn dispatch_computer_secret_input_for_authority(
        &self,
        session_id: &str,
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
        let user_config = self.owned.config_projection.snapshot().user_config;
        let credentials = crate::credential::load_user_credentials()?;
        let service = crate::secret::RuntimeSecretService::with_vault_config(
            credentials,
            &user_config.credential_vault,
        )?;
        service.validate_computer_secret_input(&args.credential_id)?;
        self.ensure_computer_secret_input_approved(session_id, agent_id, &args.credential_id)
            .await?;
        let _vault_unlock = self
            .ensure_vault_unlocked_for_agent(
                session_id,
                agent_id,
                "runtime_tool_paste_secret_to_computer",
            )
            .await?;
        let secret = zeroize::Zeroizing::new(service.computer_secret_input(&args.credential_id)?);
        let execution = self
            .execute_computer_input_as_agent(
                session_id,
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
