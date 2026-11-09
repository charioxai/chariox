//! MP-08 / MP-09 / MP-10 / MP-11: one worker, peer or goal check that does not answer never holds
//! the lifecycle sweep, so unrelated agents keep being supervised. The fixture is a leased child
//! whose worker answers pings and takes cancellations without ever answering them.
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, ExecutionState, Operation, Outcome, Registration,
};

#[path = "sweep_stall_tests/cancel.rs"]
mod cancel;
#[path = "sweep_stall_tests/pump.rs"]
mod pump;
#[path = "sweep_stall_tests/whole_sweep.rs"]
mod whole_sweep;
#[path = "sweep_stall_tests/workflow.rs"]
mod workflow;

/// A sweep that waits on nothing returns in well under a second; this is the room for a slow builder.
const SWEEP_BOUND: std::time::Duration = std::time::Duration::from_secs(10);

/// The worker takes every message and answers the relay lookups and, when asked to, pings. It never
/// answers a leased prompt cancellation.
async fn serve_worker_that_never_cancels(
    listener: TcpListener,
    worker_public_key: String,
    worker_private_key: String,
    home_public_key: String,
    answers_ping: bool,
) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let (worker_public_key, worker_private_key, home_public_key) = (
            worker_public_key.clone(),
            worker_private_key.clone(),
            home_public_key.clone(),
        );
        tokio::spawn(async move {
            let Ok(mut socket) = accept_async(stream).await else {
                return;
            };
            while let Some(Ok(message)) = socket.next().await {
                let Ok(text) = message.to_text() else {
                    continue;
                };
                let Ok(envelope) = serde_json::from_str::<RelayEnvelope>(text) else {
                    continue;
                };
                match envelope {
                    RelayEnvelope::ClientMetadataRequest { request_id, .. } => {
                        let presence = serde_json::from_value(serde_json::json!({
                            "kernel_id": WORKER_ID,
                            "machine_id": "worker-machine-sweep-stall",
                            "public_key": worker_public_key,
                        }))
                        .unwrap();
                        send_temporary_relay_envelope(
                            &mut socket,
                            RelayEnvelope::ClientMetadataResponse {
                                request_id,
                                machines: None,
                                kernels: None,
                                kernel: Some(presence),
                                error: None,
                            },
                        )
                        .await;
                    }
                    envelope @ RelayEnvelope::DaemonPeerRequest { .. } => {
                        let (request_id, _, request) =
                            decode_peer_request(envelope, &worker_private_key);
                        let RelayPeerRequest::Ping { value } = request else {
                            continue;
                        };
                        if !answers_ping {
                            continue;
                        }
                        let response = RelayPeerResponse::Pong {
                            value,
                            daemon_id: WORKER_ID.to_string(),
                            relay_peer_protocol_version: Some(
                                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                            ),
                        };
                        let encrypted_response =
                            crate::transport::relay_crypto::encrypt_payload_for_peer(
                                &worker_private_key,
                                &home_public_key,
                                &serde_json::to_vec(&response).unwrap(),
                            )
                            .unwrap();
                        send_temporary_relay_envelope(
                            &mut socket,
                            RelayEnvelope::DaemonPeerResponse {
                                request_id,
                                from_daemon_id: WORKER_ID.to_string(),
                                encrypted_response: Some(encrypted_response),
                                error: None,
                            },
                        )
                        .await;
                    }
                    _ => {}
                }
            }
        });
    }
}

/// Waits until `condition` holds, without a fixed pause; `None` when it never does.
async fn until(condition: impl Fn() -> bool) -> Option<()> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .ok()
}

