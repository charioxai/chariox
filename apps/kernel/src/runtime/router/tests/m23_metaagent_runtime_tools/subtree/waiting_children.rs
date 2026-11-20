// An agent whose turn yielded on a wake is not idle: `agent children` and the session agent
// inspection name what it waits on and since when, from the same activity the Room reads.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentWake, Operation};

/// `agent` finishes a turn that yielded on a timer armed at `armed_at_ms`.
async fn yield_on_a_timer(world: &World, alias: &str, armed_at_ms: u64) {
    yield_on_a_labelled_timer(world, alias, armed_at_ms, "ten minute timer").await;
    let waiting = world
        .app
        .lock()
        .await
        .durable_state_store()
        .agent_waiting_tasks(&world.room)
        .unwrap();
    assert_eq!(waiting.len(), 1, "the settled turn waits: {waiting:?}");
}

/// The same, for a timer the agent named `label`; ids follow the alias, so many agents can wait.
pub(super) async fn yield_on_a_labelled_timer(
    world: &World,
    alias: &str,
    armed_at_ms: u64,
    label: &str,
) {
    let agent = world.ids[alias].as_str();
    let store = world.app.lock().await.durable_state_store();
    let turn = format!("{alias}-turn");
    let wake_id = format!("{alias}-wake");
    let registration = format!("completion-{wake_id}");
    let (prompt, task) = (turn.as_str(), turn.as_str());
    store
        .agent_lifecycle(Operation::Begin {
            owner: "user-local".into(),
            room: world.room.clone(),
            agent: agent.into(),
            prompt: prompt.into(),
            run: Some("run".into()),
            now: armed_at_ms,
        })
        .unwrap();
    let wake: AgentWake = serde_json::from_value(serde_json::json!({
        "id": wake_id, "task_id": task, "room_id": world.room, "agent_id": agent,
        "registration_id": registration, "kind": "timer", "label": label,
        "state": "scheduled", "created_at_ms": armed_at_ms, "verified_at_ms": null,
        "next_due_ms": armed_at_ms + 600_000, "interval_ms": null, "command": [],
        "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
        "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null, "last_sequence": null,
        "last_delivery": null, "last_delivered_at_ms": null, "last_acknowledged_at_ms": null,
        "alerted_sequence": null
    }))
    .unwrap();
    store
        .agent_lifecycle(Operation::CreateWake {
            task: task.into(),
            prompt: prompt.into(),
            wake,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Yield {
            task: task.into(),
            prompt: prompt.into(),
            registrations: vec![registration.clone()],
            cursor: 0,
            deadline: Some(armed_at_ms + 900_000),
            reason: "wait for the timer".into(),
            now: armed_at_ms,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: world.room.clone(),
            agent: agent.into(),
            prompt: prompt.into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now: armed_at_ms,
        })
        .unwrap();
}

#[test]
fn an_agent_yielded_on_a_wake_shows_waiting_not_idle() {
    run_large_stack_async_test("subtree-waiting-children", waiting_children);
}

async fn waiting_children() {
    let world = world("subtree-waiting-children").await;
    let armed_at_ms = crate::session::unix_epoch_ms() - 120_000;
    yield_on_a_timer(&world, "s", armed_at_ms).await;

    let children = room_command(&world, "a", "agent children").await;
    assert!(children.ok, "{children:?}");
    let listed = children.payload["response"]["children"].as_array().unwrap();
    let row = |alias: &str| {
        listed
            .iter()
            .find(|child| child["alias"] == alias)
            .unwrap_or_else(|| panic!("{alias} is listed"))
    };
    assert_eq!(row("s")["state"], "waiting", "{:?}", row("s"));
    let wait = row("s")["wait"].as_str().unwrap();
    assert!(
        wait.starts_with("timer") && wait.contains("ten minute timer"),
        "{wait}"
    );
    assert_ne!(
        row("b")["state"],
        "waiting",
        "a sibling that yielded on nothing"
    );

    let inspected = world
        .router
        .dispatch_authenticated_runtime_tool_call(
            &world.tokens["a"],
            crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL,
            serde_json::json!({ "agent": world.ids["s"] }),
        )
        .await
        .unwrap();
    assert!(inspected.ok, "{inspected:?}");
    assert_eq!(inspected.payload["agent"]["state"], "waiting");
    let detail = &inspected.payload["agent"]["liveness"]["waiting_on"];
    assert_eq!(detail["kind"], waiting["kind"]);
    assert_eq!(detail["since_ms"], waiting["since_ms"]);
}
