//! Peers across rooms of one owner on one kernel: message and reply, steer and cancel, never a
//! card answer. Sub-agent trees keep their own rules.
use super::*;
use crate::provider::AgentPermissionLevel;

mod messages;

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
