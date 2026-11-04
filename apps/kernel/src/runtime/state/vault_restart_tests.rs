//! MP-08 / MP-10 / MP-11: locked Vault recovery must stay visible and retryable.
use super::*;

fn fixture(
    policy: crate::config::CredentialVaultUnlockPolicy,
) -> (
    KernelRuntimeState,
    Vec<(String, String)>,
    crate::test_support::TestWorktree,
    crate::test_support::TestWorktree,
) {
    let workspace = crate::test_support::TestWorktree::new("vault-restart-workspace");
    let private = crate::test_support::TestWorktree::new("vault-restart-state");
    let vault = private.path().join("vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, "restart passphrase").unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = vault.display().to_string();
    config.user_config.credential_vault.unlock_policy = policy;
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let sessions = (0..2)
        .map(|_| {
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(workspace.session_request())
                .unwrap();
            (session.id().to_owned(), agent.id().to_owned())
        })
        .collect();
    let state = super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
    (state, sessions, workspace, private)
}

fn interaction(
    state: &KernelRuntimeState,
    session: &str,
) -> Option<crate::session::RuntimeInteraction> {
    state
        .owned
        .session_store
        .get_session(session)
        .unwrap()
        .active_interactions()
        .first()
        .cloned()
}

async fn pending<'a>(
    state: &KernelRuntimeState,
    session: &str,
    call: &mut std::pin::Pin<
        Box<impl std::future::Future<Output = Result<VaultUnlockGuard, DaemonError>> + 'a>,
    >,
) -> crate::session::RuntimeInteraction {
    tokio::select! {
        biased;
        result = call => panic!("unlock returned before the user could answer: {}", match result { Ok(_) => "success".into(), Err(error) => error.to_string() }),
        result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(card) = interaction(state, session) { return card; }
                tokio::task::yield_now().await;
            }
        }) => result.expect("requesting session must show its own unlock interaction"),
    }
}

#[tokio::test]
async fn mp08_vault_restart_wait_in_another_session_cannot_hide_unlock() {
    for policy in [
        crate::config::CredentialVaultUnlockPolicy::KernelInit,
        crate::config::CredentialVaultUnlockPolicy::Ttl,
    ] {
        let (state, sessions, _workspace, _private) = fixture(policy);
        let mut first = Box::pin(state.ensure_vault_unlocked_for_agent(
            &sessions[0].0,
            &sessions[0].1,
            "restored launch",
        ));
        pending(&state, &sessions[0].0, &mut first).await;
        let mut second = Box::pin(state.ensure_vault_unlocked_for_agent(
            &sessions[1].0,
            &sessions[1].1,
            "new launch",
        ));
        // Keep the earlier wait polled, exactly as a restored provider launch is.
        tokio::select! {
            _ = &mut first => panic!("earlier launch unexpectedly completed"),
            _ = pending(&state, &sessions[1].0, &mut second) => {},
        }
    }
}

#[tokio::test]
async fn mp08_vault_restart_or_abandoned_wait_can_retry_before_the_pump() {
    for policy in [
        crate::config::CredentialVaultUnlockPolicy::KernelInit,
        crate::config::CredentialVaultUnlockPolicy::Ttl,
    ] {
        let (state, sessions, _workspace, _private) = fixture(policy);
        let (session, agent) = &sessions[0];
        // A durable session can retain this card after its in-memory responder dies.
        let orphan = vault_passphrase_interaction(
            &VaultRequester {
                agent: None,
                room: None,
            },
            agent,
            &crate::config::UserCredentialVaultConfig::default(),
            60,
        );
        let mut snapshot = state.owned.session_store.get_session(session).unwrap();
        snapshot.add_active_interaction(orphan.clone());
        state.owned.session_store.write().restore_session(snapshot);
        let mut call =
            Box::pin(state.ensure_vault_unlocked_for_agent(session, agent, "after restart"));
        let card = pending(&state, session, &mut call).await;
        assert_ne!(card.id(), orphan.id());
        drop(call);
        let mut retry = Box::pin(state.ensure_vault_unlocked_for_agent(
            session,
            agent,
            "retry cancelled caller",
        ));
        let next = pending(&state, session, &mut retry).await;
        assert_ne!(next.id(), card.id());
        assert_eq!(
            state
                .owned
                .session_store
                .get_session(session)
                .unwrap()
                .active_interactions()
                .len(),
            1
        );
    }
}

