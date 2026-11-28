//! A lifecycle sweep over a Room with many agents keeps its cadence.
use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, InboxEvent, Operation, Outcome};

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
    // The first Room's deliveries run apart from its pass: let them end before the next count.
    one.runtime.task_work_settled().await;
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

struct Backlog {
    runtime: KernelRuntimeState,
    healthy: String,
    fault: std::path::PathBuf,
    _temp: crate::test_support::TestTempDir,
    _worktree: crate::test_support::TestWorktree,
}

/// A Room whose agents each hold one Working turn the sweep can never settle — the restored
/// backlog of 20A7 (2466 old tasks of 612 agents) in miniature — plus one healthy agent whose
/// turn settles at once.
fn room_with_backlog(stalled: usize) -> Backlog {
    let temp = crate::test_support::TestTempDir::new("sweep-backlog");
    let worktree = crate::test_support::TestWorktree::new("sweep-backlog");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (room, _parent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let mut agents: Vec<String> = (0..stalled + 1)
        .map(|_| {
            crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(crate::agent::CreateAgentRequest::new(room.id(), "codex"))
                .unwrap()
                .id()
                .to_string()
        })
        .collect();
    let healthy = agents.pop().unwrap();
    let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    )
    .runtime_state();
    let store = &runtime.owned.durable_state_store;
    let fault = temp.path().join("task-stall");
    std::fs::write(&fault, agents.join("\n")).unwrap();
    runtime
        .owned
        .agent_wakes
        .set_task_stall_fault(fault.clone());
    for agent in agents.iter().chain([&healthy]) {
        let Outcome::Task(task) = store
            .agent_lifecycle(Operation::Begin {
                owner: room.owner_user_id().into(),
                room: room.id().into(),
                agent: agent.clone(),
                prompt: format!("turn-{agent}"),
                run: None,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap()
        else {
            panic!("begin returns the task");
        };
        assert_eq!(
            task.state,
            ExecutionState::Working,
            "the backlog is live work"
        );
    }
    Backlog {
        runtime,
        healthy,
        fault,
        _temp: temp,
        _worktree: worktree,
    }
}

fn did_not_settle_warnings(records: &[String]) -> Vec<&String> {
    records
        .iter()
        .filter(|record| record.contains("did not settle in time"))
        .collect()
}

fn warned_tasks<'a>(records: &[&'a String]) -> Vec<&'a str> {
    records
        .iter()
        .filter_map(|record| {
            record
                .split("\"task_id\":\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
        })
        .collect()
}

/// Row 689 (20A7 11-27: 43887 warns in 50 min over a restored backlog): one pass is O(new
/// work). Its warns are bounded by its own patience however much history the kernel restored,
/// a task that did not settle is asked again on a backoff instead of every pass, and a task
/// that settles is never warned.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pass_over_a_restored_backlog_warns_within_its_patience_and_settled_tasks_are_silent() {
    let backlog = room_with_backlog(40);
    let _capture = crate::logging::capture::start();

    backlog.runtime.sweep_agent_lifecycle().await.unwrap();
    let records = _capture.records();
    let pass_1 = did_not_settle_warnings(&records);
    // The pass's patience (20 s) waits for at most four settlements (TASK_WAIT 5 s each);
    // everything else is the next pass's work, not a warn per restored task.
    assert!(
        pass_1.len() <= 4,
        "one pass warns within its patience, not per restored task: {} warns:\n{}",
        pass_1.len(),
        records.join("\n")
    );
    // The healthy turn settled: no warn ever names it.
    assert!(
        !records
            .iter()
            .any(|record| record.contains(&backlog.healthy)),
        "a task that settles is never warned: {}",
        records.join("\n")
    );
    let asked_again = warned_tasks(&pass_1);

    // The tasks that did not settle are backed off: the next pass asks only work it never
    // asked, never the same task again.
    backlog.runtime.sweep_agent_lifecycle().await.unwrap();
    let later = _capture.records();
    let pass_2: Vec<_> = did_not_settle_warnings(&later)
        .into_iter()
        .filter(|record| !pass_1.contains(record))
        .collect();
    assert!(
        pass_2.len() <= 4,
        "the second pass also warns within its patience: {} warns",
        pass_2.len()
    );
    let rewarned = warned_tasks(&pass_2);
    assert!(
        asked_again.iter().all(|task| !rewarned.contains(task)),
        "a task that did not settle is not asked again next pass: {asked_again:?} vs {rewarned:?}"
    );

    // End the fault so the held settlements finish and the test leaves nothing behind.
    std::fs::remove_file(&backlog.fault).unwrap();
    backlog.runtime.task_work_settled().await;
}

/// The task where the newest walk meets its reserve was never offered yet.
/// Three slow newer tasks must not hide the only terminal reconciliation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_oldest_reserve_reaches_the_task_at_the_newest_walk_cutoff() {
    let backlog = room_with_backlog(3);
    let store = &backlog.runtime.owned.durable_state_store;
    let tasks = store.agent_tasks_to_sweep().unwrap();
    let healthy = tasks
        .iter()
        .find(|task| task.agent_id == backlog.healthy)
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: healthy.room_id.clone(),
            agent: healthy.agent_id.clone(),
            prompt: healthy.prompt_id.clone(),
            run: "finished-provider".into(),
            has_answer: true,
            cancelled: false,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    // Make the three held tasks strictly newer than the terminal task.
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    for task in tasks.iter().filter(|task| task.agent_id != backlog.healthy) {
        store
            .agent_lifecycle(Operation::Progress {
                task: task.task_id.clone(),
                prompt: task.prompt_id.clone(),
                receipt: format!("new-work-{}", task.task_id),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
    }
    assert!(!store.agent_task_terminal_recorded(healthy).unwrap());
    backlog.runtime.sweep_agent_lifecycle().await.unwrap();
    let recorded = store.agent_task_terminal_recorded(healthy).unwrap();
    // Always release the held workers before asserting, including on RED.
    std::fs::remove_file(&backlog.fault).unwrap();
    backlog.runtime.task_work_settled().await;
    assert!(
        recorded,
        "the oldest reserved wait skipped the task at the cutoff"
    );
}
