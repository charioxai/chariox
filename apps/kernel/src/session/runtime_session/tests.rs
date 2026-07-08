use super::*;

#[test]
fn terminal_metaagent_task_restart_clears_stale_plan() {
    let mut session = RuntimeSession::new(
        "session-1",
        None,
        "workspace-1",
        "worktree-1",
        "machine-1",
        "daemon-1",
    );

    let first = session.start_or_update_metaagent_task("agent-1", "Fix todo test");
    let first_task_id = first.task_id().to_string();
    session.update_metaagent_plan_markdown("agent-1", "- Delegate todo fix");
    session.complete_metaagent_task("agent-1", Some("done".to_string()));

    let second = session.start_or_update_metaagent_task("agent-1", "Fix stats test");

    assert_ne!(second.task_id(), first_task_id);
    assert_eq!(second.task_markdown(), "Fix stats test");
    assert_eq!(second.plan_markdown(), "");
    assert_eq!(second.status(), MetaagentTaskStatus::Active);
    assert_eq!(second.completion_summary(), None);
}
