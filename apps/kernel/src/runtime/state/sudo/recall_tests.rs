//! MP-08 / MP-10 / MP-11 G08: legacy `scope: "all"` covers only rooms the
//! caller can access; another own room needs live /sudo, foreign rooms never open.
use super::tests::{fixture, running};

#[tokio::test]
async fn mp11_g08_recall_all_reaches_own_rooms_under_sudo_never_foreign() {
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
    for name in ["chariox.search_recall", "chariox.query_recall"] {
        assert!(
            recall(name, &f.request.session_id).await.unwrap().ok,
            "{name}: caller room"
        );
        assert!(
            denied(recall(name, own.id()).await),
            "{name}: own room needs live /sudo"
        );
    }
    running(&f);
    for name in ["chariox.search_recall", "chariox.query_recall"] {
        assert!(
            recall(name, own.id()).await.unwrap().ok,
            "{name}: own room under /sudo"
        );
        assert!(
            denied(recall(name, foreign.id()).await),
            "{name}: foreign room stays closed"
        );
    }
}
