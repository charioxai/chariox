use std::future::Future;
use std::io::Write;
use std::net::Shutdown;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{oneshot, Mutex as TokioMutex};

use crate::attachment::ClientCapabilityLevel;
use crate::config::PersistedCloudRelayProfile;
use crate::local::api::{
    AddWorkflowEdgeRequest, AddWorkflowNodeRequest, CancelWorkflowRunRequest,
    CreateWorkflowEndpointRequest, CreateWorkflowRequest, GetWorkflowRunRequest,
    InvokeWorkflowEndpointRequest, ListWorkflowRunsRequest,
};
use crate::local::{
    AttachToSessionRequest, CompletePromptRequest, LaunchProviderRunRequest,
    PumpTerminalOutputRequest, RunShellCapabilityRequest, SpawnAgentRequest, SubmitPromptRequest,
};
use crate::session::{CreateSessionRequest, WorkflowNodeRunStatus, WorkflowRun, WorkflowRunStatus};
use crate::{DaemonApp, DaemonConfig, DaemonError};

use super::{
    read_sync_frame, run_local_ipc_server, LocalDaemonRequest, LocalDaemonResponse, LocalIpcClient,
    StdUnixStream,
};

static LOCAL_IPC_TEST_LOCK: Mutex<()> = Mutex::new(());
const LOCAL_IPC_TEST_RUNTIME_THREAD_STACK_SIZE: usize = 64 * 1024 * 1024;

fn local_ipc_test_guard() -> MutexGuard<'static, ()> {
    LOCAL_IPC_TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

fn run_local_ipc_async_test<F, Fut>(name: &str, worker_threads: usize, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    let runtime_name = name.to_string();
    let spawn_name = runtime_name.clone();
    let thread_name = runtime_name.clone();
    std::thread::Builder::new()
        .name(thread_name)
        .stack_size(LOCAL_IPC_TEST_RUNTIME_THREAD_STACK_SIZE)
        .spawn(move || {
            let _guard = local_ipc_test_guard();
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(worker_threads)
                .enable_all()
                .thread_stack_size(LOCAL_IPC_TEST_RUNTIME_THREAD_STACK_SIZE)
                .build()
                .unwrap_or_else(|error| panic!("{runtime_name} runtime should start: {error}"))
                .block_on(test());
        })
        .unwrap_or_else(|error| panic!("{spawn_name} thread should spawn: {error}"))
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[test]
fn local_ipc_round_trip_exercises_session_and_terminal_flow() {
    run_local_ipc_async_test(
        "local-ipc-round-trip-exercises-session-and-terminal-flow",
        2,
        || async {
            let config = DaemonConfig::for_tests();
            let socket_path = config.local_socket_path.clone();
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
            let app = Arc::new(TokioMutex::new(
                DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed"),
            ));
            let server_app = Arc::clone(&app);
            let server = tokio::spawn(async move {
                super::run_local_ipc_server_with_shared_app(server_app, async {
                    let _ = shutdown_rx.await;
                })
                .await
            });

            wait_for_socket(&socket_path).await;

            let client = LocalIpcClient::new(socket_path.clone());
            let session = match client
                .send(&LocalDaemonRequest::CreateSession(
                    CreateSessionRequest::new("workspace-ipc", "."),
                ))
                .expect("session create should succeed")
            {
                LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
                other => panic!("unexpected response: {other:?}"),
            };
            let attachment = match client
                .send(&LocalDaemonRequest::AttachToSession(
                    AttachToSessionRequest {
                        session_id: session.id().to_string(),
                        client_id: "ipc-client".to_string(),
                        capability_level: ClientCapabilityLevel::FullTerminal,
                    },
                ))
                .expect("attach should succeed")
            {
                LocalDaemonResponse::SessionAttached { attachment } => attachment,
                other => panic!("unexpected response: {other:?}"),
            };

            client
                .send(&LocalDaemonRequest::LaunchProviderRun(
                    LaunchProviderRunRequest {
                        session_id: session.id().to_string(),
                        agent_id: None,
                        adapter_key: "dev-stub".to_string(),
                        provider: "dev-stub".to_string(),
                        account_profile: "default".to_string(),
                        model: "default".to_string(),
                        variant: None,
                        structured_endpoint: None,
                        provider_session_id: None,
                        native_tui: false,
                    },
                ))
                .expect("launch should succeed");
            client
                .send(&LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    target_agent_id: None,
                    prompt: "ipc smoke\n".to_string(),
                    attachments: Vec::new(),
                }))
                .expect("prompt submit should succeed");

            let output = wait_for_output(&client, session.id(), attachment.id()).await;
            assert!(output.contains("ipc smoke"));

            let _ = shutdown_tx.send(());
            server
                .await
                .expect("server task should join")
                .expect("server should stop cleanly");
        },
    );
}

