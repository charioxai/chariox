use super::*;

#[test]
fn delivery_bookkeeping_is_no_blocked_stripe() {
    assert!(is_delivery_bookkeeping("Unconfirmed delivery 3"));
    assert!(is_delivery_bookkeeping("Delivery 2 refusal clock runs"));
    assert!(!is_delivery_bookkeeping("Waiting for the review"));
}

fn waiting(reason: WaitReason, position: u32) -> Waiting {
    Waiting {
        reason,
        position,
        need_bytes: 3 << 30,
        free_bytes: 1 << 30,
        command_line: "cargo build".into(),
        cwd: None,
        since_ms: 0,
    }
}

#[test]
fn a_held_command_says_what_it_waits_for_and_its_place_in_line() {
    assert_eq!(
        command_wait(&waiting(WaitReason::Memory, 1), true),
        "waiting for memory, needs 3 GB"
    );
    assert_eq!(
        command_wait(&waiting(WaitReason::Disk, 2), true),
        "waiting for disk space, 2nd in line"
    );
    assert_eq!(
        command_wait(&waiting(WaitReason::Memory, 12), true),
        "waiting for memory, 12th in line"
    );
    assert_eq!(
        command_wait(&waiting(WaitReason::Memory, 1), false),
        "paused at the memory limit"
    );
}

fn room_with_a_waiting_task(wake: Option<serde_json::Value>) -> (RuntimeSession, AgentInstance) {
    let agent = AgentInstance::new(
        "agent",
        "agent",
        "room",
        None,
        "codex",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    let mut session = RuntimeSession::new(
        "room",
        crate::session::ProjectId::test(),
        None,
        "workspace",
        "worktree",
        "machine",
        "daemon",
    );
    session.set_agents(vec![agent.clone()]);
    session.set_agent_tasks(vec![serde_json::from_value(serde_json::json!({
        "task_id": "task", "room_id": "room", "agent_id": "agent", "prompt_id": "task",
        "provider_run_id": null, "revision": 1, "state": "waiting", "reason": "wait for the timer",
        "obligations": [],
        "wait": {"registration_ids": ["reg"], "inbox_cursor": 0, "deadline_ms": 5_000, "started_at_ms": 1},
        "last_progress_at_ms": 1, "correction_used": false, "pending_prompt_id": null,
    }))
    .unwrap()]);
    if let Some(wake) = wake {
        session.set_agent_wakes(vec![serde_json::from_value(wake).unwrap()]);
    }
    (session, agent)
}

fn timer(state: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "wake", "task_id": "task", "room_id": "room", "agent_id": "agent",
        "registration_id": "reg", "kind": "timer", "label": "check the build", "state": state,
        "created_at_ms": 1, "verified_at_ms": 1, "next_due_ms": 3_000, "interval_ms": null,
        "command": [], "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
        "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null, "last_sequence": null,
        "last_delivery": null, "last_delivered_at_ms": null, "last_acknowledged_at_ms": null,
        "alerted_sequence": null,
    })
}

/// A task that arms a timer and yields to it is Waiting while the timer stays scheduled: the pane
/// keeps the armed wait's reason and countdown for the whole wait, from the wake.
#[test]
fn a_waiting_task_keeps_its_armed_timer_on_the_pane() {
    let (session, agent) = room_with_a_waiting_task(Some(timer("scheduled")));
    let shown = stripes(&session, &agent, &[]);
    let waits = shown
        .iter()
        .filter(|stripe| stripe.id.starts_with("wake:") || stripe.id.starts_with("wait:"))
        .collect::<Vec<_>>();
    assert_eq!(waits.len(), 1, "{shown:?}");
    assert_eq!(
        (
            waits[0].kind.as_str(),
            waits[0].text.as_str(),
            waits[0].at_ms
        ),
        ("/wait-in", "check the build", Some(3_000))
    );
}

/// A task waiting on a source that has no wake of its own (a child, a job) shows what it waits for.
#[test]
fn a_waiting_task_with_no_armed_wake_shows_its_reason_and_deadline() {
    let (session, agent) = room_with_a_waiting_task(None);
    let shown = stripes(&session, &agent, &[]);
    let waits = shown
        .iter()
        .filter(|stripe| stripe.id.starts_with("wait:"))
        .collect::<Vec<_>>();
    assert_eq!(waits.len(), 1, "{shown:?}");
    assert_eq!(
        (
            waits[0].kind.as_str(),
            waits[0].text.as_str(),
            waits[0].at_ms
        ),
        ("Waiting", "wait for the timer", Some(5_000))
    );
    // A fired wake is no longer armed, so the task's own row stands in.
    let (session, agent) = room_with_a_waiting_task(Some(timer("fired")));
    let shown = stripes(&session, &agent, &[]);
    assert!(
        shown.iter().all(|stripe| !stripe.id.starts_with("wake:")),
        "{shown:?}"
    );
    assert_eq!(
        shown
            .iter()
            .filter(|stripe| stripe.id.starts_with("wait:"))
            .count(),
        1
    );
}