#[tokio::test(start_paused = true)]
async fn mp08_vault_unanswered_launch_ends_with_retry_guidance() {
    let (state, sessions, _workspace, _private) =
        fixture(crate::config::CredentialVaultUnlockPolicy::KernelInit);
    let (session, agent) = &sessions[0];
    let mut call = Box::pin(state.ensure_vault_unlocked_for_agent(session, agent, "new launch"));
    let card = pending(&state, session, &mut call).await;
    assert!(
        card.timeout_sec().unwrap() <= 60,
        "a launch cannot silently wait five minutes"
    );
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    let error = call
        .await
        .err()
        .expect("an unanswered unlock must end the launch")
        .to_string();
    assert!(error.contains("timed out"), "{error}");
    assert!(error.to_ascii_lowercase().contains("retry"), "{error}");
    assert!(interaction(&state, session).is_none());
    let mut retry = Box::pin(state.ensure_vault_unlocked_for_agent(session, agent, "retry launch"));
    pending(&state, session, &mut retry).await;
}

#[tokio::test]
async fn mp08_vault_unlock_resumes_other_sessions_and_keeps_one_prompt_per_session() {
    let (state, sessions, _workspace, _private) =
        fixture(crate::config::CredentialVaultUnlockPolicy::KernelInit);
    let mut first = Box::pin(state.ensure_vault_unlocked_for_agent(
        &sessions[0].0,
        &sessions[0].1,
        "restored launch",
    ));
    pending(&state, &sessions[0].0, &mut first).await;
    let mut same_session = Box::pin(state.ensure_vault_unlocked_for_agent(
        &sessions[0].0,
        &sessions[0].1,
        "concurrent same session",
    ));
    tokio::select! {
        result = &mut same_session => panic!("a duplicate must wait for the session's prompt: {}", result.is_ok()),
        _ = tokio::task::yield_now() => {},
    }
    assert_eq!(
        state
            .owned
            .session_store
            .get_session(&sessions[0].0)
            .unwrap()
            .active_interactions()
            .len(),
        1
    );
    let mut second = Box::pin(state.ensure_vault_unlocked_for_agent(
        &sessions[1].0,
        &sessions[1].1,
        "new launch",
    ));
    let card = pending(&state, &sessions[1].0, &mut second).await;
    state
        .resolve_terminal_runtime_interaction(
            &sessions[1].0,
            card.id(),
            "unlock_kernel",
            Some("restart passphrase"),
            Some(crate::session::DEFAULT_LOCAL_USER_ID),
        )
        .await
        .unwrap();
    assert!(second.await.is_ok());
    assert!(first.await.is_ok());
    assert!(same_session.await.is_ok());
    assert!(interaction(&state, &sessions[0].0).is_none());
    assert!(interaction(&state, &sessions[1].0).is_none());
}

/// A managed kernel samples its running work while a prompt is registered; reading the session list under
/// the registration's own write guard used to hang the whole kernel (rented machine, Vault prompt).
#[test]
fn mp09_managed_kernel_registers_a_vault_prompt_without_hanging() {
    let (state, sessions, _workspace, _private) =
        fixture(crate::config::CredentialVaultUnlockPolicy::KernelInit);
    state
        .ensure_managed_activity_tracking("kernel-managed-vault-prompt")
        .unwrap();
    let (session, agent) = sessions[0].clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let registering = state.clone();
    std::thread::spawn(move || {
        let card = vault_passphrase_interaction(
            &VaultRequester {
                agent: None,
                room: None,
            },
            &agent,
            &crate::config::UserCredentialVaultConfig::default(),
            60,
        );
        let _receiver = registering
            .create_terminal_credential_interaction(&session, card)
            .unwrap();
        let _ = done_tx.send(());
    });
    done_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("registering a prompt on a managed kernel must not deadlock on the session lock");
}

#[tokio::test]
async fn mp11_vault_retry_cannot_replace_another_live_decision() {
    let (state, sessions, _workspace, _private) =
        fixture(crate::config::CredentialVaultUnlockPolicy::KernelInit);
    let (session, agent) = &sessions[0];
    let card = vault_passphrase_interaction(
        &VaultRequester {
            agent: None,
            room: None,
        },
        agent,
        &crate::config::UserCredentialVaultConfig::default(),
        60,
    );
    let _receiver = state
        .create_terminal_credential_interaction(session, card.clone())
        .unwrap();
    let result = state
        .ensure_vault_unlocked_for_agent(session, agent, "competing request")
        .await;
    assert!(result.is_err());
    assert_eq!(interaction(&state, session).unwrap().id(), card.id());
}

const RUN_TOKEN: &str = "vault-refusal-bearer";

