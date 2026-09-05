use super::*;

use std::path::{Path, PathBuf};

use crate::local::{
    LocalDaemonRequest, SliceStateSaveMode, SliceStateSaveRequest, SliceStateSaveScope,
};
use crate::slice::{CreateSliceInput, SliceBackendKind, SliceDisplayMode};
use crate::DaemonConfig;

use tokio::sync::oneshot;
use tokio::time::{timeout, Instant as TokioInstant};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::{header::AUTHORIZATION, HeaderValue, StatusCode},
        Error as WebSocketError,
    },
};

fn daemon_config_for_runtime_mcp_listener(listener: &StdTcpListener) -> DaemonConfig {
    let port = listener
        .local_addr()
        .expect("runtime MCP listener should have an address")
        .port();

    let mut config = DaemonConfig::for_tests();
    config.runtime_mcp_port = port;
    config
}

#[test]
fn process_admission_scales_with_cpu_inside_bounded_limits() {
    let limit = process_inbound_request_limit();
    assert!(
        (MIN_PROCESS_INBOUND_REQUEST_LIMIT..=MAX_PROCESS_INBOUND_REQUEST_LIMIT).contains(&limit)
    );
}

#[cfg(unix)]
#[test]
fn kernel_local_auth_file_is_opened_without_following_symlinks() {
    use std::os::unix::fs::symlink;

    let root = RuntimeTransportTempDir::new("auth-symlink");
    let target = root.path().join("target");
    let token_path = root.path().join("token");
    write_private_test_file(&target, "target-token");
    symlink(&target, &token_path).expect("symlink should be created");

    let error =
        read_kernel_local_auth_token_file(token_path.to_str().expect("test path should be UTF-8"))
            .expect_err("symlinked auth file should be rejected");

    assert!(error
        .to_string()
        .contains("read kernel websocket auth file"));
    assert_eq!(
        std::fs::read_to_string(&target).expect("target should remain readable"),
        "target-token"
    );
}

#[cfg(unix)]
#[test]
fn kernel_local_auth_file_rejects_path_replacement_after_open() {
    let root = RuntimeTransportTempDir::new("auth-replacement");
    let token_path = root.path().join("token");
    let moved_path = root.path().join("opened-token");
    write_private_test_file(&token_path, "opened-token-value");
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        options.open(&token_path).expect("auth file should open")
    };
    std::fs::rename(&token_path, &moved_path).expect("opened file should move");
    write_private_test_file(&token_path, "replacement-token-value");

    let error = read_opened_kernel_local_auth_token_file(
        &mut file,
        token_path.to_str().expect("test path should be UTF-8"),
    )
    .expect_err("replaced auth path should be rejected");

    assert!(error
        .to_string()
        .contains("auth file changed while it was being consumed"));
    assert_eq!(
        std::fs::read_to_string(&token_path).expect("replacement should not be removed"),
        "replacement-token-value"
    );
    assert_eq!(
        std::fs::read_to_string(&moved_path).expect("opened file should remain isolated"),
        "opened-token-value"
    );
}

#[cfg(unix)]
#[test]
fn kernel_local_auth_file_is_consumed_from_the_validated_descriptor() {
    let root = RuntimeTransportTempDir::new("auth-consume");
    let token_path = root.path().join("token");
    write_private_test_file(&token_path, " descriptor-token \n");

    let token =
        read_kernel_local_auth_token_file(token_path.to_str().expect("test path should be UTF-8"))
            .expect("private auth file should be accepted");

    assert_eq!(token, "descriptor-token");
    assert!(!token_path.exists(), "one-shot auth file should be removed");
}

