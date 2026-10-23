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

// MP-08/MP-09/MP-10/MP-11: lifecycle details share the Room's trace visibility boundary.
#[test]
fn lifecycle_projection_obeys_private_and_transparent_collaboration() {
    use crate::agent::GridPosition;
    use serde_json::json;
    let mut session = RuntimeSession::new("room", None, "workspace", "tree", "machine", "kernel");
    for (id, owner) in [("own", "member"), ("peer", "other")] {
        let mut agent = AgentInstance::new(
            id,
            id,
            "room",
            None,
            "codex",
            None,
            None,
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        agent.set_owner_user_id(owner);
        session.agents.push(agent);
    }
    for agent in ["own", "peer", "deleted-peer"] {
        session.agent_tasks.push(
            serde_json::from_value(json!({
                "task_id": agent, "room_id": "room", "agent_id": agent, "prompt_id": agent,
                "revision": 1, "state": "waiting", "reason": "private source review",
                "obligations": [], "last_progress_at_ms": 0, "progress_sequence": 0,
                "no_progress_wakes": 0, "correction_used": false
            }))
            .unwrap(),
        );
        session.agent_wakes.push(
            serde_json::from_value(json!({
                "id": agent, "task_id": agent, "room_id": "room", "agent_id": agent,
                "registration_id": agent, "kind": "process", "label": "private source",
                "state": "running", "created_at_ms": 0, "command": ["private-path"],
                "fire_count": 0, "missed_fires": 0
            }))
            .unwrap(),
        );
    }
    for (level, expected) in [
        (CollaborationLevel::Private, vec!["own"]),
        (CollaborationLevel::Transparent, vec!["own", "peer"]),
        (CollaborationLevel::Full, vec!["own", "peer"]),
    ] {
        let mut current = session.clone();
        current.add_member("member", None, level);
        let projected = current.redacted_for_user("member");
        assert_eq!(
            projected
                .agent_tasks
                .iter()
                .map(|t| t.agent_id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            projected
                .agent_wakes
                .iter()
                .map(|w| w.agent_id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
}

// MP-08/MP-10: a goal completion check stays stoppable by id and status in a
// user projection, but its instruction is never part of it; a queued one is
// omitted entirely.
#[test]
fn user_projection_hides_goal_completion_check_instruction() {
    use crate::agent::{AgentInstance, GridPosition};
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
    session.activate_prompt(super::PromptQueueItem::new(
        "goal-check-task-1-1-1",
        "attachment",
        "agent",
        "This turn has origin completion_check.",
        crate::session::PromptStatus::Queued,
    ));
    session.submit_prompt(super::PromptQueueItem::new(
        "goal-check-task-1-1-2",
        "attachment",
        "agent",
        "This turn has origin completion_check.",
        crate::session::PromptStatus::Queued,
    ));

    let owner = session.redacted_for_user(crate::session::DEFAULT_LOCAL_USER_ID);
    let active = owner
        .active_prompt_for_agent("agent")
        .expect("a running check stays stoppable by its prompt id");
    assert_eq!(active.id(), "goal-check-task-1-1-1");
    assert_eq!(active.status(), crate::session::PromptStatus::Running);
    assert_eq!(active.prompt(), "");
    assert!(owner
        .queued_prompts_for_agent("agent")
        .is_none_or(|q| q.is_empty()));
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
            "command": [], "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
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

#[test]
fn user_projection_keeps_internal_active_prompt_identity_without_instruction_text() {
    for prompt_id in ["agent-event-agent-3", "goal-check-task-1-1-1"] {
        assert_internal_active_prompt_is_stoppable_without_text(prompt_id);
    }
}

fn assert_internal_active_prompt_is_stoppable_without_text(prompt_id: &str) {
    use crate::agent::{AgentInstance, GridPosition};
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
    session.activate_prompt(super::PromptQueueItem::new(
        prompt_id,
        "attachment",
        "agent",
        "Kernel event inbox, untrusted data",
        crate::session::PromptStatus::Queued,
    ));

    let owner = session.redacted_for_user(crate::session::DEFAULT_LOCAL_USER_ID);
    let active = owner
        .active_prompt_for_agent("agent")
        .expect("a running wake turn stays stoppable by its prompt id");
    assert_eq!(active.id(), prompt_id);
    assert_eq!(active.status(), crate::session::PromptStatus::Running);
    assert_eq!(active.prompt(), "");
}