/// A room with a parent and a leased child whose turn runs on the worker.
struct Stalled {
    runtime: KernelRuntimeState,
    room: String,
    owner: String,
    parent: String,
    child: String,
    child_task: AgentTaskExecution,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Stalled {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Stalled {
    async fn new(answers_ping: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{}", listener.local_addr().unwrap()));
        config.relay_token = Some("sweep-stall-test-token".to_string());
        // The production default is 60 s; a sweep that waits for the worker waits this long.
        config.relay_request_timeout_ms = 30_000;
        config.room_agent_tools = true;
        let home_public_key = config.relay_public_key.clone();
        let worker_config = crate::config::DaemonConfig::for_tests();
        let server = tokio::spawn(serve_worker_that_never_cancels(
            listener,
            worker_config.relay_public_key.clone(),
            worker_config.relay_private_key.clone(),
            home_public_key,
            answers_ping,
        ));

        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (session, parent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "sweep-stall-workspace",
                std::env::current_dir()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ))
            .unwrap();
        crate::test_support::admit_room_test_turn(&mut app, session.id(), parent.id());
        let child = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_parent_agent_id(parent.id()),
            )
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "sweep-stall-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        app.agents
            .bind_remote_execution(
                child.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: WORKER_ID.to_string(),
                    worker_machine_id: "worker-machine-sweep-stall".to_string(),
                    execution_lease_id: "worker-lease-sweep-stall".to_string(),
                    leased_agent_id: LEASED_AGENT_ID.to_string(),
                    active_worker_provider_run_id: Some(WORKER_RUN_ID.to_string()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                    worker_worktree_id: None,
                },
            )
            .unwrap();
        let crate::session::PromptSubmissionOutcome::Started { prompt } = app
            .prompt_owner_submit_prepared_prompt(
                session.id(),
                crate::session::PromptQueueItem::new(
                    "home-prompt-sweep-stall",
                    attachment.id(),
                    child.id(),
                    "run on the worker",
                    crate::session::PromptStatus::Queued,
                ),
                false,
            )
            .unwrap()
        else {
            panic!("the leased child's turn should be running");
        };
        app.mark_active_prompt_delivery(
            session.id(),
            child.id(),
            prompt.id(),
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(WORKER_RUN_ID.to_string()),
            None,
        )
        .unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let Outcome::Task(child_task) = runtime
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Begin {
                owner: session.owner_user_id().into(),
                room: session.id().into(),
                agent: child.id().into(),
                prompt: prompt.id().into(),
                run: Some(crate::provider::projected_leased_provider_run_id(
                    LEASED_AGENT_ID,
                    WORKER_RUN_ID,
                )),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap()
        else {
            panic!("child task");
        };
        Self {
            runtime,
            room: session.id().into(),
            owner: session.owner_user_id().into(),
            parent: parent.id().into(),
            child: child.id().into(),
            child_task,
            server,
        }
    }

    fn store(&self) -> &crate::durable_state::DurableKernelStateStore {
        &self.runtime.owned.durable_state_store
    }

    fn provider_run(&self) -> String {
        crate::provider::projected_leased_provider_run_id(LEASED_AGENT_ID, WORKER_RUN_ID)
    }

    /// The parent's turn, begun once however many parts of a test need it.
    fn parent_task(&self) -> AgentTaskExecution {
        if let Some(task) = self
            .store()
            .agent_tasks(Some(&self.room), Some(&self.parent))
            .unwrap()
            .pop()
        {
            return task;
        }
        let Outcome::Task(task) = self
            .store()
            .agent_lifecycle(Operation::Begin {
                owner: self.owner.clone(),
                room: self.room.clone(),
                agent: self.parent.clone(),
                prompt: "parent-turn".into(),
                run: None,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap()
        else {
            panic!("parent task");
        };
        task
    }

    /// The leased child's task ends cancelled while its worker still holds the run.
    fn cancel_child_task(&self) {
        self.store()
            .agent_lifecycle(Operation::CancelTask {
                task: self.child_task.task_id.clone(),
                owner: self.owner.clone(),
                revision: self.child_task.revision,
            })
            .unwrap();
    }

    /// The child's goal turn ended and its completion check ran out of its bound.
    fn expire_child_goal_check(&self) {
        let started = crate::session::unix_epoch_ms()
            - crate::durable_state::agent_lifecycle::goals::CHECK_BOUND_MS
            - 1_000;
        self.store()
            .agent_lifecycle(Operation::StartGoal {
                task: self.child_task.task_id.clone(),
                owner: self.owner.clone(),
                objective: "Finish the leased work".into(),
                minutes: 1,
                now: started,
            })
            .unwrap();
        self.store()
            .agent_lifecycle(Operation::Settle {
                room: self.room.clone(),
                agent: self.child.clone(),
                prompt: self.child_task.prompt_id.clone(),
                run: self.provider_run(),
                has_answer: true,
                cancelled: false,
                now: started,
            })
            .unwrap();
    }

    /// The parent started a workflow run that failed; the child's task belongs to that run.
    fn fail_workflow_run_of_child_task(&self) {
        let parent = self.parent_task();
        let now = crate::session::unix_epoch_ms();
        self.store()
            .agent_lifecycle(Operation::RegisterObligation {
                owner: self.owner.clone(),
                room: self.room.clone(),
                agent: self.parent.clone(),
                prompt: parent.prompt_id.clone(),
                run: None,
                id: "stall-workflow".into(),
                kind: "workflow_run".into(),
                resource: Some("failed-run".into()),
                now,
            })
            .unwrap();
        self.store()
            .agent_lifecycle(Operation::DispatchReceipt {
                id: "stall-workflow".into(),
                accepted: true,
                resource: Some("failed-run".into()),
            })
            .unwrap();
        self.store()
            .agent_lifecycle(Operation::BindWorkflow {
                task: self.child_task.task_id.clone(),
                prompt: self.child_task.prompt_id.clone(),
                run: "failed-run".into(),
                node: "node-run-stall".into(),
            })
            .unwrap();
        let mut run = crate::session::WorkflowRun::new(
            "failed-run",
            "workflow",
            "endpoint",
            "node",
            None,
            None,
            vec![crate::session::WorkflowNodeRun::new(
                "node-run-stall",
                "node".to_owned(),
                "agent",
                0,
                crate::session::WorkflowNodeRunStatus::Failed,
            )],
            vec![],
        );
        run.set_status(crate::session::WorkflowRunStatus::Failed);
        let mut session = self
            .runtime
            .owned
            .session_store
            .get_session(&self.room)
            .unwrap();
        session.create_workflow_run(run);
        self.runtime
            .owned
            .session_store
            .write()
            .restore_session(session.clone());
        self.store()
            .persist_workflow_runtime_transition(&session, "sweep stall fixture")
            .unwrap();
    }

    /// The parent follows an agent on another kernel and Cloud's kernel directory never answers.
    /// Keep the returned listener alive: its backlog takes the connection and nothing reads it.
    fn follow_a_peer_while_the_cloud_directory_is_silent(&self) -> std::net::TcpListener {
        let parent = self.parent_task();
        self.store()
            .agent_lifecycle(Operation::Subscribe {
                task: parent.task_id.clone(),
                prompt: parent.prompt_id.clone(),
                registration: Registration {
                    id: "follow-silent-directory".into(),
                    task_id: parent.task_id,
                    source_id: "other-kernel/other-room/other-agent".into(),
                    obligation_id: None,
                    source_cursor: 0,
                    live: true,
                    peer: true,
                },
            })
            .unwrap();
        let directory = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut config = self.runtime.owned.config_projection.snapshot();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url: format!("http://{}", directory.local_addr().unwrap()),
            user_id: self.owner.clone(),
            kernel_credential: Some("sweep-stall-kernel-credential".into()),
            ..Default::default()
        });
        self.runtime.owned.config_projection.update(config);
        directory
    }

    /// An agent that has nothing to do with the stuck one: a spawn of an agent that is gone is
    /// released by the sweep.
    fn bystander(&self) {
        self.store()
            .agent_lifecycle(Operation::RegisterObligation {
                owner: self.owner.clone(),
                room: self.room.clone(),
                agent: "bystander".into(),
                prompt: "bystander-turn".into(),
                run: None,
                id: "bystander-spawn".into(),
                kind: "delegate".into(),
                resource: Some("gone-child".into()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
    }

    fn bystander_status(&self) -> String {
        self.store()
            .agent_tasks(Some(&self.room), Some("bystander"))
            .unwrap()
            .pop()
            .unwrap()
            .obligations[0]
            .status
            .clone()
    }

    /// The sweep returns while the stuck work is still stuck, and has supervised the bystander.
    async fn sweep_returns_and_supervises_the_bystander(&self, stuck: &str) {
        tokio::time::timeout(SWEEP_BOUND, self.runtime.sweep_agent_lifecycle_for_test())
            .await
            .unwrap_or_else(|_| panic!("the sweep waits on {stuck}"))
            .unwrap();
        assert_eq!(
            self.bystander_status(),
            "cancelled",
            "the sweep left an unrelated agent unsupervised while {stuck}"
        );
    }

    fn child_task_state(&self) -> ExecutionState {
        self.store()
            .agent_task(&self.child_task.task_id)
            .unwrap()
            .unwrap()
            .state
    }
}
