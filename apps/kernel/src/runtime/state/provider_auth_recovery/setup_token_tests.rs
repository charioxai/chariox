//! MP-08/MP-10/MP-11: two active account users join recovery and reload the
//! repaired Vault credential. Synthetic official CLI; supplementary evidence.
use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn setup_token_two_active_agents_share_recovery_and_reload_replacement() {
    setup_token_recovery_fixture(false).await;
}

#[tokio::test]
async fn setup_token_old_run_failure_preserves_completed_replacement() {
    setup_token_recovery_fixture(true).await;
}

async fn setup_token_recovery_fixture(stale_failure: bool) {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-dual-recovery-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let names = [
        "HOME",
        "CHARIOX_HOME",
        "CHARIOX_CLAUDE_BIN",
        "CHARIOX_LOG_DIR",
        "CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT",
    ];
    let old: Vec<_> = names.iter().map(std::env::var_os).collect();
    std::env::set_var("HOME", root.join("home"));
    std::env::set_var("CHARIOX_HOME", root.join("state"));
    std::env::set_var("CHARIOX_LOG_DIR", root.join("logs"));
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let binary = root.join("claude");
    std::env::set_var("CHARIOX_CLAUDE_BIN", &binary);
    let replacement = format!("sk-ant-oat01-{}", "B".repeat(96));
    std::fs::write(
        &binary,
        format!(
            r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
if [ "$1" = -p ]; then
  echo '{{"type":"result","is_error":false,"result":"OK"}}'; exit 0
fi
[ "$1" = setup-token ] || exit 90
echo login >> '{root}/logins'
echo 'https://claude.com/cai/oauth/authorize?code=true&client_id=fixture'
printf 'Paste code here if prompted > '
IFS= read -r response
printf '%s\n' '{replacement}'
"#,
            root = root.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut config =
        crate::config::DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let registry = app.provider_account_profile_registry();
    let profile = registry
        .create_managed("local", "claude", "shared expired account")
        .unwrap();
    crate::provider::store_provider_account_credential(
        &config,
        "local",
        "claude",
        &profile.profile_id,
        "synthetic-expired-token",
        true,
    )
    .unwrap();
    registry
        .update_observation(
            "local",
            "claude",
            &profile.profile_id,
            crate::account_profile::ProviderAccountAuthState::Expired,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    crate::provider::mark_provider_account_credential_verified(
        "local",
        &profile.profile_id,
        crate::provider::provider_account_credential_verification(
            "local",
            "claude",
            &profile.profile_id,
        )
        .unwrap()
        .revision,
    )
    .unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            root.to_string_lossy(),
            root.to_string_lossy(),
        ))
        .unwrap();
    let mut starts = Vec::new();
    for provider in ["claude-p", "claude-headless"] {
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), provider)
                    .with_model("claude-sonnet")
                    .with_account_profile(&profile.profile_id),
            )
            .unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            provider,
            &profile.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(agent.id())
        .with_provider_credential_env(
            crate::provider::resolve_provider_account_credentials(
                &config,
                "local",
                "claude",
                &profile.profile_id,
            )
            .unwrap(),
        );
        let mut started = app.start_provider_launch(request).unwrap();
        started.run = app.providers().mark_run_running(started.run.id()).unwrap();
        starts.push(started);
    }
    let app = Arc::new(Mutex::new(app));
    let state = super::super::provider_output_runtime_tests::owned_runtime_state(&app).await;
    // This calls the same automatic recovery seam used by failed live runs,
    // with two different run/agent claims on one expired account.
    let mut recovered = Box::pin(async {
        tokio::try_join!(
            state.recover_provider_login(&starts[0].run, "provider-auth-recovery:one", None),
            state.recover_provider_login(&starts[1].run, "provider-auth-recovery:two", None),
        )
    });
    let interaction = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            tokio::select! {
                outcome = &mut recovered => panic!("two recoveries must await one shared login, outcome={outcome:?}"),
                _ = tokio::time::sleep(Duration::from_millis(30)) => {}
            }
            let session = state.owned.session_store.get_session(session.id()).unwrap();
            let interactions: Vec<_> = session.active_interactions().iter().filter(|i| i.provider_login().is_some_and(|login| login.login.auth_url.is_some())).collect();
            if let Some(interaction) = interactions.first() {
                assert_eq!(interactions.len(), 1, "one human authorization per shared account");
                break (*interaction).clone();
            }
        }
    }).await.unwrap();
    state
        .resolve_runtime_interaction(
            session.id(),
            interaction.id(),
            interaction.custom_choice().unwrap().id(),
            Some("synthetic-authorization-code"),
        )
        .await
        .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(8), recovered)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome, (true, true));
    assert_eq!(
        std::fs::read_to_string(root.join("logins"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    for started in &starts {
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            started.run.provider(),
            &profile.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(started.run.agent_instance_id().unwrap());
        let prepared = state
            .prepare_provider_launch_request_with_vault(request, "reload repaired account")
            .await
            .unwrap();
        assert!(prepared
            .provider_credential_env
            .iter()
            .any(|(_, value)| value == replacement));
    }
    assert!(state
        .owned
        .session_store
        .get_session(session.id())
        .unwrap()
        .active_interactions()
        .is_empty());
    if stale_failure {
        assert!(state
            .try_provider_launch_auth_recovery(
                &starts[1],
                "API Error: 401 Invalid authentication credentials"
            )
            .await
            .unwrap());
        tokio::time::sleep(Duration::from_millis(300)).await;
        let observed = state
            .owned
            .provider_account_profiles
            .get("local", "claude", &profile.profile_id)
            .unwrap()
            .auth_state;
        let logins = std::fs::read_to_string(root.join("logins"))
            .unwrap()
            .lines()
            .count();
        app.lock().await.shutdown_cleanup().unwrap();
        for (name, value) in names.into_iter().zip(old) {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(
            observed,
            crate::account_profile::ProviderAccountAuthState::Authenticated,
            "MP-08/MP-10/MP-11 old run must not expire a repaired registration"
        );
        assert_eq!(
            logins, 1,
            "sequential stale failure must reload without new OAuth consent"
        );
        return;
    }
    app.lock().await.shutdown_cleanup().unwrap();
    for (name, value) in names.into_iter().zip(old) {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
