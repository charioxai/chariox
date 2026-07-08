use super::*;
use std::sync::Arc;
use tokio::sync::Mutex;

async fn test_runtime_state() -> KernelRuntimeState {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed"),
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

fn metadata(invocation_id: &str) -> crate::extension::RemoteExtensionInvocationMetadata {
    crate::extension::RemoteExtensionInvocationMetadata {
        invocation_id: invocation_id.to_string(),
        provider_tool_call_id: None,
        attempt: 1,
        idempotency_key: None,
        started_at_ms: crate::session::unix_epoch_ms(),
    }
}

fn metadata_with_provider_call(
    invocation_id: &str,
    provider_tool_call_id: &str,
) -> crate::extension::RemoteExtensionInvocationMetadata {
    crate::extension::RemoteExtensionInvocationMetadata {
        invocation_id: invocation_id.to_string(),
        provider_tool_call_id: Some(provider_tool_call_id.to_string()),
        attempt: 1,
        idempotency_key: None,
        started_at_ms: crate::session::unix_epoch_ms(),
    }
}

fn context(agent_id: &str) -> crate::transport::relay_peer::RemoteExtensionInvocationContext {
    crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id: "home-kernel".to_string(),
        home_session_id: "session-1".to_string(),
        home_agent_id: agent_id.to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-1".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    }
}

fn tool(name: &str) -> crate::extension::RemoteExtensionTool {
    crate::extension::RemoteExtensionTool {
        kind: crate::extension::ExtensionKind::Script,
        name: name.to_string(),
        tool_name: name.to_string(),
        description: "test tool".to_string(),
        input_schema: serde_json::json!({}),
        authority: crate::extension::ExtensionAuthority::Home,
        definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
        execution_location: crate::extension::ExtensionExecutionLocation::Home,
        safety: None,
        timeout_sec: Some(30),
        version_hash: Some(format!("{name}-hash")),
    }
}

#[tokio::test]
async fn home_extension_invocation_replays_completed_idempotent_result() {
    let state = test_runtime_state().await;
    let context = context("agent-1");
    let tool = tool("lookup");
    let mut metadata = metadata("invoke-1");
    metadata.idempotency_key = Some("idem-1".to_string());
    assert!(state
        .begin_home_extension_invocation(&context, &tool, &metadata)
        .await
        .expect("first invocation should start")
        .is_none());
    assert!(
        state
            .complete_home_extension_invocation(
                &context,
                &tool,
                &metadata,
                serde_json::json!({"ok": true, "value": 42}),
            )
            .await
    );

    let replayed = state
        .begin_home_extension_invocation(&context, &tool, &metadata)
        .await
        .expect("idempotent duplicate should replay")
        .expect("replay should return cached result");
    assert_eq!(replayed, serde_json::json!({"ok": true, "value": 42}));
}

#[tokio::test]
async fn home_extension_idempotency_replay_is_scoped_to_tool() {
    let state = test_runtime_state().await;
    let context = context("agent-1");
    let first_tool = tool("lookup");
    let second_tool = tool("create_issue");
    let mut metadata = metadata("invoke-scoped");
    metadata.idempotency_key = Some("shared-idempotency-key".to_string());
    assert!(state
        .begin_home_extension_invocation(&context, &first_tool, &metadata)
        .await
        .expect("first tool invocation should start")
        .is_none());
    assert!(
        state
            .complete_home_extension_invocation(
                &context,
                &first_tool,
                &metadata,
                serde_json::json!({"tool": "lookup"}),
            )
            .await
    );

    assert!(state
        .begin_home_extension_invocation(&context, &second_tool, &metadata)
        .await
        .expect("same idempotency key on another tool should not replay")
        .is_none());
}

#[tokio::test]
async fn home_extension_idempotency_replay_is_scoped_to_agent() {
    let state = test_runtime_state().await;
    let first_context = context("agent-1");
    let second_context = context("agent-2");
    let tool = tool("lookup");
    let mut metadata = metadata("invoke-agent-scoped");
    metadata.idempotency_key = Some("shared-agent-idempotency-key".to_string());
    assert!(state
        .begin_home_extension_invocation(&first_context, &tool, &metadata)
        .await
        .expect("first agent invocation should start")
        .is_none());
    assert!(
        state
            .complete_home_extension_invocation(
                &first_context,
                &tool,
                &metadata,
                serde_json::json!({"agent": "agent-1"}),
            )
            .await
    );

    assert!(state
        .begin_home_extension_invocation(&second_context, &tool, &metadata)
        .await
        .expect("same idempotency key on another agent should not replay")
        .is_none());
}

#[tokio::test]
async fn home_extension_invocation_rejects_completed_non_idempotent_duplicate() {
    let state = test_runtime_state().await;
    let context = context("agent-1");
    let tool = tool("lookup");
    let metadata = metadata("invoke-2");
    assert!(state
        .begin_home_extension_invocation(&context, &tool, &metadata)
        .await
        .expect("first invocation should start")
        .is_none());
    assert!(
        state
            .complete_home_extension_invocation(
                &context,
                &tool,
                &metadata,
                serde_json::json!({"ok": true}),
            )
            .await
    );

    let error = state
        .begin_home_extension_invocation(&context, &tool, &metadata)
        .await
        .expect_err("non-idempotent duplicate should be rejected");
    assert!(
        error
            .to_string()
            .contains("duplicate non-idempotent home extension invocation"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn home_extension_invocation_rejects_duplicate_provider_tool_call_without_idempotency() {
    let state = test_runtime_state().await;
    let context = context("agent-1");
    let tool = tool("lookup");
    let first = metadata_with_provider_call("invoke-provider-1", "provider-call-1");
    let second = metadata_with_provider_call("invoke-provider-2", "provider-call-1");
    assert!(state
        .begin_home_extension_invocation(&context, &tool, &first)
        .await
        .expect("first provider tool call should start")
        .is_none());
    assert!(
        state
            .complete_home_extension_invocation(
                &context,
                &tool,
                &first,
                serde_json::json!({"ok": true}),
            )
            .await
    );

    let error = state
        .begin_home_extension_invocation(&context, &tool, &second)
        .await
        .expect_err("duplicate provider tool call should be rejected");
    assert!(
        error
            .to_string()
            .contains("duplicate non-idempotent home extension invocation"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn home_extension_invocation_rejects_in_flight_duplicate_provider_tool_call() {
    let state = test_runtime_state().await;
    let context = context("agent-1");
    let tool = tool("lookup");
    let first = metadata_with_provider_call("invoke-provider-inflight-1", "provider-call-2");
    let second = metadata_with_provider_call("invoke-provider-inflight-2", "provider-call-2");
    assert!(state
        .begin_home_extension_invocation(&context, &tool, &first)
        .await
        .expect("first provider tool call should start")
        .is_none());

    let error = state
        .begin_home_extension_invocation(&context, &tool, &second)
        .await
        .expect_err("in-flight duplicate provider tool call should be rejected");
    assert!(
        error.to_string().contains("is already in progress"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn home_extension_invocation_rejects_in_flight_duplicate() {
    let state = test_runtime_state().await;
    let context = context("agent-1");
    let tool = tool("lookup");
    let metadata = metadata("invoke-3");
    assert!(state
        .begin_home_extension_invocation(&context, &tool, &metadata)
        .await
        .expect("first invocation should start")
        .is_none());

    let error = state
        .begin_home_extension_invocation(&context, &tool, &metadata)
        .await
        .expect_err("in-flight duplicate should be rejected");
    assert!(
        error.to_string().contains("is already in progress"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn home_extension_invocation_cancelled_in_flight_is_not_cached_for_replay() {
    let state = test_runtime_state().await;
    let context = context("agent-1");
    let tool = tool("lookup");
    let mut metadata = metadata("invoke-4");
    metadata.idempotency_key = Some("idem-cancelled".to_string());
    assert!(state
        .begin_home_extension_invocation(&context, &tool, &metadata)
        .await
        .expect("first invocation should start")
        .is_none());

    state
        .owned
        .remote_extension_cancellations
        .lock()
        .await
        .insert(remote_extension_invocation_cancel_key(&context, &metadata));
    assert!(
        !state
            .complete_home_extension_invocation(
                &context,
                &tool,
                &metadata,
                serde_json::json!({"ok": true, "value": "late"}),
            )
            .await,
        "late completion after cancellation must be suppressed"
    );

    assert!(state
        .begin_home_extension_invocation(&context, &tool, &metadata)
        .await
        .expect("cancelled invocation should not leave replay state behind")
        .is_none());
}

#[tokio::test]
async fn home_extension_cancellation_is_scoped_to_invocation_not_shared_idempotency_key() {
    let state = test_runtime_state().await;
    let context = context("agent-1");
    let first_tool = tool("lookup");
    let second_tool = tool("create_issue");
    let mut first_metadata = metadata("invoke-cancel-first");
    first_metadata.idempotency_key = Some("shared-cancel-idempotency-key".to_string());
    let mut second_metadata = metadata("invoke-cancel-second");
    second_metadata.idempotency_key = first_metadata.idempotency_key.clone();

    assert!(state
        .begin_home_extension_invocation(&context, &first_tool, &first_metadata)
        .await
        .expect("first invocation should start")
        .is_none());
    assert!(state
        .begin_home_extension_invocation(&context, &second_tool, &second_metadata)
        .await
        .expect("second tool with same idempotency key should start independently")
        .is_none());

    state
        .owned
        .remote_extension_cancellations
        .lock()
        .await
        .insert(remote_extension_invocation_cancel_key(
            &context,
            &first_metadata,
        ));
    assert!(
        !state
            .complete_home_extension_invocation(
                &context,
                &first_tool,
                &first_metadata,
                serde_json::json!({"tool": "lookup"}),
            )
            .await,
        "cancelled invocation must not cache a replay result"
    );
    assert!(
        state
            .complete_home_extension_invocation(
                &context,
                &second_tool,
                &second_metadata,
                serde_json::json!({"tool": "create_issue"}),
            )
            .await,
        "cancelling one tool call must not cancel another tool with the same idempotency key"
    );

    let replayed = state
        .begin_home_extension_invocation(&context, &second_tool, &second_metadata)
        .await
        .expect("second invocation replay should be available")
        .expect("second result should be cached");
    assert_eq!(replayed, serde_json::json!({"tool": "create_issue"}));
}

#[test]
fn home_extension_cancel_match_is_scoped_to_context() {
    let first_context = context("agent-1");
    let second_context = context("agent-2");
    let tool = tool("lookup");
    let metadata = metadata("invoke-shared");
    let key = remote_extension_invocation_key(&first_context, &tool, &metadata);
    let value = RemoteExtensionInvocationState {
        invocation_id: metadata.invocation_id.clone(),
        result: None,
    };

    assert!(home_extension_invocation_matches_cancel(
        &key,
        &value,
        &first_context,
        &metadata,
    ));
    assert!(
        !home_extension_invocation_matches_cancel(&key, &value, &second_context, &metadata),
        "cancelling another context with the same invocation id must not mark this call in-flight",
    );
}

#[test]
fn home_extension_runtime_result_limit_rejects_oversized_payload() {
    let result = crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "blob": "x".repeat(HOME_EXTENSION_MAX_RESULT_BYTES),
        }),
    };
    let error = enforce_home_extension_runtime_result_limit(result)
        .expect_err("oversized runtime tool result should be rejected");
    assert!(
        error.to_string().contains("home extension result exceeded"),
        "unexpected error: {error}"
    );
}

#[test]
fn home_extension_json_result_limit_rejects_oversized_payload() {
    let error = enforce_home_extension_json_result_limit(serde_json::json!({
        "blob": "x".repeat(HOME_EXTENSION_MAX_RESULT_BYTES),
    }))
    .expect_err("oversized MCP proxy result should be rejected");
    assert!(
        error.to_string().contains("home extension result exceeded"),
        "unexpected error: {error}"
    );
}
