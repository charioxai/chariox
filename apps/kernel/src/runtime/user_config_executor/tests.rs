//! MP-08 / MP-10 / MP-11: supplementary RPC regressions, never live acceptance.
use super::*;
use crate::account_profile::ProviderAccountAuthState;

async fn enrollment(alias: &str, accepted: bool) {
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
  [ "$CLAUDE_CODE_OAUTH_TOKEN" = 'synthetic-accepted' ] || exit 1
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
        let validated = registry
            .get("local", alias, &profile.profile_id)
            .unwrap()
            .last_validated_at_ms;
        for provider in ["claude", "claude-p", "claude-headless"] {
            let refreshed =
                crate::local::provider_requests::refresh_provider_account_profile_response(
                    registry,
                    "local",
                    provider,
                    &profile.profile_id,
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
            )
            .unwrap();
            assert_eq!(status.auth_state, "authenticated");
            assert!(status.login_hint.is_none());
            assert_eq!(
                registry
                    .get("local", provider, &profile.profile_id)
                    .unwrap()
                    .last_validated_at_ms,
                validated,
                "retaining a verified observation must not invent a fresh verification time"
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
            crate::local::provider_requests::refresh_provider_account_profile_with_config(
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
    enrollment("claude", true).await;
}
#[tokio::test]
async fn direct_setup_token_enrollment_claude_p() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-p", true).await;
}
#[tokio::test]
async fn direct_setup_token_enrollment_claude_headless() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-headless", true).await;
}
#[tokio::test]
async fn direct_setup_token_enrollment_rejects_unverified_secret() {
    crate::test_support::isolated_env_test!();
    enrollment("claude-p", false).await;
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
