//! MP-08 / MP-09 / MP-10 / MP-11 (ON-17): the sources a grant follows go through a kernel
//! restart with the grant, while its holder lives, and with nothing else.
use super::*;
use crate::durable_state::workflow_notifications::NotificationOperation;

#[tokio::test]
async fn followed_sources_survive_a_restart_with_a_grant_that_outlives_it() {
    let root = crate::test_support::TestWorktree::new("access-follows");
    let app = || {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
        config.user_config_path = root.path().join("config.toml");
        crate::test_support::bootstrap_authenticated_app(config).unwrap()
    };
    let kernel = |app| {
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            32,
        )
    };
    let live_holder = process::inspect(std::process::id()).unwrap().0;
    let mut child = std::process::Command::new("sleep")
        .arg("600")
        .spawn()
        .unwrap();
    let doomed_holder = process::inspect(child.id()).unwrap().0;
    let summary = |id: &str, term, holder: &ProcessIdentity| KernelAccessGrant {
        covered: Vec::new(),
        grant_id: id.into(),
        owner_user_id: "local".into(),
        holder_pid: holder.pid,
        holder_executable: holder.executable.clone(),
        holder_harness: None,
        term,
        expires_at_ms: None,
    };
    let worktree = crate::test_support::TestWorktree::new("access-follows-room");
    let (source, session) = {
        let app = app();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let router = kernel(app);
        let state = router.runtime_state();
        let source = state
            .owned
            .ensure_owner_action_notification_source(&session)
            .unwrap();
        state
            .owned
            .durable_state_store
            .persist_workflow_runtime_transition(&session, "follow restart fixture")
            .unwrap();
        for (id, term, clock, holder) in [
            (
                "kept",
                KernelAccessTerm::UntilHolderExits,
                None,
                &live_holder,
            ),
            (
                "timed",
                KernelAccessTerm::Minutes(30),
                Some(GrantClock::new(30, 5)),
                &live_holder,
            ),
            (
                "doomed",
                KernelAccessTerm::UntilHolderExits,
                None,
                &doomed_holder,
            ),
        ] {
            let mut access = state.owned.kernel_access.lock().unwrap();
            state
                .admit_grant(
                    &mut access,
                    id.into(),
                    summary(id, term, holder),
                    holder.clone(),
                    clock,
                )
                .unwrap();
            drop(access);
            let follow = || {
                state.follow_notification_sources(
                    id,
                    "local",
                    std::slice::from_ref(&source.source_id),
                )
            };
            follow().unwrap();
        }
        // The occurrence is emitted, and the kernel stops before the router delivers it.
        state
            .owned
            .durable_state_store
            .notify(NotificationOperation::EmitOwnerAction {
                source_id: source.source_id.clone(),
                occurrence_id: "question-1".into(),
                output: crate::session::WorkflowOutputPayload::new(
                    serde_json::json!({"interaction_id":"question-1"}).to_string(),
                    Vec::new(),
                ),
                at: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        (source, session)
    };
    child.kill().unwrap();
    child.wait().unwrap();
    let router = kernel(app());
    let state = router.runtime_state();
    assert_eq!(state.restore_kernel_access_grants().unwrap(), 1);
    let (_, active, _) = state
        .owned
        .durable_state_store
        .notification_inventory("local")
        .unwrap();
    assert_eq!(
        active
            .iter()
            .map(|sub| sub.endpoint_id.as_str())
            .collect::<Vec<_>>(),
        ["kept"],
        "only the grant that outlived the restart still follows its source"
    );
    assert_eq!(
        state.followed_notification_sources("kept").unwrap().len(),
        1
    );
    // The occurrence emitted before the stop reaches the grant that lived through it.
    // The fixture's room store starts empty; the room is what the restarted kernel restores.
    state.owned.session_store.write().restore_session(session);
    state.owned.route_workflow_notifications();
    let delivered = state
        .owned
        .durable_state_store
        .outside_list("kept", Some(0), 100)
        .unwrap()
        .unwrap()
        .events;
    assert_eq!(delivered.len(), 1, "{delivered:?}");
    assert!(matches!(
        &delivered[0].value,
        OutsideEventValue::Notification { source_id, occurrence_id, .. }
            if *source_id == source.source_id && occurrence_id == "question-1"
    ));
    for ended in ["timed", "doomed"] {
        assert!(state.outside_ended_response(ended).unwrap().is_some());
    }
}