#[test]
fn local_ipc_uses_linked_cloud_user_for_session_creation() {
    run_local_ipc_async_test(
        "local-ipc-uses-linked-cloud-user-for-session-creation",
        2,
        || async {
            let mut config = DaemonConfig::for_tests();
            config.cloud_relay = Some(PersistedCloudRelayProfile {
                api_url: "https://cloud.example.test".to_string(),
                email: "miguel@example.test".to_string(),
                account_id: "account-1".to_string(),
                user_id: "user-cloud".to_string(),
                account_slug: "miguel".to_string(),
                realm_id: "realm-1".to_string(),
                relay_url: "ws://relay.example.test".to_string(),
                issuer_id: "issuer-1".to_string(),
                client_id: Some("client-1".to_string()),
                client_alias: Some("local-cli".to_string()),
                machine_id: Some("machine-1".to_string()),
                machine_alias: Some("macbook".to_string()),
                machine_credential: None,
                cloud_session_token: Some("session-token".to_string()),
                cloud_session_expires_at_ms: None,
                token_expires_at_ms: None,
            });
            let socket_path = config.local_socket_path.clone();
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
            let server = tokio::spawn(async move {
                let app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
                run_local_ipc_server(app, async {
                    let _ = shutdown_rx.await;
                })
                .await
            });

            wait_for_socket(&socket_path).await;

            let client = LocalIpcClient::new(socket_path.clone());
            let response = client
                .send(&LocalDaemonRequest::CreateSession(
                    CreateSessionRequest::new("workspace-ipc-cloud", "."),
                ))
                .expect("session create should succeed");
            let session = match response {
                LocalDaemonResponse::SessionCreated { session, .. } => session,
                other => panic!("unexpected response: {other:?}"),
            };
            assert_eq!(session.owner_user_id(), "user-cloud");
            assert!(session.has_member("user-cloud"));

            let _ = shutdown_tx.send(());
            server
                .await
                .expect("server task should join")
                .expect("server should stop cleanly");
        },
    );
}

