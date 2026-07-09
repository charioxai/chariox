use super::*;
use std::sync::Arc;
use tokio::sync::Mutex;

#[test]
fn remote_extension_manifest_pending_revoke_uses_explicit_intent_not_hash_change() {
    let previous = crate::extension::RemoteExtensionManifestSyncStatus::synced(
        "hash-before-grant".to_string(),
    );

    assert!(!remote_extension_manifest_pending_revoke(
        Some(&previous),
        Some(false),
    ));
    assert!(remote_extension_manifest_pending_revoke(
        Some(&previous),
        Some(true),
    ));
}

#[test]
fn remote_extension_manifest_pending_revoke_preserves_retry_state_only_without_intent() {
    let pending_revoke = crate::extension::RemoteExtensionManifestSyncStatus::pending(
        "hash-after-revoke".to_string(),
        true,
    )
    .failed("worker unavailable".to_string());

    assert!(remote_extension_manifest_pending_revoke(
        Some(&pending_revoke),
        None,
    ));
    assert!(!remote_extension_manifest_pending_revoke(
        Some(&pending_revoke),
        Some(false),
    ));
}

#[tokio::test]
async fn agent_config_update_ignores_legacy_processing_without_active_prompt() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    {
        let app = app.lock().await;
        app.agents_mut()
            .set_agent_processing(&agent_id, true)
            .expect("agent processing should update");
    }

    let agent = runtime
        .update_agent_config(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some(Some("workspace-next".to_string())),
            None,
        )
        .await
        .expect("stale legacy processing alone should not block config update");

    assert_eq!(agent.workspace_id(), Some("workspace-next"));
}

#[tokio::test]
async fn agent_profile_update_ignores_legacy_processing_without_active_prompt() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    {
        let app = app.lock().await;
        app.agents_mut()
            .set_agent_processing(&agent_id, true)
            .expect("agent processing should update");
    }

    let agent = runtime
        .update_agent_profile(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            Some("opencode".to_string()),
            Some("model-next".to_string()),
            None,
        )
        .await
        .expect("stale legacy processing alone should not block profile update");

    assert_eq!(agent.provider(), "opencode");
    assert_eq!(agent.model(), Some("model-next"));
}

#[tokio::test]
async fn agent_config_update_still_blocks_active_prompt_owner() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    sync_active_prompt(&app, &session_id, &agent_id).await;

    let error = runtime
        .update_agent_config(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some(Some("workspace-next".to_string())),
            None,
        )
        .await
        .expect_err("active prompt ownership should block config update");

    assert_active_turn_error(error, "update agent config");
}

#[tokio::test]
async fn agent_profile_update_still_blocks_active_prompt_owner() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    sync_active_prompt(&app, &session_id, &agent_id).await;

    let error = runtime
        .update_agent_profile(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            Some("opencode".to_string()),
            Some("model-next".to_string()),
            None,
        )
        .await
        .expect_err("active prompt ownership should block profile update");

    assert_active_turn_error(error, "update agent profile");
}

async fn agent_config_runtime() -> (Arc<Mutex<DaemonApp>>, KernelRuntimeState, String, String) {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    (app, runtime, session_id, agent_id)
}

async fn sync_active_prompt(app: &Arc<Mutex<DaemonApp>>, session_id: &str, agent_id: &str) {
    let prompt = crate::session::PromptQueueItem::new(
        "active-prompt",
        "attachment-1",
        agent_id,
        "active prompt",
        crate::session::PromptStatus::Running,
    );
    app.lock()
        .await
        .prompt_owner_sync_external_active_prompt(session_id, agent_id, Some(prompt))
        .expect("active prompt should sync");
}

fn assert_active_turn_error(error: DaemonError, operation: &'static str) {
    match error {
        DaemonError::LocalTransport {
            operation: actual,
            message,
        } => {
            assert_eq!(actual, operation);
            assert!(message.contains("has an active turn"));
        }
        other => panic!("expected active turn error, got {other:?}"),
    }
}

async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
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
        history_store,
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
            app_locked.history_store(),
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
    KernelRuntimeState::new_with_owned_state(
        Arc::clone(app),
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        history_store,
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
