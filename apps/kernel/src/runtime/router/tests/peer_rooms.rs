//! Peers across rooms of one owner on one kernel: message and reply, steer and cancel, never a
//! card answer. Sub-agent trees keep their own rules.
use super::*;
use crate::provider::AgentPermissionLevel;

mod control;
mod kernels;
mod kernels_sender;
mod messages;
mod queued;

/// Room `home` holds the sender; room `peer_room` holds an ordinary agent and its hidden
/// sub-agent; `foreign_room` belongs to another owner.
struct Rooms {
    app: Arc<Mutex<DaemonApp>>,
    router: CommandRouter,
    home: String,
    sender: crate::agent::AgentInstance,
    sender_token: String,
    sender_prompt: String,
    peer_room: String,
    peer: crate::agent::AgentInstance,
    hidden: crate::agent::AgentInstance,
    foreign_room: String,
    foreign: crate::agent::AgentInstance,
}

fn create_room(app: &mut DaemonApp, name: &str, owner: Option<&str>) -> String {
    let mut request = CreateSessionRequest::new(name, std::env::temp_dir().to_string_lossy());
    if let Some(owner) = owner {
        request = request.with_owner_user_id(owner);
    }
    let (room, _) = crate::app::KernelSessionService::new(app)
        .create_session(request)
        .expect("room should create");
    room.id().to_string()
}

fn spawn_with_level(
    app: &mut DaemonApp,
    room: &str,
    alias: &str,
    level: AgentPermissionLevel,
) -> crate::agent::AgentInstance {
    crate::app::KernelSessionService::new(app)
        .spawn_agent(
            CreateAgentRequest::new(room, "dev-stub")
                .with_alias(alias)
                .with_permission_level_override(level),
        )
        .expect("agent should spawn")
}

/// Starts a running turn for `agent` and returns its runtime MCP token and prompt id.
fn running_turn(app: &mut DaemonApp, room: &str, agent: &str) -> (String, String) {
    let run = launch_test_provider(app, room, agent, "dev-stub", "dev-stub", "model");
    let prompt = app
        .sessions()
        .get_session(room)
        .unwrap()
        .active_prompt_for_agent(agent)
        .expect("the agent turn is running")
        .id()
        .to_string();
    (
        run.runtime_mcp_auth_token()
            .expect("runtime MCP auth")
            .to_string(),
        prompt,
    )
}

