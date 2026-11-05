//! MP-08 / MP-09 / MP-10 / MP-11: Setup and Import share managed work and stop admission.
use crate::error::DaemonError;
use crate::local::{
    EnvironmentOperationRequest, ImportProjectEnvironmentEntriesRequest, LocalDaemonResponse,
    ProjectEnvironmentSetupField, RunProjectEnvironmentSetupRequest, SaveProjectEnvironmentRequest,
};
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
use crate::runtime::router::CommandRouter;
use crate::runtime::state::{KernelRuntimeState, ManagedKernelQuiescenceChallenge};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

struct Fixture {
    worktree: crate::test_support::TestWorktree,
    _router: CommandRouter,
    runtime: KernelRuntimeState,
    project_id: String,
    owner: String,
    challenge: ManagedKernelQuiescenceChallenge,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // A failed assertion must also stop any real script before deleting its checkout.
        for id in [
            "setup-work",
            "local-fenced",
            "peer-fenced",
            "import-work",
            "import-fenced",
        ] {
            let _ = self.runtime.cancel_environment_operation(
                EnvironmentOperationRequest {
                    operation_id: id.into(),
                },
                &self.owner,
            );
        }
        let _ = std::fs::write(self.worktree.path().join("release"), []);
    }
}

fn fixture(label: &str) -> Fixture {
    let mut app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let worktree = crate::test_support::TestWorktree::new(label);
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let kernel_id = app.config().daemon_id.clone();
    app.managed_kernel_registration = Some(ConfirmedManagedKernelRegistration {
        environment_id: "setup-environment".into(),
        machine_id: "setup-machine".into(),
        kernel_id: kernel_id.clone(),
        context_plan: None,
    });
    let project_id = session.project_id().to_string();
    let owner = app
        .sessions()
        .get_project(&project_id)
        .unwrap()
        .owner_user_id()
        .to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let runtime = router.runtime_state();
    runtime
        .ensure_managed_activity_tracking(&kernel_id)
        .unwrap();
    runtime
        .save_project_environment(
            SaveProjectEnvironmentRequest {
                project_id: project_id.clone(),
                expected_revision: 0,
                setup: ProjectEnvironmentSetupField {
                    script: "while [ ! -f release ]; do sleep 0.02; done".into(),
                },
                variables: String::new(),
                secrets: Vec::new(),
                files: Vec::new(),
            },
            &owner,
        )
        .unwrap();
    Fixture {
        worktree,
        _router: router,
        runtime,
        project_id,
        owner,
        challenge: ManagedKernelQuiescenceChallenge {
            challenge_id: format!("challenge-{label}"),
            account_id: "setup-account".into(),
            environment_id: "setup-environment".into(),
            machine_id: "setup-machine".into(),
            kernel_id,
            desired_revision: 1,
            idle_sequence: 7,
            idle_deadline_at: "2026-11-05T00:00:00.000Z".into(),
            stop_operation_id: format!("stop-{label}"),
            nonce: format!("nonce-{label}"),
        },
    }
}

impl Fixture {
    fn start(&self, id: &str) -> Result<LocalDaemonResponse, DaemonError> {
        self.runtime.run_project_environment_setup(
            RunProjectEnvironmentSetupRequest {
                project_id: self.project_id.clone(),
                operation_id: id.into(),
                checkout_path: None,
            },
            &self.owner,
        )
    }

    fn reserve(&self) -> bool {
        let (_, observation, sequence) = self
            .runtime
            .managed_activity_report_snapshot_with_transition()
            .unwrap();
        self.runtime.confirm_managed_activity_report(
            self.challenge.idle_sequence,
            sequence,
            observation,
        );
        self.runtime
            .reserve_managed_kernel_for_stop(self.challenge.clone())
            .unwrap()
    }

