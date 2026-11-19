//! A lifecycle sweep over a Room with many agents keeps its cadence.
use super::*;
use crate::durable_state::agent_lifecycle::{InboxEvent, Operation, Outcome};

struct Room {
    runtime: KernelRuntimeState,
    agents: Vec<String>,
    _worktree: crate::test_support::TestWorktree,
}

fn room_with_agents(count: usize) -> Room {
    let worktree = crate::test_support::TestWorktree::new("sweep-load");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (room, parent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let agents: Vec<String> = (0..count)
        .map(|_| {
            crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(crate::agent::CreateAgentRequest::new(room.id(), "codex"))
                .unwrap()
                .id()
                .to_string()
        })
        .collect();
    let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    )
    .runtime_state();
    let store = &runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Begin {
            owner: room.owner_user_id().into(),
            room: room.id().into(),
            agent: parent.id().into(),
            prompt: "parent-turn".into(),
            run: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    for agent in &agents {
        let Outcome::Event(_) = store
            .agent_lifecycle(Operation::Send {
                task: "parent-turn".into(),
                prompt: "parent-turn".into(),
                event: InboxEvent {
                    sequence: 0,
                    room_id: room.id().into(),
                    agent_id: agent.clone(),
                    source_id: parent.id().into(),
                    occurrence_id: format!("work-{agent}"),
                    kind: "message".into(),
                    payload: serde_json::json!({"message":"start"}),
                    urgent: true,
                    reply_requested: false,
                    state: "pending".into(),
                    prompt_id: None,
                    target_prompt_id: None,
                    provider_run_id: None,
                    arrived_at_ms: None,
                    attempted_at_ms: None,
                    submit_epoch: None,
                    resumed_at_ms: None,
                },
            })
            .unwrap()
        else {
            panic!("send returns the event")
        };
    }
    Room {
        runtime,
        agents,
        _worktree: worktree,
    }
}

impl Room {
    /// Full Room snapshots one lifecycle pass builds before it returns.
    async fn snapshots_in_one_pass(&self) -> usize {
        super::session::call_counts::SNAPSHOT_BUILDS.with(|count| count.set(0));
        self.runtime.sweep_agent_lifecycle().await.unwrap();
        super::session::call_counts::SNAPSHOT_BUILDS.with(|count| count.get())
    }
}

/// Each delivery projects the whole Room, which on a busy kernel takes about a second: a pass that
/// delivered every recipient's inbox itself ran past the dead-man tolerance with a few dozen agents
/// and reported the supervision sweep stalled. The pass's own work does not grow with recipients.
#[tokio::test]
async fn a_sweep_over_many_waiting_recipients_keeps_its_cost_and_still_delivers_each() {
    let one = room_with_agents(1);
    let many = room_with_agents(8);
    let alone = one.snapshots_in_one_pass().await;
    let crowded = many.snapshots_in_one_pass().await;
    assert_eq!(
        crowded, alone,
        "a pass over 8 waiting recipients projects the Room as often as a pass over one"
    );

    // Each recipient's delivery runs in its own work and projects the Room it changed.
    super::session::call_counts::SNAPSHOT_BUILDS.with(|count| count.set(0));
    many.runtime.task_work_settled().await;
    let delivered = super::session::call_counts::SNAPSHOT_BUILDS.with(|count| count.get());
    assert!(
        delivered >= many.agents.len(),
        "every recipient's delivery still runs: {delivered} projections for {} recipients",
        many.agents.len()
    );
}
