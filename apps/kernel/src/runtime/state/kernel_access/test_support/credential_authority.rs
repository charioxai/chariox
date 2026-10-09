use super::*;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn kernel_access_refuses_raw_registry_credentials_and_provider_imports() {
    let worktree = crate::test_support::TestWorktree::new("access-mcp-secret-read");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(app)),
        32,
    )
    .runtime_state();
    let grant = state.insert_access_grant_for_test(session.id());
    let registry =
        crate::mcp::CharioxMcpRegistry::new(vec![worktree.path().join("registered-mcps")]);
    let mut stdio = crate::mcp::CharioxMcpServerConfig::stdio("literal-env", "true", vec![]);
    if let crate::mcp::CharioxMcpTransportConfig::Stdio { env, .. } = &mut stdio.transport {
        env.insert("API_KEY".into(), "test-only-env-secret".into());
    }
    let mut http = crate::mcp::CharioxMcpServerConfig::streamable_http(
        "literal-header",
        "https://example.com/mcp",
    );
    if let crate::mcp::CharioxMcpTransportConfig::StreamableHttp { http_headers, .. } =
        &mut http.transport
    {
        http_headers.insert(
            "Authorization".into(),
            "Bearer test-only-header-secret".into(),
        );
    }
    for config in [stdio, http] {
        registry.install(&config).unwrap();
        // These endpoints return the stored transport verbatim. Admission must
        // reject them before serialization, even with a live ordinary grant.
        assert!(registry.get(&config.name).unwrap().as_ref() == Some(&config));
        assert!(state
            .authorize_external_request(
                &grant,
                &LocalDaemonRequest::GetMcpServer(crate::local::GetMcpServerRequest {
                    workspace_id: None,
                    name: config.name,
                })
            )
            .is_err());
    }
    let credentials = crate::credential::CharioxCredentialRegistry::new(
        worktree.path().join("registered-credentials"),
    );
    let credential = crate::config::UserCredentialConfig {
        id: "literal-credential-header".into(),
        description: None,
        source: crate::config::UserCredentialSourceConfig::Env {
            name: "TEST_UNUSED".into(),
        },
        allowed_hosts: vec!["example.com".into()],
        allowed_uses: vec![crate::config::UserCredentialUse::Http],
        injection: crate::config::UserCredentialInjectionConfig::Header {
            name: "Authorization".into(),
            value: "Bearer test-only-credential-secret".into(),
        },
        metadata: None,
    };
    credentials.upsert(credential.clone()).unwrap();
    assert_eq!(credentials.get(&credential.id).unwrap(), Some(credential));
    for request in [
        LocalDaemonRequest::GetCredential(crate::local::GetCredentialRequest {
            id: "literal-credential-header".into(),
        }),
        LocalDaemonRequest::ListCredentials(crate::local::ListCredentialsRequest),
    ] {
        assert!(state.authorize_external_request(&grant, &request).is_err());
    }
    for request in [
        LocalDaemonRequest::ListMcpServers(crate::local::ListMcpServersRequest {
            workspace_id: None,
        }),
        LocalDaemonRequest::ImportMcpServers(crate::local::ImportMcpServersRequest {
            workspace_id: None,
            provider: "codex".into(),
            name: None,
        }),
    ] {
        assert!(state.authorize_external_request(&grant, &request).is_err());
    }
}

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
