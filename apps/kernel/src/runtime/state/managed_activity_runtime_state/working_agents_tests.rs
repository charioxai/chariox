//! MP-08 / MP-09 / MP-10 / MP-11: a managed machine stays up for agents that
//! run a turn and for idle agents something they registered can still wake.
use std::sync::Arc;
use tokio::sync::Mutex;

use super::KernelRuntimeState;
use crate::config::DaemonConfig;
use crate::durable_state::agent_lifecycle::{AgentWake, Operation, Registration};
use crate::runtime::router::CommandRouter;
use crate::session::{AgentTrigger, AgentTriggerBusyPolicy, AgentTriggerRepeat, RuntimeSession};
use crate::DaemonApp;

const ROOM: &str = "session-1";

async fn runtime() -> KernelRuntimeState {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    CommandRouter::with_interactive_capacity(app, 1).runtime_state()
}

fn restore_room(runtime: &KernelRuntimeState, room: &str) {
    runtime
        .owned
        .session_store
        .restore_session(RuntimeSession::new(
            room,
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        ));
}

/// An agent's turn in the ledger. The runtime has no turn running for it.
fn begin_task(runtime: &KernelRuntimeState, room: &str, agent: &str) -> String {
    let prompt = format!("prompt-{agent}");
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: "owner".into(),
            room: room.into(),
            agent: agent.into(),
            prompt: prompt.clone(),
            run: Some(format!("run-{agent}")),
            now: 1,
        })
        .expect("task should begin");
    prompt
}

fn arm_timer(runtime: &KernelRuntimeState, room: &str, agent: &str, task: &str, id: &str) {
    let wake = AgentWake {
        id: id.into(),
        task_id: task.into(),
        room_id: room.into(),
        agent_id: agent.into(),
        registration_id: format!("completion-{id}"),
        kind: "timer".into(),
        label: "check back".into(),
        state: String::new(),
        created_at_ms: 10,
        verified_at_ms: None,
        next_due_ms: Some(crate::session::unix_epoch_ms() + 3_600_000),
        interval_ms: None,
        command: vec![],
        match_text: None,
        matched_at_ms: None,
        pid: None,
        exit_code: None,
        summary: String::new(),
        detail: serde_json::Value::Null,
        fire_count: 0,
        missed_fires: 0,
        last_fired_at_ms: None,
        last_sequence: None,
        last_delivery: None,
        last_delivered_at_ms: None,
        last_acknowledged_at_ms: None,
        alerted_sequence: None,
    };
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::CreateWake {
            task: task.into(),
            prompt: task.into(),
            wake,
        })
        .expect("timer should arm");
}

fn cancel_timer(runtime: &KernelRuntimeState, task: &str, id: &str) {
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::CancelWake {
            id: id.into(),
            task: task.into(),
            prompt: Some(task.into()),
            pending: false,
        })
        .expect("timer should cancel");
}

fn subscribe(runtime: &KernelRuntimeState, task: &str, id: &str) {
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Subscribe {
            task: task.into(),
            prompt: task.into(),
            registration: Registration {
                id: id.into(),
                task_id: task.into(),
                source_id: "child".into(),
                obligation_id: None,
                source_cursor: 0,
                live: true,
                peer: false,
            },
        })
        .expect("subscription should register");
}

fn unsubscribe(runtime: &KernelRuntimeState, task: &str, id: &str) {
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Unsubscribe {
            task: task.into(),
            prompt: task.into(),
            registration: id.into(),
        })
        .expect("subscription should end");
}

fn start_turn(runtime: &KernelRuntimeState, agent: &str, provider_run: &str) {
    runtime.start_active_turn_with_trace_id(
        ROOM,
        agent,
        &format!("prompt-{agent}"),
        provider_run,
        "trace-1",
    );
}

#[tokio::test]
async fn managed_working_agents_idle_kernel_has_none() {
    let runtime = runtime().await;
    restore_room(&runtime, ROOM);
    begin_task(&runtime, ROOM, "agent-1");
    assert_eq!(runtime.managed_working_agent_count(), 0);
    assert_eq!(runtime.managed_working_agent_count(), 0);
}

#[tokio::test]
async fn managed_working_agents_counts_a_running_turn() {
    let runtime = runtime().await;
    start_turn(&runtime, "agent-1", "provider-run-1");
    assert_eq!(runtime.managed_working_agent_count(), 1);
    assert_eq!(runtime.managed_working_agent_count(), 1);
    runtime.clear_prompt_activity_for_managed_activity_test("provider-run-1");
    assert_eq!(runtime.managed_working_agent_count(), 0);
}

#[tokio::test]
async fn managed_working_agents_counts_an_idle_agent_while_a_timer_is_armed() {
    let runtime = runtime().await;
    restore_room(&runtime, ROOM);
    let task = begin_task(&runtime, ROOM, "agent-1");
    arm_timer(&runtime, ROOM, "agent-1", &task, "timer-1");
    assert_eq!(runtime.managed_working_agent_count(), 1);
    cancel_timer(&runtime, &task, "timer-1");
    assert_eq!(runtime.managed_working_agent_count(), 0);
}

#[tokio::test]
async fn managed_working_agents_counts_an_idle_agent_while_a_subscription_is_live() {
    let runtime = runtime().await;
    restore_room(&runtime, ROOM);
    let task = begin_task(&runtime, ROOM, "agent-1");
    subscribe(&runtime, &task, "reg-1");
    assert_eq!(runtime.managed_working_agent_count(), 1);
    unsubscribe(&runtime, &task, "reg-1");
    assert_eq!(runtime.managed_working_agent_count(), 0);
}

