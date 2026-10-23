//! A managed kernel must not stop under a terminal that is running a command,
//! and must not start terminal commands once the stop is reserved.

use std::sync::Arc;

use crate::config::DaemonConfig;
use crate::local::{RoomTerminalOperation, RoomTerminalResult};
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
use crate::room_terminal::TerminalActor;
use crate::runtime::router::CommandRouter;
use crate::runtime::state::{KernelRuntimeState, ManagedKernelQuiescenceChallenge};
use crate::DaemonApp;
use base64::Engine as _;
use tokio::sync::Mutex;

struct Fixture {
    _worktree: crate::test_support::TestWorktree,
    _router: CommandRouter,
    runtime: KernelRuntimeState,
    session_id: String,
    challenge: ManagedKernelQuiescenceChallenge,
}

fn fixture(label: &str) -> Fixture {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let worktree = crate::test_support::TestWorktree::new(label);
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let kernel_id = app.config().daemon_id.clone();
    app.managed_kernel_registration = Some(ConfirmedManagedKernelRegistration {
        environment_id: "terminal-test-environment".to_string(),
        machine_id: "terminal-test-machine".to_string(),
        kernel_id: kernel_id.clone(),
        context_plan: None,
    });
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let runtime = router.runtime_state();
    runtime
        .ensure_managed_activity_tracking(&kernel_id)
        .expect("managed activity tracking should initialize");
    let challenge = ManagedKernelQuiescenceChallenge {
        challenge_id: format!("challenge-{label}"),
        account_id: "terminal-test-account".to_string(),
        environment_id: "terminal-test-environment".to_string(),
        machine_id: "terminal-test-machine".to_string(),
        kernel_id,
        desired_revision: 1,
        idle_sequence: 7,
        idle_deadline_at: "2026-09-26T00:00:00.000Z".to_string(),
        stop_operation_id: format!("stop-{label}"),
        nonce: format!("nonce-{label}"),
    };
    Fixture {
        session_id: session.id().to_string(),
        _worktree: worktree,
        _router: router,
        runtime,
        challenge,
    }
}

impl Fixture {
    async fn terminal(
        &self,
        operation: RoomTerminalOperation,
    ) -> Result<RoomTerminalResult, crate::error::DaemonError> {
        let directory = self
            .runtime
            .room_project_directory(&self.session_id)
            .unwrap();
        self.runtime
            .run_room_terminal_operation(
                &self.session_id,
                directory,
                TerminalActor::User,
                operation,
            )
            .await
    }

    async fn open(
        &self,
        command: Option<Vec<String>>,
    ) -> Result<String, crate::error::DaemonError> {
        match self
            .terminal(RoomTerminalOperation::Open {
                cols: 80,
                rows: 24,
                title: None,
                command,
            })
            .await?
        {
            RoomTerminalResult::Opened { terminal } => Ok(terminal.terminal_id),
            other => panic!("unexpected open result {other:?}"),
        }
    }