#[test]
fn local_ipc_prompt_submit_acks_while_shell_capability_is_slow() {
    run_local_ipc_async_test(
        "local-ipc-prompt-submit-acks-while-shell-capability-is-slow",
        4,
        || async {
            let config = DaemonConfig::for_tests();
            let socket_path = config.local_socket_path.clone();
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
            let server = tokio::spawn(async move {
                let app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
                run_local_ipc_server(app, async {
                    let _ = shutdown_rx.await;
                })
                .await
            });

            wait_for_socket(&socket_path).await;

            let client = LocalIpcClient::new(socket_path.clone());
            let cwd = std::env::current_dir()
                .expect("current directory should be available")
                .to_string_lossy()
                .to_string();
            let session = match client
                .send(&LocalDaemonRequest::CreateSession(
                    CreateSessionRequest::new(cwd.as_str(), cwd.as_str()),
                ))
                .expect("session create should succeed")
            {
                LocalDaemonResponse::SessionCreated { session, .. } => session,
                other => panic!("unexpected response: {other:?}"),
            };
            let agent_id = session
                .agents()
                .first()
                .expect("default agent should exist")
                .id()
                .to_string();
            let attachment = match client
                .send(&LocalDaemonRequest::AttachToSession(
                    AttachToSessionRequest {
                        session_id: session.id().to_string(),
                        client_id: "ipc-responsive-client".to_string(),
                        capability_level: ClientCapabilityLevel::FullTerminal,
                    },
                ))
                .expect("attach should succeed")
            {
                LocalDaemonResponse::SessionAttached { attachment } => attachment,
                other => panic!("unexpected response: {other:?}"),
            };

            client
                .send(&LocalDaemonRequest::LaunchProviderRun(
                    LaunchProviderRunRequest {
                        session_id: session.id().to_string(),
                        agent_id: Some(agent_id.clone()),
                        adapter_key: "dev-stub".to_string(),
                        provider: "dev-stub".to_string(),
                        account_profile: "default".to_string(),
                        model: "default".to_string(),
                        variant: None,
                        structured_endpoint: None,
                        provider_session_id: None,
                        native_tui: false,
                    },
                ))
                .expect("launch should succeed");

            let slow_client = LocalIpcClient::new(socket_path.clone());
            let slow_session_id = session.id().to_string();
            let slow_attachment_id = attachment.id().to_string();
            let slow_task = tokio::task::spawn_blocking(move || {
                slow_client.send(&LocalDaemonRequest::RunShellCommand(
                    RunShellCapabilityRequest {
                        session_id: slow_session_id,
                        attachment_id: slow_attachment_id,
                        command: "sh".to_string(),
                        args: vec!["-c".to_string(), "sleep 2".to_string()],
                        working_directory: None,
                        timeout_ms: Some(3_000),
                    },
                ))
            });
            tokio::time::sleep(Duration::from_millis(50)).await;

            let submit_client = LocalIpcClient::new(socket_path.clone());
            let submit_session_id = session.id().to_string();
            let submit_attachment_id = attachment.id().to_string();
            let submit_agent_id = agent_id.clone();
            let submit_task = tokio::task::spawn_blocking(move || {
                submit_client.send(&LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                    session_id: submit_session_id,
                    attachment_id: submit_attachment_id,
                    target_agent_id: Some(submit_agent_id),
                    prompt: "ipc prompt should ack while shell command is still running"
                        .to_string(),
                    attachments: Vec::new(),
                }))
            });
            tokio::pin!(slow_task);
            tokio::pin!(submit_task);
            let submit_response = tokio::time::timeout(Duration::from_secs(3), async {
                    tokio::select! {
                        submit = &mut submit_task => submit
                            .expect("prompt submit task should join")
                            .expect("prompt submit should succeed"),
                        shell = &mut slow_task => {
                            let shell_response = shell
                                .expect("slow shell task should join")
                                .expect("slow shell request should succeed");
                            panic!("prompt submit should finish before slow shell completes: {shell_response:?}");
                        }
                    }
                })
                .await
                .expect("prompt submit should respond before slow shell completes");
            assert!(matches!(
                submit_response,
                LocalDaemonResponse::PromptSubmitted { .. }
            ));

            let shell_response = slow_task
                .await
                .expect("slow shell task should join")
                .expect("slow shell request should succeed");
            assert!(matches!(
                shell_response,
                LocalDaemonResponse::ShellCommandCompleted { .. }
            ));

            let _ = shutdown_tx.send(());
            server
                .await
                .expect("server task should join")
                .expect("server should stop cleanly");
        },
    );
}

#[test]
fn malformed_request_does_not_block_followup_request() {
    run_local_ipc_async_test(
        "malformed-request-does-not-block-followup-request",
        2,
        || async {
            let config = DaemonConfig::for_tests();
            let socket_path = config.local_socket_path.clone();
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
            let app = Arc::new(TokioMutex::new(
                DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed"),
            ));
            let server_app = Arc::clone(&app);
            let server = tokio::spawn(async move {
                super::run_local_ipc_server_with_shared_app(server_app, async {
                    let _ = shutdown_rx.await;
                })
                .await
            });

            wait_for_socket(&socket_path).await;

            let mut bad_client =
                StdUnixStream::connect(&socket_path).expect("socket should accept");
            bad_client
                .set_write_timeout(Some(Duration::from_secs(1)))
                .expect("write timeout should configure");
            bad_client
                .write_all(&2_u32.to_be_bytes())
                .expect("bad frame header should write");
            bad_client.write_all(b"{").expect("bad body should write");
            bad_client
                .shutdown(Shutdown::Write)
                .expect("bad client should close write side");
            let _ =
                read_sync_frame(&mut bad_client).expect("server should answer malformed request");

            let client = LocalIpcClient::new(socket_path.clone());
            let response = client
                .send(&LocalDaemonRequest::CreateSession(
                    CreateSessionRequest::new("workspace-ipc-followup", "."),
                ))
                .expect("followup request should still succeed");
            match response {
                LocalDaemonResponse::SessionCreated { .. } => {}
                other => panic!("unexpected response: {other:?}"),
            }

            let _ = shutdown_tx.send(());
            server
                .await
                .expect("server task should join")
                .expect("server should stop cleanly");
        },
    );
}

