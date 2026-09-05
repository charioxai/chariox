use super::*;

use std::path::{Path, PathBuf};

use crate::local::{
    CreateSliceRequest, LocalDaemonRequest, ReleaseRoomEnvironmentInputRequest,
    RequestRoomEnvironmentInputTakeoverRequest, RestoreSliceBackupRequest, SliceCreateBase,
    SliceStateSaveMode, SliceStateSaveRequest, SliceStateSaveScope,
};
use crate::session::{
    agent_environment_actor_id, human_environment_actor_id, ActionAdmission, CanonicalViewport,
    CreateSessionRequest, EnvironmentActionRequest, EnvironmentActionTerminal, EnvironmentActor,
    EnvironmentActorKind, EnvironmentEventKind, EnvironmentLifecycle, EnvironmentReplay,
    InputTarget, TakeoverOutcome, DEFAULT_LOCAL_USER_ID,
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
#[tokio::test(flavor = "current_thread")]
async fn slice_backup_restore_interruption_after_container_creation_rolls_back_on_restart() {
    use sha2::{Digest as _, Sha256};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::ExitStatusExt;

    const TEST_NAME: &str = "runtime_transport::tests::slice_backup_restore_interruption_after_container_creation_rolls_back_on_restart";
    const CHILD_ROOT_ENV: &str = "CHARIOX_RESTORE_INTERRUPTION_CHILD_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT_ENV) {
        let root = PathBuf::from(root);
        let config = restore_interruption_config(&root);
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config).expect("child kernel should restore its seeded state"),
        ));
        let router = CommandRouter::with_interactive_capacity(
            app,
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        let request = LocalDaemonRequest::RestoreSliceBackup(RestoreSliceBackupRequest {
            slice_ref: "restore-interruption".to_string(),
            backup_ref: "restore-target".to_string(),
        });
        let command = KernelCommand::from_local_request(
            "restore-interruption-command",
            None,
            Some("restore-interruption-drill".to_string()),
            &request,
        );
        let result = router.dispatch(command, request).await;
        panic!("restore-interruption child survived its injected SIGKILL: {result:?}");
    }

    let _environment = crate::env_lock::lock();
    let root = RuntimeTransportTempDir::new("slice-restore-interruption");
    let bin = root.path().join("bin");
    let docker = bin.join("docker");
    let provisioner = root.path().join("restore-provisioner");
    let docker_log = root.path().join("docker.log");
    let provisioner_log = root.path().join("provisioner.log");
    let interruption_marker = root.path().join("interruption-triggered");
    let partial_runtime = root.path().join("partial-target-runtime");
    let recovered_runtime = root.path().join("recovered-rollback-runtime");
    std::fs::create_dir_all(&bin).expect("fixture bin should create");
    std::fs::write(
        &docker,
        r#"#!/bin/sh
set -eu
root=$CHARIOX_RESTORE_INTERRUPTION_ROOT
printf '%s\n' "$*" >> "$root/docker.log"
case "$*" in
  "info") ;;
  "info --format {{.MemTotal}}") printf '17179869184\n' ;;
  "info --format {{.DockerRootDir}}") printf '/tmp\n' ;;
  "ps -a --format {{.Names}}") printf 'chariox-slice-restore-interruption\n' ;;
  "ps --format {{.Names}}") ;;
  "inspect --size --format {{.SizeRw}} chariox-slice-restore-interruption") printf '1024\n' ;;
  *" du -sb /home-src") printf '1024 /home-src\n' ;;
  *" find /home-src -printf . | wc -c") printf '1\n' ;;
  *" df -B1 --output=avail /tmp") printf '107374182400\n' ;;
  "inspect -f {{.State.Running}} chariox-slice-restore-interruption") printf 'false\n' ;;
  "image inspect --format {{.Id}} chariox-slice-backup:restore-target") printf 'sha256:1111111111111111111111111111111111111111111111111111111111111111\n' ;;
  "image inspect --format {{.Id}} "*) printf 'sha256:2222222222222222222222222222222222222222222222222222222222222222\n' ;;
  cp\ *)
    destination=
    for argument in "$@"; do destination=$argument; done
    printf 'prior-home-generation' > "$destination"
    ;;
