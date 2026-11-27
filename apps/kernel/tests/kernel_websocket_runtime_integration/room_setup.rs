use std::time::Duration;

use crate::support::kernel_websocket::*;
use chariox_kernel::attachment::ClientCapabilityLevel;
use chariox_kernel::local::{
    AttachToSessionRequest, DismissRoomSetupRequest, GetSessionStateRequest,
    LaunchProviderRunRequest, LocalDaemonRequest, RetryRoomSetupRequest, SubmitPromptRequest,
    SuggestSessionAliasRequest,
};
use chariox_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use chariox_kernel::session::{RoomSetupRequest, SessionAgentDefaults};
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
    start_kernel_with_config(config, listener).await
}

async fn start_kernel_with_config(
    config: DaemonConfig,
    listener: std::net::TcpListener,
) -> (
    DaemonConfig,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<Result<(), chariox_kernel::DaemonError>>,
) {
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
    open_room_with_request(socket, workspace.session_request().with_setup(setup)).await
}

async fn open_room_with_request(
    socket: &mut Socket,
    request: chariox_kernel::session::CreateSessionRequest,
) -> Room {
    let created = send_request(
        socket,
        "create-session",
        LocalDaemonRequest::CreateSession(request),
    )
    .await;
    let session = response_variant(&created, "SessionCreated")["session"].clone();
    let session_id = session["id"].as_str().unwrap().to_string();
    let agent_id = session["agents"][0]["id"].as_str().unwrap().to_string();
    let attachment_id = attach_room(socket, &session_id).await;
    Room {
        session_id,
        agent_id,
        attachment_id,
        created: session,
    }
}

/// Opens a Room whose first agent runs on the dev-stub provider, which answers no utility turns:
/// publishing fails at once, with no model call, on every machine. The placeholder provider would
/// launch the OpenCode installed there, which runs account-free models (#1605); a real provider
/// reads its model catalog while the Room is created, which is slow on a hosted runner.
async fn open_room_that_cannot_publish(
    socket: &mut Socket,
    workspace: &ExecutionWorkspace,
    setup: RoomSetupRequest,
) -> Room {
    let request = workspace
        .session_request()
        .with_setup(setup)
        .with_agent_defaults(SessionAgentDefaults::new("dev-stub"));
    open_room_with_request(socket, request).await
}

