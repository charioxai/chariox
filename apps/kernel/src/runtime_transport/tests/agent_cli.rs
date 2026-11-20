use super::*;

#[tokio::test]
async fn agent_cli_credentials_admit_only_their_own_room() {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mcp_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let mut config = daemon_config_for_runtime_mcp_listener(&mcp_listener);
    config.kernel_websocket_port = addr.port();
    let mut app = DaemonApp::bootstrap(config).unwrap();

    let worktree = crate::test_support::TestWorktree::new("agent-cli-rooms");
    let mut rooms = Vec::new();
    for model in ["terminal-echo-a", "terminal-echo-b"] {
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
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
        let agent_token = environment
            .get("CHARIOX_AGENT_CLI_TOKEN")
            .expect("agent provider launch must receive its own CLI credential")
            .clone();
        assert!(!serde_json::to_string(&run).unwrap().contains(&agent_token));
        rooms.push((session.id().to_string(), agent_token));
    }
    assert_ne!(rooms[0].0, rooms[1].0);
    assert!(
        rooms[0].1 != rooms[1].1,
        "each agent must get its own credential"
    );

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(run_kernel_websocket_server_on_listeners_with_auth(
        Arc::new(Mutex::new(app)),
        listener,
        mcp_listener,
        KernelLocalAuth::HostToken(Arc::from("synthetic-controller-auth")),
        async {
            let _ = shutdown_rx.await;
        },
    ));
    for (room, token) in &rooms {
        let mut request = format!("ws://{addr}").into_client_request().unwrap();
        request
            .headers_mut()
            .insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        let (mut socket, _) = connect_async(request)
            .await
            .expect("agent token is admitted");
        for (target, _) in &rooms {
            socket
                .send(Message::Text(
                    serde_json::json!({"type": "request", "request_id": target,
                    "request": {"GetSessionState": {"session_id": target}}})
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let frame = timeout(Duration::from_secs(5), async {
                loop {
                    if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                        let frame: Value = serde_json::from_str(&text).unwrap();
                        if frame["request_id"] == target.as_str() {
                            break frame;
                        }
                    }
                }
            })
            .await
            .unwrap();
            if target == room {
                assert!(frame["error"].is_null(), "{frame}");
            } else {
                assert_eq!(frame["error"]["code"], "kernel_access_denied");
            }
        }
        socket.close(None).await.unwrap();
    }
    let _ = shutdown_tx.send(());
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