#[cfg(unix)]
#[test]
fn kernel_local_auth_file_rejects_oversized_tokens_without_consuming_them() {
    let root = RuntimeTransportTempDir::new("auth-oversized");
    let token_path = root.path().join("token");
    write_private_test_file(
        &token_path,
        &"x".repeat(MAX_KERNEL_LOCAL_AUTH_TOKEN_BYTES as usize + 1),
    );

    let error =
        read_kernel_local_auth_token_file(token_path.to_str().expect("test path should be UTF-8"))
            .expect_err("oversized auth file should be rejected");

    assert!(error.to_string().contains("bounded, single-link"));
    assert!(
        token_path.exists(),
        "rejected auth file should remain untouched"
    );
}

#[test]
fn process_admission_reserves_capacity_for_interactive_commands() {
    let admission = InboundRequestAdmission::new(10);
    let connection = Arc::new(Semaphore::new(32));
    let normal = (0..2)
        .map(|_| {
            admission
                .try_acquire(&connection, &KernelCommandPriority::Normal)
                .expect("non-interactive capacity should be available")
        })
        .collect::<Vec<_>>();
    assert!(admission
        .try_acquire(&connection, &KernelCommandPriority::Background)
        .is_err());

    let interactive = (0..8)
        .map(|_| {
            admission
                .try_acquire(&connection, &KernelCommandPriority::Interactive)
                .expect("reserved interactive capacity should remain available")
        })
        .collect::<Vec<_>>();
    assert!(admission
        .try_acquire(&connection, &KernelCommandPriority::Interactive)
        .is_err());

    drop(interactive);
    drop(normal);
}

#[test]
fn connection_admission_prevents_one_client_from_consuming_process_capacity() {
    let admission = InboundRequestAdmission::new(64);
    let connection = Arc::new(Semaphore::new(2));
    let first = admission
        .try_acquire(&connection, &KernelCommandPriority::Interactive)
        .expect("first request should enter");
    let second = admission
        .try_acquire(&connection, &KernelCommandPriority::Interactive)
        .expect("second request should enter");
    assert!(admission
        .try_acquire(&connection, &KernelCommandPriority::Interactive)
        .is_err());
    drop((first, second));
}

#[test]
fn kernel_event_writer_coalesces_event_lane_with_stable_deadline() {
    let now = TokioInstant::now();
    let mut coalescer = EventWriteCoalescer::new(33);

    assert!(coalescer.push_event("event-1", now).is_none());
    assert_eq!(coalescer.ready_at(), Some(now + Duration::from_millis(33)));
    assert!(coalescer
        .push_event("event-2", now + Duration::from_millis(10))
        .is_none());
    assert_eq!(coalescer.ready_at(), Some(now + Duration::from_millis(33)));

    assert_eq!(coalescer.drain_ready(), vec!["event-1", "event-2"]);
    assert_eq!(coalescer.ready_at(), None);
}

#[test]
fn kernel_event_writer_can_disable_event_coalescing_for_tests() {
    let now = TokioInstant::now();
    let mut coalescer = EventWriteCoalescer::new(0);

    assert_eq!(coalescer.push_event("event-1", now), Some("event-1"));
    assert_eq!(coalescer.ready_at(), None);
    assert!(coalescer.drain_ready().is_empty());
}

#[tokio::test]
async fn kernel_websocket_replies_to_ping_frames() {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let mcp_listener =
        StdTcpListener::bind("127.0.0.1:0").expect("runtime MCP listener should bind");
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(daemon_config_for_runtime_mcp_listener(&mcp_listener))
            .expect("daemon should boot"),
    ));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listeners_with_auth(
            app,
            listener,
            mcp_listener,
            None,
            async {
                let _ = shutdown_rx.await;
            },
        )
        .await
    });

    let (mut socket, _) = connect_async(format!("ws://{addr}"))
        .await
        .expect("client should connect");
    socket
        .send(Message::Ping(Vec::from("probe").into()))
        .await
        .expect("ping should send");

    let pong = timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Pong(payload))) => break payload.to_vec(),
                Some(Ok(_)) => continue,
                Some(Err(error)) => panic!("websocket read failed: {error}"),
                None => panic!("websocket closed before pong"),
            }
        }
    })
    .await
    .expect("pong should arrive");

    assert_eq!(pong, b"probe");

    let _ = socket.close(None).await;
    let _ = shutdown_tx.send(());
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server should stop")
        .expect("server task should finish")
        .expect("server should exit cleanly");
}

