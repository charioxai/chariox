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

/// Row 614: every agent run, on every provider, gets the credential of its own
/// kernel in its process environment, which the tools of its shell read.
#[test]
fn a_claude_run_launched_for_an_agent_gets_its_agent_cli_credential() {
    let _environment = crate::env_lock::lock();
    let fixture = std::env::temp_dir().join(format!("agent-cli-claude-{}", std::process::id()));
    std::fs::write(&fixture, "#!/bin/sh\nsleep 60\n").unwrap();
    std::fs::set_permissions(
        &fixture,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", &fixture);
    let mcp_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let mut config = daemon_config_for_runtime_mcp_listener(&mcp_listener);
    config.kernel_websocket_port = 43_651;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let worktree = crate::test_support::TestWorktree::new("agent-cli-claude");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    // The fixture's own Claude account is verified; no ambient login is read.
    let registry = app.provider_account_profile_registry();
    for profile in registry.list_all().unwrap() {
        crate::test_support::authenticate_provider_account(
            &registry,
            &profile.owner_user_id,
            &profile.provider,
            &profile.profile_id,
        )
        .unwrap();
    }
    let launched = app.launch_provider(
        crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            "default",
            "claude/claude-sonnet-4-6",
        )
        .with_client_interface(crate::provider::ProviderClientInterface::NativeTui)
        .with_agent_id(agent.id()),
    );
    std::env::remove_var("CHARIOX_CLAUDE_BIN");
    let run = launched.expect("the Claude run launches");
    let environment = run.process_environment();
    assert_eq!(
        environment.get("CHARIOX_KERNEL_URL").map(String::as_str),
        Some("ws://127.0.0.1:43651")
    );
    assert!(
        environment.contains_key("CHARIOX_AGENT_CLI_TOKEN"),
        "a Claude run for an agent must receive its own CLI credential"
    );
    let _ = std::fs::remove_file(&fixture);
}

fn run_for_agent(
    provider: &str,
    mode: crate::provider::AgentEndpointMode,
) -> crate::provider::RuntimeProviderRun {
    let mut request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        provider,
        provider,
        "default",
        "model",
    )
    .with_agent_id("agent-1")
    .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
        "http://127.0.0.1:43652/mcp".to_string(),
        "run-key".to_string(),
    ));
    request.agent_cli_endpoint = Some("ws://127.0.0.1:43651".to_string());
    crate::provider::RuntimeProviderRun::new(
        "run-1",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: mode,
            process_label: provider.to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    )
}

/// Row 614: the credential is the agent's own way into its kernel, whatever the
/// provider and however the kernel talks to the provider. The default Claude,
/// Codex and OpenCode launches are `External` endpoints (structured protocol); the
/// kernel's credential must reach their shells exactly like a managed run's.
#[test]
fn every_provider_launch_mode_gets_the_agent_cli_credential() {
    for provider in ["claude", "codex", "opencode"] {
        for mode in [
            crate::provider::AgentEndpointMode::Managed,
            crate::provider::AgentEndpointMode::External,
        ] {
            let environment = run_for_agent(provider, mode).process_environment();
            assert_eq!(
                environment.get("CHARIOX_KERNEL_URL").map(String::as_str),
                Some("ws://127.0.0.1:43651"),
                "{provider} {mode}"
            );
            assert!(
                environment.contains_key("CHARIOX_AGENT_CLI_TOKEN"),
                "a {provider} run with a {mode} endpoint must receive its CLI credential"
            );
        }
    }
}

/// The credential stays the run's own: different runs have different credentials,
/// and a run that is not an agent's, or has ended, has none.
#[test]
fn the_agent_cli_credential_is_the_runs_own_and_ends_with_it() {
    let external = crate::provider::AgentEndpointMode::External;
    let first = run_for_agent("claude", external).process_environment();
    let mut other = run_for_agent("claude", external);
    other.adopt_handed_over_identity("run-2".to_string(), None, None);
    let second = other.process_environment();
    assert_ne!(
        first.get("CHARIOX_AGENT_CLI_TOKEN"),
        second.get("CHARIOX_AGENT_CLI_TOKEN")
    );

    let mut ended = run_for_agent("claude", external);
    ended.mark_ended();
    assert!(!ended
        .process_environment()
        .contains_key("CHARIOX_AGENT_CLI_TOKEN"));

    let request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "claude",
        "claude",
        "default",
        "model",
    );
    let session_run = crate::provider::RuntimeProviderRun::new(
        "run-3",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: external,
            process_label: "claude".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    assert!(!session_run
        .process_environment()
        .contains_key("CHARIOX_AGENT_CLI_TOKEN"));
}