    async fn finish(&self, id: &str) {
        std::fs::write(self.worktree.path().join("release"), []).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let LocalDaemonResponse::EnvironmentOperation { operation } = self
                    .runtime
                    .get_environment_operation(
                        EnvironmentOperationRequest {
                            operation_id: id.into(),
                        },
                        &self.owner,
                    )
                    .unwrap()
                else {
                    panic!("Setup operation expected")
                };
                if operation.state != crate::local::EnvironmentOperationState::Running {
                    assert_eq!(
                        operation.state,
                        crate::local::EnvironmentOperationState::Succeeded
                    );
                    if self.runtime.managed_working_agent_count() == 0 {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn running_setup_blocks_stop_and_completion_reports_idle() {
    let fixture = fixture("setup-stop-work");
    let before = fixture
        .runtime
        .managed_activity_record_call_count_for_test();
    fixture.start("setup-work").unwrap();
    assert!(!fixture.reserve(), "Setup must keep the machine running");
    assert_eq!(fixture.runtime.managed_working_agent_count(), 1);
    let during = fixture
        .runtime
        .managed_activity_record_call_count_for_test();
    assert!(
        during > before,
        "admission publishes a managed activity transition"
    );
    fixture.finish("setup-work").await;
    assert_eq!(fixture.runtime.managed_working_agent_count(), 0);
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture
            .runtime
            .managed_activity_record_call_count_for_test()
            <= during
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("completion publishes the idle transition without a poll");
    assert!(fixture.reserve());
}

#[tokio::test]
async fn reserved_stop_refuses_setup_from_local_and_peer_entry_points() {
    let fixture = fixture("setup-stop-fence");
    assert!(fixture.reserve());
    assert!(
        fixture.start("local-fenced").is_err(),
        "local Setup crosses the stop fence"
    );
    let response = fixture
        .runtime
        .receive_project_environment_setup_peer(
            &fixture.owner,
            RelayPeerRequest::RunProjectEnvironmentSetup {
                project_id: fixture.project_id.clone(),
                operation_id: "peer-fenced".into(),
                checkout_path: None,
            },
        )
        .unwrap();
    assert!(
        matches!(
            response,
            RelayPeerResponse::ProjectEnvironmentRefused { .. }
        ),
        "peer Setup crosses the stop fence"
    );
    for id in ["local-fenced", "peer-fenced"] {
        assert!(
            fixture
                .runtime
                .get_environment_operation(
                    EnvironmentOperationRequest {
                        operation_id: id.into()
                    },
                    &fixture.owner
                )
                .is_err(),
            "refused Setup must not publish an operation"
        );
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: Import is work from admission through cleanup.
#[tokio::test]
async fn admitted_import_blocks_stop_until_its_task_settles() {
    let fixture = fixture("import-stop-work");
    let before = fixture
        .runtime
        .managed_activity_record_call_count_for_test();
    fixture
        .runtime
        .import_project_environment_entries(
            ImportProjectEnvironmentEntriesRequest {
                project_id: fixture.project_id.clone(),
                operation_id: "import-work".into(),
                source_kernel_id: "source-kernel".into(),
                checkout_path: None,
                entries: None,
            },
            &fixture.owner,
        )
        .unwrap();
    // The current-thread executor has not polled the detached source request yet.
    assert_eq!(
        fixture.runtime.managed_working_agent_count(),
        1,
        "an admitted Import is already work while waiting for its source"
    );
    assert!(!fixture.reserve(), "Import must keep the machine running");
    let during = fixture
        .runtime
        .managed_activity_record_call_count_for_test();
    assert!(during > before, "admission publishes managed activity");
    fixture
        .runtime
        .cancel_environment_operation(
            EnvironmentOperationRequest {
                operation_id: "import-work".into(),
            },
            &fixture.owner,
        )
        .unwrap();
    assert_eq!(
        fixture.runtime.managed_working_agent_count(),
        1,
        "Stop must not release work before cancellation cleanup"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.runtime.managed_working_agent_count() != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        fixture
            .runtime
            .managed_activity_record_call_count_for_test()
            > during,
        "task cleanup publishes idle"
    );
    assert!(fixture.reserve());
}

#[tokio::test]
async fn reserved_stop_refuses_import_without_publishing_an_operation() {
    let fixture = fixture("import-stop-fence");
    assert!(fixture.reserve());
    let result = fixture.runtime.import_project_environment_entries(
        ImportProjectEnvironmentEntriesRequest {
            project_id: fixture.project_id.clone(),
            operation_id: "import-fenced".into(),
            source_kernel_id: "source-kernel".into(),
            checkout_path: None,
            entries: None,
        },
        &fixture.owner,
    );
    assert!(result.is_err(), "Import must not cross the stop fence");
    assert!(
        fixture
            .runtime
            .get_environment_operation(
                EnvironmentOperationRequest {
                    operation_id: "import-fenced".into()
                },
                &fixture.owner,
            )
            .is_err(),
        "a refused Import must not publish an operation"
    );
}