#[tokio::test]
async fn kernel_websocket_auth_rejects_missing_or_wrong_tokens_before_accepting_requests() {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let mcp_listener =
        StdTcpListener::bind("127.0.0.1:0").expect("runtime MCP listener should bind");
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(daemon_config_for_runtime_mcp_listener(&mcp_listener))
            .expect("daemon should boot"),
    ));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listeners_with_auth(
            app,
            listener,
            mcp_listener,
            Some(Arc::<str>::from("kernel-local-auth-sentinel")),
            async {
                let _ = shutdown_rx.await;
            },
        )
        .await
    });

    assert_unauthorized(
        connect_async(format!("ws://{addr}"))
            .await
            .expect_err("missing auth should be rejected"),
    );

    let mut wrong = format!("ws://{addr}")
        .into_client_request()
        .expect("request should build");
    wrong.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer wrong-kernel-local-auth-sentinel"),
    );
    assert_unauthorized(
        connect_async(wrong)
            .await
            .expect_err("wrong auth should be rejected"),
    );

    let mut authenticated = format!("ws://{addr}")
        .into_client_request()
        .expect("request should build");
    authenticated.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer kernel-local-auth-sentinel"),
    );
    let (mut socket, _) = connect_async(authenticated)
        .await
        .expect("authenticated client should connect");
    socket
        .send(Message::Ping(Vec::from("authenticated").into()))
        .await
        .expect("authenticated ping should send");
    let pong = timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Pong(payload))) => break payload.to_vec(),
                Some(Ok(_)) => continue,
                Some(Err(error)) => panic!("websocket read failed: {error}"),
                None => panic!("websocket closed before pong"),
            }
        }
    })
    .await
    .expect("authenticated pong should arrive");
    assert_eq!(pong, b"authenticated");

    let _ = socket.close(None).await;
    let _ = shutdown_tx.send(());
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server should stop")
        .expect("server task should finish")
        .expect("server should exit cleanly");
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn slice_state_save_acknowledgement_replays_without_a_second_dispatch() {
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let root = RuntimeTransportTempDir::new("slice-save-ack-replay");
    let bin = root.path().join("bin");
    let docker = bin.join("docker");
    let docker_log = root.path().join("docker.log");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    std::fs::write(
        &docker,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> '{}'
case "$*" in
  "info --format {{{{.DockerRootDir}}}}") printf '/tmp\n' ;;
  "inspect --size --format {{{{.SizeRw}}}} chariox-slice-save-replay") printf '1024\n' ;;
  *" du -sb /home-src") printf '1024 /home-src\n' ;;
  *" find /home-src -printf . | wc -c") printf '1\n' ;;
  *" df -B1 --output=avail /tmp") printf '107374182400\n' ;;
  "inspect -f {{{{.State.Running}}}} chariox-slice-save-replay") printf 'false\n' ;;
  cp\ *)
    destination=
    for argument in "$@"; do destination=$argument; done
    printf 'saved-home-generation' > "$destination"
    ;;
