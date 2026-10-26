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
        let orphan = vault_passphrase_interaction(session, agent, "before restart");
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
            "passphrase",
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

#[tokio::test]
async fn mp11_vault_retry_cannot_replace_another_live_decision() {
    let (state, sessions, _workspace, _private) =
        fixture(crate::config::CredentialVaultUnlockPolicy::KernelInit);
    let (session, agent) = &sessions[0];
    let card = vault_passphrase_interaction(session, agent, "live credential request");
    let _receiver = state
        .create_terminal_credential_interaction(session, card.clone())
        .unwrap();
    let result = state
        .ensure_vault_unlocked_for_agent(session, agent, "competing request")
        .await;
    assert!(result.is_err());
    assert_eq!(interaction(&state, session).unwrap().id(), card.id());
}
