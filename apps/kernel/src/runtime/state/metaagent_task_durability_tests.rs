use super::*;
use crate::app::{DaemonApp, KernelSessionService};
use crate::config::DaemonConfig;
use crate::runtime::router::CommandRouter;
use crate::session::CreateSessionRequest;
use crate::transport::runtime_tools::*;
use std::sync::Arc;
use tokio::sync::Mutex;

fn meta_runtime() -> (KernelRuntimeState, DaemonConfig, String, String) {
    let config = DaemonConfig::for_tests();
    let mut app = DaemonApp::bootstrap(config.clone()).expect("test daemon should boot");
    let (session, agent) = KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "meta-durability",
            "meta-durability",
        ))
        .expect("test session should create");
    app.agents_mut()
        .activate_agent_meta_mode(agent.id(), None)
        .expect("agent should enter meta mode");
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), agent.id(), "Original task")
        .expect("task should start");
    let runtime =
        CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1).runtime_state();
    (
        runtime,
        config,
        session.id().to_string(),
        agent.id().to_string(),
    )
}

#[tokio::test]
async fn meta_update_task_tool_survives_restart() {
    let (runtime, config, session_id, agent_id) = meta_runtime();
    runtime
        .dispatch_meta_runtime_tool_call_for_agent(
            &session_id,
            &agent_id,
            META_UPDATE_TASK_TOOL,
            serde_json::json!({"markdown": "Durable task edit"}),
        )
        .await
        .expect("task edit should succeed");
    let events = runtime
        .owned
        .durable_state_store
        .load_events_by_kind("session.updated")
        .expect("events should load");
    assert!(
        events
            .iter()
            .any(|event| event.payload["reason"] == "metaagent_task_updated"),
        "successful task edit must append its durable session event"
    );
    drop(runtime);
    let restored = DaemonApp::bootstrap(config).expect("daemon should restore");
    let session = restored
        .sessions()
        .get_session(&session_id)
        .expect("session should restore");
    assert_eq!(
        session
            .metaagent_task(&agent_id)
            .expect("task should restore")
            .task_markdown(),
        "Durable task edit"
    );
}

async fn assert_meta_tool_append_failure_is_retryable(
    tool: &str,
    arguments: serde_json::Value,
    reason: &str,
    terminal: bool,
) {
    let (runtime, _, session_id, agent_id) = meta_runtime();
    runtime
        .ensure_managed_activity_tracking("meta-tool-durability")
        .expect("activity tracking should activate");
    let before = runtime
        .owned
        .session_snapshot(&session_id)
        .expect("baseline should project");
    let activity_sequence = runtime.managed_activity_change_sequence();
    let projection_sequence = runtime.owned.session_projection.change_sequence();
    let connection = rusqlite::Connection::open(runtime.owned.durable_state_store.path())
        .expect("failure injection connection should open");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_meta_tool_append BEFORE INSERT ON durable_state_events
         WHEN NEW.kind = 'session.updated'
         BEGIN SELECT RAISE(FAIL, 'injected meta tool append failure'); END;",
        )
        .expect("append failure should install");

    let error = runtime
        .dispatch_meta_runtime_tool_call_for_agent(&session_id, &agent_id, tool, arguments.clone())
        .await
        .expect_err("failed append must reject the tool mutation");
    assert!(
        error
            .to_string()
            .contains("injected meta tool append failure"),
        "{error}"
    );
    let raw = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .expect("session should exist");
    assert_eq!(
        serde_json::to_value(raw.metaagent_task(&agent_id)).unwrap(),
        serde_json::to_value(before.metaagent_task(&agent_id)).unwrap(),
        "failed tool mutation must not survive in authoritative memory"
    );
    assert!(runtime
        .owned
        .agent_store
        .get_agent(&agent_id)
        .unwrap()
        .is_metaagent());
    assert_eq!(
        runtime.managed_activity_change_sequence(),
        activity_sequence
    );
    assert_eq!(
        runtime.owned.session_projection.change_sequence(),
        projection_sequence
    );
    let projected = runtime
        .owned
        .session_snapshot(&session_id)
        .expect("later snapshot should work");
    assert_eq!(
        serde_json::to_value(projected.metaagent_task(&agent_id)).unwrap(),
        serde_json::to_value(before.metaagent_task(&agent_id)).unwrap(),
        "later snapshots must not leak a rejected mutation"
    );

    connection
        .execute_batch("DROP TRIGGER fail_meta_tool_append;")
        .expect("failure should clear");
    let result = runtime
        .dispatch_meta_runtime_tool_call_for_agent(&session_id, &agent_id, tool, arguments)
        .await
        .expect("same tool mutation should retry successfully");
    assert!(result.ok);
    let events = runtime
        .owned
        .durable_state_store
        .load_events_by_kind("session.updated")
        .expect("committed events should load");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.payload["reason"] == reason)
            .count(),
        1,
        "retry should commit the requested transition exactly once"
    );
    assert_eq!(
        !runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .unwrap()
            .is_metaagent(),
        terminal
    );
}

#[tokio::test]
async fn meta_update_task_append_failure_is_retryable() {
    assert_meta_tool_append_failure_is_retryable(
        META_UPDATE_TASK_TOOL,
        serde_json::json!({"markdown": "Changed task"}),
        "metaagent_task_updated",
        false,
    )
    .await;
}

#[tokio::test]
async fn meta_update_plan_append_failure_is_retryable() {
    assert_meta_tool_append_failure_is_retryable(
        META_UPDATE_PLAN_TOOL,
        serde_json::json!({"markdown": "Changed plan"}),
        "metaagent_plan_updated",
        false,
    )
    .await;
}

#[tokio::test]
async fn meta_complete_task_append_failure_is_retryable() {
    assert_meta_tool_append_failure_is_retryable(
        META_COMPLETE_TASK_TOOL,
        serde_json::json!({"summary": "Finished"}),
        "metaagent_task_completed",
        true,
    )
    .await;
}

#[tokio::test]
async fn meta_block_task_append_failure_is_retryable() {
    assert_meta_tool_append_failure_is_retryable(
        META_MARK_BLOCKED_TOOL,
        serde_json::json!({"reason": "External blocker"}),
        "metaagent_task_blocked",
        true,
    )
    .await;
}