esac
exit 0
"#,
            docker_log.display()
        ),
    )
    .expect("fake Docker should write");
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700))
        .expect("fake Docker should become executable");
    let _path = RuntimeTransportPathGuard::prepend(bin);

    let mut config = DaemonConfig::for_tests();
    config.publication_control_state_root = Some(root.path().join("control"));
    config.user_config_path = root.path().join("config/chariox.toml");
    config.user_config.slices.root = Some(root.path().join("slices").display().to_string());
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should boot"),
    ));
    let slice = app
        .lock()
        .await
        .slices()
        .create(
            &config.daemon_id,
            &config.host_machine_id,
            CreateSliceInput {
                name: "save-replay".to_string(),
                backend: SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: SliceDisplayMode::Headless,
                display_backend: Default::default(),
                workspace_id: None,
                worktree_id: None,
                workspace_mount: Some("/workspace".to_string()),
                development: None,
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 1,
            },
        )
        .expect("slice should create");
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app),
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    let event_counter_path = root.path().join("transport/event-counter.json");
    let command_cache_path = event_counter_path.with_file_name("command-results.jsonl");
    let runtime = Arc::new(
        KernelTransportRuntime::new_with_persistent_event_ids(
            router.transport_health_store(),
            event_counter_path.clone(),
        )
        .expect("transport runtime should initialize"),
    );
    let command_id = "slice-save-command";
    let save_request = LocalDaemonRequest::SaveSliceState(SliceStateSaveRequest {
        slice_ref: slice.id.clone(),
        mode: Some(SliceStateSaveMode::Shutdown),
        scope: Some(SliceStateSaveScope::ThisSlice),
    });

    dispatch_transport_test_request(
        Arc::clone(&runtime),
        Arc::clone(&router),
        "first-transport-attempt",
        command_id,
        save_request.clone(),
        false,
    )
    .await;
    timeout(Duration::from_secs(5), async {
        loop {
            let cache = std::fs::read_to_string(&command_cache_path).unwrap_or_default();
            if cache.contains(command_id) && docker_commit_count(&docker_log) == 1 {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first save should complete after its response is lost");

    let same_process = dispatch_transport_test_request(
        Arc::clone(&runtime),
        Arc::clone(&router),
        "same-process-retry",
        command_id,
        save_request.clone(),
        true,
    )
    .await
    .expect("same-process retry should reply");
    let same_process_generation = slice_saved_generation(&same_process);
    assert_eq!(docker_commit_count(&docker_log), 1);

    drop(runtime);
    let restarted_runtime = Arc::new(
        KernelTransportRuntime::new_with_persistent_event_ids(
            router.transport_health_store(),
            event_counter_path,
        )
        .expect("transport command cache should reload"),
    );
    let after_restart = dispatch_transport_test_request(
        Arc::clone(&restarted_runtime),
        Arc::clone(&router),
        "restart-retry",
        command_id,
        save_request,
        true,
    )
    .await
    .expect("restart retry should reply");
    let restart_generation = slice_saved_generation(&after_restart);
    assert_eq!(restart_generation, same_process_generation);
    assert_eq!(docker_commit_count(&docker_log), 1);

    let conflicting = dispatch_transport_test_request(
        Arc::clone(&restarted_runtime),
        Arc::clone(&router),
        "conflicting-retry",
        command_id,
        LocalDaemonRequest::SaveSliceState(SliceStateSaveRequest {
            slice_ref: slice.id,
            mode: Some(SliceStateSaveMode::RestartAgents),
            scope: Some(SliceStateSaveScope::FutureSlices),
        }),
        true,
    )
    .await
    .expect("conflicting retry should reply");
    let KernelOutgoingFrame::Response { error, .. } = conflicting else {
        panic!("conflicting retry should return a response")
    };
    assert_eq!(
        error.expect("conflicting retry should fail").code,
        "duplicate_command_conflict"
    );
    assert_eq!(docker_commit_count(&docker_log), 1);

    drop(restarted_runtime);
    drop(router);
    drop(app);
    std::fs::remove_dir_all(root.path()).expect("drill artifacts should be removed");
    assert!(!root.path().exists(), "drill artifacts should stay removed");
    println!(
        "CHARIOX_SLICE_SAVE_ACK_LOSS_PROBE:{}",
        serde_json::json!({
            "schema": "chariox.slice_save_ack_loss_probe.v1",
            "sameProcessReplay": true,
            "restartReplay": true,
            "savedStateRefPreserved": true,
            "conflictingReuseRejected": true,
            "backendSaveCount": 1,
            "savedStateRef": same_process_generation.0,
            "homeArchiveGeneration": same_process_generation.1,
            "cleanupComplete": true
        })
    );
}

#[cfg(unix)]
async fn dispatch_transport_test_request(
    runtime: Arc<KernelTransportRuntime>,
    router: Arc<CommandRouter>,
    request_id: &str,
    command_id: &str,
    request: LocalDaemonRequest,
    receive_response: bool,
) -> Option<KernelOutgoingFrame> {
    let (priority_tx, mut priority_rx) = mpsc::channel(8);
    let (event_tx, _event_rx) = mpsc::channel(8);
    let outgoing = KernelOutgoingSender::new(priority_tx, event_tx);
    if !receive_response {
        priority_rx.close();
    }
    let (close_tx, _close_rx) = mpsc::unbounded_channel();
    let payload = serde_json::to_vec(&KernelIncomingFrame::Request {
        request_id: request_id.to_string(),
        command_id: Some(command_id.to_string()),
        causation_id: None,
        correlation_id: Some("slice-save-ack-loss-drill".to_string()),
        request,
    })
    .expect("transport request should encode");
    handle_incoming_payload(
        &runtime,
        &router,
        &Arc::new(Mutex::new(ConnectionState {
            subscription: None,
            watch_task: None,
        })),
        &InboundRequestAdmission::new(process_inbound_request_limit()),
        &Arc::new(Semaphore::new(CONNECTION_INBOUND_REQUEST_LIMIT)),
        &outgoing,
        &close_tx,
        &Arc::new(AtomicBool::new(false)),
        &payload,
    )
    .await;
    if !receive_response {
        return None;
    }
    timeout(Duration::from_secs(5), priority_rx.recv())
        .await
        .expect("transport response should arrive")
}

#[cfg(unix)]
fn slice_saved_generation(frame: &KernelOutgoingFrame) -> (String, String) {
    let KernelOutgoingFrame::Response {
        response, error, ..
    } = frame
    else {
        panic!("slice save should return a response")
    };
    assert!(error.is_none(), "slice save should succeed: {error:?}");
    let payload = response
        .as_ref()
        .as_ref()
        .and_then(|value| value.get("SliceStateSaved"))
        .expect("slice save payload should be present");
    let state = payload
        .get("state")
        .expect("slice save state should be present");
    (
        state["id"]
            .as_str()
            .expect("saved-state ref should be present")
            .to_string(),
        state["home_archive_path"]
            .as_str()
            .expect("saved-state archive generation should be present")
            .to_string(),
    )
}

#[cfg(unix)]
fn docker_commit_count(log: &Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("commit "))
        .count()
}

fn assert_unauthorized(error: WebSocketError) {
    match error {
        WebSocketError::Http(response) => assert_eq!(response.status(), StatusCode::UNAUTHORIZED),
        other => panic!("expected HTTP unauthorized handshake, got {other}"),
    }
}

#[cfg(unix)]
struct RuntimeTransportTempDir {
    path: PathBuf,
}

#[cfg(unix)]
impl RuntimeTransportTempDir {
    fn new(label: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "chariox-runtime-transport-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).expect("test directory should be created");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("test directory should be private");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(unix)]
impl Drop for RuntimeTransportTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(unix)]
struct RuntimeTransportPathGuard(Option<std::ffi::OsString>);

#[cfg(unix)]
impl RuntimeTransportPathGuard {
    fn prepend(directory: PathBuf) -> Self {
        let previous = std::env::var_os("PATH");
        let mut search_path = vec![directory];
        if let Some(existing) = &previous {
            search_path.extend(std::env::split_paths(existing));
        }
        std::env::set_var(
            "PATH",
            std::env::join_paths(search_path).expect("test PATH should join"),
        );
        Self(previous)
    }
}

#[cfg(unix)]
impl Drop for RuntimeTransportPathGuard {
    fn drop(&mut self) {
        match self.0.take() {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
    }
}

#[cfg(unix)]
fn write_private_test_file(path: &Path, value: &str) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .expect("private test file should be created");
    file.write_all(value.as_bytes())
        .expect("private test file should be written");
}
