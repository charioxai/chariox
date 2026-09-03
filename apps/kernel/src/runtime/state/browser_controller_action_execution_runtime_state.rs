use std::future::Future;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::runtime::browser_controller_process::CONTROLLER_RESTARTED_BEFORE_OPERATION;
use crate::session::{
    agent_environment_actor_id, ActionAdmission, EnvironmentActionRequest, EnvironmentActionState,
    EnvironmentActionTerminal, EnvironmentError,
};
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand, RoomBrowserControllerResult, RoomComputerInputAction,
};

use super::KernelRuntimeState;

const ACTION_QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const ACTION_QUEUE_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub(crate) struct BrowserControllerActionExecution<T> {
    pub(crate) action_id: String,
    pub(crate) actor_id: String,
    pub(crate) value: T,
}

#[derive(Debug)]
pub(crate) struct ComputerControllerActionExecution {
    pub(crate) action_id: String,
    pub(crate) actor_id: String,
    pub(crate) action_kind: &'static str,
    pub(crate) environment_id: String,
    pub(crate) runtime_generation: u64,
}

impl KernelRuntimeState {
    pub(crate) async fn execute_computer_input_as_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        input: RoomComputerInputAction,
    ) -> Result<ComputerControllerActionExecution, DaemonError> {
        let environment = self
            .reconcile_room_environment_actors(session_id, None)
            .map_err(action_environment_error)?;
        let actor_id = agent_environment_actor_id(agent_id);
        crate::runtime::computer_input_action::validate_computer_input_action(
            &environment.viewport,
            &input,
        )
        .map_err(action_environment_error)?;
        let metadata = crate::runtime::computer_input_action::computer_input_action_metadata(
            &input,
            environment.viewport.revision,
        );
        let mut request = EnvironmentActionRequest::computer_mutation(
            &actor_id,
            environment.runtime_generation,
            metadata.kind,
            environment.focused_tab_id.as_deref(),
        );
        if let Some(arguments) = metadata.arguments {
            request = request.with_arguments(arguments);
        }
        let (admission, _) = self
            .submit_room_environment_action(session_id, request)
            .map_err(action_environment_error)?;
        let action_id = match admission {
            ActionAdmission::Accepted { action_id } => action_id,
            ActionAdmission::Queued { action_id, .. } => {
                self.wait_for_environment_action_admission(session_id, &action_id)
                    .await?;
                action_id
            }
            ActionAdmission::Existing { action_id, state } => {
                return Err(action_dispatch_error(format!(
                    "unexpected existing computer action `{action_id}` in state {state:?}"
                )));
            }
            ActionAdmission::RejectedSaturated { capacity } => {
                return Err(action_dispatch_error(format!(
                    "computer action queue reached its capacity of {capacity}"
                )));
            }
            ActionAdmission::RejectedBusy {
                target,
                active_action_id,
            } => {
                return Err(action_dispatch_error(format!(
                    "computer action target {target:?} is reserved by `{active_action_id}`"
                )));
            }
            ActionAdmission::RejectedTakeover {
                target,
                human_actor_id,
            } => {
                return Err(action_dispatch_error(format!(
                    "computer action target {target:?} belongs to `{human_actor_id}`"
                )));
            }
        };

        let current = self
            .room_environment_snapshot(session_id)
            .map_err(action_environment_error)?;
        if current.runtime_generation != environment.runtime_generation {
            let _ = self.finish_room_environment_action(
                session_id,
                &action_id,
                EnvironmentActionTerminal::Failed,
            );
            return Err(action_dispatch_error(
                "computer action runtime generation changed before execution".to_string(),
            ));
        }
        let execution = self
            .await_cancellable_browser_action(
                session_id,
                &action_id,
                &action_id,
                self.room_browser_controller_command(
                    session_id,
                    RoomBrowserControllerCommand::ComputerInput {
                        action_id: action_id.clone(),
                        actor_id: actor_id.clone(),
                        runtime_generation: current.runtime_generation,
                        viewport_revision: current.viewport.revision,
                        desktop_pixel_width: current.viewport.desktop_pixel_width,
                        desktop_pixel_height: current.viewport.desktop_pixel_height,
                        action: input,
                    },
                ),
            )
            .await;
        let terminal = match &execution {
            Ok(RoomBrowserControllerResult::ComputerInputApplied {
                action_id: returned_action_id,
            }) if returned_action_id == &action_id => EnvironmentActionTerminal::Completed,
            Ok(RoomBrowserControllerResult::ActionCancelled { .. })
            | Err(DaemonError::BrowserControllerActionCancelled { .. }) => {
                EnvironmentActionTerminal::Cancelled
            }
            _ => EnvironmentActionTerminal::Failed,
        };
        self.finish_room_environment_action(session_id, &action_id, terminal)
            .map_err(action_environment_error)?;
        match execution {
            Ok(RoomBrowserControllerResult::ComputerInputApplied {
                action_id: returned_action_id,
            }) if returned_action_id == action_id => Ok(ComputerControllerActionExecution {
                action_id,
                actor_id,
                action_kind: metadata.kind,
                environment_id: current.environment_id,
                runtime_generation: current.runtime_generation,
            }),
            Ok(RoomBrowserControllerResult::ActionCancelled { controller_fenced }) => {
                Err(DaemonError::BrowserControllerActionCancelled { controller_fenced })
            }
            Ok(_) => Err(action_dispatch_error(
                "computer input returned a mismatched controller response".to_string(),
            )),
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn execute_browser_mutation_as_agent<T, F>(
        &self,
        session_id: &str,
        agent_id: &str,
        tab_id: &str,
        document_revision: u64,
        action_kind: &str,
        execution_id: Option<&str>,
        execution: F,
    ) -> Result<BrowserControllerActionExecution<T>, DaemonError>
    where
        F: Future<Output = Result<T, DaemonError>>,
    {
        let environment = self
            .reconcile_room_environment_actors(session_id, None)
            .map_err(action_environment_error)?;
        let actor_id = agent_environment_actor_id(agent_id);
        let request = EnvironmentActionRequest::browser_mutation(
            &actor_id,
            environment.runtime_generation,
            action_kind,
            tab_id,
            document_revision,
        );
        let (admission, _) = self
            .submit_room_environment_action(session_id, request)
            .map_err(action_environment_error)?;
        let action_id = match admission {
            ActionAdmission::Accepted { action_id } => action_id,
            ActionAdmission::Queued { action_id, .. } => {
                self.wait_for_environment_action_admission(session_id, &action_id)
                    .await?;
                action_id
            }
            ActionAdmission::Existing { action_id, state } => {
                return Err(action_dispatch_error(format!(
                    "unexpected existing browser action `{action_id}` in state {state:?}"
                )));
            }
            ActionAdmission::RejectedSaturated { capacity } => {
                return Err(action_dispatch_error(format!(
                    "browser action queue reached its capacity of {capacity}"
                )));
            }
            ActionAdmission::RejectedBusy {
                target,
                active_action_id,
            } => {
                return Err(action_dispatch_error(format!(
                    "browser action target {target:?} is reserved by `{active_action_id}`"
                )));
            }
            ActionAdmission::RejectedTakeover {
                target,
                human_actor_id,
            } => {
                return Err(action_dispatch_error(format!(
                    "browser action target {target:?} belongs to `{human_actor_id}`"
                )));
            }
        };

        if let Err(error) =
            self.validate_browser_action_precondition(session_id, tab_id, document_revision)
        {
            let _ = self.finish_room_environment_action(
                session_id,
                &action_id,
                EnvironmentActionTerminal::Failed,
            );
            return Err(action_environment_error(error));
        }

        let result = match execution_id {
            Some(execution_id) => {
                self.await_cancellable_browser_action(
                    session_id,
                    &action_id,
                    execution_id,
                    execution,
                )
                .await
            }
            None => execution.await,
        };
        let controller_restart_generation = match &result {
            Err(DaemonError::BrowserControllerRecoveryRequired { runtime_generation }) => {
                Some(*runtime_generation)
            }
            _ => None,
        };
        if let Some(runtime_generation) = controller_restart_generation {
            self.recover_browser_controller_after_restart(session_id, runtime_generation)
                .await?;
            return Err(DaemonError::LocalTransport {
                operation: "browser_controller.route",
                message: CONTROLLER_RESTARTED_BEFORE_OPERATION.to_string(),
            });
        }
        let controller_fenced = matches!(
            &result,
            Err(DaemonError::BrowserControllerActionCancelled {
                controller_fenced: true,
            })
        );
        let terminal = if controller_fenced
            || matches!(
                &result,
                Err(DaemonError::BrowserControllerActionCancelled { .. })
            ) {
            EnvironmentActionTerminal::Cancelled
        } else if result.is_ok() {
            EnvironmentActionTerminal::Completed
        } else {
            EnvironmentActionTerminal::Failed
        };
        self.finish_room_environment_action(session_id, &action_id, terminal)
            .map_err(action_environment_error)?;
        if controller_fenced {
            if let Err(recovery_error) = self
                .recover_browser_controller_after_fence(session_id)
                .await
            {
                return Err(DaemonError::LocalTransport {
                    operation: "browser_controller.recovery_failed",
                    message: format!(
                        "browser action was cancelled, but controller recovery failed: {recovery_error}"
                    ),
                });
            }
        }
        Ok(BrowserControllerActionExecution {
            action_id,
            actor_id,
            value: result?,
        })
    }

    async fn wait_for_environment_action_admission(
        &self,
        session_id: &str,
        action_id: &str,
    ) -> Result<(), DaemonError> {
        let started = Instant::now();
        loop {
            let environment = self
                .room_environment_snapshot(session_id)
                .map_err(action_environment_error)?;
            let action = environment
                .actions
                .iter()
                .find(|action| action.action_id == action_id)
                .ok_or_else(|| {
                    action_dispatch_error(format!(
                        "queued environment action `{action_id}` disappeared before execution"
                    ))
                })?;
            match action.state {
                EnvironmentActionState::Running => return Ok(()),
                EnvironmentActionState::Queued if started.elapsed() < ACTION_QUEUE_WAIT_TIMEOUT => {
                    tokio::time::sleep(ACTION_QUEUE_POLL_INTERVAL).await;
                }
                EnvironmentActionState::Queued => {
                    let _ = self.finish_room_environment_action(
                        session_id,
                        action_id,
                        EnvironmentActionTerminal::Cancelled,
                    );
                    return Err(action_dispatch_error(format!(
                        "queued environment action `{action_id}` timed out"
                    )));
                }
                state => {
                    return Err(action_dispatch_error(format!(
                        "queued environment action `{action_id}` became {state:?} before execution"
                    )));
                }
            }
        }
    }

    fn validate_browser_action_precondition(
        &self,
        session_id: &str,
        tab_id: &str,
        document_revision: u64,
    ) -> Result<(), EnvironmentError> {
        let environment = self.room_environment_snapshot(session_id)?;
        let tab = environment
            .tabs
            .iter()
            .find(|tab| tab.tab_id == tab_id)
            .ok_or_else(|| EnvironmentError::UnknownTab {
                tab_id: tab_id.to_string(),
            })?;
        if tab.document_revision != document_revision {
            return Err(EnvironmentError::StaleDocumentRevision {
                tab_id: tab_id.to_string(),
                expected: tab.document_revision,
                actual: document_revision,
            });
        }
        Ok(())
    }
}

fn action_environment_error(error: EnvironmentError) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "browser_controller.action_execution",
        message: format!("{}: {error:?}", error.code()),
    }
}

