//! MP-08 / MP-10 / MP-11 G08: legacy recall and room history tools stay in the
//! caller room, even under live /sudo. Another room is read only through
//! `chariox_kernel_request`.
use super::tests::{fixture, fixture_with_options, running};
use super::*;

#[tokio::test]
async fn mp11_g08_recall_tools_never_leave_the_caller_room_even_under_sudo() {
    let f = fixture();
    let own = crate::session::RuntimeSession::new("recall-own-room", None, "w", "wt", "m", "k");
    let mut foreign =
        crate::session::RuntimeSession::new("recall-foreign-room", None, "w", "wt", "m", "k");
    foreign.set_owner_user_id("another-user");
    for session in [own.clone(), foreign.clone()] {
        f.state.owned.session_store.write().restore_session(session);
    }
    let recall = |name: &'static str, session_id: &str| {
        let mut arguments = serde_json::json!({"scope":"all", "session_id":session_id});
        if name == "chariox.search_recall" {
            arguments["query"] = serde_json::json!("g08-marker");
        }
        f.router
            .dispatch_authenticated_runtime_tool_call("sudo-fixture-bearer", name, arguments)
    };
    let denied = |result: Result<_, crate::error::DaemonError>| {
        matches!(
            result,
            Err(crate::error::DaemonError::LocalTransport {
                operation: "runtime_tool_recall_scope",
                ..
            })
        )
    };
    running(&f);
    for name in ["chariox.search_recall", "chariox.query_recall"] {
        assert!(
            recall(name, &f.request.session_id).await.unwrap().ok,
            "{name}: caller room"
        );
        for other in [own.id(), foreign.id()] {
            assert!(
                denied(recall(name, other).await),
                "{name}: {other} stays closed under /sudo"
            );
        }
    }
    assert!(f.state.passkey_prompts_for("local").is_empty());

    // The typed request reaches another own room without a second approval.
    let searched = tokio::time::timeout(
        Duration::from_secs(5),
        f.router.dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request": {"SearchRecall": {"query": "g08-marker", "session_id": own.id()}}}),
        ),
    )
    .await
    .expect("another own room asked for consent again")
    .unwrap();
    assert!(f.state.passkey_prompts_for("local").is_empty());
    assert!(searched.payload.get("RecallEvents").is_some());
}

#[tokio::test]
async fn mp11_g08_history_tools_never_leave_the_caller_room_even_under_sudo() {
    let f = fixture_with_options(None, true);
    let own = crate::session::RuntimeSession::new("history-own-room", None, "w", "wt", "m", "k");
    f.state
        .owned
        .session_store
        .write()
        .restore_session(own.clone());
    let history = |name: &'static str, session_id: &str| {
        let arguments = if name == "chariox.history.search" {
            serde_json::json!({"query":"g08-marker", "session_id":session_id})
        } else {
            serde_json::json!({"event_ref":"evt_guessed", "session_id":session_id})
        };
        f.router
            .dispatch_authenticated_runtime_tool_call("sudo-fixture-bearer", name, arguments)
    };
    running(&f);
    assert!(
        history("chariox.history.search", &f.request.session_id)
            .await
            .unwrap()
            .ok,
        "caller room"
    );
    for name in ["chariox.history.search", "chariox.history.read"] {
        let result = history(name, own.id()).await;
        assert!(
            matches!(&result, Err(crate::error::DaemonError::LocalTransport { message, .. })
                if message.contains("limited to the caller room")),
            "{name}: another own room stays closed under /sudo: {result:?}"
        );
    }
    assert!(f.state.passkey_prompts_for("local").is_empty());
}