#[test]
fn local_ipc_round_trip_exercises_workflow_run_lifecycle() {
    run_local_ipc_async_test(
        "local-ipc-round-trip-exercises-workflow-run-lifecycle",
        2,
        || async {
            let config = DaemonConfig::for_tests();
            let socket_path = config.local_socket_path.clone();
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
            let app = Arc::new(TokioMutex::new(
                DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed"),
            ));
            let server_app = Arc::clone(&app);
            let server = tokio::spawn(async move {
                super::run_local_ipc_server_with_shared_app(server_app, async {
                    let _ = shutdown_rx.await;
                })
                .await
            });

            wait_for_socket(&socket_path).await;

            let client = LocalIpcClient::new(socket_path.clone());
            let session = match client
                .send(&LocalDaemonRequest::CreateSession(
                    CreateSessionRequest::new("workspace-ipc-workflow", "."),
                ))
                .expect("session create should succeed")
            {
                LocalDaemonResponse::SessionCreated { session, .. } => session,
                other => panic!("unexpected response: {other:?}"),
            };

            let agent = match client
                .send(&LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                    session_id: session.id().to_string(),
                    alias: Some("reviewer".to_string()),
                    provider: Some("dev-stub".to_string()),
                    model: Some("default".to_string()),
                    effort: None,
                    execution_mode: None,
                    permission_level: None,
                    worktree_id: None,
                    kernel_ref: None,
                    slice_ref: None,
                    worktree_placement: None,
                    metaagent: false,
                }))
                .expect("workflow agent should spawn")
            {
                LocalDaemonResponse::AgentSpawned { agent } => agent,
                other => panic!("unexpected response: {other:?}"),
            };

            let workflow = match client
                .send(&LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                    session_id: session.id().to_string(),
                    alias: Some("review".to_string()),
                }))
                .expect("workflow create should succeed")
            {
                LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
                other => panic!("unexpected response: {other:?}"),
            };

            let node = match client
                .send(&LocalDaemonRequest::AddWorkflowNode(
                    AddWorkflowNodeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow.id().to_string(),
                        agent_id: agent.id().to_string(),
                        expected_workflow_revision: None,
                    },
                ))
                .expect("workflow node add should succeed")
            {
                LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
                other => panic!("unexpected response: {other:?}"),
            };

            let endpoint = match client
                .send(&LocalDaemonRequest::CreateWorkflowEndpoint(
                    CreateWorkflowEndpointRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow.id().to_string(),
                        entry_node_id: node.id().to_string(),
                        alias: Some("entry".to_string()),
                        expected_workflow_revision: None,
                    },
                ))
                .expect("workflow endpoint create should succeed")
            {
                LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
                other => panic!("unexpected response: {other:?}"),
            };

            match client
                .send(&LocalDaemonRequest::LaunchProviderRun(
                    LaunchProviderRunRequest {
                        session_id: session.id().to_string(),
                        agent_id: Some(agent.id().to_string()),
                        adapter_key: "dev-stub".to_string(),
                        provider: "dev-stub".to_string(),
                        account_profile: "default".to_string(),
                        model: "default".to_string(),
                        variant: None,
                        structured_endpoint: None,
                        provider_session_id: None,
                        native_tui: false,
                    },
                ))
                .expect("provider run launch should succeed")
            {
                LocalDaemonResponse::ProviderRunLaunched { .. }
                | LocalDaemonResponse::ProviderRunLaunchAccepted { .. } => {}
                other => panic!("unexpected response: {other:?}"),
            }

            let workflow_run = match client
                .send(&LocalDaemonRequest::InvokeWorkflowEndpoint(
                    InvokeWorkflowEndpointRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow.id().to_string(),
                        endpoint_ref: endpoint.id().to_string(),
                        prompt: Some("socket drill".to_string()),
                        queue_ref: None,
                        publication_invocation: None,
                    },
                ))
                .expect("workflow invoke should succeed")
            {
                LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
                other => panic!("unexpected response: {other:?}"),
            };
            assert_eq!(workflow_run.workflow_id(), workflow.id());
            assert_eq!(format!("{:?}", workflow_run.status()), "Running");

            let listed = match client
                .send(&LocalDaemonRequest::ListWorkflowRuns(
                    ListWorkflowRunsRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: Some(workflow.id().to_string()),
                    },
                ))
                .expect("workflow runs list should succeed")
            {
                LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => workflow_runs,
                other => panic!("unexpected response: {other:?}"),
            };
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].id(), workflow_run.id());

            let resolved = match client
                .send(&LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                }))
                .expect("workflow run get should succeed")
            {
                LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
                other => panic!("unexpected response: {other:?}"),
            };
            assert_eq!(resolved.id(), workflow_run.id());
            assert_eq!(format!("{:?}", resolved.status()), "Running");

            fan_out_ipc_workflow_output(&app, session.id(), "workflow-backed prompt").await;
            match client
                .send(&LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                    session_id: session.id().to_string(),
                }))
                .expect("workflow-backed prompt should complete")
            {
                LocalDaemonResponse::PromptCompleted { .. } => {}
                other => panic!("unexpected response: {other:?}"),
            }

            let completed = match client
                .send(&LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                }))
                .expect("settled workflow run should resolve")
            {
                LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
                other => panic!("unexpected response: {other:?}"),
            };
            assert!(matches!(
                completed.status(),
                WorkflowRunStatus::Completed | WorkflowRunStatus::Failed
            ));

            let second_run = match client
                .send(&LocalDaemonRequest::InvokeWorkflowEndpoint(
                    InvokeWorkflowEndpointRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow.id().to_string(),
                        endpoint_ref: endpoint.id().to_string(),
                        prompt: Some("socket drill again".to_string()),
                        queue_ref: None,
                        publication_invocation: None,
                    },
                ))
                .expect("second workflow invoke should succeed")
            {
                LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
                other => panic!("unexpected response: {other:?}"),
            };

            let cancelled = match client
                .send(&LocalDaemonRequest::CancelWorkflowRun(
                    CancelWorkflowRunRequest {
                        session_id: session.id().to_string(),
                        workflow_run_ref: second_run.id().to_string(),
                    },
                ))
                .expect("workflow run cancel should succeed")
            {
                LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } => workflow_run,
                other => panic!("unexpected response: {other:?}"),
            };
            assert_eq!(format!("{:?}", cancelled.status()), "Stopped");

            let _ = shutdown_tx.send(());
            server
                .await
                .expect("server task should join")
                .expect("server should stop cleanly");
        },
    );
}