impl Rooms {
    fn new(sender_level: AgentPermissionLevel, peer_level: AgentPermissionLevel) -> Self {
        let mut config = DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let home = create_room(&mut app, "peer-home", None);
        let peer_room = create_room(&mut app, "peer-other", None);
        let foreign_room = create_room(&mut app, "peer-foreign", Some("another-owner"));
        let sender = spawn_with_level(&mut app, &home, "sender", sender_level);
        let peer = spawn_with_level(&mut app, &peer_room, "peer", peer_level);
        running_turn(&mut app, &peer_room, peer.id());
        let hidden = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&peer_room, "dev-stub")
                    .with_alias("hidden")
                    .with_parent_agent_id(peer.id()),
            )
            .expect("sub-agent should spawn");
        let foreign = spawn_with_level(
            &mut app,
            &foreign_room,
            "foreign",
            AgentPermissionLevel::Required,
        );
        let (sender_token, sender_prompt) = running_turn(&mut app, &home, sender.id());
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(app.clone(), 4);
        Self {
            app,
            router,
            home,
            sender,
            sender_token,
            sender_prompt,
            peer_room,
            peer,
            hidden,
            foreign_room,
            foreign,
        }
    }

    async fn send(&self, args: serde_json::Value) -> Result<serde_json::Value, DaemonError> {
        let mut args = args;
        args["origin_prompt_id"] = self.sender_prompt.clone().into();
        let result = self
            .router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &self.sender_token,
                crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
                args,
            )
            .await?;
        assert!(result.ok, "{}", result.payload);
        Ok(result.payload)
    }

    /// The sender promotes a message it queued.
    async fn promote(&self, message_id: &str) -> serde_json::Value {
        let result = self
            .router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &self.sender_token,
                crate::transport::runtime_tools::PROMOTE_AGENT_MESSAGE_TOOL,
                serde_json::json!({ "message_id": message_id }),
            )
            .await
            .expect("promotion answers");
        assert!(result.ok, "{}", result.payload);
        result.payload
    }

    async fn room_command(
        &self,
        token: &str,
        command: String,
    ) -> crate::transport::runtime_tools::RuntimeToolResult {
        self.router
            .dispatch_authenticated_runtime_tool_call(
                token,
                "chariox.room.run_command",
                serde_json::json!({ "command": command }),
            )
            .await
            .expect("room command answers")
    }

    async fn turn_running(&self, room: &str, agent: &str) -> bool {
        self.app
            .lock()
            .await
            .sessions()
            .get_session(room)
            .unwrap()
            .active_prompt_for_agent(agent)
            .is_some_and(|prompt| prompt.status() == PromptStatus::Running)
    }

    /// Whether the kernel started a turn for the agent's first inbox message, the way it does
    /// for an agent that is free to take it; a stopped agent keeps the message undelivered. The
    /// stub provider cannot finish that turn, so its task is the evidence.
    async fn message_was_delivered(&self, room: &str, agent: &str) -> bool {
        let message = self
            .inbox(room, agent)
            .await
            .pop()
            .expect("the message is in the inbox");
        let prompt = format!("agent-event-{agent}-{}", message.sequence);
        self.app
            .lock()
            .await
            .durable_state_store()
            .agent_tasks(Some(room), Some(agent))
            .unwrap()
            .iter()
            .any(|task| task.prompt_id == prompt)
    }

    async fn wait_for_message_delivery(&self, room: &str, agent: &str) -> bool {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !self.message_was_delivered(room, agent).await {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok()
    }

    // The abort task may have settled the turn before the fixture gets here.
    async fn settle_cancelled_turn(&self) {
        match self
            .router
            .runtime_state
            .end_agent_turn_for_tests(&self.peer_room, self.peer.id(), true)
            .await
        {
            Ok(()) | Err(DaemonError::NoActivePrompt { .. }) => {}
            Err(error) => panic!("the cancelled turn settles: {error}"),
        }
    }

    /// The owner's Stop on `agent`'s running turn, left in flight: the provider abort is not
    /// dispatched, so the prompt stays cancelling until the turn is settled.
    async fn owner_stop(&self, room: &str, agent: &str) {
        let attach = LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: room.to_string(),
            client_id: "owner-client".into(),
            capability_level: ClientCapabilityLevel::InteractiveStructured,
        });
        let command = KernelCommand::from_local_request("owner-attach", None, None, &attach);
        let LocalDaemonResponse::SessionAttached { attachment } =
            self.router.dispatch(command, attach).await.unwrap()
        else {
            panic!("expected an attachment")
        };
        self.router
            .runtime_state
            .cancel_agent_prompt_with_external_authority(room, agent, attachment.id(), None)
            .await
            .expect("the owner's Stop begins");
    }

    /// Parks the agent's task on a timer and ends its provider turn: the state a task is in after
    /// `events.yield`, with no active prompt.
    async fn park_task(&self, room: &str, agent: &str) {
        let task = self.last_task(room, agent).await;
        let now = crate::session::unix_epoch_ms();
        let wake: crate::durable_state::agent_lifecycle::AgentWake =
            serde_json::from_value(serde_json::json!({
                "id": "timer-parked", "task_id": task.task_id, "room_id": room, "agent_id": agent,
                "registration_id": "completion-timer-parked", "kind": "timer",
                "label": "wait for review", "state": "scheduled", "created_at_ms": now,
                "verified_at_ms": null, "next_due_ms": now + 3_600_000, "interval_ms": null,
                "command": [], "match_text": null, "matched_at_ms": null, "pid": null,
                "exit_code": null, "fire_count": 0, "missed_fires": 0,
                "last_fired_at_ms": null, "last_sequence": null, "last_delivery": null,
                "last_delivered_at_ms": null, "last_acknowledged_at_ms": null,
                "alerted_sequence": null
            }))
            .unwrap();
        {
            use crate::durable_state::agent_lifecycle::Operation;
            let app = self.app.lock().await;
            let store = app.durable_state_store();
            store
                .agent_lifecycle(Operation::CreateWake {
                    task: task.task_id.clone(),
                    prompt: task.prompt_id.clone(),
                    wake,
                })
                .unwrap();
            store
                .agent_lifecycle(Operation::Yield {
                    task: task.task_id.clone(),
                    prompt: task.prompt_id.clone(),
                    registrations: vec!["completion-timer-parked".into()],
                    cursor: 0,
                    deadline: Some(now + 3_600_000),
                    reason: "Waiting for review".into(),
                    now,
                })
                .unwrap_or_else(|error| panic!("{error:?}: {:?}", task.obligations));
        }
        self.router
            .runtime_state
            .end_agent_turn_for_tests(room, agent, false)
            .await
            .expect("the parked turn ends");
    }

    async fn last_task(
        &self,
        room: &str,
        agent: &str,
    ) -> crate::durable_state::agent_lifecycle::AgentTaskExecution {
        self.app
            .lock()
            .await
            .durable_state_store()
            .agent_tasks(Some(room), Some(agent))
            .unwrap()
            .pop()
            .expect("the agent has a task")
    }

    async fn inbox(
        &self,
        room: &str,
        agent: &str,
    ) -> Vec<crate::durable_state::agent_lifecycle::InboxEvent> {
        self.app
            .lock()
            .await
            .durable_state_store()
            .agent_inbox(room, agent, 0)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "message")
            .collect()
    }
}

fn run_peer_test<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
                .enable_all()
                .build()
                .expect("peer test runtime should build")
                .block_on(test());
        })
        .expect("peer test thread should spawn")
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}