    /// Open a login shell and wait until its profile scripts are done: while
    /// they run, a child owns the foreground and the shell counts as work.
    async fn open_settled_shell(&self) -> String {
        let shell = self.open(None).await.expect("shell opens");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut idle_since = std::time::Instant::now();
        loop {
            if self.runtime.managed_running_agent_count() != 0 {
                idle_since = std::time::Instant::now();
            } else if idle_since.elapsed() >= std::time::Duration::from_millis(500) {
                return shell;
            }
            assert!(std::time::Instant::now() < deadline, "shell never settled");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    fn reserve(&self) -> bool {
        let (sequence, observation, local_sequence) = self
            .runtime
            .managed_activity_report_snapshot_with_transition()
            .expect("activity should be readable");
        let _ = sequence;
        self.runtime.confirm_managed_activity_report(
            self.challenge.idle_sequence,
            local_sequence,
            observation,
        );
        self.runtime
            .reserve_managed_kernel_for_stop(self.challenge.clone())
            .expect("reservation should not fail")
    }
}

fn input(terminal_id: &str, text: &str) -> RoomTerminalOperation {
    RoomTerminalOperation::Input {
        terminal_id: terminal_id.to_string(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(text),
    }
}

fn sleep_command() -> Option<Vec<String>> {
    Some(vec!["sleep".to_string(), "30".to_string()])
}

#[tokio::test]
async fn a_running_terminal_command_blocks_the_stop_reservation() {
    let fixture = fixture("terminal-stop-blocked");
    let terminal_id = fixture.open(sleep_command()).await.expect("command opens");
    assert_eq!(fixture.runtime.managed_running_agent_count(), 1);
    assert!(
        !fixture.reserve(),
        "a stop must not be reserved under a running terminal command"
    );

    fixture
        .terminal(RoomTerminalOperation::Close { terminal_id })
        .await
        .expect("close");
    assert_eq!(fixture.runtime.managed_running_agent_count(), 0);
    assert!(fixture.reserve(), "an idle kernel reserves the stop");
}

#[tokio::test]
async fn an_idle_shell_does_not_count_as_work() {
    let fixture = fixture("terminal-idle-shell");
    fixture.open_settled_shell().await;
    assert_eq!(fixture.runtime.managed_running_agent_count(), 0);
    assert!(fixture.reserve());
}

#[tokio::test]
async fn terminal_commands_are_refused_after_the_stop_reservation() {
    let fixture = fixture("terminal-after-reservation");
    let shell = fixture.open_settled_shell().await;
    assert!(fixture.reserve());

    assert!(fixture.open(sleep_command()).await.is_err());
    assert!(
        fixture.terminal(input(&shell, "sleep 30\n")).await.is_err(),
        "a command typed at the prompt starts work behind the fence"
    );
    assert!(
        fixture.terminal(input(&shell, "ls")).await.is_ok(),
        "keystrokes that start nothing stay usable"
    );
    assert!(fixture.open(None).await.is_ok(), "a shell is not work");
}

#[tokio::test]
async fn a_command_typed_at_the_prompt_counts_as_work_until_it_ends() {
    let fixture = fixture("terminal-typed-command");
    let shell = fixture.open_settled_shell().await;
    fixture
        .terminal(input(&shell, "sleep 2\n"))
        .await
        .expect("input");
    assert_eq!(fixture.runtime.managed_running_agent_count(), 1);
    // Past the launch window, the running program itself holds the count.
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    assert_eq!(fixture.runtime.managed_running_agent_count(), 1);
    assert!(!fixture.reserve());

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while fixture.runtime.managed_running_agent_count() != 0 {
        assert!(std::time::Instant::now() < deadline, "command never ended");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    assert!(fixture.reserve(), "the stop can be reserved once it ended");
}

#[tokio::test]
async fn a_background_job_keeps_the_stop_back_after_the_prompt_returns() {
    let fixture = fixture("terminal-background-job");
    let shell = fixture.open_settled_shell().await;
    fixture
        .terminal(input(&shell, "sleep 30 >/dev/null 2>&1 &\n"))
        .await
        .expect("input");
    // Past the launch window the shell owns the foreground again.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(
        fixture.runtime.managed_running_agent_count(),
        1,
        "the job still runs in the terminal's session"
    );
    assert!(!fixture.reserve());

    // Closing the terminal sweeps the job.
    fixture
        .terminal(RoomTerminalOperation::Close { terminal_id: shell })
        .await
        .expect("close");
    assert_eq!(fixture.runtime.managed_running_agent_count(), 0);
    assert!(fixture.reserve());
}

#[tokio::test]
async fn a_finished_background_job_stops_counting() {
    let fixture = fixture("terminal-job-finished");
    let shell = fixture.open_settled_shell().await;
    fixture
        .terminal(input(&shell, "sleep 2 >/dev/null 2>&1 &\n"))
        .await
        .expect("input");
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    assert_eq!(fixture.runtime.managed_running_agent_count(), 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while fixture.runtime.managed_running_agent_count() != 0 {
        assert!(std::time::Instant::now() < deadline, "job never ended");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    assert!(fixture.reserve());
}

#[tokio::test]
async fn an_ended_room_admits_no_new_terminal() {
    let fixture = fixture("terminal-ended-room");
    fixture.open(None).await.expect("shell opens");
    fixture
        .runtime
        .end_session(&fixture.session_id)
        .await
        .expect("room ends");
    assert!(
        fixture.open(None).await.is_err(),
        "an ended Room must not gain a shell"
    );
    assert!(fixture.open(sleep_command()).await.is_err());
}

/// Deleting a Room sweeps its terminals only once the Room is gone, so an open
/// that races the delete cannot leave a shell behind a deleted Room. The wide
/// window is the managed-environment stop awaited before the delete, which a
/// unit fixture does not have; this guards the outcome, not the ordering.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_terminal_opened_while_the_room_is_deleted_does_not_survive_it() {
    let fixture = Arc::new(fixture("terminal-deleted-room"));
    let directory = fixture
        .runtime
        .room_project_directory(&fixture.session_id)
        .unwrap();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let openers: Vec<_> = (0..3)
        .map(|_| {
            let (fixture, stop) = (Arc::clone(&fixture), Arc::clone(&stop));
            let directory = directory.clone();
            tokio::spawn(async move {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = fixture
                        .runtime
                        .run_room_terminal_operation(
                            &fixture.session_id,
                            directory.clone(),
                            TerminalActor::User,
                            RoomTerminalOperation::Open {
                                cols: 80,
                                rows: 24,
                                title: None,
                                command: sleep_command(),
                            },
                        )
                        .await;
                    tokio::task::yield_now().await;
                }
            })
        })
        .collect();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    fixture
        .runtime
        .delete_session_id(&fixture.session_id)
        .await
        .expect("room deletes");
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for opener in openers {
        opener.await.expect("opener joins");
    }
    let RoomTerminalResult::Listed { terminals } = fixture
        .runtime
        .run_room_terminal_operation(
            &fixture.session_id,
            directory,
            TerminalActor::User,
            RoomTerminalOperation::List,
        )
        .await
        .expect("list")
    else {
        panic!("list expected")
    };
    assert!(
        terminals.is_empty(),
        "a deleted Room must not keep terminals: {terminals:?}"
    );
}

/// Destroying a worker's lease ends its backing Room before the terminal
/// sweep, so an open admitted just before the cleanup removes its own shell
/// instead of outliving it. A failure between the two shows the order: the Room
/// is already closed to new terminals while the sweep is still to come.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_lease_closes_the_room_to_terminals_before_sweeping_them() {
    use crate::app::{LeaseCallerBinding, LeasedAgentCleanupPhase, RemoteLeaseRuntime};
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.kernel_runtime_role = crate::config::KernelRuntimeRole::RemoteLeaseWorker;
    config.remote_lease_capacity = Some(1);
    let mut app = DaemonApp::bootstrap(config).expect("worker boots");
    let caller = LeaseCallerBinding {
        home_kernel_id: "home-kernel".to_string(),
        authenticated_machine_id: "home-machine".to_string(),
        owner_user_id: "user-home".to_string(),
        realm_id: "realm-home".to_string(),
        public_key_thumbprint: "key-home".to_string(),
    };
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_bound_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
            caller.clone(),
        )
        .expect("lease");
    let worktree = crate::test_support::TestWorktree::new("lease-sweep-order");
    let leased = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent_for_caller(
            &lease.id,
            &caller,
            "opencode",
            "default",
            None,
            None,
            None,
            None,
            None,
            Some(worktree.path().display().to_string()),
            None,
        )
        .expect("leased agent");
    let backing = leased.backing_session_id.clone();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let runtime = router.runtime_state();
    let open = || {
        runtime.run_room_terminal_operation(
            &backing,
            worktree.path().to_path_buf(),
            TerminalActor::User,
            RoomTerminalOperation::Open {
                cols: 80,
                rows: 24,
                title: None,
                command: sleep_command(),
            },
        )
    };
    open().await.expect("the worker's Room opens a terminal");

    let leased_id = leased.id.clone();
    let failed = runtime
        .with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).inject_next_leased_agent_cleanup_failure(
                &leased_id,
                LeasedAgentCleanupPhase::BackingTerminals,
            );
            RemoteLeaseRuntime::new(app).destroy_leased_agent(&leased_id)
        })
        .await;
    assert!(failed.is_err(), "the injected sweep failure is reported");
    assert!(
        open().await.is_err(),
        "before the sweep the Room is already closed to new terminals"
    );

    let leased_id = leased.id.clone();
    runtime
        .with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).destroy_leased_agent(&leased_id)
        })
        .await
        .expect("the retry finishes the cleanup");
    let backing_id = backing.clone();
    let left = runtime
        .with_app_side_effect(move |app| app.room_terminals().list(&backing_id).len())
        .await;
    assert_eq!(left, 0, "a destroyed lease leaves no terminals");
}

// MP-08 / MP-09 / MP-11: managed terminal activity callbacks must not keep
// the durable writer and exclusive state owner alive after runtime teardown.
#[tokio::test]
async fn dropping_managed_runtime_releases_durable_state_owner() {
    let config = DaemonConfig::for_tests();
    let mut app = DaemonApp::bootstrap(config.clone()).expect("managed daemon should boot");
    app.managed_kernel_registration = Some(ConfirmedManagedKernelRegistration {
        environment_id: "drop-test-environment".to_string(),
        machine_id: "drop-test-machine".to_string(),
        kernel_id: config.daemon_id.clone(),
        context_plan: None,
    });
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let runtime = router.runtime_state();
    let survivor = runtime.clone();
    drop(runtime);
    drop(router);
    drop(app);
    assert!(
        crate::durable_state::DurableKernelStateStore::open_owned(config.durable_state_path())
            .is_err(),
        "a surviving runtime clone must retain the durable state owner"
    );
    drop(survivor);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match DaemonApp::bootstrap(config.clone()) {
            Ok(_) => break,
            Err(crate::error::DaemonError::LocalTransport {
                operation: "durable_state.acquire_owner",
                ..
            }) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(error) => panic!("managed runtime retained its durable state owner: {error}"),
        }
    }
}
