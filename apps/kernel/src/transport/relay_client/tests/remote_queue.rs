//! Home-side queue settlement for prompts sent to a leased worker agent.

use super::support::*;
use crate::session::{
    DurablePromptDeliveryPhase, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
};
use crate::transport::relay_peer::{RelayPeerEvent, RelayProjectedCompletion};

struct RemoteQueueFixture {
    config_home: DaemonConfig,
    config_worker: DaemonConfig,
    app_home: Arc<Mutex<DaemonApp>>,
    app_worker: Arc<Mutex<DaemonApp>>,
    state_home: Arc<tokio::sync::RwLock<RelayClientState>>,
    router: Arc<crate::runtime::router::CommandRouter>,
    session_id: String,
    attachment_id: String,
    agent_id: String,
    leased_agent_id: String,
    shutdown_home: watch::Sender<bool>,
    shutdown_worker: watch::Sender<bool>,
    connector_home: tokio::task::JoinHandle<()>,
    connector_worker: tokio::task::JoinHandle<()>,
    server_shutdown: oneshot::Sender<()>,
    server_task: tokio::task::JoinHandle<()>,
}

impl RemoteQueueFixture {
    async fn start(name: &str) -> Self {
        Self::start_with_relay_timeout(name, 60_000).await
    }

    async fn start_with_relay_timeout(name: &str, relay_request_timeout_ms: u64) -> Self {
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        let registry = server.registry();
        let (server_shutdown, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });
        let relay_url = format!("ws://{}:{}", addr.ip(), addr.port());

