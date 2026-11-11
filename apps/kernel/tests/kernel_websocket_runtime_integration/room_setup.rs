use std::time::Duration;

use crate::support::kernel_websocket::*;
use chariox_kernel::attachment::ClientCapabilityLevel;
use chariox_kernel::local::{
    AttachToSessionRequest, DismissRoomSetupRequest, GetSessionStateRequest,
    LaunchProviderRunRequest, LocalDaemonRequest, RetryRoomSetupRequest, SubmitPromptRequest,
    SuggestSessionAliasRequest,
};
use chariox_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use chariox_kernel::session::RoomSetupRequest;
use chariox_kernel::{DaemonApp, DaemonConfig};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::sleep;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Room {
    session_id: String,
    agent_id: String,
    attachment_id: String,
    created: Value,
}

async fn start_kernel() -> (
    DaemonConfig,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<Result<(), chariox_kernel::DaemonError>>,
) {
    let mut config = DaemonConfig::for_tests();
    let (port, listener) = reserved_kernel_listener();
    config.kernel_websocket_port = port;
    config.runtime_mcp_port = unused_tcp_port();
    let app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");
    let (shutdown, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listener(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            listener,
            async {
                let _ = shutdown_rx.await;
            },
        )
        .await
    });
    (config, shutdown, server)
}

/// Creates a Room with `setup`, attaches and gives its first agent a running provider.
async fn create_room(
    socket: &mut Socket,
    workspace: &ExecutionWorkspace,
    setup: RoomSetupRequest,
) -> Room {
    let room = open_room(socket, workspace, setup).await;
    launch_stub_provider(socket, &room).await;
    room
}

async fn open_room(
    socket: &mut Socket,
    workspace: &ExecutionWorkspace,
    setup: RoomSetupRequest,
) -> Room {
    let created = send_request(
        socket,
        "create-session",
        LocalDaemonRequest::CreateSession(workspace.session_request().with_setup(setup)),
    )
    .await;
    let session = response_variant(&created, "SessionCreated")["session"].clone();
    let session_id = session["id"].as_str().unwrap().to_string();
    let agent_id = session["agents"][0]["id"].as_str().unwrap().to_string();
    let attached = send_request(
        socket,
        "attach-session",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "room-setup-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        }),
    )
    .await;
    let attachment_id = response_variant(&attached, "SessionAttached")["attachment"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    Room {
        session_id,
        agent_id,
        attachment_id,
        created: session,
    }
}

async fn launch_stub_provider(socket: &mut Socket, room: &Room) {
    let launched = send_request(
        socket,
        "launch-provider",
        LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: room.session_id.clone(),
            agent_id: Some(room.agent_id.clone()),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
            structured_endpoint: None,
            provider_session_id: None,
            native_checkpoint_binding: None,
            native_tui: false,
        }),
    )
    .await;
    let run_id = provider_run_id_from_launch_response(&launched);
    wait_for_provider_run_state(socket, &run_id, "Running").await;
}

async fn session_state(socket: &mut Socket, room: &Room) -> Value {
    let state = send_request(
        socket,
        "state",
        LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: room.session_id.clone(),
        }),
    )
    .await;
    response_variant(&state, "SessionState")["session"].clone()
}