async fn attach_room(socket: &mut Socket, session_id: &str) -> String {
    let attached = send_request(
        socket,
        &format!("attach-session-{}", rand::random::<u64>()),
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.to_string(),
            client_id: "room-setup-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        }),
    )
    .await;
    response_variant(&attached, "SessionAttached")["attachment"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn launch_stub_provider(socket: &mut Socket, room: &Room) {
    let launched = send_request(
        socket,
        &format!("launch-provider-{}", rand::random::<u64>()),
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
        &format!("submit-prompt-{}", rand::random::<u64>()),
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
    let (saved, restore) = std::sync::mpsc::sync_channel(1);
    crate::run_kernel_websocket_runtime_test(async move {
        // Publishing fails the same way for every provider that cannot run it.
        let workspace = ExecutionWorkspace::new();
        let (config, shutdown, server) = start_kernel().await;
        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let room = open_room_that_cannot_publish(
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

        // A new client (including a reloaded page) reads the same failure.
        let mut other = connect_with_retry(&config.kernel_websocket_url()).await;
        let reloaded = wait_for_state(&mut other, &room, "failure on another client", |session| {
            session["room_setup"] == failed["room_setup"]
        })
        .await;
        assert_eq!(reloaded["room_setup"], failed["room_setup"]);
        drop(other);
        drop(socket);
        let _ = shutdown.send(());
        server.await.unwrap().unwrap();

        saved.send((config, room, failed, workspace)).unwrap();
    });
    // Dropping the old runtime ends all kernel tasks and releases database ownership,
    // just as a process exit does. Restart against the same durable files.
    let (config, room, failed, workspace) = restore.recv().unwrap();
    crate::run_kernel_websocket_runtime_test(async move {
        let _workspace = workspace;
        let (port, listener) = reserved_kernel_listener();
        let mut config = config;
        config.kernel_websocket_port = port;
        let (config, shutdown, server) = start_kernel_with_config(config, listener).await;
        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let restored = send_request(
            &mut socket,
            "restored",
            LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                session_id: room.session_id.clone(),
            }),
        )
        .await;
        assert_eq!(
            response_variant(&restored, "SessionState")["session"]["room_setup"],
            failed["room_setup"],
            "failed publish must survive a kernel restart"
        );

        launch_stub_provider(&mut socket, &room).await;

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

#[test]
fn rejected_git_init_start_survives_restart() {
    rejected_room_setup_progress_survives_restart("git_init", "running", false);
}

#[test]
fn rejected_git_init_result_survives_restart() {
    rejected_room_setup_progress_survives_restart("git_init", "done", false);
}

#[test]
fn rejected_publish_start_survives_restart() {
    rejected_room_setup_progress_survives_restart("publish", "running", false);
}

#[test]
fn rejected_publish_result_survives_restart() {
    rejected_room_setup_progress_survives_restart("publish", "failed", false);
}

#[test]
fn rejected_recovery_write_leaves_room_available() {
    rejected_room_setup_progress_survives_restart("publish", "failed", true);
}

fn rejected_room_setup_progress_survives_restart(
    step: &'static str,
    rejected_state: &'static str,
    reject_marker: bool,
) {
    let (saved, restore) = std::sync::mpsc::sync_channel(1);
    crate::run_kernel_websocket_runtime_test(async move {
        let workspace = ExecutionWorkspace::directory("room-setup-rejected-progress");
        let (config, shutdown, server) = start_kernel().await;
        let injection = rusqlite::Connection::open(config.durable_state_path()).unwrap();
        // Reject only this setup transition, after its initial pending state committed.
        injection
            .execute_batch(&format!(
                "CREATE TRIGGER reject_setup_progress BEFORE INSERT ON durable_room_setups
             WHEN NEW.state_json LIKE '%\"id\":\"{step}\",\"state\":\"{rejected_state}\"%'
             BEGIN SELECT RAISE(FAIL, 'injected setup progress rejection'); END;"
            ))
            .unwrap();
        if reject_marker {
            injection
                .execute_batch(
                    "CREATE TRIGGER reject_setup_storage_marker
                     BEFORE INSERT ON durable_room_setup_storage_failures
                     BEGIN SELECT RAISE(FAIL, 'injected setup marker rejection'); END;",
                )
                .unwrap();
        }
        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let room = open_room_that_cannot_publish(
            &mut socket,
            &workspace,
            RoomSetupRequest {
                git_init: step == "git_init",
                publish: true,
                ..Default::default()
            },
        )
        .await;
        let failed = wait_for_state(
            &mut socket,
            &room,
            "rejected setup progress to fail",
            |session| session["room_setup"]["steps"][0]["state"] == "failed",
        )
        .await;
        assert_eq!(failed["room_setup"]["steps"][0]["id"], step);
        let reason = failed["room_setup"]["steps"][0]["message"]
            .as_str()
            .unwrap();
        assert!(reason.contains("storage"), "{reason}");
        assert!(
            reason.contains(if step == "publish" {
                "retry"
            } else {
                "dismiss"
            }),
            "{reason}"
        );
        drop(socket);
        let _ = shutdown.send(());
        server.await.unwrap().unwrap();
        saved.send((config, room, failed, workspace)).unwrap();
    });

    let (mut config, mut room, failed, _workspace) = restore.recv().unwrap();
    let (saved_repair, restore_repair) = std::sync::mpsc::sync_channel(1);
    crate::run_kernel_websocket_runtime_test(async move {
        let (port, listener) = reserved_kernel_listener();
        config.kernel_websocket_port = port;
        let (config, shutdown, server) = start_kernel_with_config(config, listener).await;
        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let restored = session_state(&mut socket, &room).await;
        assert_eq!(restored["room_setup"], failed["room_setup"]);
        assert_eq!(restored["agents"][0]["id"], room.agent_id);
        room.attachment_id = attach_room(&mut socket, &room.session_id).await;
        launch_stub_provider(&mut socket, &room).await;
        let outcome = submit(
            &mut socket,
            &room,
            "continue after restarting with a storage failure",
        )
        .await;
        assert!(
            outcome["Started"].is_object(),
            "the recovered storage failure must release prompts: {outcome}"
        );

        let injection = rusqlite::Connection::open(config.durable_state_path()).unwrap();
        injection
            .execute_batch(
                "DROP TRIGGER reject_setup_progress;
                 DROP TRIGGER IF EXISTS reject_setup_storage_marker;",
            )
            .unwrap();
        let recovered = if step == "publish" {
            send_request(
                &mut socket,
                "retry-after-storage-repair",
                LocalDaemonRequest::RetryRoomSetup(RetryRoomSetupRequest {
                    session_id: room.session_id.clone(),
                }),
            )
            .await
        } else {
            send_request(
                &mut socket,
                "dismiss-after-storage-repair",
                LocalDaemonRequest::DismissRoomSetup(DismissRoomSetupRequest {
                    session_id: room.session_id.clone(),
                }),
            )
            .await
        };
        let updated = &response_variant(&recovered, "RoomSetupUpdated")["session"];
        let repaired = if step == "publish" {
            assert_eq!(updated["room_setup"]["steps"][0]["state"], "pending");
            wait_for_state(
                &mut socket,
                &room,
                "the publish retry to finish",
                |session| session["room_setup"]["steps"][0]["state"] == "failed",
            )
            .await
        } else {
            assert!(updated["room_setup"].is_null());
            updated.clone()
        };
        assert_ne!(repaired["room_setup"], failed["room_setup"]);
        let storage_failed: bool = injection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM durable_room_setup_storage_failures WHERE session_id = ?1)",
                [&room.session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !storage_failed,
            "repair must clear the durable failure marker"
        );
        drop(socket);
        let _ = shutdown.send(());
        server.await.unwrap().unwrap();
        saved_repair.send((config, room, repaired)).unwrap();
    });

    let (mut config, room, repaired) = restore_repair.recv().unwrap();
    crate::run_kernel_websocket_runtime_test(async move {
        let (port, listener) = reserved_kernel_listener();
        config.kernel_websocket_port = port;
        let (config, shutdown, server) = start_kernel_with_config(config, listener).await;
        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let restored = session_state(&mut socket, &room).await;
        assert_eq!(restored["room_setup"], repaired["room_setup"]);
        assert_eq!(restored["agents"][0]["id"], room.agent_id);
        drop(socket);
        let _ = shutdown.send(());
        server.await.unwrap().unwrap();
    });
}