        let mut config_worker = DaemonConfig::for_tests();
        config_worker.daemon_id = format!("{name}-worker");
        config_worker.host_machine_id = format!("{name}-worker-machine");
        config_worker.host_machine_alias = Some(format!("{name}-builder"));
        config_worker.relay_url = Some(relay_url.clone());
        config_worker.relay_token = Some("secret".to_string());
        config_worker.relay_heartbeat_ms = 50;
        config_worker.relay_request_timeout_ms = relay_request_timeout_ms;
        config_worker.accept_remote_leases = true;
        let app_worker = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
        ));
        let state_worker = app_worker.lock().await.relay_client_state();
        let (shutdown_worker, shutdown_worker_rx) = watch::channel(false);
        let connector_worker = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_worker),
            state_worker,
            shutdown_worker_rx,
        ));
        wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

        let mut config_home = DaemonConfig::for_tests();
        config_home.daemon_id = format!("{name}-home");
        config_home.host_machine_id = format!("{name}-home-machine");
        config_home.relay_url = Some(relay_url);
        config_home.relay_token = Some("secret".to_string());
        config_home.relay_heartbeat_ms = 50;
        let app_home = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
        ));
        let state_home = app_home.lock().await.relay_client_state();
        let (shutdown_home, shutdown_home_rx) = watch::channel(false);
        let connector_home = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_home),
            Arc::clone(&state_home),
            shutdown_home_rx,
        ));
        wait_for_daemon_registration(registry, &config_home.daemon_id).await;
        refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
            .await
            .expect("home remote inventory should refresh");

        let (session_id, attachment_id, agent_id, leased_agent_id) = {
            let mut app = app_home.lock().await;
            let session_id = create_test_session(&mut app, "workspace-home", "worktree-home");
            let attachment_id = attach_test_client(
                &mut app,
                &session_id,
                "home-client",
                ClientCapabilityLevel::InteractiveStructured,
            );
            // The idle stub keeps each worker turn open until the test settles it.
            let agent = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(
                    CreateAgentRequest::new(&session_id, "managed-dev-stub")
                        .with_model("native-tui-idle")
                        .with_kernel(&config_worker.daemon_id),
                )
                .expect("remote agent should spawn");
            let leased_agent_id = agent
                .remote_execution()
                .expect("agent should be leased to the worker")
                .leased_agent_id
                .clone();
            (
                session_id,
                attachment_id,
                agent.id().to_string(),
                leased_agent_id,
            )
        };
        let router = Arc::new(
            crate::runtime::router::CommandRouter::with_interactive_capacity(
                Arc::clone(&app_home),
                1,
            ),
        );
        Self {
            config_home,
            config_worker,
            app_home,
            app_worker,
            state_home,
            router,
            session_id,
            attachment_id,
            agent_id,
            leased_agent_id,
            shutdown_home,
            shutdown_worker,
            connector_home,
            connector_worker,
            server_shutdown,
            server_task,
        }
    }

    async fn dispatch(&self, command_id: &str, request: LocalDaemonRequest) -> LocalDaemonResponse {
        let command = KernelCommand::from_local_request(command_id, None, None, &request);
        self.router
            .dispatch(command, request)
            .await
            .unwrap_or_else(|error| panic!("{command_id} should succeed: {error}"))
    }

    async fn submit(&self, attachment_id: &str, prompt: &str) -> PromptSubmissionOutcome {
        let request = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
            session_id: self.session_id.clone(),
            attachment_id: attachment_id.to_string(),
            target_agent_id: Some(self.agent_id.clone()),
            prompt: prompt.to_string(),
            attachments: Vec::new(),
        });
        let LocalDaemonResponse::PromptSubmitted { outcome, .. } = self
            .dispatch(&format!("submit-{}", prompt.trim()), request)
            .await
        else {
            panic!("prompt submission should answer with PromptSubmitted");
        };
        outcome
    }

    /// Submits from a short-lived client that disconnects before its prompt
    /// runs, as one-shot CLI drivers do.
    async fn submit_and_detach(&self, client_id: &str, prompt: &str) {
        let attachment_id = attach_test_client(
            &mut *self.app_home.lock().await,
            &self.session_id,
            client_id,
            ClientCapabilityLevel::FullTerminal,
        );
        assert!(matches!(
            self.submit(&attachment_id, prompt).await,
            PromptSubmissionOutcome::Queued { .. }
        ));
        self.dispatch(
            &format!("detach-{client_id}"),
            LocalDaemonRequest::DetachFromSession(DetachFromSessionRequest { attachment_id }),
        )
        .await;
    }

    async fn home_active_prompt(&self) -> Option<PromptQueueItem> {
        self.app_home
            .lock()
            .await
            .prompt_owner_active_prompt_for_agent(&self.session_id, &self.agent_id)
            .expect("home prompt state should load")
    }

    async fn worker_active_home_prompt_id(&self) -> Option<String> {
        let mut worker = self.app_worker.lock().await;
        RemoteLeaseRuntime::new(&mut worker)
            .leased_agent_snapshot_for_test(&self.leased_agent_id)
            .expect("worker leased agent should exist")
            .active_home_prompt_id
    }

    /// Settles the worker's active turn and returns its provider run id.
    async fn complete_worker_turn(&self) -> String {
        let mut worker = self.app_worker.lock().await;
        let leased = RemoteLeaseRuntime::new(&mut worker)
            .leased_agent_snapshot_for_test(&self.leased_agent_id)
            .expect("worker leased agent should exist");
        let run_id = worker
            .providers()
            .get_run_for_agent(&leased.backing_session_id, &leased.backing_agent_id)
            .expect("worker run should exist")
            .id()
            .to_string();
        worker
            .complete_active_prompt(
                &leased.backing_session_id,
                &leased.backing_agent_id,
                Some(&run_id),
            )
            .expect("worker turn should settle");
        run_id
    }

    /// Waits until the worker runs the home prompt with this text.
    async fn wait_for_delivered_prompt(&self, text: &str) -> PromptQueueItem {
        for _ in 0..400 {
            if let Some(active) = self.home_active_prompt().await {
                if active.prompt() == text
                    && active.durable_delivery_phase()
                        == Some(DurablePromptDeliveryPhase::Delivered)
                    && self.worker_active_home_prompt_id().await.as_deref() == Some(active.id())
                {
                    return active;
                }
            }
            sleep(Duration::from_millis(25)).await;
        }
        panic!(
            "`{text}` was not delivered to the worker; home active prompt: {:?}, worker home prompt: {:?}",
            self.home_active_prompt().await,
            self.worker_active_home_prompt_id().await,
        );
    }

    async fn queued_prompt_texts(&self) -> Vec<String> {
        let app = self.app_home.lock().await;
        let session = app
            .sessions()
            .get_session(&self.session_id)
            .expect("home session should load");
        app.prompt_state_owner()
            .state_parts(&session, &self.agent_id)
            .1
            .iter()
            .map(|prompt| prompt.prompt().to_string())
            .collect()
    }

    async fn shutdown(self) {
        let _ = self.shutdown_home.send(true);
        let _ = self.shutdown_worker.send(true);
        self.connector_home
            .await
            .expect("home connector should join");
        self.connector_worker
            .await
            .expect("worker connector should join");
        let _ = self.server_shutdown.send(());
        self.server_task.await.expect("relay server should join");
    }
}

