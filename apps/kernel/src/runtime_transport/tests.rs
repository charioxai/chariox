use super::*;

use std::path::{Path, PathBuf};

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

fn daemon_config_with_available_runtime_mcp_port() -> DaemonConfig {
    let listener =
        StdTcpListener::bind("127.0.0.1:0").expect("temporary runtime MCP listener should bind");
    let port = listener
        .local_addr()
        .expect("temporary runtime MCP listener should have an address")
        .port();
    drop(listener);

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
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(daemon_config_with_available_runtime_mcp_port())
            .expect("daemon should boot"),
    ));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listener(app, listener, async {
            let _ = shutdown_rx.await;
        })
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
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(daemon_config_with_available_runtime_mcp_port())
            .expect("daemon should boot"),
    ));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listener_with_auth(
            app,
            listener,
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
