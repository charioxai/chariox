//! Row 690: the supervision sweep's warning "moved on from a task that did not settle in time" is for
//! a task that was asked and did not answer, not for every task the pass ran out of patience before
//! reaching; the sweep set of settled history does not make passes slower.
use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};

struct Crowd {
    runtime: KernelRuntimeState,
    room: String,
    hung: Vec<String>,
    ordinary: Vec<String>,
    fault: std::path::PathBuf,
    logs: crate::logging::capture::LogCapture,
    _worktree: crate::test_support::TestWorktree,
}

impl Drop for Crowd {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.fault);
    }
}

/// A Room of agents, each with a working turn. The `hung` agents come first in the ledger and their
/// settlement never ends while the fault file names them; the `ordinary` ones settle at once.
fn crowd(hung: usize, ordinary: usize) -> Crowd {
    let logs = crate::logging::capture::start();
    let worktree = crate::test_support::TestWorktree::new("sweep-patience");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (room, _parent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let mut spawn = || {
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(crate::agent::CreateAgentRequest::new(room.id(), "codex"))
            .unwrap()
            .id()
            .to_string()
    };
    let hung_agents: Vec<String> = (0..hung).map(|_| spawn()).collect();
    let ordinary_agents: Vec<String> = (0..ordinary).map(|_| spawn()).collect();
    let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    )
    .runtime_state();
    for agent in hung_agents.iter().chain(&ordinary_agents) {
        runtime
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Begin {
                owner: room.owner_user_id().into(),
                room: room.id().into(),
                agent: agent.clone(),
                prompt: format!("turn-{agent}"),
                run: None,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
    }
    let fault = std::env::temp_dir().join(format!("sweep-patience-fault-{}", room.id()));
    std::fs::write(&fault, hung_agents.join("\n")).unwrap();
    runtime.owned.agent_wakes.set_task_stall_fault(fault.clone());
    Crowd {
        runtime,
        room: room.id().into(),
        hung: hung_agents,
        ordinary: ordinary_agents,
        fault,
        logs,
        _worktree: worktree,
    }
}

impl Crowd {
    /// How many times the sweep said an agent's task did not settle in time.
    fn did_not_settle(&self, agents: &[String]) -> usize {
        self.logs
            .records()
            .iter()
            .filter(|record| record.contains("did not settle in time"))
            .filter(|record| agents.iter().any(|agent| record.contains(agent.as_str())))
            .count()
    }

    fn still_working(&self, agents: &[String]) -> usize {
        agents
            .iter()
            .filter(|agent| {
                self.runtime
                    .owned
                    .durable_state_store
                    .agent_tasks(Some(&self.room), Some(agent))
                    .unwrap()
                    .iter()
                    .any(|task| task.state == ExecutionState::Working)
            })
            .count()
    }

    fn release_the_hung(&self) {
        std::fs::remove_file(&self.fault).unwrap();
    }
}

/// How many hung settlements the pass's patience pays for: each waits `TASK_WAIT`.
fn waited_for() -> usize {
    (task_work_flight::SWEEP_PATIENCE.as_secs() / task_work_flight::TASK_WAIT.as_secs()) as usize
}

/// A task that did not answer warns, alone: the other agents' settled tasks do not.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn one_task_that_does_not_settle_is_the_only_one_the_sweep_warns_about() {
    let room = crowd(1, 31);
    room.runtime.sweep_agent_lifecycle().await.unwrap();
    assert_eq!(room.did_not_settle(&room.hung), 1);
    assert_eq!(room.did_not_settle(&room.ordinary), 0);
}

/// Cell 3: tasks hold the pass's whole patience; the tasks it never got to are not "unsettled".
/// The pass stops, says how far it got, warns only for the tasks that waited, and the next pass
/// reaches the rest.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_pass_that_runs_out_of_patience_warns_only_for_the_tasks_that_waited() {
    let hung = waited_for() + 2;
    let room = crowd(hung, 31);
    let total = hung + 31;
    room.runtime.sweep_agent_lifecycle().await.unwrap();
    assert_eq!(
        room.did_not_settle(&room.hung),
        waited_for(),
        "only the tasks that waited their turn are reported"
    );
    assert_eq!(
        room.did_not_settle(&room.ordinary),
        0,
        "tasks the pass did not reach are not tasks that did not settle"
    );
    let summary = room
        .logs
        .records()
        .into_iter()
        .find(|record| {
            record.contains("supervision sweep ran out of patience")
                && record.contains(&format!("\"of\":{total}"))
        })
        .expect("the pass says how far it got");
    assert!(summary.contains(&format!("\"swept\":{}", waited_for())), "{summary}");

    // The hung settlements end; the next passes reach every task the first one did not.
    room.release_the_hung();
    for _ in 0..3 {
        room.runtime.task_work_settled().await;
        room.runtime.sweep_agent_lifecycle().await.unwrap();
    }
    assert_eq!(
        room.still_working(&room.ordinary),
        0,
        "every task the first pass did not reach was reached by a later one"
    );
}

/// Cell 4: a kernel with no dev App does not read the shared durable connection for dev
/// installations on every maintenance pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_kernel_with_no_dev_app_reads_no_dev_installations_after_the_first_look() {
    let room = crowd(0, 1);
    let store = room.runtime.owned.durable_state_store.clone();
    for _ in 0..3 {
        room.runtime.sync_dev_app_bindings().await.unwrap();
        // The scan is due once a second.
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    }
    assert!(
        store.dev_installation_reads_for_test() <= 1,
        "{} reads for 3 passes with no dev installation and no App write",
        store.dev_installation_reads_for_test()
    );
}