#[test]
fn remote_completion_dispatches_prompts_queued_by_detached_clients() {
    run_async_with_large_test_stack("remote-queue-completion", || async {
        let _relay_test_guard = relay_client_test_guard().await;
        let _test_home = RelayTestHome::new();
        let fixture = RemoteQueueFixture::start("remote-queue-completion").await;

        let PromptSubmissionOutcome::Started { prompt: first } = fixture
            .submit(&fixture.attachment_id, "FIRST_REMOTE_TURN\n")
            .await
        else {
            panic!("first remote prompt should start");
        };
        fixture
            .wait_for_delivered_prompt("FIRST_REMOTE_TURN\n")
            .await;
        fixture
            .submit_and_detach("one-shot-a", "QUEUED_AFTER_DETACH_A\n")
            .await;
        fixture
            .submit_and_detach("one-shot-b", "QUEUED_AFTER_DETACH_B\n")
            .await;

        // The worker finishes the first turn and reports it to the home.
        let worker_run_id = fixture.complete_worker_turn().await;
        let event = RelayPeerEvent::LeasedRuntimeProjection {
            account_copy_observations: Vec::new(),
            home_session_id: fixture.session_id.clone(),
            home_agent_id: fixture.agent_id.clone(),
            provider_run_id: worker_run_id,
            provider_run: None,
            prompts: Vec::new(),
            output_chunks: Vec::new(),
            notices: Vec::new(),
            completions: vec![RelayProjectedCompletion {
                message_id: "first-remote-turn-completed".to_string(),
                completed_at_ms: crate::session::unix_epoch_ms(),
                home_prompt_id: Some(first.id().to_string()),
                provider_termination: None,
            }],
        };
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &fixture.config_worker.relay_private_key,
            &fixture.config_home.relay_public_key,
            &serde_json::to_vec(&event).expect("projection should serialize"),
        )
        .expect("projection should encrypt");
        handle_daemon_peer_event(
            &fixture.router,
            &fixture.state_home,
            &fixture.config_worker.daemon_id,
            None,
            encrypted,
        )
        .await
        .expect("worker completion should project to the home");

        let promoted = fixture
            .wait_for_delivered_prompt("QUEUED_AFTER_DETACH_A\n")
            .await;
        assert_eq!(promoted.status(), PromptStatus::Running);
        assert_eq!(
            fixture.queued_prompt_texts().await,
            vec!["QUEUED_AFTER_DETACH_B\n".to_string()],
            "only the promoted prompt may leave the queue"
        );

        // The home keeps draining the promoted worker turn, so its completion
        // settles it and promotes the last queued prompt without a push.
        fixture.complete_worker_turn().await;
        fixture
            .wait_for_delivered_prompt("QUEUED_AFTER_DETACH_B\n")
            .await;
        assert!(fixture.queued_prompt_texts().await.is_empty());

        fixture.shutdown().await;
    });
}

