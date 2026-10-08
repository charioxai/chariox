//! MP-08/MP-10/MP-11: one human-only provider login workflow for first use,
//! account enrollment and later recovery, projected through runtime interactions.
use super::*;
use crate::local::{LocalDaemonResponse, ProviderLoginProcessState};
use crate::session::{RuntimeInteraction, RuntimeProviderLogin};

impl KernelRuntimeState {
    pub(in crate::runtime) fn spawn_provider_login_workflow(
        &self,
        session_id: &str,
        agent_id: &str,
        owner: &str,
        login: crate::provider::ProviderLoginStart,
    ) {
        let state = self.clone();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        let owner = owner.to_string();
        tokio::spawn(async move {
            let id = format!(
                "provider-auth-recovery:{}",
                login.login_id.as_deref().unwrap_or_default()
            );
            let _ = state
                .wait_for_account_login(&session_id, &agent_id, &owner, &id, &login)
                .await;
        });
    }

    pub(super) async fn wait_for_account_login(
        &self,
        session_id: &str,
        agent_id: &str,
        owner: &str,
        id: &str,
        login: &crate::provider::ProviderLoginStart,
    ) -> Result<bool, DaemonError> {
        let result = self
            .drive_account_login(session_id, agent_id, owner, id, login)
            .await;
        let _ = self
            .update_provider_login_interaction(session_id, agent_id, id, None)
            .await;
        if !result.as_ref().is_ok_and(|succeeded| *succeeded) {
            if let Some(login_id) = login.login_id.clone() {
                let _ =
                    crate::runtime::provider_auth_control::execute_cancel_provider_login_request(
                        self,
                        owner,
                        crate::local::CancelProviderLoginRequest { login_id },
                    )
                    .await;
            }
        }
        result
    }

    async fn drive_account_login(
        &self,
        session_id: &str,
        agent_id: &str,
        owner: &str,
        id: &str,
        login: &crate::provider::ProviderLoginStart,
    ) -> Result<bool, DaemonError> {
        let Some(login_id) = login.login_id.as_deref() else {
            return Ok(false);
        };
        let mut receiver = None;
        let mut phase = None;
        loop {
            let response =
                crate::runtime::provider_auth_control::execute_get_provider_login_status_request(
                    self,
                    owner,
                    crate::local::GetProviderLoginStatusRequest {
                        login_id: login_id.into(),
                    },
                )
                .await?;
            let LocalDaemonResponse::ProviderLoginStatus { login: status } = response else {
                return Ok(false);
            };
            match status.state {
                ProviderLoginProcessState::Succeeded => return Ok(true),
                ProviderLoginProcessState::Failed | ProviderLoginProcessState::Cancelled => {
                    return Ok(false)
                }
                ProviderLoginProcessState::Running => {}
            }
            let Some(template) = status.interaction else {
                return Ok(false);
            };
            let mut projection =
                template
                    .provider_login()
                    .cloned()
                    .unwrap_or_else(|| RuntimeProviderLogin {
                        kernel_id: self.provider_login_kernel_id(),
                        login: login.clone(),
                        terminal_output_base64: String::new(),
                    });
            if login.login_kind != "terminal_setup_token" {
                projection.terminal_output_base64 = status.terminal_output_base64;
            }
            let next_phase = template.title().map(str::to_string);
            if receiver.is_none() || next_phase != phase {
                if receiver.is_some() {
                    self.update_provider_login_interaction(session_id, agent_id, id, None)
                        .await?;
                }
                let interaction = RuntimeInteraction::new(
                    id,
                    agent_id,
                    template.kind(),
                    template.level(),
                    next_phase.clone(),
                    template.message(),
                    template.choices().to_vec(),
                    template.custom_choice().cloned(),
                    Some(600),
                    None,
                )
                .with_provider_login(projection);
                receiver = Some(
                    self.create_runtime_interaction(session_id, interaction)
                        .await?,
                );
                phase = next_phase;
            } else {
                self.update_provider_login_interaction(session_id, agent_id, id, Some(projection))
                    .await?;
            }
            tokio::select! {
                reply = receiver.as_mut().expect("login interaction registered") => {
                    receiver = None;
                    let Ok(reply) = reply else { return Ok(false) };
                    if reply.choice_id.as_deref() == Some("cancel") { return Ok(false) }
                    let Some(mut input) = reply.reply else { return Ok(false) };
                    input.push('\r');
                    let data_base64 = base64::engine::general_purpose::STANDARD.encode(input.as_bytes());
                    use zeroize::Zeroize;
                    input.zeroize();
                    crate::runtime::provider_auth_control::execute_send_provider_login_input_request(
                        self, owner, crate::local::SendProviderLoginInputRequest { login_id: login_id.into(), data_base64 },
                    ).await?;
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
        }
    }
}
