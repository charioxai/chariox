use super::*;

#[test]
fn room_workflow_controls_capture_ids_report_each_outcome_and_preserve_other_runs() {
    let harness = LocalRouterTestHarness::new();
    let worktree = crate::test_support::TestWorktree::new("room-controls");
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected: {other:?}"),
    };
    harness.with_app_mut(|app| {
        let mut room = app.sessions().get_session(session.id()).unwrap();
        room.create_workflow(crate::session::WorkflowDefinition::new("first", None));
        room.create_workflow(crate::session::WorkflowDefinition::new("other", None));
        for (id, workflow_id, status) in [
            (
                "captured-one",
                "first",
                crate::session::WorkflowRunStatus::Running,
            ),
            (
                "captured-two",
                "first",
                crate::session::WorkflowRunStatus::Waiting,
            ),
            ("later", "first", crate::session::WorkflowRunStatus::Running),
            (
                "wrong-workflow",
                "other",
                crate::session::WorkflowRunStatus::Running,
            ),
            (
                "finished",
                "first",
                crate::session::WorkflowRunStatus::Completed,
            ),
        ] {
            let mut run = crate::session::WorkflowRun::new(
                id,
                workflow_id,
                "endpoint",
                "node",
                None,
                None,
                vec![],
                vec![],
            );
            run.set_status(status);
            room.create_workflow_run(run);
        }
        app.sessions_mut().restore_session(room);
    });
    let response = harness
        .dispatch(LocalDaemonRequest::ControlRoomWorkflowRuns(
            crate::local::ControlRoomWorkflowRunsRequest {
                session_id: session.id().into(),
                workflow_id: "first".into(),
                action: crate::local::RoomWorkflowRunAction::Pause,
                run_ids: [
                    "captured-one",
                    "captured-two",
                    "wrong-workflow",
                    "missing",
                    "finished",
                ]
                .map(String::from)
                .to_vec(),
            },
        ))
        .unwrap();
    let LocalDaemonResponse::RoomWorkflowRunsControlled { results, inventory } = response else {
        panic!("unexpected")
    };
    assert_eq!(results.len(), 5);
    assert_eq!(
        results[0].outcome,
        crate::local::RoomWorkflowRunControlOutcome::Applied
    );
    assert_eq!(
        results[1].outcome,
        crate::local::RoomWorkflowRunControlOutcome::Applied
    );
    assert_eq!(
        results[2].outcome,
        crate::local::RoomWorkflowRunControlOutcome::Failed
    );
    assert_eq!(
        results[3].outcome,
        crate::local::RoomWorkflowRunControlOutcome::Failed
    );
    assert_eq!(
        results[4].outcome,
        crate::local::RoomWorkflowRunControlOutcome::Unchanged
    );
    let first = inventory
        .workflows
        .iter()
        .find(|workflow| workflow.workflow_id == "first")
        .unwrap();
    assert_eq!(first.running_count, 1);
    assert_eq!(first.paused_count, 2);
    assert_eq!(
        first
            .runs
            .iter()
            .find(|run| run.run_id == "later")
            .unwrap()
            .status,
        crate::session::WorkflowRunStatus::Running
    );
    let stopped = harness
        .dispatch(LocalDaemonRequest::ControlRoomWorkflowRuns(
            crate::local::ControlRoomWorkflowRunsRequest {
                session_id: session.id().into(),
                workflow_id: "first".into(),
                action: crate::local::RoomWorkflowRunAction::Stop,
                run_ids: vec!["captured-one".into(), "captured-two".into()],
            },
        ))
        .unwrap();
    let LocalDaemonResponse::RoomWorkflowRunsControlled { results, inventory } = stopped else {
        panic!("unexpected")
    };
    assert!(results
        .iter()
        .all(|result| result.outcome == crate::local::RoomWorkflowRunControlOutcome::Applied));
    let first = inventory
        .workflows
        .iter()
        .find(|workflow| workflow.workflow_id == "first")
        .unwrap();
    assert_eq!(first.running_count, 1);
    assert_eq!(first.paused_count, 0);
    assert_eq!(first.runs[0].run_id, "later");
}