/// Row 614: the agent's shell finds `chariox` on its PATH next to the credential,
/// for an External run as for a managed one. With no `chariox` installed beside
/// the kernel, nothing changes.
#[test]
fn an_agent_run_finds_the_chariox_command_on_its_path() {
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let folder = std::env::temp_dir().join(format!("agent-cli-path-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let cli = folder.join("chariox");
    std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let previous = std::env::var_os("CHARIOX_CLI_BIN");
    std::env::set_var("CHARIOX_CLI_BIN", &cli);
    let folder = folder.canonicalize().unwrap();
    let cli = cli.canonicalize().unwrap();

    for mode in [
        crate::provider::AgentEndpointMode::Managed,
        crate::provider::AgentEndpointMode::External,
    ] {
        let environment = run_for_agent("claude", mode).process_environment();
        assert_eq!(
            environment.get("CHARIOX_CLI").map(PathBuf::from),
            Some(cli.clone()),
            "{mode}"
        );
        let path = environment.get("PATH").expect("the run names its PATH");
        assert_eq!(
            std::env::split_paths(path).next(),
            Some(folder.clone()),
            "{mode}: the folder of chariox leads the PATH"
        );
    }
    // A run that is not an agent's gets neither the credential nor the command.
    let request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "claude",
        "claude",
        "default",
        "model",
    );
    let session_run = crate::provider::RuntimeProviderRun::new(
        "run-3",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "claude".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    assert!(!session_run
        .process_environment()
        .contains_key("CHARIOX_CLI"));

    match previous {
        Some(value) => std::env::set_var("CHARIOX_CLI_BIN", value),
        None => std::env::remove_var("CHARIOX_CLI_BIN"),
    }
    let _ = std::fs::remove_dir_all(&folder);
}

/// Row 614 cells (a), (d): the credential of an agent run lists the reachable
/// kernels (`chariox kernels`) with no other step; a process outside any run, or
/// with another run's forged credential, is refused at the handshake.
#[tokio::test]
async fn an_agent_credential_lists_kernels_and_a_process_outside_any_run_is_refused() {
    let kernel = AgentCliKernel::start();
    let room = &kernel.rooms[0];
    let (mut socket, _) = connect_async(kernel.upgrade(room))
        .await
        .expect("the agent credential is admitted");
    let listed = request(
        &mut socket,
        serde_json::json!({"type": "request", "request": {"ListGatewayKernels": {}}}),
    )
    .await;
    assert!(
        listed["error"].is_null(),
        "chariox kernels was refused for the agent: {}",
        listed["error"]
    );
    socket.close(None).await.unwrap();

    let no_credential = format!("ws://{}", kernel.address)
        .into_client_request()
        .unwrap();
    let Err(tokio_tungstenite::tungstenite::Error::Http(response)) =
        connect_async(no_credential).await
    else {
        panic!("a process with no credential must be refused");
    };
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let mut forged = format!("ws://{}", kernel.address)
        .into_client_request()
        .unwrap();
    forged.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {}x", room.credential).parse().unwrap(),
    );
    let Err(tokio_tungstenite::tungstenite::Error::Http(response)) = connect_async(forged).await
    else {
        panic!("a forged credential must be refused");
    };
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    kernel.stop().await;
}

/// An install's `chariox` may be a link to the real program elsewhere (a release
/// bundle's bin/ names it `chariox`): the agent's PATH leads with the folder that
/// names the command, not the folder the link points to, or `chariox` would not
/// be found by that name.
#[test]
fn the_agents_path_leads_with_the_folder_that_names_chariox_even_when_it_is_a_link() {
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!("agent-cli-link-{}", std::process::id()));
    let bin = root.join("bin");
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    let real = elsewhere.join("chariox-launcher");
    std::fs::write(&real, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&real, bin.join("chariox")).unwrap();
    let previous = std::env::var_os("CHARIOX_CLI_BIN");
    std::env::set_var("CHARIOX_CLI_BIN", bin.join("chariox"));

    let environment =
        run_for_agent("claude", crate::provider::AgentEndpointMode::External).process_environment();
    let bin = bin.canonicalize().unwrap();
    assert_eq!(
        environment.get("CHARIOX_CLI").map(PathBuf::from),
        Some(bin.join("chariox"))
    );
    let path = environment.get("PATH").expect("the run names its PATH");
    assert_eq!(std::env::split_paths(path).next(), Some(bin));

    match previous {
        Some(value) => std::env::set_var("CHARIOX_CLI_BIN", value),
        None => std::env::remove_var("CHARIOX_CLI_BIN"),
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// The release bundle's layout: `bin/chariox` beside `bin/chariox-kernel`. The
/// kernel finds it from its own install folder, with no setting.
#[test]
fn the_kernel_finds_chariox_beside_itself_in_the_release_bundle_layout() {
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::temp_dir().join(format!("agent-cli-bundle-{}", std::process::id()));
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let program = |name: &str| {
        let path = bin.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let kernel = program("chariox-kernel");
    for sibling in [
        "chariox-relay",
        "chariox-app-package",
        "chariox-app-wasm-worker",
    ] {
        program(sibling);
    }
    assert_eq!(crate::provider::chariox_cli_beside(&kernel), None);
    let launcher = program("chariox-cli");
    assert_eq!(crate::provider::chariox_cli_beside(&kernel), Some(launcher));
    let chariox = program("chariox");
    assert_eq!(crate::provider::chariox_cli_beside(&kernel), Some(chariox));
    // A file that cannot run is not the command.
    std::fs::set_permissions(bin.join("chariox"), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        crate::provider::chariox_cli_beside(&kernel),
        Some(bin.join("chariox-cli"))
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Review P2 on #1772: on a real launch the launch preparation has already put
/// `CHARIOX_CLI` into the run's environment (with the local access socket). The
/// agent's PATH must still lead with that command's folder, or `chariox` is "not
/// found" although `CHARIOX_CLI` names it.
#[test]
fn the_path_leads_with_the_chariox_folder_when_launch_preparation_already_named_it() {
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!("agent-cli-prepared-{}", std::process::id()));
    let bin = root.join("opt-chariox-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let cli = bin.join("chariox");
    std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let bin = bin.canonicalize().unwrap();
    let cli = bin.join("chariox");

    let mut request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "claude",
        "claude",
        "default",
        "model",
    )
    .with_agent_id("agent-1")
    .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
        "http://127.0.0.1:43652/mcp".to_string(),
        "run-key".to_string(),
    ));
    request.agent_cli_endpoint = Some("ws://127.0.0.1:43651".to_string());
    for mode in [
        crate::provider::AgentEndpointMode::Managed,
        crate::provider::AgentEndpointMode::External,
    ] {
        let run = crate::provider::RuntimeProviderRun::new(
            "run-1",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: mode,
                process_label: "claude".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                // what apply_managed_provider_isolation leaves for a launch with a live access socket
                pty_env: std::collections::BTreeMap::from([
                    ("CHARIOX_CLI".to_string(), cli.display().to_string()),
                    ("PATH".to_string(), "/usr/bin:/bin".to_string()),
                ]),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        let environment = run.process_environment();
        assert_eq!(
            environment.get("CHARIOX_CLI").map(PathBuf::from),
            Some(cli.clone()),
            "{mode}: the command launch preparation selected is kept"
        );
        let path = environment.get("PATH").expect("the run names its PATH");
        assert_eq!(
            std::env::split_paths(path).next(),
            Some(bin.clone()),
            "{mode}: the folder of the prepared chariox leads the PATH"
        );
        assert!(std::env::split_paths(path).any(|folder| folder == Path::new("/usr/bin")));
    }
    let _ = std::fs::remove_dir_all(&root);
}