esac
exit 0
"#,
    )
    .expect("fake Docker should write");
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700))
        .expect("fake Docker should become executable");
    std::fs::write(
        &provisioner,
        r#"#!/bin/sh
set -eu
root=$CHARIOX_RESTORE_INTERRUPTION_ROOT
printf '%s %s\n' "$1" "$CHARIOX_SLICE_DOCKER_IMAGE" >> "$root/provisioner.log"
if [ "$1" = restore-state ] && [ ! -f "$root/interruption-triggered" ]; then
  : > "$root/partial-target-runtime"
  : > "$root/interruption-triggered"
  kill -9 "$PPID"
  exit 137
fi
if [ "$1" = restore-state ]; then
  rm -f "$root/partial-target-runtime"
  printf '%s\n' "$CHARIOX_SLICE_DOCKER_IMAGE" > "$root/recovered-rollback-runtime"
fi
exit 0
"#,
    )
    .expect("fake provisioner should write");
    std::fs::set_permissions(&provisioner, std::fs::Permissions::from_mode(0o700))
        .expect("fake provisioner should become executable");

    let config = restore_interruption_config(root.path());
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("seed kernel should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(
        Arc::clone(&app),
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    );
    let runtime = router.runtime_state();
    let slice = runtime
        .create_slice(CreateSliceRequest {
            name: "restore-interruption".to_string(),
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
            base: Some(SliceCreateBase::Clean),
        })
        .await
        .expect("slice seed should persist");
    let target_dir = root.path().join("target-backup");
    let target_archive = target_dir.join("home.tar.zst");
    let target_manifest = target_dir.join("manifest.json");
    std::fs::create_dir_all(&target_dir).expect("target backup directory should create");
    let target_home = b"target-home-generation";
    std::fs::write(&target_archive, target_home).expect("target archive should write");
    let target_backup = crate::slice::SliceBackupRecord {
        id: "restore-target".to_string(),
        name: "restore-target".to_string(),
        source_slice_id: slice.id.clone(),
        source_state_id: "restore-interruption".to_string(),
        image_ref: "chariox-slice-backup:restore-target".to_string(),
        home_archive_path: target_archive.display().to_string(),
        manifest_path: target_manifest.display().to_string(),
        created_at_ms: 2,
        size_bytes: Some(target_home.len() as u64),
        home_archive_sha256: Some(format!("{:x}", Sha256::digest(target_home))),
        image_id: Some(format!("sha256:{}", "1".repeat(64))),
    };
    std::fs::write(
        &target_manifest,
        serde_json::to_vec_pretty(&target_backup).expect("target manifest should encode"),
    )
    .expect("target manifest should write");
    runtime
        .save_slice_backup_record(target_backup)
        .expect("target backup should persist");
    drop(runtime);
    drop(router);
    drop(app);

    let mut child_path = vec![bin.clone()];
    if let Some(existing) = std::env::var_os("PATH") {
        child_path.extend(std::env::split_paths(&existing));
    }
    let child = std::process::Command::new(
        std::env::current_exe().expect("test executable should resolve"),
    )
    .args(["--exact", TEST_NAME, "--nocapture"])
    .env(CHILD_ROOT_ENV, root.path())
    .env("CHARIOX_RESTORE_INTERRUPTION_ROOT", root.path())
    .env("CHARIOX_SLICE_DOCKER_PROVISIONER", &provisioner)
    .env(
        "PATH",
        std::env::join_paths(child_path).expect("child PATH should join"),
    )
    .output()
    .expect("interrupted child kernel should run");
    assert_eq!(
        child.status.signal(),
        Some(libc::SIGKILL),
        "child must die at the injected post-create boundary: stdout={} stderr={}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr),
    );
    assert!(interruption_marker.exists(), "fault boundary must trigger");
    assert!(
        partial_runtime.exists(),
        "target replacement must exist at interruption"
    );

    let interrupted_store =
        crate::durable_state::DurableKernelStateStore::open(config.durable_state_path())
            .expect("interrupted durable state should remain readable");
    let started = interrupted_store
        .load_events_by_kind("slice.backup.restore.started")
        .expect("restore-start events should read");
    assert_eq!(started.len(), 1, "one durable restore intent must survive");
    assert!(interrupted_store
        .load_events_by_kind("slice.backup.restore.committed")
        .expect("restore-commit events should read")
        .is_empty());
    assert!(interrupted_store
        .load_events_by_kind("slice.backup.restore.rolled_back")
        .expect("rollback events should read")
        .is_empty());
    let transaction: crate::slice::SliceBackupRestoreTransactionRecord =
        serde_json::from_value(started[0].payload["transaction"].clone())
            .expect("restore transaction should decode");
    drop(interrupted_store);

    let path_guard = RuntimeTransportPathGuard::prepend(bin);
    let root_guard =
        RuntimeTransportEnvGuard::set("CHARIOX_RESTORE_INTERRUPTION_ROOT", root.path().as_os_str());
    let provisioner_guard =
        RuntimeTransportEnvGuard::set("CHARIOX_SLICE_DOCKER_PROVISIONER", provisioner.as_os_str());
    let recovered = DaemonApp::bootstrap(config.clone())
        .expect("kernel startup should roll back the interrupted restore");
    assert!(recovered.slices().list_pending_backup_restores().is_empty());
    let recovered_slice = recovered
        .slices()
        .resolve(&slice.id)
        .expect("recovered slice should remain addressable");
    assert_eq!(recovered_slice.status, crate::slice::SliceStatus::Stopped);
    assert_eq!(
        recovered_slice.last_operation_status,
        Some(crate::slice::SliceOperationStatus::Failed),
    );
    assert!(recovered_slice
        .last_error
        .as_deref()
        .is_some_and(|message| message.contains("interrupted backup restore rolled back")));
    let recovered_state = recovered
        .slices()
        .active_saved_state_for_slice(&slice.id)
        .expect("active state lookup should work")
        .expect("rollback must publish a recoverable state");
    assert_eq!(
        std::fs::read(&recovered_state.home_archive_path)
            .expect("recovered home generation should exist"),
        b"prior-home-generation",
    );
    let durable = recovered.durable_state_store();
    assert_eq!(
        durable
            .load_events_by_kind("slice.backup.restore.rolled_back")
            .expect("rollback events should read")
            .len(),
        1,
    );
    assert!(durable
        .load_events_by_kind("slice.backup.restore.committed")
        .expect("commit events should read")
        .is_empty());
    assert!(
        !partial_runtime.exists(),
        "partial target runtime must be removed"
    );
    let recovered_image = std::fs::read_to_string(&recovered_runtime)
        .expect("rollback provisioner should identify its image");
    assert_eq!(
        recovered_image.trim(),
        transaction.rollback_backup.image_ref
    );
    assert!(
        !Path::new(&transaction.rollback_backup.manifest_path).exists(),
        "resolved rollback manifest must be reclaimed",
    );
    let provisioner_calls =
        std::fs::read_to_string(&provisioner_log).expect("provisioner calls should read");
    assert_eq!(
        provisioner_calls
            .lines()
            .filter(|line| line.starts_with("restore-state "))
            .count(),
        2,
        "only the interrupted target and startup rollback should restore: {provisioner_calls}",
    );
    let docker_calls = std::fs::read_to_string(&docker_log).expect("Docker calls should read");
    assert!(docker_calls
        .lines()
        .any(|line| { line == format!("image rm -f {}", transaction.rollback_backup.image_ref) }));

    let recovered_state_ref = recovered_state.id.clone();
    drop(durable);
    drop(recovered);
    drop(provisioner_guard);
    drop(root_guard);
    drop(path_guard);
    std::fs::remove_dir_all(root.path()).expect("private restore fixture should be removed");
    assert!(!root.path().exists(), "restore fixture must stay removed");
    println!(
        "CHARIOX_SLICE_RESTORE_INTERRUPTION_PROBE:{}",
        serde_json::json!({
            "schema": "chariox.slice_restore_interruption_probe.v1",
            "childInterruptedAfterReplacement": true,
            "durableIntentSurvived": true,
            "rollbackRestoredOnRestart": true,
            "partialRuntimeRemoved": true,
            "priorGenerationRecoverable": true,
            "noCommittedRestore": true,
            "cleanupComplete": true,
            "backendRestoreCount": 2,
            "recoveredStateRef": recovered_state_ref,
        })
    );
}