async fn wait_for_state(
    socket: &mut Socket,
    room: &Room,
    what: &str,
    done: impl Fn(&Value) -> bool,
) -> Value {
    for _ in 0..200 {
        let session = session_state(socket, room).await;
        if done(&session) {
            return session;
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn submit(socket: &mut Socket, room: &Room, prompt: &str) -> Value {
    let response = send_request(
        socket,
        "submit-prompt",
        LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: room.session_id.clone(),
            attachment_id: room.attachment_id.clone(),
            target_agent_id: Some(room.agent_id.clone()),
            prompt: prompt.to_string(),
            attachments: Vec::new(),
        }),
    )
    .await;
    response_variant(&response, "PromptSubmitted")["outcome"].clone()
}

async fn suggest(socket: &mut Socket, workspace: &ExecutionWorkspace, request_id: &str) -> String {
    let response = send_request(
        socket,
        request_id,
        LocalDaemonRequest::SuggestSessionAlias(SuggestSessionAliasRequest {
            workspace_id: workspace.path().to_string(),
        }),
    )
    .await;
    response_variant(&response, "SessionAliasSuggested")["alias"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn git_init_runs_in_the_background_and_the_setup_clears() {
    crate::run_kernel_websocket_runtime_test(async {
        let workspace = ExecutionWorkspace::directory("room-setup-git-init");
        let (config, shutdown, server) = start_kernel().await;
        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let room = create_room(
            &mut socket,
            &workspace,
            RoomSetupRequest {
                git_init: true,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            room.created["room_setup"],
            json!({"steps": [{"id": "git_init", "state": "pending"}]}),
            "the Room is created with its setup already visible"
        );
        wait_for_state(&mut socket, &room, "the setup to clear", |session| {
            session.get("room_setup").is_none()
        })
        .await;
        assert!(std::path::Path::new(workspace.path()).join(".git").exists());
        assert!(submit(&mut socket, &room, "after setup").await["Started"].is_object());
        let _ = shutdown.send(());
        server.await.unwrap().unwrap();
    });
}

#[test]
fn a_failed_step_holds_prompts_until_dismissed_then_they_start() {
    crate::run_kernel_websocket_runtime_test(async {
        // A folder that is not a repository cannot get a worktree.
        let workspace = ExecutionWorkspace::directory("room-setup-worktree");
        let (config, shutdown, server) = start_kernel().await;
        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let room = create_room(
            &mut socket,
            &workspace,
            RoomSetupRequest {
                new_worktree: true,
                ..Default::default()
            },
        )
        .await;
        let failed = wait_for_state(&mut socket, &room, "the worktree step to fail", |session| {
            session["room_setup"]["steps"][0]["state"] == "failed"
        })
        .await;
        let message = failed["room_setup"]["steps"][0]["message"]
            .as_str()
            .unwrap();
        assert!(
            message.contains("dismiss"),
            "the message says what to do next: {message}"
        );

        let queued = submit(&mut socket, &room, "wait for the worktree").await;
        assert!(
            queued["Queued"].is_object(),
            "prompts wait while the setup is failed: {queued}"
        );

        let dismissed = send_request(
            &mut socket,
            "dismiss",
            LocalDaemonRequest::DismissRoomSetup(DismissRoomSetupRequest {
                session_id: room.session_id.clone(),
            }),
        )
        .await;
        assert!(response_variant(&dismissed, "RoomSetupUpdated")["session"]
            .get("room_setup")
            .is_none());
        let started = wait_for_state(
            &mut socket,
            &room,
            "the queued prompt to start",
            |session| session["prompt_states"][&room.agent_id]["active_prompt"].is_object(),
        )
        .await;
        assert!(started["prompt_states"][&room.agent_id]["active_prompt"]
            .to_string()
            .contains("wait for the worktree"));
        let _ = shutdown.send(());
        server.await.unwrap().unwrap();
    });
}

#[test]
fn git_init_refuses_a_folder_that_is_already_a_repository() {
    crate::run_kernel_websocket_runtime_test(async {
        let workspace = ExecutionWorkspace::new();
        let (config, shutdown, server) = start_kernel().await;
        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let room = create_room(
            &mut socket,
            &workspace,
            RoomSetupRequest {
                git_init: true,
                ..Default::default()
            },
        )
        .await;
        let failed = wait_for_state(&mut socket, &room, "git init to fail", |session| {
            session["room_setup"]["steps"][0]["state"] == "failed"
        })
        .await;
        assert!(failed["room_setup"]["steps"][0]["message"]
            .as_str()
            .unwrap()
            .contains("already a Git repository"));
        let _ = shutdown.send(());
        server.await.unwrap().unwrap();
    });
}

#[test]
fn retrying_when_nothing_failed_is_refused() {
    crate::run_kernel_websocket_runtime_test(async {
        let workspace = ExecutionWorkspace::directory("room-setup-retry");
        let (config, shutdown, server) = start_kernel().await;
        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let room = create_room(&mut socket, &workspace, RoomSetupRequest::default()).await;
        send_frame(
            &mut socket,
            json!({"type": "request", "request_id": "retry", "request": LocalDaemonRequest::RetryRoomSetup(
                RetryRoomSetupRequest { session_id: room.session_id.clone() }
            )}),
        )
        .await;
        let refused = wait_for_error_response(&mut socket, "retry").await;
        assert!(
            refused.to_string().contains("nothing to retry"),
            "{refused}"
        );
        let _ = shutdown.send(());
        server.await.unwrap().unwrap();
    });
}

#[test]
fn the_suggested_name_is_what_the_next_room_gets() {
    crate::run_kernel_websocket_runtime_test(async {
        let workspace = ExecutionWorkspace::directory("room-setup-alias");
        let (config, shutdown, server) = start_kernel().await;
        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let alias = suggest(&mut socket, &workspace, "suggest-before").await;
        assert!(alias.ends_with("-1"), "{alias}");
        let room = create_room(&mut socket, &workspace, RoomSetupRequest::default()).await;
        assert_eq!(room.created["alias"], alias.as_str());
        let next = suggest(&mut socket, &workspace, "suggest-after").await;
        assert!(next.ends_with("-2"), "{alias} then {next}");
        let _ = shutdown.send(());
        server.await.unwrap().unwrap();
    });
}

#[test]
fn a_failed_publish_never_holds_prompts_and_can_be_retried() {
    crate::run_kernel_websocket_runtime_test(async {
        // The stub provider answers no utility turns, so publishing fails the same way for
        // every provider that cannot run it.
        let workspace = ExecutionWorkspace::new();
        let (config, shutdown, server) = start_kernel().await;
        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let room = open_room(
            &mut socket,
            &workspace,
            RoomSetupRequest {
                publish: true,
                ..Default::default()
            },
        )
        .await;
        let failed = wait_for_state(&mut socket, &room, "publishing to fail", |session| {
            session["room_setup"]["steps"][0]["state"] == "failed"
        })
        .await;
        assert_eq!(failed["room_setup"]["steps"][0]["id"], "publish");
        let message = failed["room_setup"]["steps"][0]["message"]
            .as_str()
            .unwrap();
        assert_eq!(message, "The agent could not start publishing");

        launch_stub_provider(&mut socket, &room).await;
        assert!(submit(&mut socket, &room, "work goes on").await["Started"].is_object());

        let retried = send_request(
            &mut socket,
            "retry",
            LocalDaemonRequest::RetryRoomSetup(RetryRoomSetupRequest {
                session_id: room.session_id.clone(),
            }),
        )
        .await;
        let step =
            &response_variant(&retried, "RoomSetupUpdated")["session"]["room_setup"]["steps"][0];
        assert_ne!(
            step["state"], "failed",
            "the retry runs the step again: {step}"
        );
        wait_for_state(&mut socket, &room, "the retry to fail again", |session| {
            session["room_setup"]["steps"][0]["state"] == "failed"
        })
        .await;
        let _ = shutdown.send(());
        server.await.unwrap().unwrap();
    });
}
