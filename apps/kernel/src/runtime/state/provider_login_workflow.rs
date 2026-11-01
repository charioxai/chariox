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
        self.wait_for_account_login_state(session_id, agent_id, owner, id, login)
            .await
            .map(|state| state == ProviderLoginProcessState::Succeeded)
    }

    /// MP-08/MP-11: the terminal login state lets recovery publish a public
    /// succeeded/failed/cancelled outcome notice.
    pub(super) async fn wait_for_account_login_state(
        &self,
        session_id: &str,
        agent_id: &str,
        owner: &str,
        id: &str,
        login: &crate::provider::ProviderLoginStart,
    ) -> Result<ProviderLoginProcessState, DaemonError> {
        let result = self
            .drive_account_login(session_id, agent_id, owner, id, login)
            .await;
        let _ = self
            .update_provider_login_interaction(session_id, agent_id, id, None)
            .await;
        if !result
            .as_ref()
            .is_ok_and(|state| *state == ProviderLoginProcessState::Succeeded)
        {
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
    ) -> Result<ProviderLoginProcessState, DaemonError> {
        let Some(login_id) = login.login_id.as_deref() else {
            return Ok(ProviderLoginProcessState::Failed);
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
                return Ok(ProviderLoginProcessState::Failed);
            };
            match status.state {
                ProviderLoginProcessState::Succeeded
                | ProviderLoginProcessState::Failed
                | ProviderLoginProcessState::Cancelled => return Ok(status.state),
                ProviderLoginProcessState::Running => {}
            }
            let Some(template) = status.interaction else {
                return Ok(ProviderLoginProcessState::Failed);
            };
            let mut projection =
                template
                    .provider_login()
                    .cloned()
                    .unwrap_or_else(|| RuntimeProviderLogin {
                        problem: status.message.clone(),
                        kernel_id: self.provider_login_kernel_id(),
                        login: login.clone(),
                        terminal_output_base64: String::new(),
                    });
            if login.login_kind != "terminal_setup_token" {
                projection.terminal_output_base64 = status.terminal_output_base64;
            }
            let next_title = template.title().map(str::to_string);
            let next_phase = (next_title.clone(), template.message().to_string());
            if receiver.is_none() || Some(&next_phase) != phase.as_ref() {
                if receiver.is_some() {
                    self.update_provider_login_interaction(session_id, agent_id, id, None)
                        .await?;
                }
                let interaction = RuntimeInteraction::new(
                    id,
                    agent_id,
                    template.kind(),
                    template.level(),
                    next_title,
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
                phase = Some(next_phase);
            } else {
                self.update_provider_login_interaction(session_id, agent_id, id, Some(projection))
                    .await?;
            }
            tokio::select! {
                reply = receiver.as_mut().expect("login interaction registered") => {
                    receiver = None;
                    let Ok(reply) = reply else { return Ok(ProviderLoginProcessState::Failed) };
                    if reply.choice_id.as_deref() == Some("cancel") { return Ok(ProviderLoginProcessState::Cancelled) }
                    let Some(mut input) = reply.reply else { return Ok(ProviderLoginProcessState::Failed) };
                    input.push('\r');
                    let data_base64 = base64::engine::general_purpose::STANDARD.encode(input.as_bytes());
                    use zeroize::Zeroize;
                    input.zeroize();
                    match crate::runtime::provider_auth_control::execute_send_provider_login_input_request(
                        self, owner, crate::local::SendProviderLoginInputRequest { login_id: login_id.into(), data_base64 },
                    ).await {
                        Err(error @ (DaemonError::ProviderLoginCodeIncomplete | DaemonError::ProviderLoginCodeInvalid)) => {
                            self.provider_login_process_store().set_message(owner, login_id, &error.to_string(), crate::session::unix_epoch_ms())?;
                        }
                        Err(error) => return Err(error),
                        Ok(_) => {}
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
        }
    }
}