#[tokio::test]
async fn managed_working_agents_counts_an_idle_agent_with_an_enabled_trigger() {
    let runtime = runtime().await;
    restore_room(&runtime, ROOM);
    let mut room = runtime.owned.session_store.get_session(ROOM).unwrap();
    room.add_agent_trigger(
        AgentTrigger::schedule(
            "trigger-1",
            "agent-1",
            "owner",
            crate::session::AgentSchedule::interval(3_600, AgentTriggerRepeat::Every),
            AgentTriggerBusyPolicy::Queue,
            "check in",
            1,
        )
        .expect("schedule should be valid"),
    );
    runtime.owned.session_store.restore_session(room);
    assert_eq!(runtime.managed_working_agent_count(), 1);

    let mut room = runtime.owned.session_store.get_session(ROOM).unwrap();
    room.remove_agent_trigger("trigger-1")
        .expect("trigger should exist");
    runtime.owned.session_store.restore_session(room);
    assert_eq!(runtime.managed_working_agent_count(), 0);
}

#[tokio::test]
async fn managed_working_agents_counts_each_agent_once() {
    let runtime = runtime().await;
    restore_room(&runtime, ROOM);
    // Agent 1 runs a turn and has a timer; agent 2 only waits; agent 3 only runs.
    let first = begin_task(&runtime, ROOM, "agent-1");
    arm_timer(&runtime, ROOM, "agent-1", &first, "timer-1");
    start_turn(&runtime, "agent-1", "provider-run-1");
    let second = begin_task(&runtime, ROOM, "agent-2");
    subscribe(&runtime, &second, "reg-2");
    start_turn(&runtime, "agent-3", "provider-run-3");
    assert_eq!(runtime.managed_working_agent_count(), 3);
}

#[tokio::test]
async fn managed_working_agents_ignores_registrations_of_rooms_that_are_gone() {
    let runtime = runtime().await;
    let task = begin_task(&runtime, "gone-room", "agent-1");
    arm_timer(&runtime, "gone-room", "agent-1", &task, "timer-1");
    assert_eq!(runtime.managed_working_agent_count(), 0);
}

#[tokio::test]
async fn managed_working_agents_unreadable_ledger_counts_as_working() {
    let runtime = runtime().await;
    restore_room(&runtime, ROOM);
    rusqlite::Connection::open(runtime.owned.durable_state_store.path())
        .expect("durable database should open")
        .execute_batch("ALTER TABLE agent_wakes RENAME TO agent_wakes_unreadable;")
        .expect("ledger table should be unreadable");
    assert_eq!(runtime.managed_working_agent_count(), 1);
}

#[tokio::test]
async fn managed_working_agents_registration_while_idle_is_its_own_transition() {
    let runtime = runtime().await;
    runtime
        .ensure_managed_activity_tracking("kernel-working")
        .expect("managed activity tracking should activate");
    restore_room(&runtime, ROOM);
    let (_, idle, idle_transition) = runtime
        .managed_activity_report_snapshot_with_transition()
        .expect("initial activity should be durable");
    assert_eq!(idle.working_agent_count, 0);

    let task = begin_task(&runtime, ROOM, "agent-1");
    std::thread::sleep(std::time::Duration::from_millis(3));
    arm_timer(&runtime, ROOM, "agent-1", &task, "timer-1");
    let (_, armed, armed_transition) = runtime
        .managed_activity_report_snapshot_with_transition()
        .expect("registered wake should be reported");
    assert_eq!(armed.working_agent_count, 1);
    assert!(armed.changed_at_ms > idle.changed_at_ms);
    assert!(armed_transition > idle_transition);
    let (_, again, again_transition) = runtime
        .managed_activity_report_snapshot_with_transition()
        .expect("unchanged activity should stay readable");
    assert_eq!(again, armed);
    assert_eq!(again_transition, armed_transition);

    std::thread::sleep(std::time::Duration::from_millis(3));
    cancel_timer(&runtime, &task, "timer-1");
    let (_, cancelled, cancelled_transition) = runtime
        .managed_activity_report_snapshot_with_transition()
        .expect("cancelled wake should be reported");
    assert_eq!(cancelled.working_agent_count, 0);
    assert!(cancelled.changed_at_ms > armed.changed_at_ms);
    assert!(cancelled_transition > armed_transition);
}

#[tokio::test]
async fn managed_working_agents_wait_wakes_for_a_registration_made_while_idle() {
    let runtime = runtime().await;
    runtime
        .ensure_managed_activity_tracking("kernel-working-wait")
        .expect("managed activity tracking should activate");
    restore_room(&runtime, ROOM);
    let (sequence, idle, _) = runtime
        .managed_activity_report_snapshot_with_transition()
        .expect("initial activity should be durable");
    let task = begin_task(&runtime, ROOM, "agent-1");
    arm_timer(&runtime, ROOM, "agent-1", &task, "timer-1");

    let (_, armed, _) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        runtime.wait_for_managed_activity_transition_after(sequence, idle),
    )
    .await
    .expect("a registration made while idle should wake the reporter")
    .expect("activity should remain readable");
    assert_eq!(armed.working_agent_count, 1);
}