/// A kernel with a locked Vault, one handle bound to `api.example.com`, and an agent whose
/// provider run can call the runtime tools.
fn locked_vault_with_a_run() -> (
    KernelRuntimeState,
    String,
    crate::test_support::TestWorktree,
    crate::test_support::TestWorktree,
    crate::env_lock::EnvGuard,
) {
    let environment = crate::env_lock::lock();
    let workspace = crate::test_support::TestWorktree::new("vault-refusal-workspace");
    let private = crate::test_support::TestWorktree::new("vault-refusal-state");
    std::env::set_var("CHARIOX_HOME", private.path().join("home"));
    let vault = private.path().join("vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, "refusal passphrase").unwrap();
    crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .upsert(crate::config::UserCredentialConfig {
            id: "bound_key".into(),
            description: None,
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: "bound_key".into(),
            },
            allowed_hosts: vec!["api.example.com".into()],
            allowed_uses: vec![crate::config::UserCredentialUse::Http],
            injection: crate::config::UserCredentialInjectionConfig::Header {
                name: "authorization".into(),
                value: "Bearer ${secret}".into(),
            },
            metadata: None,
        })
        .unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = vault.display().to_string();
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let launch = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "default",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "vault-refusal-run",
        &launch,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: Some(workspace.path().to_owned()),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    run.set_runtime_mcp_auth_token(Some(RUN_TOKEN.into()));
    app.providers_mut().insert_run_for_test(run.clone());
    let state = super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
    state.owned.provider_run_projection.update(run);
    (
        state,
        session.id().to_owned(),
        workspace,
        private,
        environment,
    )
}

async fn call_tool(
    state: &KernelRuntimeState,
    tool: &str,
    arguments: serde_json::Value,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    state
        .dispatch_authenticated_runtime_tool_call(RUN_TOKEN, tool, arguments)
        .await
}

#[tokio::test]
async fn a_locked_vault_refuses_a_request_before_it_asks_for_the_unlock() {
    let (state, session, _workspace, _private, _environment) = locked_vault_with_a_run();
    let refused = [
        (
            crate::transport::runtime_tools::HTTP_REQUEST_WITH_CREDENTIAL_TOOL,
            serde_json::json!({"credential_id": "bound_key", "url": "http://127.0.0.1:19/echo"}),
        ),
        (
            crate::transport::runtime_tools::HTTP_REQUEST_WITH_CREDENTIAL_TOOL,
            serde_json::json!({"credential_id": "no_such_key", "url": "https://api.example.com/"}),
        ),
        (
            crate::transport::runtime_tools::SEND_SECRET_TO_TERMINAL_TOOL,
            serde_json::json!({"credential_id": "bound_key"}),
        ),
    ];
    for (tool, arguments) in refused {
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            call_tool(&state, tool, arguments),
        )
        .await
        .expect("a refused request must end at once, not wait for an unlock");
        assert!(outcome.is_err(), "{tool} should be refused");
        assert!(
            interaction(&state, &session).is_none(),
            "{tool} asked for the unlock before refusing"
        );
    }
}

#[tokio::test]
async fn a_request_that_passes_every_check_still_asks_for_the_unlock() {
    let (state, session, _workspace, _private, _environment) = locked_vault_with_a_run();
    let mut call = Box::pin(call_tool(
        &state,
        crate::transport::runtime_tools::HTTP_REQUEST_WITH_CREDENTIAL_TOOL,
        serde_json::json!({"credential_id": "bound_key", "url": "https://api.example.com/v1"}),
    ));
    tokio::select! {
        biased;
        result = &mut call => panic!("the call ended before the unlock was answered: {result:?}"),
        card = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(card) = interaction(&state, &session) { return card; }
                tokio::task::yield_now().await;
            }
        }) => {
            let card = card.expect("an allowed request asks for the unlock");
            assert!(card.message().contains("needs the Vault"), "{}", card.message());
        }
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: a person must find the card, so an agent's tool call waits longer
// than a launch, which ends quickly with retry guidance.
#[test]
fn vflag_agent_tools_get_a_longer_unlock_card_than_launches() {
    for tool in [
        "vault_generate",
        "credential_vault_manage",
        "runtime_tool_http_request_with_credential",
    ] {
        assert_eq!(vault_unlock_card_seconds(tool), 280, "{tool}");
    }
    for launch in [
        "launch provider run",
        "restore interrupted prompt",
        "new launch",
    ] {
        assert_eq!(vault_unlock_card_seconds(launch), 60, "{launch}");
    }
}
