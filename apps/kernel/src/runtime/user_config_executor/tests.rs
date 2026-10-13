//! MP-08 / MP-10 / MP-11: supplementary RPC regressions, never live acceptance.
use super::*;
use crate::account_profile::ProviderAccountAuthState;

async fn enrollment(alias: &str, accepted: bool, recheck: Option<&str>) {
    let root = std::env::temp_dir().join(format!("claudesetup-rpc-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("CHARIOX_HOME", root.join("home"));
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let source = root.join("fixture.sh");
    let executable = root.join("fixture");
    std::fs::write(&source, r#"#!/bin/sh
case "$1" in
--version) printf '%s\n' 'Claude test fixture';;
auth) printf '%s\n' '{"loggedIn":false}'; exit 1;;
-p)
  [ "$CLAUDE_CODE_OAUTH_TOKEN" = 'synthetic-accepted' ] || {
    printf '%s\n' '{"type":"result","is_error":true,"api_error_status":401}'; exit 1;
  }
  printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"CHARIOX_AUTH_OK","usage":{"output_tokens":7}}';;
*) exit 2;;
esac
"#).unwrap();
    assert!(std::process::Command::new("install")
        .args(["-m", "700"])
        .arg(&source)
        .arg(&executable)
        .status()
        .unwrap()
        .success());
    std::env::set_var("CHARIOX_CLAUDE_BIN", &executable);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    if recheck == Some("locked-vault") {
        let path = root.join("vault.json");
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = path.display().to_string();
        crate::secret::unlock_chariox_encrypted_vault(
            &path,
            "synthetic-passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
    }
    let app = crate::DaemonApp::bootstrap(config).unwrap();
    let projection = app.config_projection_store();
    let runtime = runtime(app);
    let registry = runtime.provider_account_profile_registry();
    let profile = registry
        .create_managed("local", alias, "setup-token")
        .unwrap();
    let request =
        LocalDaemonRequest::SetProviderAccountCredential(SetProviderAccountCredentialRequest {
            session_id: None,
            agent_id: None,
            provider: alias.into(),
            account_profile: profile.profile_id.clone(),
            value: if accepted {
                "synthetic-accepted"
            } else {
                "synthetic-rejected"
            }
            .into(),
            run: false,
            overwrite: false,
        });
    let command = KernelCommand::from_local_request("setup-token", None, None, &request);
    let result = execute_user_config_request(&projection, &runtime, &command, request).await;
    if accepted {
        assert!(matches!(
            result.unwrap(),
            LocalDaemonResponse::ProviderAccountCredentialStored { .. }
        ));
        assert_eq!(
            registry
                .get("local", alias, &profile.profile_id)
                .unwrap()
                .auth_state,
            ProviderAccountAuthState::Authenticated,
            "MP-08/MP-10/MP-11: stored-token receipt must establish verified authentication"
        );
        if recheck == Some("concurrent-rejection") {
            let ready = root.join("check-ready");
            let release = root.join("check-release");
            std::fs::write(&executable, format!("#!/bin/sh\ncase \"$1\" in\n--version) echo fixture;;\n*) touch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.02; done\nprintf '%s\\n' '{{\"type\":\"result\",\"is_error\":true,\"api_error_status\":401}}'\nexit 1;;\nesac\n", ready.display(), release.display())).unwrap();
            let writer_registry = registry.clone();
            let writer_profile = profile.profile_id.clone();
            let writer_alias = alias.to_string();
            let writer_config = projection.snapshot();
            let writer = std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while !ready.exists() {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "MP-08/MP-11: check must begin before replacement"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                crate::provider::store_provider_account_credential(
                    &writer_config,
                    "local",
                    &writer_alias,
                    &writer_profile,
                    "synthetic-replacement",
                    true,
                )
                .unwrap();
                writer_registry
                    .update_observation(
                        "local",
                        &writer_alias,
                        &writer_profile,
                        ProviderAccountAuthState::Authenticated,
                        None,
                        None,
                        None,
                        None,
                    )
                    .unwrap();
                std::fs::write(release, "release").unwrap();
            });
            let refreshed =
                crate::local::provider_requests::refresh_provider_account_profile_explicitly(
                    registry,
                    "local",
                    alias,
                    &profile.profile_id,
                    &projection.snapshot(),
                );
            writer.join().unwrap();
            assert!(refreshed.is_err());
            assert_eq!(
                registry
                    .get("local", alias, &profile.profile_id)
                    .unwrap()
                    .auth_state,
                ProviderAccountAuthState::Error,
                "MP-08/MP-11: rejection of the old token must not expire its replacement"
            );
            drop(runtime);
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        if matches!(recheck, Some("transient" | "unexpected-result")) {
            let before = registry.get("local", alias, &profile.profile_id).unwrap();
            let response = if recheck == Some("transient") {
                serde_json::json!({"type":"result", "is_error":true, "api_error_status":503, "num_turns":0,
                    "result":"synthetic-private-provider-detail"})
            } else {
                serde_json::json!({"type":"result", "subtype":"success", "is_error":false, "num_turns":0,
                    "result":"synthetic-private-provider-detail", "usage":{"output_tokens":0}})
            };
            std::fs::write(
                &executable,
                format!("#!/bin/sh\nprintf '%s\\n' '{response}'\n"),
            )
            .unwrap();
            let config = projection.snapshot();
            let error = crate::local::provider_requests::observe_provider_auth_status(
                registry,
                "local",
                alias,
                &profile.profile_id,
                &config,
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("inconclusive")
                    && error.contains("turns=0; valid_single_turn=false"),
                "MP-08/MP-11: classify unverifiable turns: {error}"
            );
            assert!(!error.contains("synthetic-private-provider-detail"));
            assert_eq!(
                registry.get("local", alias, &profile.profile_id).unwrap(),
                before
            );
            let error =
                crate::local::provider_requests::refresh_provider_account_profile_explicitly(
                    registry,
                    "local",
                    alias,
                    &profile.profile_id,
                    &config,
                )
                .unwrap_err()
                .to_string();
            assert!(error.contains("inconclusive"));
            assert_eq!(
                registry.get("local", alias, &profile.profile_id).unwrap(),
                before
            );
            drop(runtime);
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        if recheck == Some("automatic") {
            let before = registry.get("local", alias, &profile.profile_id).unwrap();
            // If an automatic call attempts another paid turn, this unavailable
            // executable makes the regression fail without waiting for timeout.
            std::fs::remove_file(&executable).unwrap();
            for _ in 0..3 {
                let refreshed =
                    crate::local::provider_requests::refresh_provider_account_profile_with_config(
                        registry,
                        "local",
                        alias,
                        &profile.profile_id,
                        &projection.snapshot(),
                    )
                    .expect("MP-08/MP-11: automatic refresh must not run token verification");
                assert_eq!(refreshed, before);
            }
            drop(runtime);
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        if matches!(recheck, Some("locked-vault" | "missing-executable")) {
            let before = registry.get("local", alias, &profile.profile_id).unwrap();
            if recheck == Some("locked-vault") {
                crate::secret::lock_chariox_encrypted_vault(root.join("vault.json")).unwrap();
            } else {
                std::fs::remove_file(&executable).unwrap();
            }
            let config = projection.snapshot();
            assert!(
                crate::local::provider_requests::observe_provider_auth_status(
                    registry,
                    "local",
                    alias,
                    &profile.profile_id,
                    &config
                )
                .is_err()
            );
            assert_eq!(registry.get("local", alias, &profile.profile_id).unwrap(), before,
                "MP-08/MP-11: unavailable local verification must preserve the enrolled observation");
            assert!(
                crate::local::provider_requests::refresh_provider_account_profile_explicitly(
                    registry,
                    "local",
                    alias,
                    &profile.profile_id,
                    &config
                )
                .is_err()
            );
            assert_eq!(
                registry.get("local", alias, &profile.profile_id).unwrap(),
                before
            );
            registry
                .require_authenticated(
                    "local",
                    alias,
                    &profile.profile_id,
                    Some("haiku"),
                    "MP-08 locked-vault regression",
                )
                .unwrap();
            drop(runtime);
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        if let Some(case) = recheck {
            if case == "revoked" {
                // A definitive provider refusal after enrollment, without
                // changing the stored token or touching a real login.
                std::fs::write(&source, "#!/bin/sh\ncase \"$1\" in\n--version) echo fixture;;\nauth) echo '{\"loggedIn\":false}'; exit 1;;\n*) echo '{\"type\":\"result\",\"is_error\":true,\"api_error_status\":401}'; exit 1;;\nesac\n").unwrap();
                assert!(std::process::Command::new("install")
                    .args(["-m", "700"])
                    .arg(&source)
                    .arg(&executable)
                    .status()
                    .unwrap()
                    .success());
            } else {
                crate::provider::store_provider_account_credential(
                    &projection.snapshot(),
                    "local",
                    alias,
                    &profile.profile_id,
                    "synthetic-rejected",
                    true,
                )
                .unwrap();
                if case == "native-observation" {
                    registry
                        .update_observation(
                            "local",
                            alias,
                            &profile.profile_id,
                            ProviderAccountAuthState::Authenticated,
                            Some("old native login".into()),
                            None,
                            None,
                            None,
                        )
                        .unwrap();
                }
            }
            let refresh =
                crate::local::provider_requests::refresh_provider_account_profile_explicitly(
                    registry,
                    "local",
                    alias,
                    &profile.profile_id,
                    &projection.snapshot(),
                );
            assert!(
                refresh.is_err(),
                "MP-08/MP-11: Refresh must verify the current token ({case})"
            );
            assert_eq!(
                registry
                    .get("local", alias, &profile.profile_id)
                    .unwrap()
                    .auth_state,
                ProviderAccountAuthState::Expired,
                "MP-08/MP-11: definitive rejection must skip a second paid first-use check"
            );
            registry
                .update_observation(
                    "local",
                    alias,
                    &profile.profile_id,
                    ProviderAccountAuthState::Authenticated,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
            let status = crate::local::provider_requests::observe_provider_auth_status(
                registry,
                "local",
                alias,
                &profile.profile_id,
                &projection.snapshot(),
            );
            assert!(
                status.is_err(),
                "MP-08/MP-11: Status must verify the current token ({case})"
            );
            assert_eq!(
                registry
                    .get("local", alias, &profile.profile_id)
                    .unwrap()
                    .auth_state,
                ProviderAccountAuthState::Expired,
                "MP-08/MP-11: rejected Status must agree with first-use recovery"
            );
            drop(runtime);
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        let validated = registry
            .get("local", alias, &profile.profile_id)
            .unwrap()
            .last_validated_at_ms;
        for provider in ["claude", "claude-p", "claude-headless"] {
            let refreshed =
                crate::local::provider_requests::refresh_provider_account_profile_explicitly(
                    registry,
                    "local",
                    provider,
                    &profile.profile_id,
                    &projection.snapshot(),
                )
                .unwrap();
            assert_eq!(
                refreshed.auth_state,
                ProviderAccountAuthState::Authenticated
            );
            let status = crate::local::provider_requests::observe_provider_auth_status(
                registry,
                "local",
                provider,
                &profile.profile_id,
                &projection.snapshot(),
            )
            .unwrap();
            assert_eq!(status.auth_state, "authenticated");
            assert!(status.login_hint.is_none());
            assert!(
                registry
                    .get("local", provider, &profile.profile_id)
                    .unwrap()
                    .last_validated_at_ms
                    > validated,
                "MP-08/MP-11: an explicit status check must freshly verify the current token"
            );
        }
        registry
            .update_observation(
                "local",
                alias,
                &profile.profile_id,
                ProviderAccountAuthState::Unknown,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let recovered =
            crate::local::provider_requests::refresh_provider_account_profile_explicitly(
                registry,
                "local",
                alias,
                &profile.profile_id,
                &projection.snapshot(),
            )
            .unwrap();
        assert_eq!(
            recovered.auth_state,
            ProviderAccountAuthState::Authenticated,
            "an older unverified import must recover through real verification during refresh"
        );
    } else {
        assert!(
            result.is_err(),
            "MP-08/MP-11: rejected token must not be stored or authenticate"
        );
        assert!(!crate::provider::provider_account_credential_registered(
            "local",
            alias,
            &profile.profile_id
        )
        .unwrap());
        assert_eq!(
            registry
                .get("local", alias, &profile.profile_id)
                .unwrap()
                .auth_state,
            ProviderAccountAuthState::Unknown
        );
    }
    drop(runtime);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn direct_setup_token_enrollment_claude() {
    crate::test_support::isolated_env_test!();
    enrollment("claude", true, None).await;
}
#[tokio::test]
async fn direct_setup_token_enrollment_claude_p() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-p", true, None).await;
}
#[tokio::test]
async fn direct_setup_token_enrollment_claude_headless() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-headless", true, None).await;
}
#[tokio::test]
async fn direct_setup_token_enrollment_rejects_unverified_secret() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-p", false, None).await;
}

#[tokio::test]
async fn refresh_rechecks_revoked_setup_token() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-p", true, Some("revoked")).await;
}
#[tokio::test]
async fn rejected_check_does_not_expire_a_concurrent_replacement() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-p", true, Some("concurrent-rejection")).await;
}
#[tokio::test]
async fn refresh_rechecks_replaced_setup_token() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-headless", true, Some("replaced")).await;
}
#[tokio::test]
async fn refresh_rechecks_token_after_native_observation() {
    crate::test_support::isolated_env_test!();
    enrollment("claude", true, Some("native-observation")).await;
}

#[tokio::test]
async fn status_and_refresh_preserve_authentication_with_locked_vault() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-p", true, Some("locked-vault")).await;
}

