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
    fixture.open(None).await.expect("shell opens");
    assert_eq!(fixture.runtime.managed_running_agent_count(), 0);
    assert!(fixture.reserve());
}

#[tokio::test]
async fn terminal_commands_are_refused_after_the_stop_reservation() {
    let fixture = fixture("terminal-after-reservation");
    let shell = fixture.open(None).await.expect("shell opens");
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
    let shell = fixture.open(None).await.expect("shell opens");
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
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(fixture.reserve(), "the stop can be reserved once it ended");
}
