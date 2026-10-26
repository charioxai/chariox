//! MP-08 / MP-10 / MP-11: enrollment preserves the invoker's own workflow traces.
use super::historical_workflows::terminal_command;
use super::*;

#[test]
fn mp11_f4_enrolled_owner_sees_its_workflow_invocations() {
    run_remote_authorization_large_stack_test("f4-workflow-visibility", workflow_visibility);
}

async fn workflow_visibility() {
    let root = crate::test_support::TestWorktree::new("f4-workflow-visibility");
    let mut config = DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "kernel-owner".into(),
        realm_id: "test-realm".into(),
        ..Default::default()
    });
    let app = DaemonApp::bootstrap(config).unwrap();
    let room = app
        .sessions_mut()
        .create_session(root.session_request())
        .unwrap();
    let id = room.id().to_string();
    let workflow = app
        .sessions_mut()
        .create_workflow(&id, Some("visible".into()))
        .unwrap();
    let mut snapshot = app.session_state_store().get_session(&id).unwrap();
    snapshot.add_member("bob", None, crate::session::CollaborationLevel::Private);
    for (name, invoker) in [
        ("before", DEFAULT_LOCAL_USER_ID),
        ("after", "kernel-owner"),
        ("peer", "bob"),
    ] {
        let mut run = crate::session::WorkflowRun::new(
            name,
            workflow.id(),
            "endpoint",
            "node",
            Some(format!("{name} private trace")),
            None,
            vec![],
            vec![],
        );
        run.set_status(crate::session::WorkflowRunStatus::Completed);
        run.set_invocation_context(1, None, Some(name.into()), 0, None);
        snapshot.create_workflow_run(run);
        app.durable_state_store()
            .persist_workflow_runtime_transition_with_invocation(
                &snapshot,
                "workflow_prompt_enqueued",
                Some(
                    crate::durable_state::workflow_invocation_provenance::WorkflowUserInvocation {
                        queue_item_id: name.into(),
                        user_id: invoker.into(),
                    },
                ),
            )
            .unwrap();
    }
    app.session_state_store().write().restore_session(snapshot);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    for (user, expected) in [
        ("kernel-owner", vec!["before", "after"]),
        ("bob", vec!["peer"]),
    ] {
        let command = |request: &LocalDaemonRequest| {
            let mut command = terminal_command(request, Some(user));
            command.caller.caller_id = format!("client-{user}");
            command.caller.client_id = Some(format!("client-{user}"));
            command
        };
        let list = LocalDaemonRequest::ListWorkflowRuns(crate::local::ListWorkflowRunsRequest {
            session_id: id.clone(),
            workflow_ref: None,
            cursor: None,
            limit: None,
        });
        let LocalDaemonResponse::WorkflowRunsListed { workflow_runs, .. } =
            router.dispatch(command(&list), list).await.unwrap()
        else {
            panic!("workflow list")
        };
        let check = |runs: &[crate::session::WorkflowRun]| {
            assert_eq!(runs.len(), expected.len(), "visible runs for {user}");
            for name in &expected {
                assert!(
                    runs.iter().any(|run| run.id() == *name),
                    "{user} sees {name}"
                );
            }
        };
        check(&workflow_runs);
        for name in ["before", "after", "peer"] {
            let get = LocalDaemonRequest::GetWorkflowRun(crate::local::GetWorkflowRunRequest {
                session_id: id.clone(),
                workflow_run_ref: name.into(),
            });
            let result = router.dispatch(command(&get), get).await;
            assert_eq!(
                result.is_ok(),
                expected.contains(&name),
                "{user} reads {name}: {result:?}"
            );
        }
        let get = LocalDaemonRequest::GetSessionState(crate::local::GetSessionStateRequest {
            session_id: id.clone(),
        });
        let LocalDaemonResponse::SessionState { session, .. } =
            router.dispatch(command(&get), get).await.unwrap()
        else {
            panic!("session state")
        };
        check(session.workflow_runs());
        let attach = LocalDaemonRequest::AttachToSession(crate::local::AttachToSessionRequest {
            session_id: id.clone(),
            client_id: format!("client-{user}"),
            capability_level: crate::attachment::ClientCapabilityLevel::FullTerminal,
        });
        let LocalDaemonResponse::SessionAttached { attachment } =
            router.dispatch(command(&attach), attach).await.unwrap()
        else {
            panic!("attachment")
        };
        let pushed = router
            .session_snapshot_projection_for_attachment(&id, attachment.id(), 0)
            .unwrap();
        check(pushed.session.workflow_runs());
    }
}