#[cfg(unix)]
#[test]
fn room_takeover_response_loss_and_reconnect_retain_human_input_authority() {
    let test_thread = std::thread::Builder::new()
        .name("room-takeover-reconnect".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("takeover reconnect runtime should start")
                .block_on(async {
                    let root = RuntimeTransportTempDir::new("takeover-reconnect");
                    let root_path = root.path().to_path_buf();
                    let mut config = DaemonConfig::for_tests();
                    config.publication_control_state_root = Some(root.path().join("control"));
                    config.user_config_path = root.path().join("config/chariox.toml");
                    config.user_config.slices.root =
                        Some(root.path().join("slices").display().to_string());

                    let mut daemon = DaemonApp::bootstrap(config).expect("daemon should boot");
                    let (session, agent) = daemon
                        .create_session(CreateSessionRequest::new(
                            "workspace-takeover-reconnect",
                            "worktree-takeover-reconnect",
                        ))
                        .expect("Room should be created");
                    let session_id = session.id().to_string();
                    let agent_actor_id = agent_environment_actor_id(agent.id());
                    let human_actor_id = human_environment_actor_id(DEFAULT_LOCAL_USER_ID);
                    let session_store = daemon.session_state_store();
                    let viewport = CanonicalViewport::new(1280, 800, 1, 1280, 800)
                        .expect("canonical viewport should be valid");
                    session_store
                        .create_room_environment(
                            &session_id,
                            "environment-takeover-reconnect",
                            viewport.clone(),
                        )
                        .expect("Room Environment should be created");
                    session_store
                        .start_room_environment(&session_id, viewport)
                        .expect("Room Environment should start");
                    session_store
                        .transition_room_environment(&session_id, EnvironmentLifecycle::Ready)
                        .expect("Room Environment should become ready");
                    let baseline = session_store
                        .reconcile_room_environment_actors(
                            &session_id,
                            vec![EnvironmentActor::new(
                                &agent_actor_id,
                                EnvironmentActorKind::Agent,
                                agent.agent_ref(),
                            )],
                        )
                        .expect("default agent should be present in the Room Environment");

                    let app = Arc::new(Mutex::new(daemon));
                    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
                        Arc::clone(&app),
                        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
                    ));
                    let event_counter_path = root.path().join("transport/event-counter.json");
                    let command_cache_path =
                        event_counter_path.with_file_name("command-results.jsonl");
                    let runtime = Arc::new(
                        KernelTransportRuntime::new_with_persistent_event_ids(
                            router.transport_health_store(),
                            event_counter_path,
                        )
                        .expect("transport runtime should initialize"),
                    );
                    let command_id = "human-desktop-takeover";
                    let takeover_request = LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
                        RequestRoomEnvironmentInputTakeoverRequest {
                            session_id: session_id.clone(),
                            target: InputTarget::Desktop,
                        },
                    );

                    dispatch_transport_test_request(
                        Arc::clone(&runtime),
                        Arc::clone(&router),
                        "takeover-response-lost",
                        command_id,
                        takeover_request.clone(),
                        false,
                    )
                    .await;
                    timeout(Duration::from_secs(5), async {
                        loop {
                            let environment = session_store
                                .room_environment_snapshot(&session_id)
                                .expect("Room Environment should remain readable");
                            let cache =
                                std::fs::read_to_string(&command_cache_path).unwrap_or_default();
                            if environment.input_ownership.iter().any(|ownership| {
                                ownership.target == InputTarget::Desktop
                                    && ownership.actor_id == human_actor_id
                            }) && cache.contains(command_id)
                            {
                                break environment;
                            }
                            sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .expect("takeover should commit after its response is lost");
                    let committed = session_store
                        .room_environment_snapshot(&session_id)
                        .expect("committed takeover should remain readable");

                    let replayed = dispatch_transport_test_request(
                        Arc::clone(&runtime),
                        Arc::clone(&router),
                        "takeover-reconnect-retry",
                        command_id,
                        takeover_request,
                        true,
                    )
                    .await
                    .expect("reconnected client should receive the cached takeover response");
                    let KernelOutgoingFrame::Response {
                        response, error, ..
                    } = replayed
                    else {
                        panic!("takeover retry should return a response")
                    };
                    assert!(error.is_none(), "takeover retry should succeed: {error:?}");
                    let replayed_response =
                        serde_json::from_value::<crate::local::LocalDaemonResponse>(
                            response
                                .as_ref()
                                .clone()
                                .expect("takeover retry should retain its response payload"),
                        )
                        .expect("takeover retry response should decode");
                    let crate::local::LocalDaemonResponse::RoomEnvironmentTakeoverUpdated {
                        outcome,
                        environment: replayed_environment,
                    } = replayed_response
                    else {
                        panic!("takeover retry should retain the takeover result")
                    };
                    assert_eq!(outcome, TakeoverOutcome::Granted);
                    assert_eq!(replayed_environment, committed);
                    let after_replay = session_store
                        .room_environment_snapshot(&session_id)
                        .expect("Room Environment should remain readable after replay");
                    assert_eq!(after_replay.event_cursor, committed.event_cursor);
                    assert!(after_replay.input_ownership.iter().any(|ownership| {
                        ownership.target == InputTarget::Desktop
                            && ownership.actor_id == human_actor_id
                    }));

                    let takeover_event_count = match session_store
                        .room_environment_events_after(&session_id, baseline.event_cursor)
                        .expect("takeover events should replay")
                    {
                        EnvironmentReplay::Events { events, .. } => events
                            .into_iter()
                            .filter(|event| {
                                event.kind == EnvironmentEventKind::InputOwnershipChanged
                            })
                            .count(),
                        EnvironmentReplay::SnapshotRequired { .. } => {
                            panic!("bounded takeover replay should not require a snapshot")
                        }
                    };
                    assert_eq!(takeover_event_count, 1);

                    let blocked = session_store.submit_room_environment_action(
                        &session_id,
                        EnvironmentActionRequest::computer_mutation(
                            &agent_actor_id,
                            committed.runtime_generation,
                            "pointer_click",
                            None,
                        ),
                    );
                    let (blocked_admission, blocked_environment) =
                        blocked.expect("agent mutation should return a takeover rejection");
                    assert_eq!(
                        blocked_admission,
                        ActionAdmission::RejectedTakeover {
                            target: InputTarget::Desktop,
                            human_actor_id: human_actor_id.clone(),
                        }
                    );
                    assert!(blocked_environment.input_ownership.iter().any(|ownership| {
                        ownership.target == InputTarget::Desktop
                            && ownership.actor_id == human_actor_id
                    }));

                    let released = dispatch_transport_test_request(
                        Arc::clone(&runtime),
                        Arc::clone(&router),
                        "explicit-human-release",
                        "explicit-human-release",
                        LocalDaemonRequest::ReleaseRoomEnvironmentInput(
                            ReleaseRoomEnvironmentInputRequest {
                                session_id: session_id.clone(),
                                target: InputTarget::Desktop,
                            },
                        ),
                        true,
                    )
                    .await
                    .expect("human should explicitly release desktop input");
                    let KernelOutgoingFrame::Response { error, .. } = released else {
                        panic!("input release should return a response")
                    };
                    assert!(error.is_none(), "input release should succeed: {error:?}");
                    let (admission, _) = session_store
                        .submit_room_environment_action(
                            &session_id,
                            EnvironmentActionRequest::computer_mutation(
                                &agent_actor_id,
                                committed.runtime_generation,
                                "pointer_click",
                                None,
                            ),
                        )
                        .expect("agent mutation should be admitted only after explicit release");
                    let ActionAdmission::Accepted { action_id } = admission else {
                        panic!("agent mutation should start after release: {admission:?}")
                    };
                    session_store
                        .finish_room_environment_action(
                            &session_id,
                            &action_id,
                            EnvironmentActionTerminal::Completed,
                        )
                        .expect("admitted agent mutation should settle");

                    drop(runtime);
                    drop(router);
                    drop(app);
                    drop(session_store);
                    drop(root);
                    assert!(
                        !root_path.exists(),
                        "takeover reconnect fixture should be removed"
                    );
                    println!(
                        "CHARIOX_ROOM_TAKEOVER_RECONNECT_PROBE:{}",
                        serde_json::json!({
                            "schema": "chariox.room_takeover_reconnect_probe.v1",
                            "responseLostAfterCommit": true,
                            "replayedResponseMatched": true,
                            "humanOwnershipRetained": true,
                            "agentMutationBlocked": true,
                            "takeoverAppliedExactlyOnce": true,
                            "explicitReleaseRequired": true,
                            "agentMutationAdmittedAfterRelease": true,
                            "cleanupComplete": true,
                            "takeoverEventCount": takeover_event_count,
                        })
                    );
                });
        })
        .expect("takeover reconnect test thread should start");
    test_thread
        .join()
        .expect("takeover reconnect test thread should finish");
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

#[cfg(unix)]
fn restore_interruption_config(root: &Path) -> DaemonConfig {
    let mut config = DaemonConfig::for_tests();
    config.publication_control_state_root = Some(root.join("control"));
    config.user_config_path = root.join("config/chariox.toml");
    config.user_config.slices.root = Some(root.join("slices").display().to_string());
    config.local_socket_path = root.join("kernel.sock");
    config
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
struct RuntimeTransportEnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

#[cfg(unix)]
impl RuntimeTransportEnvGuard {
    fn set(key: &'static str, value: &std::ffi::OsStr) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

#[cfg(unix)]
impl Drop for RuntimeTransportEnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
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
