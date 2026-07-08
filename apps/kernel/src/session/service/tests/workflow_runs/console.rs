use super::*;

#[test]
fn workflow_console_supports_append_read_and_clear() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");

    let initial = service
        .read_workflow_console(session.id(), workflow.id())
        .expect("console should read");
    assert_eq!(initial.workflow_id(), workflow.id());
    assert!(initial.entries().is_empty());

    let first = service
        .append_workflow_console_entry(
            session.id(),
            workflow.id(),
            Some("node-run-1".to_string()),
            Some("agent-1".to_string()),
            "hello\n",
        )
        .expect("console append should succeed");
    assert_eq!(first.text(), "hello\n");

    let second = service
        .append_workflow_console_entry(
            session.id(),
            workflow.id(),
            Some("node-run-2".to_string()),
            Some("agent-2".to_string()),
            "world\n",
        )
        .expect("console append should succeed");
    assert_eq!(second.text(), "world\n");

    let populated = service
        .read_workflow_console(session.id(), workflow.id())
        .expect("console should read");
    assert_eq!(populated.entries().len(), 2);
    assert_eq!(populated.entries()[0].text(), "hello\n");
    assert_eq!(populated.entries()[1].text(), "world\n");

    let cleared = service
        .clear_workflow_console(session.id(), workflow.id())
        .expect("console clear should succeed");
    assert!(cleared.entries().is_empty());
}