#[tokio::test]
async fn status_and_refresh_preserve_authentication_without_executable() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-headless", true, Some("missing-executable")).await;
}

#[tokio::test]
async fn automatic_refresh_does_not_verify_setup_token_or_change_observation() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-p", true, Some("automatic")).await;
}

#[tokio::test]
async fn explicit_status_preserves_authentication_on_transient_provider_failure() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-p", true, Some("transient")).await;
}

#[tokio::test]
async fn explicit_status_preserves_authentication_on_unexpected_verification_result() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-headless", true, Some("unexpected-result")).await;
}

fn runtime(app: crate::DaemonApp) -> KernelRuntimeState {
    let config_projection = app.config_projection_store();
    let session_store = app.session_state_store();
    let agent_store = app.agents().clone();
    let attachment_store = app.attachments().clone();
    let provider_store = app.providers().clone();
    let provider_process_tracking = app.provider_process_tracking_store();
    let slice_store = app.slices();
    let session_projection = app.session_state_projection_store();
    let provider_run_projection = app.provider_run_projection_store();
    let operational_history_store = app.operational_history_store();
    let durable_state_store = app.durable_state_store();
    let prompt_state_owner = app.prompt_state_owner();
    let active_turns = app.active_turn_store();
    let prompt_activity = app.prompt_activity_store();
    let prompt_workspace_claims = app.prompt_workspace_claim_store();
    let structured_output_records = app.structured_output_record_store();
    let terminal_stream = app.terminal_stream_store();
    let workflow_design_events = app.workflow_design_event_store();
    let metaagent_events = app.metaagent_event_store();
    let workspace_coordinator = app.workspace_coordinator();
    KernelRuntimeState::new_with_owned_state(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    )
}
