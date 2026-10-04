use super::super::run_workflow_command_lane;
use super::*;
use crate::local::KernelConnectionClass;
use crate::test_support::TestWorktree;
use tokio::sync::mpsc;

#[tokio::test]
async fn kernel_access_revocation_refuses_an_already_queued_workflow_mutation() {
    let worktree = TestWorktree::new("access-workflow-queue");
    let mut daemon = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request())
        .unwrap();
    let projection = daemon.session_state_projection_store();
    projection.update(session.clone());
    let agent_projection = daemon.agent_runtime_projection_store();
    let app = Arc::new(Mutex::new(daemon));
    let state = owned_runtime_state(&app).await;
    let before = state
        .session_snapshot(session.id())
        .await
        .unwrap()
        .workflows()
        .len();
    let grant = state.insert_access_grant_for_test(session.id());
    let runtime = WorkflowRuntime::new(state.clone(), projection.clone(), agent_projection.clone());
    let request = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
        session_id: session.id().into(),
        alias: Some("external-queued-workflow".into()),
    });
    state.authorize_external_request(&grant, &request).unwrap();
    let mut command =
        KernelCommand::from_local_request("external-queued-workflow", None, None, &request);
    command.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant.clone();

    let (tx, rx) = mpsc::channel(super::super::WORKFLOW_COMMAND_QUEUE_LIMIT);
    runtime
        .lanes
        .lock()
        .await
        .insert(session.id().into(), tx.clone());
    let queued_runtime = runtime.clone();
    let queued_request = request.clone();
    let pending = tokio::spawn(async move {
        queued_runtime
            .dispatch_workflow_command(command, queued_request)
            .await
    });
    timeout(Duration::from_secs(2), async {
        while tx.capacity() == super::super::WORKFLOW_COMMAND_QUEUE_LIMIT {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    let consumer = tokio::spawn(run_workflow_command_lane(
        super::super::WorkflowRuntimeStore::new(state.clone()),
        projection,
        agent_projection,
        session.id().into(),
        rx,
    ));
    let error = timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        error.to_string().contains("grant revoked or expired"),
        "{error}"
    );
    assert_eq!(
        state
            .session_snapshot(session.id())
            .await
            .unwrap()
            .workflows()
            .len(),
        before
    );

    let terminal =
        KernelCommand::from_local_request("terminal-queued-workflow", None, None, &request);
    runtime
        .dispatch_workflow_command(terminal, request)
        .await
        .unwrap();
    assert_eq!(
        state
            .session_snapshot(session.id())
            .await
            .unwrap()
            .workflows()
            .len(),
        before + 1
    );
    consumer.abort();
    let _ = consumer.await;
}
