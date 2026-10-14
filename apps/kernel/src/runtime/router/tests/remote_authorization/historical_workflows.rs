//! MP-08 / MP-10 / MP-11: pre-proof history is owner-only and read-only.
use super::*;

#[test]
fn mp11_amfix_historical_workflows_owner_only_read_only() {
    run_remote_authorization_large_stack_test("historical-workflows", historical_workflows_inner);
}

async fn historical_workflows_inner() {
    let root = crate::test_support::TestWorktree::new("historical-workflows");
    let mut config = DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "kernel-owner".into(),
        realm_id: "test-realm".into(),
        ..Default::default()
    });
    let app = DaemonApp::bootstrap(config).unwrap();
    let room = app
        .sessions_mut()
        .create_session(root.session_request().with_owner_user_id("room-owner"))
        .unwrap();
    let id = room.id().to_string();
    for (user, level) in [
        ("kernel-owner", crate::session::CollaborationLevel::Private),
        ("collaborator", crate::session::CollaborationLevel::Full),
    ] {
        let (_, invite) = app
            .sessions_mut()
            .create_session_invite(
                &id,
                format!("invite-{user}"),
                "room-owner".into(),
                None,
                Some(1),
                level,
            )
            .unwrap();
        app.sessions_mut()
            .join_session_invite(&id, invite.invite_id(), user.into(), 1)
            .unwrap();
    }
    let workflow = app
        .sessions_mut()
        .create_workflow(&id, Some("historical".into()))
        .unwrap();
    let mut snapshot = app.session_state_store().get_session(&id).unwrap();
    let make_run = |name: &str, status| {
        let mut run = crate::session::WorkflowRun::new(
            name,
            workflow.id(),
            "endpoint",
            "node",
            Some("historical prompt".into()),
            None,
            vec![],
            vec![],
        );
        run.set_status(status);
        run
    };
    snapshot.create_workflow_run(make_run(
        "old-active",
        crate::session::WorkflowRunStatus::Running,
    ));
    snapshot.create_workflow_run(make_run(
        "old-archive",
        crate::session::WorkflowRunStatus::Completed,
    ));
    app.session_state_store()
        .write()
        .restore_session(snapshot.clone());
    assert!(
        !snapshot
            .workflow_run_visible_for_user(&snapshot.workflow_runs()[0], "collaborator", None)
            .unwrap(),
        "MP-11: missing provenance context fails closed even for Full members"
    );
    app.durable_state_store()
        .persist_workflow_runtime_transition(&snapshot, "historical-fixture")
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    for user in ["kernel-owner", "room-owner", "collaborator"] {
        let req = LocalDaemonRequest::ListWorkflowRuns(crate::local::ListWorkflowRunsRequest {
            session_id: id.clone(),
            workflow_ref: None,
            cursor: None,
            limit: None,
        });
        let response = router
            .dispatch(remote_command_for_request(&req, Some(user)), req)
            .await
            .unwrap();
        let LocalDaemonResponse::WorkflowRunsListed { workflow_runs, .. } = response else {
            panic!("list");
        };
        if user == "kernel-owner" {
            assert_eq!(
                workflow_runs.len(),
                2,
                "MP-11: kernel owner sees pre-proof history"
            );
            for run in &workflow_runs {
                assert_eq!(
                    serde_json::to_value(run).unwrap()["created_before_ownership_tracking"],
                    true
                );
                assert_eq!(run.invocation_prompt(), Some("historical prompt"));
            }
        } else {
            assert!(
                workflow_runs.is_empty(),
                "MP-11: even Full collaborators cannot read legacy history"
            );
        }
        let req = LocalDaemonRequest::GetSessionState(crate::local::GetSessionStateRequest {
            session_id: id.clone(),
        });
        let response = router
            .dispatch(remote_command_for_request(&req, Some(user)), req)
            .await
            .unwrap();
        let LocalDaemonResponse::SessionState {
            session,
            room_workflows,
            ..
        } = response
        else {
            panic!("snapshot");
        };
        if user == "kernel-owner" {
            assert_eq!(session.workflow_runs().len(), 2);
            assert!(session
                .workflow_runs()
                .iter()
                .all(|run| serde_json::to_value(run).unwrap()
                    ["created_before_ownership_tracking"]
                    == true));
            assert_eq!(room_workflows.workflows[0].runs.len(), 1);
            let run = &room_workflows.workflows[0].runs[0];
            assert!(!run.can_pause && !run.can_resume && !run.can_stop);
        } else {
            assert!(session.workflow_runs().is_empty());
            assert!(room_workflows.workflows[0].runs.is_empty());
        }
        for name in ["old-active", "old-archive"] {
            let req = LocalDaemonRequest::GetWorkflowRun(crate::local::GetWorkflowRunRequest {
                session_id: id.clone(),
                workflow_run_ref: name.into(),
            });
            let got = router
                .dispatch(remote_command_for_request(&req, Some(user)), req)
                .await;
            assert_eq!(
                got.is_ok(),
                user == "kernel-owner",
                "MP-11: Get legacy {user}"
            );
        }
        for req in [
            LocalDaemonRequest::PauseWorkflowRun(crate::local::PauseWorkflowRunRequest {
                session_id: id.clone(),
                workflow_run_ref: "old-active".into(),
            }),
            LocalDaemonRequest::CancelWorkflowRun(crate::local::CancelWorkflowRunRequest {
                session_id: id.clone(),
                workflow_run_ref: "old-active".into(),
            }),
            LocalDaemonRequest::ResumeWorkflowRun(crate::local::ResumeWorkflowRunRequest {
                session_id: id.clone(),
                workflow_run_ref: "old-active".into(),
            }),
        ] {
            let error = router
                .dispatch(remote_command_for_request(&req, Some(user)), req)
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains("ownership tracking"),
                "MP-11: legacy mutation denied by provenance for {user}: {error}"
            );
        }
        let req = LocalDaemonRequest::ControlRoomWorkflowRuns(
            crate::local::ControlRoomWorkflowRunsRequest {
                session_id: id.clone(),
                workflow_id: workflow.id().into(),
                action: crate::local::RoomWorkflowRunAction::Stop,
                run_ids: vec!["old-active".into()],
            },
        );
        let response = router
            .dispatch(remote_command_for_request(&req, Some(user)), req)
            .await
            .unwrap();
        let LocalDaemonResponse::RoomWorkflowRunsControlled { results, .. } = response else {
            panic!("control");
        };
        assert_eq!(
            results[0].outcome,
            crate::local::RoomWorkflowRunControlOutcome::Failed
        );
    }
    assert_eq!(
        app.lock()
            .await
            .session_state_store()
            .get_session(&id)
            .unwrap()
            .workflow_runs()[0]
            .status(),
        crate::session::WorkflowRunStatus::Running
    );
}

