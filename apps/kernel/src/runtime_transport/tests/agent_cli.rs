use super::*;

struct AgentRoom {
    session_id: String,
    agent_id: String,
    attachment_id: String,
    credential: String,
}

struct AgentCliKernel {
    app: Arc<Mutex<DaemonApp>>,
    address: std::net::SocketAddr,
    shutdown: oneshot::Sender<()>,
    server: JoinHandle<Result<(), DaemonError>>,
    rooms: Vec<AgentRoom>,
    _worktree: crate::test_support::TestWorktree,
}

impl AgentCliKernel {
    fn start() -> Self {
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mcp_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let mut config = daemon_config_for_runtime_mcp_listener(&mcp_listener);
        config.kernel_websocket_port = address.port();
        config.room_agent_tools = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();

        let worktree = crate::test_support::TestWorktree::new("agent-cli-rooms");
        let mut rooms = Vec::new();
        for model in ["terminal-echo-a", "terminal-echo-b"] {
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(worktree.session_request())
                .unwrap();
            let attachment = crate::app::KernelSessionService::new(&mut app)
                .attach(crate::attachment::AttachRequest::new(
                    session.id(),
                    format!("agent-cli-{model}"),
                    crate::attachment::ClientCapabilityLevel::InteractiveStructured,
                ))
                .unwrap();
            let run = app
                .launch_provider(
                    crate::provider::LaunchProviderRequest::new(
                        session.id(),
                        "dev-stub",
                        "dev-stub",
                        "default",
                        model,
                    )
                    .with_agent_id(agent.id()),
                )
                .unwrap();
            let environment = run.process_environment();
            assert!(!environment.contains_key("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN"));
            assert!(!environment.contains_key("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE"));
            let credential = environment
                .get("CHARIOX_AGENT_CLI_TOKEN")
                .expect("agent provider launch must receive its own CLI credential")
                .clone();
            assert!(!serde_json::to_string(&run).unwrap().contains(&credential));
            rooms.push(AgentRoom {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                attachment_id: attachment.id().to_string(),
                credential,
            });
        }
        assert_ne!(rooms[0].session_id, rooms[1].session_id);
        assert!(
            rooms[0].credential != rooms[1].credential,
            "each agent must get its own credential"
        );

        let app = Arc::new(Mutex::new(app));
        let (shutdown, shutdown_rx) = oneshot::channel();
        let server = tokio::spawn(run_kernel_websocket_server_on_listeners_with_auth(
            app.clone(),
            listener,
            mcp_listener,
            KernelLocalAuth::HostToken(Arc::from("synthetic-controller-auth")),
            async {
                let _ = shutdown_rx.await;
            },
        ));
        Self {
            app,
            address,
            shutdown,
            server,
            rooms,
            _worktree: worktree,
        }
    }

    fn upgrade(
        &self,
        room: &AgentRoom,
    ) -> tokio_tungstenite::tungstenite::handshake::client::Request {
        let mut request = format!("ws://{}", self.address)
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", room.credential).parse().unwrap(),
        );
        request
    }

    async fn stop(self) {
        let _ = self.shutdown.send(());
        timeout(Duration::from_secs(5), self.server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn request(socket: &mut Socket, mut frame: Value) -> Value {
    let request_id = format!("agent-cli-{:016x}", rand::random::<u64>());
    frame["request_id"] = request_id.clone().into();
    socket
        .send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                let frame: Value = serde_json::from_str(&text).unwrap();
                if frame["request_id"] == request_id.as_str() {
                    return frame;
                }
            }
        }
    })
    .await
    .expect("agent CLI request should receive a response")
}

#[tokio::test]
async fn agent_cli_credentials_allow_authorized_peer_requests_only() {
    let kernel = AgentCliKernel::start();
    for (room, peer) in [
        (&kernel.rooms[0], &kernel.rooms[1]),
        (&kernel.rooms[1], &kernel.rooms[0]),
    ] {
        let (mut socket, _) = connect_async(kernel.upgrade(room))
            .await
            .expect("agent token is admitted");
        let own = request(
            &mut socket,
            serde_json::json!({"type": "request", "request": {
                "GetSessionState": {"session_id": room.session_id},
            }}),
        )
        .await;
        assert!(
            own["error"].is_null(),
            "own Room request denied: {}",
            own["error"]
        );
        let peer_state = request(
            &mut socket,
            serde_json::json!({"type": "request", "request": {
                "GetSessionState": {"session_id": peer.session_id},
            }}),
        )
        .await;
        assert_eq!(peer_state["error"]["code"], "room_tool_refused");
        let peer_deadline = request(
            &mut socket,
            serde_json::json!({"type": "request", "request": {
                "GetAgentDeadline": {"session_id": peer.session_id, "agent_id": peer.agent_id},
            }}),
        )
        .await;
        assert!(
            peer_deadline["error"].is_null(),
            "authorized peer deadline denied: {}",
            peer_deadline["error"]
        );
        assert!(peer_deadline["response"].get("AgentDeadline").is_some());
        socket.close(None).await.unwrap();
    }
    kernel.stop().await;
}

#[tokio::test]
async fn agent_cli_credentials_expire_an_idle_subscription_when_the_run_ends() {
    let kernel = AgentCliKernel::start();
    let room = &kernel.rooms[0];
    let (mut socket, _) = connect_async(kernel.upgrade(room)).await.unwrap();
    let subscribed = request(
        &mut socket,
        serde_json::json!({
            "type": "subscribe", "session_id": room.session_id, "attachment_id": room.attachment_id,
        }),
    )
    .await;
    assert_eq!(subscribed["response"]["ok"], true);

    kernel
        .app
        .lock()
        .await
        .end_agent_provider_run(&room.session_id, &room.agent_id)
        .unwrap();
    assert_unauthorized(
        connect_async(kernel.upgrade(room))
            .await
            .expect_err("ended run must fail a fresh handshake"),
    );
    timeout(Duration::from_secs(3), async {
        while let Some(Ok(message)) = socket.next().await {
            if matches!(message, Message::Close(_)) {
                return;
            }
        }
    })
    .await
    .expect("an ended run must lose its idle subscription without another request");
    kernel.stop().await;
}
