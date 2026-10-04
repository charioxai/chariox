use super::*;

impl KernelRuntimeState {
    pub(super) async fn control_credential_access_for_test(
        &self,
        action: &str,
        vault: &std::path::Path,
    ) -> bool {
        const SESSION: &str = "access-session";
        const AGENT: &str = "access-vault-agent";
        const PASSKEY: &str = "Access TEST Passkey";
        let root = vault.parent().unwrap().to_owned();
        match action {
            "agent-question" => {
                let receiver = self
                    .create_runtime_interaction(
                        SESSION,
                        crate::session::RuntimeInteraction::new(
                            "routine-access-question",
                            AGENT,
                            crate::session::RuntimeInteractionKind::Choice,
                            crate::session::RuntimeInteractionLevel::Info,
                            None,
                            "Continue this agent task?",
                            vec![crate::session::RuntimeInteractionChoice::new(
                                "continue", "Continue", "continue", None,
                            )],
                            None,
                            Some(300),
                            Some("continue".into()),
                        ),
                    )
                    .await
                    .unwrap();
                tokio::spawn(async move {
                    let _ = receiver.await;
                });
            }
            "vault-manage" => {
                crate::secret::unlock_chariox_encrypted_vault(
                    vault,
                    PASSKEY,
                    crate::secret::VaultUnlockLease::KernelShutdown,
                )
                .unwrap();
                let state = self.clone();
                tokio::spawn(async move {
                    let result = state.manage_credential_vault_unlock(SESSION, AGENT).await;
                    std::fs::write(
                        root.join("vault-result"),
                        if result.is_ok() { "ok" } else { "cancelled" },
                    )
                    .unwrap();
                });
            }
            "vault-unlock" => {
                crate::secret::lock_chariox_encrypted_vault(vault).unwrap();
                let state = self.clone();
                tokio::spawn(async move {
                    let result = state
                        .ensure_vault_unlocked_for_agent(SESSION, AGENT, "access-test")
                        .await;
                    std::fs::write(
                        root.join("vault-result"),
                        if result.is_ok() { "ok" } else { "cancelled" },
                    )
                    .unwrap();
                });
            }
            "vault-check" => {
                let status = crate::secret::chariox_encrypted_vault_status(vault).unwrap();
                let verifier = crate::secret::VaultPasskeyVerifier::from_passphrase(vault, PASSKEY)
                    .unwrap()
                    .unwrap();
                assert!(verifier.verify(PASSKEY).unwrap());
                std::fs::write(
                    root.join("vault-status"),
                    serde_json::to_vec(&serde_json::json!({
                        "unlocked":status.unlocked, "expires_at_ms":status.expires_at_ms,
                    }))
                    .unwrap(),
                )
                .unwrap();
            }
            "provider-batch" => {
                let config = self.owned.config_projection.snapshot();
                let profile = self
                    .owned
                    .provider_account_profiles
                    .get(crate::session::DEFAULT_LOCAL_USER_ID, "claude", "default")
                    .unwrap();
                crate::secret::unlock_chariox_encrypted_vault(
                    vault,
                    PASSKEY,
                    crate::secret::VaultUnlockLease::KernelShutdown,
                )
                .unwrap();
                crate::provider::store_provider_account_credential(
                    &config,
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    "claude",
                    &profile.profile_id,
                    "synthetic-access-test-token",
                    false,
                )
                .unwrap();
                crate::secret::lock_chariox_encrypted_vault(vault).unwrap();
                let grant = self
                    .owned
                    .kernel_access
                    .lock()
                    .unwrap()
                    .grants
                    .keys()
                    .next()
                    .unwrap()
                    .clone();
                let request = crate::local::LaunchProviderRunsRequest {
                    launches: [AGENT, "access-second-agent"]
                        .into_iter()
                        .map(|agent| crate::local::LaunchProviderRunRequest {
                            session_id: SESSION.into(),
                            agent_id: Some(agent.into()),
                            adapter_key: "dev-stub".into(),
                            provider: "claude".into(),
                            account_profile: "default".into(),
                            model: "sonnet".into(),
                            variant: None,
                            structured_endpoint: None,
                            provider_session_id: None,
                            native_tui: false,
                        })
                        .collect(),
                    max_concurrency: Some(1),
                };
                let local = LocalDaemonRequest::LaunchProviderRuns(request.clone());
                self.authorize_external_request(&grant, &local).unwrap();
                let mut command = crate::runtime::command::KernelCommand::from_local_request(
                    "external-vault-batch",
                    None,
                    None,
                    &local,
                );
                command.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
                command.caller.caller_id = grant;
                let state = self.clone();
                tokio::spawn(async move {
                    let response = crate::runtime::provider_launch_executor::execute_provider_batch_launch_command(&state,&command,request).await.unwrap();
                    let LocalDaemonResponse::ProviderRunsLaunchAccepted {
                        provider_runs,
                        failures,
                    } = response
                    else {
                        panic!("unexpected provider batch response");
                    };
                    std::fs::write(
                        root.join("batch-result"),
                        serde_json::to_vec(&serde_json::json!({
                            "provider_run_count":provider_runs.len(),
                            "failures":failures.into_iter().map(|failure| failure.message).collect::<Vec<_>>(),
                        })).unwrap(),
                    )
                    .unwrap();
                });
            }
            _ => return false,
        }
        true
    }
}
