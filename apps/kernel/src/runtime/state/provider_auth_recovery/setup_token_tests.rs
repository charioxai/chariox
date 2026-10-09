//! MP-08/MP-10/MP-11: two active account users join recovery and reload the
//! repaired Vault credential. Synthetic official CLI; supplementary evidence.
use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn setup_token_two_active_agents_share_recovery_and_reload_replacement() {
    setup_token_recovery_fixture("shared").await;
}

#[tokio::test]
async fn setup_token_old_run_failure_preserves_completed_replacement() {
    setup_token_recovery_fixture("completed").await;
}

#[tokio::test]
async fn setup_token_old_run_failure_during_verification_reloads_replacement() {
    setup_token_recovery_fixture("verifying").await;
}

async fn setup_token_recovery_fixture(mode: &str) {
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
  if [ '{mode}' = verifying ]; then
    touch '{root}/verifying'
    while [ ! -f '{root}/release-verification' ]; do sleep 0.02; done
  fi
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
        let mut started = if mode == "verifying" {
            // Exercise recovery with a provenance-bearing run on the base.
            // Client activation's separate revision-loss regression is in
            // app::provider_runtime::tests::launch.
            let request = request.with_working_directory(root.clone());
            let provider_credential_env = request.provider_credential_env.clone();
            let run = app
                .providers()
                .start_run_provider_only(request)
                .unwrap()
                .into_run();
            crate::app::StartedProviderLaunch {
                run,
                previous_active_run_id: None,
                provider_credential_env,
            }
        } else {
            app.start_provider_launch(request).unwrap()
        };
        started.run = app.providers().mark_run_running(started.run.id()).unwrap();
        starts.push(started);
    }
    let app = Arc::new(Mutex::new(app));
    let state = super::super::provider_output_runtime_tests::owned_runtime_state(&app).await;
    // This calls the same automatic recovery seam used by failed live runs,
    // with two different run/agent claims on one expired account.
    let mut recovered = Box::pin(async {
        let first =
            state.recover_provider_login(&starts[0].run, "provider-auth-recovery:one", None);
        if mode == "verifying" {
            first.await.map(|succeeded| (succeeded, true))
        } else {
            tokio::try_join!(
                first,
                state.recover_provider_login(&starts[1].run, "provider-auth-recovery:two", None)
            )
        }
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
    if mode == "verifying" {
        // Hold the first repair inside official-CLI verification, before the
        // account commit. Admit a late failure through the actual launch seam.
        tokio::time::timeout(Duration::from_secs(8), async {
            while !root.join("verifying").exists() {
                tokio::select! {
                    outcome = &mut recovered => panic!("repair completed before the verification gate: {outcome:?}"),
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {}
                }
            }
        }).await.unwrap();
        let rejected_revision = crate::provider::provider_account_credential_verification(
            "local",
            "claude",
            &profile.profile_id,
        )
        .unwrap()
        .revision;
        assert!(rejected_revision.is_some());
        assert_eq!(
            starts[1].run.account_credential_revision(),
            rejected_revision,
            "MP-08/MP-10/MP-11 late failure must belong to the registration still being repaired"
        );
        assert!(state
            .try_provider_launch_auth_recovery(
                &starts[1],
                "API Error: 401 Invalid authentication credentials",
            )
            .await
            .unwrap());
        // Give the spawned recovery its turn while verification is gated.
        // The replacement cannot commit until the gate below is released.
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Admission observed the old revision. Its spawned login must now
        // reconcile the verifying repair and recheck the committed revision.
        std::fs::write(root.join("release-verification"), "release").unwrap();
    }
    let outcome = tokio::time::timeout(Duration::from_secs(8), recovered)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome, (true, true));
    if mode != "verifying" {
        assert_eq!(
            std::fs::read_to_string(root.join("logins"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
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
    if mode != "verifying" {
        assert!(state
            .owned
            .session_store
            .get_session(session.id())
            .unwrap()
            .active_interactions()
            .is_empty());
    }
    if mode != "shared" {
        if mode == "completed" {
            assert!(state
                .try_provider_launch_auth_recovery(
                    &starts[1],
                    "API Error: 401 Invalid authentication credentials"
                )
                .await
                .unwrap());
        }
        let settled = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if state
                    .owned
                    .provider_auth_recovery_runs
                    .lock()
                    .unwrap()
                    .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .is_ok();
        let replacement_revision = crate::provider::provider_account_credential_verification(
            "local",
            "claude",
            &profile.profile_id,
        )
        .unwrap()
        .revision;
        let reloaded = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if state
                    .owned
                    .provider_store
                    .get_latest_run_for_agent(
                        session.id(),
                        starts[1].run.agent_instance_id().unwrap(),
                    )
                    .is_some_and(|run| {
                        run.id() != starts[1].run.id()
                            && run.account_credential_revision() == replacement_revision
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .is_ok();
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
        // Settle a redundant login on RED too, before checking the regression.
        for interaction in state
            .owned
            .session_store
            .get_session(session.id())
            .unwrap()
            .active_interactions()
        {
            if interaction.provider_login().is_some() {
                state
                    .resolve_runtime_interaction(session.id(), interaction.id(), "cancel", None)
                    .await
                    .unwrap();
            }
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while !state
                .owned
                .provider_auth_recovery_runs
                .lock()
                .unwrap()
                .is_empty()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
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
            "MP-08/MP-10/MP-11 stale failure must reload without new OAuth consent"
        );
        assert!(
            settled,
            "MP-08/MP-10/MP-11 recovery must finish without another authorization"
        );
        assert!(
            reloaded,
            "MP-08/MP-10/MP-11 recovery must launch a replacement with current provenance"
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