#[test]
fn local_ipc_round_trip_routes_downstream_workflow_nodes() {
    run_local_ipc_async_test(
        "local-ipc-round-trip-routes-downstream-workflow-nodes",
        2,
        || async {
            let config = DaemonConfig::for_tests();
            let socket_path = config.local_socket_path.clone();
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
            let app = Arc::new(TokioMutex::new(
                DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed"),
            ));
            let server_app = Arc::clone(&app);
            let server = tokio::spawn(async move {
                super::run_local_ipc_server_with_shared_app(server_app, async {
                    let _ = shutdown_rx.await;
                })
                .await
            });

            wait_for_socket(&socket_path).await;

            let client = LocalIpcClient::new(socket_path.clone());
            let session = match client
                .send(&LocalDaemonRequest::CreateSession(
                    CreateSessionRequest::new("workspace-ipc-workflow-chain", "."),
                ))
                .expect("session create should succeed")
            {
                LocalDaemonResponse::SessionCreated { session, .. } => session,
                other => panic!("unexpected response: {other:?}"),
            };

            let first_agent = match client
                .send(&LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                    session_id: session.id().to_string(),
                    alias: Some("planner".to_string()),
                    provider: Some("dev-stub".to_string()),
                    model: Some("default".to_string()),
                    effort: None,
                    execution_mode: None,
                    permission_level: None,
                    worktree_id: None,
                    kernel_ref: None,
                    slice_ref: None,
                    worktree_placement: None,
                    metaagent: false,
                }))
                .expect("first workflow agent should spawn")
            {
                LocalDaemonResponse::AgentSpawned { agent } => agent,
                other => panic!("unexpected response: {other:?}"),
            };

            let second_agent = match client
                .send(&LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                    session_id: session.id().to_string(),
                    alias: Some("reviewer".to_string()),
                    provider: Some("dev-stub".to_string()),
                    model: Some("default".to_string()),
                    effort: None,
                    execution_mode: None,
                    permission_level: None,
                    worktree_id: None,
                    kernel_ref: None,
                    slice_ref: None,
                    worktree_placement: None,
                    metaagent: false,
                }))
                .expect("second workflow agent should spawn")
            {
                LocalDaemonResponse::AgentSpawned { agent } => agent,
                other => panic!("unexpected response: {other:?}"),
            };

            let workflow = match client
                .send(&LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                    session_id: session.id().to_string(),
                    alias: Some("review".to_string()),
                }))
                .expect("workflow create should succeed")
            {
                LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
                other => panic!("unexpected response: {other:?}"),
            };

            let first_node = match client
                .send(&LocalDaemonRequest::AddWorkflowNode(
                    AddWorkflowNodeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow.id().to_string(),
                        agent_id: first_agent.id().to_string(),
                        expected_workflow_revision: None,
                    },
                ))
                .expect("first workflow node add should succeed")
            {
                LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
                other => panic!("unexpected response: {other:?}"),
            };

            let duplicate_node = client
                .send(&LocalDaemonRequest::AddWorkflowNode(
                    AddWorkflowNodeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow.id().to_string(),
                        agent_id: first_agent.id().to_string(),
                        expected_workflow_revision: None,
                    },
                ))
                .expect_err("duplicate workflow node add should be rejected");
            assert!(matches!(
                duplicate_node,
                DaemonError::LocalTransport { operation: "handle local response", ref message }
                    if message.contains("already has a node for agent")
            ));

            let second_node = match client
                .send(&LocalDaemonRequest::AddWorkflowNode(
                    AddWorkflowNodeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow.id().to_string(),
                        agent_id: second_agent.id().to_string(),
                        expected_workflow_revision: None,
                    },
                ))
                .expect("second workflow node add should succeed")
            {
                LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
                other => panic!("unexpected response: {other:?}"),
            };

            match client
                .send(&LocalDaemonRequest::AddWorkflowEdge(
                    AddWorkflowEdgeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow.id().to_string(),
                        from_node_id: first_node.id().to_string(),
                        to_node_id: second_node.id().to_string(),
                        handoff_schema_ref: None,
                        validation_policy: None,
                        expected_workflow_revision: None,
                        source_side: None,
                        target_side: None,
                    },
                ))
                .expect("workflow edge add should succeed")
            {
                LocalDaemonResponse::WorkflowEdgeAdded { .. } => {}
                other => panic!("unexpected response: {other:?}"),
            }

            let endpoint = match client
                .send(&LocalDaemonRequest::CreateWorkflowEndpoint(
                    CreateWorkflowEndpointRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow.id().to_string(),
                        entry_node_id: first_node.id().to_string(),
                        alias: Some("entry".to_string()),
                        expected_workflow_revision: None,
                    },
                ))
                .expect("workflow endpoint create should succeed")
            {
                LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
                other => panic!("unexpected response: {other:?}"),
            };

            let workflow_run = match client
                .send(&LocalDaemonRequest::InvokeWorkflowEndpoint(
                    InvokeWorkflowEndpointRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow.id().to_string(),
                        endpoint_ref: endpoint.id().to_string(),
                        prompt: Some("socket chain drill".to_string()),
                        queue_ref: None,
                        publication_invocation: None,
                    },
                ))
                .expect("workflow invoke should succeed")
            {
                LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
                other => panic!("unexpected response: {other:?}"),
            };
            assert_eq!(format!("{:?}", workflow_run.status()), "Running");

            fan_out_ipc_workflow_output(&app, session.id(), "entry workflow prompt").await;
            match client
                .send(&LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                    session_id: session.id().to_string(),
                }))
                .expect("entry workflow prompt should complete")
            {
                LocalDaemonResponse::PromptCompleted { .. } => {}
                other => panic!("unexpected response: {other:?}"),
            }

            let routed =
                wait_for_ipc_workflow_run(&client, session.id(), workflow_run.id(), |run| {
                    run.status() == WorkflowRunStatus::Running
                        && run.active_node_run_id().is_some()
                        && run.node_runs().iter().any(|node_run| {
                            node_run.node_id() == second_node.id()
                                && node_run.status() == WorkflowNodeRunStatus::Running
                        })
                })
                .await;
            assert_eq!(format!("{:?}", routed.status()), "Running");
            assert_eq!(routed.node_runs().len(), 2);
            assert_eq!(
                routed.active_node_run_id(),
                Some(routed.node_runs()[1].id())
            );
            assert_eq!(routed.node_runs()[1].node_id(), second_node.id());

            fan_out_ipc_workflow_output(&app, session.id(), "downstream workflow prompt").await;
            match client
                .send(&LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                    session_id: session.id().to_string(),
                }))
                .expect("downstream workflow prompt should complete")
            {
                LocalDaemonResponse::PromptCompleted { .. } => {}
                other => panic!("unexpected response: {other:?}"),
            }

            let completed = match client
                .send(&LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                }))
                .expect("completed workflow run should resolve")
            {
                LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
                other => panic!("unexpected response: {other:?}"),
            };
            assert_eq!(format!("{:?}", completed.status()), "Completed");
            assert_eq!(completed.node_runs().len(), 2);

            let _ = shutdown_tx.send(());
            server
                .await
                .expect("server task should join")
                .expect("server should stop cleanly");
        },
    );
}

