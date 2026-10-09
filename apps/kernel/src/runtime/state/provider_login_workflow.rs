//! MP-08/MP-10/MP-11: one human-only provider login workflow for first use,
//! account enrollment and later recovery, projected through runtime interactions.
use super::*;
use crate::local::{LocalDaemonResponse, ProviderLoginProcessState};
use crate::session::{RuntimeInteraction, RuntimeProviderLogin};

impl KernelRuntimeState {
    pub(in crate::runtime) fn provider_login_operation_lanes(
        &self,
    ) -> crate::provider::ProviderRunOperationLanes {
        self.provider_runtime_lanes.clone()
    }

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
        // All callers of an account login join one driver. Only that driver
        // owns the human challenge and input; other affected runs await it.
        let _workflow = self
            .provider_runtime_lanes
            .acquire(&format!(
                "provider-login-workflow:{}",
                login.login_id.as_deref().unwrap_or(id)
            ))
            .await;
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
        let outcome = match &result {
            Ok(true) if login.login_kind == "terminal_setup_token" => {
                let label = self.provider_account_profile_registry().get(owner, &login.provider, &login.account_profile)
                    .map(|profile| profile.label).unwrap_or_else(|_| login.account_profile.clone());
                Some(format!("Signed in to Claude · {label}. Verified setup token saved. Manage this account in Provider Accounts."))
            }
            Ok(true) => None,
            Ok(false) => {
                let cancelled = login.login_id.as_deref().and_then(|id| self.provider_login_process_store().record_for_owner(owner, id).ok())
                    .is_some_and(|record| record.state == ProviderLoginProcessState::Cancelled);
                let failure_notice = login.login_id.as_deref().and_then(|id| self.provider_login_process_store().record_for_owner(owner, id).ok())
                    .and_then(|record| record.setup_token.and_then(|login| login.failure_notice()));
                Some(if cancelled {
                    "Provider sign-in cancelled. Choose Log in in Provider Accounts to try again."
                } else {
                    failure_notice.unwrap_or("Provider authorization or credential verification failed. Choose Log in in Provider Accounts to try again.")
                }.to_string())
            }
            Err(error) if super::runtime_interaction_owned_state::interaction_waits(error) => Some(
                "Provider authorization could not open because another interaction is pending. Resolve it, then choose Log in in Provider Accounts to try again.".to_string()
            ),
            Err(_) => Some(
                "Provider authorization could not complete on this machine. Choose Log in in Provider Accounts to try again.".to_string()
            ),
        };
        if let Some(message) = outcome {
            // Never put raw provider output, OAuth codes or transport errors
            // in durable notices. These fixed reasons are safe for all clients.
            self.owned.record_notice_for_agent(
                session_id,
                None,
                Some(agent_id),
                self.owned
                    .attachment_store
                    .list_session_attachment_ids(session_id),
                &message,
            );
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
                    let mut input = if reply.choice_id.as_deref() == Some("retry")
                        && template.choices().iter().any(|choice| choice.id() == "retry") {
                        String::new()
                    } else {
                        let Some(input) = reply.reply else { return Ok(false) };
                        input
                    };
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
