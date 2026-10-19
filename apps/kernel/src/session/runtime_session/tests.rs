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

#[test]
fn agent_tasks_and_wakes_follow_agent_trace_visibility() {
    use crate::agent::{AgentInstance, GridPosition};
    let task = |agent: &str| -> crate::durable_state::agent_lifecycle::AgentTaskExecution {
        serde_json::from_value(serde_json::json!({
            "task_id": "task", "room_id": "room", "agent_id": agent, "prompt_id": "task",
            "provider_run_id": null, "revision": 1, "state": "blocked", "reason": "private",
            "obligations": [], "wait": null, "last_progress_at_ms": 1, "progress_sequence": 0,
            "no_progress_wakes": 0, "correction_used": false, "pending_prompt_id": null,
        }))
        .unwrap()
    };
    let wake = |agent: &str| -> crate::durable_state::agent_lifecycle::AgentWake {
        serde_json::from_value(serde_json::json!({
            "id": "wake", "task_id": "task", "room_id": "room", "agent_id": agent,
            "registration_id": "reg", "kind": "timer", "label": "tick", "state": "scheduled",
            "created_at_ms": 1, "verified_at_ms": 1, "next_due_ms": 9, "interval_ms": null,
            "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null, "last_sequence": null,
            "last_delivery": null, "last_delivered_at_ms": null, "last_acknowledged_at_ms": null,
            "alerted_sequence": null,
        }))
        .unwrap()
    };
    let mut session =
        RuntimeSession::new("room", None, "workspace", "worktree", "machine", "daemon");
    session.set_agents(vec![AgentInstance::new(
        "agent",
        "agent",
        "room",
        None,
        "codex",
        None,
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    )]);
    session.set_agent_tasks(vec![task("agent")]);
    session.set_agent_wakes(vec![wake("agent")]);

    let owner = session
        .clone()
        .redacted_for_user(crate::session::DEFAULT_LOCAL_USER_ID);
    assert_eq!(
        (owner.agent_tasks().len(), owner.agent_wakes().len()),
        (1, 1)
    );
    let stranger = session.redacted_for_user("stranger");
    assert!(stranger.agent_tasks().is_empty() && stranger.agent_wakes().is_empty());
}