#[test]
fn remote_cancel_reconciles_a_prompt_the_worker_never_started() {
    run_async_with_large_test_stack("remote-queue-cancel", || async {
        let _relay_test_guard = relay_client_test_guard().await;
        let _test_home = RelayTestHome::new();
        let fixture = RemoteQueueFixture::start("remote-queue-cancel").await;

        // Leave the home holding an admitted prompt whose worker dispatch has
        // not run yet, the state a lost or delayed dispatch leaves behind.
        let (stuck, held_dispatch) = {
            let mut app = fixture.app_home.lock().await;
            let queued = PromptQueueItem::new(
                "pending-draft:stuck",
                &fixture.attachment_id,
                &fixture.agent_id,
                "STUCK_HOME_PROMPT\n",
                PromptStatus::Queued,
            );
            app.prompt_owner_submit_prepared_prompt(&fixture.session_id, queued, true)
                .expect("stuck prompt should queue");
            let (stuck, intent) = crate::app::KernelAgentService::new(&mut app)
                .admit_next_queued_remote_prompt(&fixture.session_id, &fixture.agent_id, None)
                .expect("stuck prompt should be admitted")
                .expect("stuck prompt should become active");
            (stuck, intent.dispatch)
        };
        assert_eq!(stuck.status(), PromptStatus::Dispatching);
        assert_eq!(
            stuck.durable_delivery_phase(),
            Some(DurablePromptDeliveryPhase::Accepted)
        );
        assert_eq!(held_dispatch.prompt_id, stuck.id());
        assert!(matches!(
            fixture
                .submit(&fixture.attachment_id, "QUEUED_BEHIND_STUCK\n")
                .await,
            PromptSubmissionOutcome::Queued { .. }
        ));

        // The worker never received this prompt, so cancellation cannot ask it
        // to stop anything: the home durably records the intent and holds the
        // prompt until its dispatch observes it.
        let LocalDaemonResponse::PromptCancelled { cancellation } = fixture
            .dispatch(
                "cancel-stuck",
                LocalDaemonRequest::CancelActivePrompt(crate::local::CancelActivePromptRequest {
                    session_id: fixture.session_id.clone(),
                    attachment_id: fixture.attachment_id.clone(),
                    target_agent_id: Some(fixture.agent_id.clone()),
                }),
            )
            .await
        else {
            panic!("cancellation should answer with PromptCancelled");
        };
        assert_eq!(cancellation.prompt.id(), stuck.id());
        assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);

        // The held dispatch now runs. It must settle the cancelled prompt
        // without submitting it and hand the queue to the next prompt, so the
        // agent is not left blocked behind a turn its worker never started.
        fixture
            .router
            .runtime_state()
            .spawn_remote_prompt_dispatch(held_dispatch);
        let promoted = fixture
            .wait_for_delivered_prompt("QUEUED_BEHIND_STUCK\n")
            .await;
        assert_ne!(promoted.id(), stuck.id());
        assert!(fixture.queued_prompt_texts().await.is_empty());
        let settlement = fixture
            .app_home
            .lock()
            .await
            .operational_history_store()
            .load_prompt_settlement_event(&fixture.session_id, &fixture.agent_id, stuck.id())
            .expect("settlement history should load")
            .expect("the cancelled prompt should have a durable settlement");
        assert_eq!(
            settlement
                .metadata
                .get(crate::history::PROMPT_SETTLEMENT_STATUS_METADATA_KEY)
                .and_then(serde_json::Value::as_str),
            Some("cancelled")
        );
        {
            let mut worker = fixture.app_worker.lock().await;
            assert!(
                !RemoteLeaseRuntime::new(&mut worker)
                    .leased_prompt_receipt_recorded(&fixture.leased_agent_id, stuck.id()),
                "the cancelled prompt must never reach the worker"
            );
        }

        fixture.shutdown().await;
    });
}

mod leased_popup;