async fn wait_for_socket(socket_path: &Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);

    loop {
        if socket_path.exists() && StdUnixStream::connect(socket_path).is_ok() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for socket {}",
            socket_path.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_ipc_workflow_run(
    client: &LocalIpcClient,
    session_id: &str,
    workflow_run_id: &str,
    predicate: impl Fn(&WorkflowRun) -> bool,
) -> WorkflowRun {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let workflow_run = match client
            .send(&LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session_id.to_string(),
                workflow_run_ref: workflow_run_id.to_string(),
            }))
            .expect("workflow run should resolve while waiting")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            other => panic!("unexpected response: {other:?}"),
        };
        if predicate(&workflow_run) {
            return workflow_run;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for workflow run `{workflow_run_id}` to reach expected state"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn fan_out_ipc_workflow_output(
    app: &Arc<TokioMutex<DaemonApp>>,
    session_id: &str,
    label: &str,
) {
    let payload = serde_json::json!({
        "summary": format!("{label} completed"),
        "output": {
            "message": format!("{label} output"),
        },
    });
    let output = format!(
        "```json\n{}\n```\n",
        serde_json::to_string(&payload).expect("workflow test output should serialize")
    );
    let mut app = crate::runtime::app_lock::lock_app_instrumented(&app, "local_ipc").await;
    let provider_run_id = app
        .sessions()
        .get_session(session_id)
        .expect("session should resolve")
        .active_provider_run_id()
        .expect("provider run should be active")
        .to_string();
    app.fan_out_output(
        session_id,
        &provider_run_id,
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        Vec::new(),
        output.as_bytes(),
    );
}

async fn wait_for_output(client: &LocalIpcClient, session_id: &str, attachment_id: &str) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);

    loop {
        let response = client
            .send(&LocalDaemonRequest::PumpTerminalOutput(
                PumpTerminalOutputRequest {
                    session_id: session_id.to_string(),
                    attachment_id: attachment_id.to_string(),
                },
            ))
            .expect("output poll should succeed");
        if let LocalDaemonResponse::TerminalOutput { records } = response {
            if !records.is_empty() {
                let combined = records
                    .into_iter()
                    .flat_map(|record| record.bytes)
                    .collect::<Vec<u8>>();
                return String::from_utf8_lossy(&combined).into_owned();
            }
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for output"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