fn action_dispatch_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "browser_controller.action_execution",
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::{oneshot, Mutex};

    #[test]
    fn browser_mutation_targets_only_its_room_tab() {
        let request =
            EnvironmentActionRequest::browser_mutation("agent:test", 1, "click", "tab-7", 3);
        assert_eq!(
            request.targets,
            vec![crate::session::InputTarget::BrowserTab("tab-7".to_string())]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn same_tab_controller_mutations_execute_in_ledger_order() {
        let test_root = TestRoot::new("browser-action-ledger");
        let test_root_path = test_root.path().to_string_lossy().into_owned();
        let mut app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                &test_root_path,
                &test_root_path,
            ))
            .expect("session should be created");
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("browser-agent"),
            )
            .expect("agent should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let runtime = runtime_state_from_test_app(app);
        let viewport = crate::session::CanonicalViewport::new(1280, 800, 1, 1280, 800)
            .expect("test viewport should be valid");
        runtime
            .owned
            .session_store
            .create_room_environment(&session_id, "environment-test", viewport.clone())
            .expect("environment should be created");
        runtime
            .start_room_environment(&session_id, viewport)
            .expect("environment should start");
        runtime
            .transition_room_environment(&session_id, crate::session::EnvironmentLifecycle::Ready)
            .expect("environment should become ready");
        runtime
            .reconcile_room_environment_controller_tabs(
                &session_id,
                vec![
                    crate::session::EnvironmentTabObservation {
                        runtime_target_id: "target-a".to_string(),
                        document_id: "loader-a".to_string(),
                        url: "https://example.test".to_string(),
                        title: "Example".to_string(),
                    },
                    crate::session::EnvironmentTabObservation {
                        runtime_target_id: "target-b".to_string(),
                        document_id: "loader-b".to_string(),
                        url: "https://other.test".to_string(),
                        title: "Other".to_string(),
                    },
                ],
                Some("target-a"),
            )
            .expect("test tab should be reconciled");

        let (first_started_tx, first_started_rx) = oneshot::channel();
        let (release_first_tx, release_first_rx) = oneshot::channel();
        let first_runtime = runtime.clone();
        let first_session_id = session_id.clone();
        let first_agent_id = agent_id.clone();
        let first = tokio::spawn(async move {
            first_runtime
                .execute_browser_mutation_as_agent(
                    &first_session_id,
                    &first_agent_id,
                    "tab-1",
                    1,
                    "first-click",
                    None,
                    async move {
                        first_started_tx.send(()).ok();
                        release_first_rx.await.expect("first action should release");
                        Ok::<_, DaemonError>("first")
                    },
                )
                .await
        });
        first_started_rx
            .await
            .expect("first action should start execution");

        let other_tab = runtime
            .execute_browser_mutation_as_agent(
                &session_id,
                &agent_id,
                "tab-2",
                1,
                "other-tab-click",
                None,
                async { Ok::<_, DaemonError>("other") },
            )
            .await
            .expect("a different tab should not wait for the first tab");
        assert_eq!(other_tab.value, "other");

        let (second_started_tx, second_started_rx) = oneshot::channel();
        let second_runtime = runtime.clone();
        let second_session_id = session_id.clone();
        let second_agent_id = agent_id.clone();
        let second = tokio::spawn(async move {
            second_runtime
                .execute_browser_mutation_as_agent(
                    &second_session_id,
                    &second_agent_id,
                    "tab-1",
                    1,
                    "second-click",
                    None,
                    async move {
                        second_started_tx.send(()).ok();
                        Ok::<_, DaemonError>("second")
                    },
                )
                .await
        });
        let mut second_started_rx = Box::pin(second_started_rx);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut second_started_rx)
                .await
                .is_err(),
            "second mutation must stay queued while the first owns the tab"
        );
        let queued = runtime
            .room_environment_snapshot(&session_id)
            .expect("environment snapshot should be available");
        assert_eq!(
            queued
                .actions
                .iter()
                .map(|action| action.state)
                .collect::<Vec<_>>(),
            vec![
                EnvironmentActionState::Running,
                EnvironmentActionState::Completed,
                EnvironmentActionState::Queued
            ]
        );

        release_first_tx
            .send(())
            .expect("first action release should be received");
        second_started_rx
            .await
            .expect("second action should start after the first completes");
        let first = first.await.expect("first task should join").unwrap();
        let second = second.await.expect("second task should join").unwrap();
        assert_eq!(first.value, "first");
        assert_eq!(second.value, "second");
        assert_eq!(first.actor_id, agent_environment_actor_id(&agent_id));
        assert_eq!(second.actor_id, first.actor_id);
        assert_ne!(first.action_id, second.action_id);
        assert!(runtime
            .execute_browser_mutation_as_agent(
                &session_id,
                &agent_id,
                "tab-2",
                1,
                "failed-click",
                None,
                async { Err::<(), _>(action_dispatch_error("expected failure".to_string())) },
            )
            .await
            .is_err());
        let completed = runtime
            .room_environment_snapshot(&session_id)
            .expect("environment snapshot should remain available");
        assert_eq!(
            completed
                .actions
                .iter()
                .map(|action| action.state)
                .collect::<Vec<_>>(),
            vec![
                EnvironmentActionState::Completed,
                EnvironmentActionState::Completed,
                EnvironmentActionState::Completed,
                EnvironmentActionState::Failed
            ]
        );

        let (blocker_started_tx, blocker_started_rx) = oneshot::channel();
        let (release_blocker_tx, release_blocker_rx) = oneshot::channel();
        let blocker_runtime = runtime.clone();
        let blocker_session_id = session_id.clone();
        let blocker_agent_id = agent_id.clone();
        let blocker = tokio::spawn(async move {
            blocker_runtime
                .execute_browser_mutation_as_agent(
                    &blocker_session_id,
                    &blocker_agent_id,
                    "tab-1",
                    1,
                    "blocking-click",
                    None,
                    async move {
                        blocker_started_tx.send(()).ok();
                        release_blocker_rx
                            .await
                            .expect("blocking action should release");
                        Ok::<_, DaemonError>(())
                    },
                )
                .await
        });
        blocker_started_rx
            .await
            .expect("blocking action should start");
        let (stale_executed_tx, stale_executed_rx) = oneshot::channel();
        let stale_runtime = runtime.clone();
        let stale_session_id = session_id.clone();
        let stale_agent_id = agent_id.clone();
        let stale = tokio::spawn(async move {
            stale_runtime
                .execute_browser_mutation_as_agent(
                    &stale_session_id,
                    &stale_agent_id,
                    "tab-1",
                    1,
                    "stale-click",
                    None,
                    async move {
                        stale_executed_tx.send(()).ok();
                        Ok::<_, DaemonError>(())
                    },
                )
                .await
        });
        for _ in 0..100 {
            let environment = runtime
                .room_environment_snapshot(&session_id)
                .expect("environment snapshot should be available");
            if environment.actions.len() == 6 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let queued_before_reload = runtime
            .room_environment_snapshot(&session_id)
            .expect("environment snapshot should be available");
        assert_eq!(queued_before_reload.actions.len(), 6);
        assert_eq!(
            queued_before_reload.actions[5].state,
            EnvironmentActionState::Queued
        );
        runtime
            .reconcile_room_environment_controller_tabs(
                &session_id,
                vec![
                    crate::session::EnvironmentTabObservation {
                        runtime_target_id: "target-a".to_string(),
                        document_id: "loader-a2".to_string(),
                        url: "https://example.test/reloaded".to_string(),
                        title: "Reloaded".to_string(),
                    },
                    crate::session::EnvironmentTabObservation {
                        runtime_target_id: "target-b".to_string(),
                        document_id: "loader-b".to_string(),
                        url: "https://other.test".to_string(),
                        title: "Other".to_string(),
                    },
                ],
                Some("target-a"),
            )
            .expect("test tab reload should reconcile");
        release_blocker_tx
            .send(())
            .expect("blocking action release should be received");
        blocker
            .await
            .expect("blocking task should join")
            .expect("blocking action should complete");
        let stale_error = stale
            .await
            .expect("stale task should join")
            .expect_err("queued action must fail after its document changes");
        assert!(stale_error
            .to_string()
            .contains("environment_stale_document_revision"));
        assert!(
            stale_executed_rx.await.is_err(),
            "a stale queued action must not reach the controller"
        );
        let stale_snapshot = runtime
            .room_environment_snapshot(&session_id)
            .expect("environment snapshot should remain available");
        assert_eq!(
            stale_snapshot.actions[5].state,
            EnvironmentActionState::Failed
        );
    }

    struct TestRoot(std::path::PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "chariox-{label}-{}-{}",
                std::process::id(),
                crate::session::unix_epoch_ms()
            ));
            std::fs::create_dir_all(&path).expect("test root should be created");
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn runtime_state_from_test_app(app: crate::DaemonApp) -> KernelRuntimeState {
        let config_projection = app.config_projection_store();
        let session_store = app.session_state_store();
        let agent_store = app.agents().clone();
        let attachment_store = app.attachments().clone();
        let provider_store = app.providers().clone();
        let provider_process_tracking = app.provider_process_tracking_store();
        let slice_store = app.slices();
        let session_projection = app.session_state_projection_store();
        let provider_run_projection = app.provider_run_projection_store();
        let operational_history_store = app.operational_history_store();
        let durable_state_store = app.durable_state_store();
        let prompt_state_owner = app.prompt_state_owner();
        let active_turns = app.active_turn_store();
        let prompt_activity = app.prompt_activity_store();
        let prompt_workspace_claims = app.prompt_workspace_claim_store();
        let structured_output_records = app.structured_output_record_store();
        let terminal_stream = app.terminal_stream_store();
        let workflow_design_events = app.workflow_design_event_store();
        let metaagent_events = app.metaagent_event_store();
        let workspace_coordinator = app.workspace_coordinator();
        KernelRuntimeState::new_with_owned_state(
            Arc::new(Mutex::new(app)),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }
}