// MP-08 / MP-10 / MP-11: completed history is pruned from hot state but remains readable.
#[test]
fn mp11_amfix_historical_archive_resume_reports_read_only() {
    run_remote_authorization_large_stack_test(
        "historical-archive-resume",
        historical_archive_resume_inner,
    );
}

async fn historical_archive_resume_inner() {
    let root = crate::test_support::TestWorktree::new("historical-archive-resume");
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let room = app
        .sessions_mut()
        .create_session(root.session_request())
        .unwrap();
    let mut snapshot = app.session_state_store().get_session(room.id()).unwrap();
    let mut run = crate::session::WorkflowRun::new(
        "archived",
        "workflow",
        "endpoint",
        "node",
        None,
        None,
        vec![],
        vec![],
    );
    run.set_status(crate::session::WorkflowRunStatus::Completed);
    snapshot.create_workflow_run(run);
    app.durable_state_store()
        .persist_workflow_runtime_transition(&snapshot, "historical-archive-fixture")
        .unwrap();
    // Keep the Room and durable archive; the terminal run is no longer hot.
    assert!(room.workflow_runs().is_empty());
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let req = LocalDaemonRequest::GetWorkflowRun(crate::local::GetWorkflowRunRequest {
        session_id: room.id().into(),
        workflow_run_ref: "archived".into(),
    });
    let response = router
        .dispatch(remote_command_for_request(&req, Some("local")), req)
        .await
        .unwrap();
    let LocalDaemonResponse::WorkflowRun { workflow_run } = response else {
        panic!("archive read");
    };
    assert!(workflow_run.created_before_ownership_tracking());
    let req = LocalDaemonRequest::ResumeWorkflowRun(crate::local::ResumeWorkflowRunRequest {
        session_id: room.id().into(),
        workflow_run_ref: "archived".into(),
    });
    let error = router
        .dispatch(remote_command_for_request(&req, Some("local")), req)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("ownership tracking"),
        "MP-11: an archived historical run must report its read-only policy: {error}"
    );
}
