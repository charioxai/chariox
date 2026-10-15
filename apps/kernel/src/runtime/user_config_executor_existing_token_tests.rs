// MP-08/MP-10/MP-11: direct existing-token enrollment must verify before admission.
use super::*;
use crate::app::DaemonApp;
use crate::runtime::state::KernelRuntimeState;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn existing_claude_token_is_verified_before_storage_and_admission() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("claude-existing-token");
    let home = root.path().join("home");
    std::env::set_var("CHARIOX_HOME", &home);
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let executable = root.path().join("claude-probe");
    let status = root.path().join("status");
    let calls = root.path().join("calls");
    std::fs::write(
        &executable,
        format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo '2.1.292'; exit 0; fi
[ "$1" = "-p" ] || exit 2
[ "$CLAUDE_CODE_OAUTH_TOKEN" = "synthetic-token" ] || exit 3
echo probe >> '{}'
cat '{}'
"#,
            calls.display(),
            status.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", &executable);
    let mut config = crate::config::DaemonConfig::for_tests()
        .with_session_history_root(root.path().join("history"));
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    let (state, projection) = runtime(config).await;
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "local",
        None,
        None,
        &crate::local::LocalDaemonRequest::GetCredentialVaultStatus(
            crate::local::GetCredentialVaultStatusRequest,
        ),
    );
    let owner = state.provider_account_authority_owner_user_id(
        &crate::runtime::command::command_caller_user_id(&command),
    );
    let profile = state
        .provider_account_profile_registry()
        .create_managed(&owner, "claude", "Existing token")
        .unwrap();
    let request = crate::local::SetProviderAccountCredentialRequest {
        session_id: None,
        agent_id: None,
        provider: "claude-p".into(),
        account_profile: profile.profile_id.clone(),
        value: "synthetic-token".into(),
        run: false,
        overwrite: true,
    };
    std::fs::write(&status, r#"{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"CHARIOX_AUTH_OK","usage":{"output_tokens":7}}"#).unwrap();
    let response = execute_set_provider_account_credential_request(
        &projection,
        &state,
        &command,
        request.clone(),
    )
    .await;
    assert!(
        response.is_ok(),
        "MP-08 existing verified token must store: {response:?}"
    );
    assert_eq!(
        state
            .provider_account_profile_registry()
            .get(&owner, "claude", &profile.profile_id)
            .unwrap()
            .auth_state,
        crate::account_profile::ProviderAccountAuthState::Authenticated,
        "MP-08 direct storage left authentication unverified"
    );
    assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 1);
    for mode in ["claude", "claude-p", "claude-headless"] {
        assert!(state
            .provider_account_profile_registry()
            .require_authenticated(
                &owner,
                mode,
                &profile.profile_id,
                Some("haiku"),
                "submit prompt"
            )
            .is_ok());
    }
    // Failed replacement must preserve the verified old credential and state.
    std::fs::write(
        &status,
        r#"{"type":"result","is_error":true,"api_error_status":401,"error":"synthetic-private-error"}"#,
    )
    .unwrap();
    let rejected = execute_set_provider_account_credential_request(
        &projection,
        &state,
        &command,
        request.clone(),
    )
    .await;
    assert!(rejected.is_err(), "MP-11 invalid replacement must reject");
    assert!(!rejected
        .unwrap_err()
        .to_string()
        .contains("synthetic-private-error"));
    let fresh = state
        .provider_account_profile_registry()
        .create_managed(&owner, "claude", "Rejected token")
        .unwrap();
    let mut invalid = request;
    invalid.account_profile = fresh.profile_id.clone();
    assert!(execute_set_provider_account_credential_request(
        &projection,
        &state,
        &command,
        invalid
    )
    .await
    .is_err());
    assert!(!crate::provider::provider_account_credential_registered(
        &owner,
        "claude",
        &fresh.profile_id
    )
    .unwrap());
    assert_eq!(
        state
            .provider_account_profile_registry()
            .get(&owner, "claude", &fresh.profile_id)
            .unwrap()
            .auth_state,
        crate::account_profile::ProviderAccountAuthState::Unknown
    );
    assert_eq!(
        state
            .provider_account_profile_registry()
            .get(&owner, "claude", &profile.profile_id)
            .unwrap()
            .auth_state,
        crate::account_profile::ProviderAccountAuthState::Authenticated
    );
    std::env::remove_var("CHARIOX_CLAUDE_BIN");
    std::env::remove_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT");
    std::env::remove_var("CHARIOX_HOME");
}

async fn runtime(
    config: crate::config::DaemonConfig,
) -> (
    KernelRuntimeState,
    crate::runtime::projection::DaemonConfigProjectionStore,
) {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed"),
    ));
    let (
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
    ) = {
        let app_locked = app.lock().await;
        (
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.slices(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
            app_locked.prompt_state_owner(),
            app_locked.active_turn_store(),
            app_locked.prompt_activity_store(),
            app_locked.prompt_workspace_claim_store(),
            app_locked.structured_output_record_store(),
            app_locked.terminal_stream_store(),
            app_locked.workflow_design_event_store(),
            app_locked.metaagent_event_store(),
            app_locked.workspace_coordinator(),
        )
    };
    let projection = config_projection.clone();
    let state = KernelRuntimeState::new_with_owned_state(
        Arc::clone(&app),
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
    );
    (state, projection)
}
