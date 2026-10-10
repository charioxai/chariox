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

// MP-08/MP-10: exhaust only an isolated child's descriptor table, never the
// shared test runner or host. Exercise actual accept errors and existing traffic.
#[cfg(target_os = "linux")]
#[test]
fn fd_exhaustion_preserves_transport_and_recovers_admission() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "runtime_transport::tests::fd_exhaustion_transport_child",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .expect("isolated FD probe should start");
    assert!(
        output.status.success(),
        "FD probe failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "process-wide RLIMIT_NOFILE; explicitly executed by parent"]
async fn fd_exhaustion_transport_child() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mcp_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let mcp_addr = mcp_listener.local_addr().unwrap();
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(
            DaemonApp::bootstrap(daemon_config_for_runtime_mcp_listener(&mcp_listener)).unwrap(),
        )),
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    let health = router.transport_health_store();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(run_kernel_websocket_server_with_bound_listeners(
        router,
        listener,
        adopt_std_listener(mcp_listener, "test MCP").unwrap(),
        KernelLocalAuth::Unconfigured,
        async {
            let _ = shutdown_rx.await;
        },
    ));
    let (mut socket, _) = connect_async(format!("ws://{addr}")).await.unwrap();

    struct RestoreLimit(libc::rlimit);
    impl Drop for RestoreLimit {
        fn drop(&mut self) {
            assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &self.0) }, 0);
        }
    }
    let mut original = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut original) },
        0
    );
    let restore = RestoreLimit(original);
    let limited = libc::rlimit {
        rlim_cur: original.rlim_cur.min(256),
        ..original
    };
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limited) }, 0);

    // Queue admissions without yielding to either server until the table is full.
    let pending = std::net::TcpStream::connect(addr).unwrap();
    let pending_mcp = std::net::TcpStream::connect(mcp_addr).unwrap();
    let mut files = Vec::new();
    loop {
        match std::fs::File::open("/dev/null") {
            Ok(file) => files.push(file),
            Err(error) => {
                assert_eq!(error.raw_os_error(), Some(libc::EMFILE));
                break;
            }
        }
    }
    sleep(Duration::from_millis(750)).await;
    assert!(
        !server.is_finished(),
        "admission exhaustion must not exit kernel authority"
    );
    let rejected = health.snapshot(0, 0, 0).inbound_overload_rejections;
    assert!(
        rejected > 0 && rejected <= 10,
        "admission must report bounded backoff: {rejected}"
    );
    socket
        .send(Message::Ping(b"under-pressure".to_vec().into()))
        .await
        .unwrap();
    timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Pong(payload))) => {
                    assert_eq!(payload.as_ref(), b"under-pressure");
                    break;
                }
                Some(Ok(_)) => {}
                other => panic!("existing transport lost under pressure: {other:?}"),
            }
        }
    })
    .await
    .expect("existing terminal transport must stay responsive");
    let request = KernelIncomingFrame::Request {
        request_id: "fd-pressure-list".into(),
        command_id: None,
        causation_id: None,
        correlation_id: None,
        request: LocalDaemonRequest::ListSessions(crate::local::ListSessionsRequest),
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&request).unwrap().into(),
        ))
        .await
        .unwrap();
    timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let frame: KernelOutgoingFrame = serde_json::from_str(&text).unwrap();
                    if let KernelOutgoingFrame::Response {
                        request_id,
                        response,
                        error,
                    } = frame
                    {
                        assert_eq!(request_id, "fd-pressure-list");
                        assert!(
                            error.is_none(),
                            "existing kernel command must succeed: {error:?}"
                        );
                        assert!(response.is_some());
                        break;
                    }
                }
                Some(Ok(_)) => {}
                other => panic!("existing command traffic lost: {other:?}"),
            }
        }
    })
    .await
    .expect("existing kernel requests must remain responsive");

    drop(files);
    drop(restore);
    drop(pending);
    drop(pending_mcp);
    let (mut recovered, _) = timeout(
        Duration::from_secs(3),
        connect_async(format!("ws://{addr}")),
    )
    .await
    .unwrap()
    .expect("new admission must recover without restart");
    // MCP must recover too; it must not silently lose its accept task to EMFILE.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut mcp = tokio::net::TcpStream::connect(mcp_addr).await.unwrap();
    mcp.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = [0; 16];
    let count = timeout(Duration::from_secs(3), mcp.read(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response[..count].starts_with(b"HTTP/1.1"));
    let _ = recovered.close(None).await;
    let _ = socket.close(None).await;
    let _ = shutdown_tx.send(());
    timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
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
            KernelLocalAuth::Unconfigured,
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
async fn kernel_websocket_refuses_browser_origins_with_or_without_local_auth() {
    for token in [None, Some("kernel-origin-auth-sentinel")] {
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
                KernelLocalAuth::host_token_or_unconfigured(token.map(Arc::<str>::from)),
                async {
                    let _ = shutdown_rx.await;
                },
            )
            .await
        });
        let request = |origin: Option<&'static str>| {
            let mut request = format!("ws://{addr}")
                .into_client_request()
                .expect("request should build");
            if let Some(token) = token {
                request.headers_mut().insert(
                    AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {token}")).expect("header"),
                );
            }
            if let Some(origin) = origin {
                request
                    .headers_mut()
                    .insert("origin", HeaderValue::from_static(origin));
            }
            request
        };

        // A page in the owner's browser (any origin, even loopback) is refused.
        for origin in ["https://example.test", "http://127.0.0.1:4351", "null"] {
            match connect_async(request(Some(origin)))
                .await
                .expect_err("a browser origin should be refused")
            {
                WebSocketError::Http(response) => {
                    assert_eq!(response.status(), StatusCode::FORBIDDEN, "{origin}")
                }
                other => panic!("expected HTTP forbidden handshake, got {other}"),
            }
        }
        // Kernel clients send no Origin and still connect.
        let (mut socket, _) = connect_async(request(None))
            .await
            .expect("a client without Origin should connect");
        socket.close(None).await.expect("close should send");

        shutdown_tx
            .send(())
            .expect("server should still be running");
        server
            .await
            .expect("server task should join")
            .expect("server should exit cleanly");
    }
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
            KernelLocalAuth::HostToken(Arc::<str>::from("kernel-local-auth-sentinel")),
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

#[tokio::test]
async fn laptop_kernel_websocket_enforces_local_tokens() {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let mcp_listener =
        StdTcpListener::bind("127.0.0.1:0").expect("runtime MCP listener should bind");
    let config = daemon_config_for_runtime_mcp_listener(&mcp_listener);
    let unix_socket = config.local_socket_path.clone();
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("daemon should boot"),
    ));
    let auth = Arc::new(local_auth::LocalTokenAuth::new(
        local_auth::generate_kernel_local_auth_token(),
    ));
    let server_auth = KernelLocalAuth::LocalToken(Arc::clone(&auth));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listeners_with_auth(
            app,
            listener,
            mcp_listener,
            server_auth,
            async {
                let _ = shutdown_rx.await;
            },
        )
        .await
    });

    for authorization in [
        None,
        Some("Bearer chx_kat_wrong".to_string()),
        Some(format!("Bearer {}", auth.token())),
    ] {
        let mut request = format!("ws://{addr}")
            .into_client_request()
            .expect("request should build");
        if let Some(authorization) = &authorization {
            request.headers_mut().insert(
                AUTHORIZATION,
                HeaderValue::from_str(authorization).expect("header should be valid"),
            );
        }
        let result = connect_async(request).await;
        if authorization.as_deref() != Some(&format!("Bearer {}", auth.token())) {
            let Err(tokio_tungstenite::tungstenite::Error::Http(response)) = result else {
                panic!("missing and wrong tokens must fail the upgrade");
            };
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            let message = String::from_utf8(response.body().clone().unwrap()).unwrap();
            assert!(message.contains(
                &DaemonConfig::default_kernel_local_auth_token_path(addr.port())
                    .display()
                    .to_string()
            ));
            assert!(message.contains(&format!("ws+unix://{}", unix_socket.display())));
            assert!(!message.contains(auth.token()));
            assert!(!message.contains("chx_kat_wrong"));
            continue;
        }
        let (mut socket, _) = result.expect("correct token should admit the connection");
        socket
            .send(Message::Ping(Vec::from("authenticated").into()))
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
        assert_eq!(pong, b"authenticated");
        let _ = socket.close(None).await;
    }
    // One authenticated, one missing and one wrong connection were counted.
    assert_eq!(auth.counts(), (1, 1, 1));

    let _ = shutdown_tx.send(());
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server should stop")
        .expect("server task should finish")
        .expect("server should exit cleanly");
}

/// Protocol 402: each websocket credential admits its connection class, and
/// that class is the one a critical approval answer is audited with. A
/// host cannot submit a passkey or use the owner's terminal remember window.
#[cfg(unix)]
#[tokio::test]
async fn kernel_websocket_credentials_attribute_critical_approval_audits_to_their_class() {
    const PASSKEY: &str = "correct horse battery";
    let root = RuntimeTransportTempDir::new("connection-class");
    let vault = root.path().join("vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, PASSKEY)
        .expect("test vault should be created");

    let laptop_auth = Arc::new(local_auth::LocalTokenAuth::new(
        local_auth::generate_kernel_local_auth_token(),
    ));
    let laptop = ClassAuditKernel::start(
        &root,
        &vault,
        KernelLocalAuth::LocalToken(Arc::clone(&laptop_auth)),
    )
    .await;
    let terminal = Some(format!("Bearer {}", laptop_auth.token()));
    assert_eq!(
        laptop.answer(terminal, "approve", Some(PASSKEY)).await,
        None
    );
    assert_eq!(
        laptop.audits(),
        [("verified".to_string(), serde_json::json!("terminal"))]
    );
    laptop.stop().await;

    let host = ClassAuditKernel::start(
        &root,
        &vault,
        KernelLocalAuth::HostToken(Arc::<str>::from("kernel-local-auth-sentinel")),
    )
    .await;
    let host_token = Some("Bearer kernel-local-auth-sentinel".to_string());
    let refused = host
        .answer(host_token.clone(), "approve", Some(PASSKEY))
        .await
        .expect("a host passkey should be refused");
    assert!(refused.contains("PASSKEY_NOT_ACCEPTED"), "{refused}");
    // Open the owner's remember window through the terminal command path on
    // this same runtime, then answer the other pending decision over the host
    // websocket. Owner routing must not turn the host into a terminal.
    let seed_id = format!("remember-seed-{:016x}", rand::random::<u64>());
    let _seed = host
        .router
        .runtime_state()
        .raise_critical_approval_for_test(&host.session_id, &seed_id)
        .await;
    let seed =
        LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
            session_id: host.session_id.clone(),
            interaction_id: seed_id,
            choice_id: "approve".into(),
            custom_reply: None,
            passkey: Some(crate::local::ApprovalPasskey::new(PASSKEY)),
            passkey_remember_minutes: Some(5),
        });
    let command = KernelCommand::from_local_request_with_caller(
        "remember-seed",
        KernelCommandSource::LocalCli,
        crate::runtime::command::KernelCaller::for_source(&KernelCommandSource::LocalCli)
            .with_connection_class(KernelConnectionClass::Terminal),
        None,
        None,
        &seed,
    );
    host.router.dispatch(command, seed).await.unwrap();
    let missing = host
        .answer(host_token.clone(), "approve", None)
        .await
        .expect("approving still needs the passkey");
    assert!(missing.contains("PASSKEY_REQUIRED"), "{missing}");
    assert_eq!(host.answer(host_token, "deny", None).await, None);
    assert!(
        host.audits().is_empty(),
        "hosts stay outside the passkey gate"
    );
    host.stop().await;
}

struct ClassAuditKernel {
    config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
    router: Arc<CommandRouter>,
    addr: std::net::SocketAddr,
    durable: crate::durable_state::DurableKernelStateStore,
    session_id: String,
    interaction_id: String,
    _decision: Box<dyn std::any::Any + Send>,
    shutdown: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<Result<(), DaemonError>>,
}

impl ClassAuditKernel {
    async fn start(root: &RuntimeTransportTempDir, vault: &Path, auth: KernelLocalAuth) -> Self {
        Self::start_for_owner(root, vault, auth, crate::session::DEFAULT_LOCAL_USER_ID).await
    }

    async fn start_for_owner(
        root: &RuntimeTransportTempDir,
        vault: &Path,
        auth: KernelLocalAuth,
        owner: &str,
    ) -> Self {
        let listener = StdTcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        let mcp_listener =
            StdTcpListener::bind("127.0.0.1:0").expect("runtime MCP listener should bind");
        let mut config = daemon_config_for_runtime_mcp_listener(&mcp_listener);
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = vault.display().to_string();
        config.user_config_path = root
            .path()
            .join(format!("config-{}.toml", rand::random::<u64>()));
        if owner != crate::session::DEFAULT_LOCAL_USER_ID {
            config.cloud_relay = Some(
                serde_json::from_value(serde_json::json!({
                    "api_url": "https://fixture.invalid", "email": "test@fixture.invalid",
                    "account_id": "synthetic-account", "user_id": owner, "account_slug": "fixture",
                    "realm_id": "synthetic-realm", "relay_url": "wss://fixture.invalid",
                    "issuer_id": "synthetic-issuer", "client_id": "synthetic-terminal"
                }))
                .unwrap(),
            );
        }
        let app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let mut session = crate::session::RuntimeSession::new(
            format!("class-audit-{:016x}", rand::random::<u64>()),
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        );
        session.set_owner_user_id(owner);
        let session_id = session.id().to_owned();
        app.sessions_mut().restore_session(session);
        let durable = app.durable_state_store();
        let config_projection = app.config_projection_store();
        let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
            Arc::new(Mutex::new(app)),
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        ));
        let interaction_id = format!("class-audit-{:016x}", rand::random::<u64>());
        let decision = router
            .runtime_state()
            .raise_critical_approval_for_test(&session_id, &interaction_id)
            .await;
        let listener = adopt_std_listener(listener, "kernel websocket").expect("listener");
        let mcp_listener = adopt_std_listener(mcp_listener, "runtime mcp").expect("mcp listener");
        let (shutdown, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(run_kernel_websocket_server_with_bound_listeners(
            Arc::clone(&router),
            listener,
            mcp_listener,
            auth,
            async move {
                let _ = shutdown_rx.await;
            },
        ));
        Self {
            config_projection,
            router,
            addr,
            durable,
            session_id,
            interaction_id,
            _decision: decision,
            shutdown,
            server,
        }
    }

    /// Answers the pending decision over a new connection; the error message
    /// on refusal.
    async fn answer(
        &self,
        authorization: Option<String>,
        choice: &str,
        passkey: Option<&str>,
    ) -> Option<String> {
        let mut request = format!("ws://{}", self.addr)
            .into_client_request()
            .expect("request should build");
        if let Some(authorization) = authorization {
            request.headers_mut().insert(
                AUTHORIZATION,
                HeaderValue::from_str(&authorization).expect("header should be valid"),
            );
        }
        let (mut socket, _) = connect_async(request)
            .await
            .expect("the connection should be accepted");
        let request_id = format!("answer-{:016x}", rand::random::<u64>());
        let frame = serde_json::json!({
            "type": "request",
            "request_id": request_id,
            "request": LocalDaemonRequest::RespondToInteraction(
                crate::local::RespondToInteractionRequest {
                    session_id: self.session_id.clone(),
                    interaction_id: self.interaction_id.clone(),
                    choice_id: choice.into(),
                    custom_reply: None,
                    passkey: passkey.map(crate::local::ApprovalPasskey::new),
                    passkey_remember_minutes: None,
                },
            ),
        });
        socket
            .send(Message::Text(frame.to_string().into()))
            .await
            .expect("request should send");
        let response = timeout(Duration::from_secs(10), async {
            loop {
                match socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        let value: Value = serde_json::from_str(&text).expect("frame is JSON");
                        if value["type"] == "response" && value["request_id"] == request_id {
                            break value;
                        }
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => panic!("websocket read failed: {error}"),
                    None => panic!("websocket closed before the response"),
                }
            }
        })
        .await
        .expect("the response should arrive");
        let _ = socket.close(None).await;
        response["error"]["message"].as_str().map(str::to_owned)
    }

    /// The decision's audits: outcome and connection class, never a passkey.
    fn audits(&self) -> Vec<(String, Value)> {
        self.durable
            .load_subject_events_by_kind(
                &format!("validation:{}", self.interaction_id),
                "critical_approval.passkey",
                50,
            )
            .expect("audits should load")
            .into_iter()
            .map(|event| {
                let payload = event.payload.to_string();
                assert!(!payload.contains("correct horse") && !payload.contains("guess"));
                assert!(!payload.contains("chx_kat_") && !payload.contains("sentinel"));
                (
                    event.payload["outcome"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    event.payload["connection_class"].clone(),
                )
            })
            .collect()
    }

    async fn stop(self) {
        let _ = self.shutdown.send(());
        timeout(Duration::from_secs(5), self.server)
            .await
            .expect("server should stop")
            .expect("server task should finish")
            .expect("server should exit cleanly");
    }
}

#[cfg(unix)]
mod kernel_access_config;
#[cfg(unix)]
mod passkey_prompts;

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn unsupported_slice_save_refusal_replays_without_backend_side_effects() {
    crate::test_support::isolated_env_test!();
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
  *"tar --zstd -C /home-src -cf - .") printf 'saved-home-generation' ;;
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
                source_slice_ref: None,
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
            if cache.contains(command_id) {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first refusal should persist after its response is lost");

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
    let same_process_refusal = slice_capture_refusal(&same_process);
    assert_eq!(docker_commit_count(&docker_log), 0);
    assert!(
        std::fs::read_to_string(&docker_log)
            .unwrap_or_default()
            .is_empty(),
        "unsupported capture must not invoke Docker before or after replay"
    );

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
    let restart_refusal = slice_capture_refusal(&after_restart);
    assert_eq!(restart_refusal, same_process_refusal);
    assert_eq!(docker_commit_count(&docker_log), 0);
    assert!(
        std::fs::read_to_string(&docker_log)
            .unwrap_or_default()
            .is_empty(),
        "unsupported capture must not invoke Docker before or after replay"
    );

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
    assert_eq!(docker_commit_count(&docker_log), 0);
    assert!(
        std::fs::read_to_string(&docker_log)
            .unwrap_or_default()
            .is_empty(),
        "unsupported capture must not invoke Docker before or after replay"
    );

    drop(restarted_runtime);
    drop(router);
    drop(app);
    std::fs::remove_dir_all(root.path()).expect("drill artifacts should be removed");
    assert!(!root.path().exists(), "drill artifacts should stay removed");
    println!(
        "CHARIOX_SLICE_SAVE_ACK_LOSS_PROBE:{}",
        serde_json::json!({
            "schema": "chariox.slice_save_ack_loss_probe.v2",
            "sameProcessReplay": true,
            "restartReplay": true,
            "unsupportedCaptureRefusalPreserved": true,
            "conflictingReuseRejected": true,
            "backendSaveCount": 0,
            "successfulSaveReplayStillRequiresProtectedLiveFixture": true,
            "cleanupComplete": true
        })
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn slice_pending_backup_restore_interruption_rolls_back_on_restart() {
    crate::test_support::isolated_env_test!();
    use sha2::{Digest as _, Sha256};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::ExitStatusExt;

    const TEST_NAME: &str =
        "runtime_transport::tests::slice_pending_backup_restore_interruption_rolls_back_on_restart";
    const CHILD_ROOT_ENV: &str = "CHARIOX_RESTORE_INTERRUPTION_CHILD_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT_ENV) {
        let root = PathBuf::from(root);
        let config = restore_interruption_config(&root);
        let result = DaemonApp::bootstrap(config);
        panic!(
            "pending-restore child survived its injected recovery SIGKILL: {}",
            result.is_ok()
        );
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
    for (name, contents) in [
        (
            "slice-command-guard.py",
            include_str!("../../slice-linux-docker/slice-command-guard.py"),
        ),
        (
            "home-archive-policy.json",
            include_str!("../../slice-linux-docker/home-archive-policy.json"),
        ),
        (
            "owned_process_signals.py",
            include_str!("../../slice-linux-docker/owned_process_signals.py"),
        ),
    ] {
        std::fs::write(root.path().join(name), contents)
            .expect("restore fixture should include its provisioner's archive verifier and policy");
    }
    std::fs::write(
        &docker,
        r#"#!/bin/sh
set -eu
root=$CHARIOX_RESTORE_INTERRUPTION_ROOT
printf '%s\n' "$*" >> "$root/docker.log"
if [ -f "$root/docker-unavailable" ]; then
  printf 'Cannot connect to the Docker daemon\n' >&2
  exit 1
fi
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
  *"tar --zstd -C /home-src -cf - .") printf 'prior-home-generation' ;;
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
  case "$PPID" in ''|*[!0-9]*) exit 1 ;; esac
  [ "$PPID" -gt 1 ] || exit 1
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
            source_slice_ref: None,
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
        .save_slice_backup_record(target_backup.clone())
        .expect("target backup should persist");
    // This fixture exercises recovery of a legitimate preexisting durable
    // transaction, not acceptance of a new capture on an unsupported layout.
    // Successful protected capture and post-target-creation interruption remain
    // separate live validation obligations.
    let rollback_dir = root.path().join("rollback-backup");
    std::fs::create_dir_all(&rollback_dir).expect("rollback directory should create");
    let rollback_archive = rollback_dir.join("home.tar.zst");
    let rollback_manifest = rollback_dir.join("manifest.json");
    let rollback_home = b"prior-home-generation";
    std::fs::write(&rollback_archive, rollback_home).expect("rollback archive should write");
    let rollback_backup = crate::slice::SliceBackupRecord {
        id: "restore-rollback-fixture".to_string(),
        name: "restore-rollback-fixture".to_string(),
        image_ref: "chariox-slice-backup:restore-rollback-fixture".to_string(),
        home_archive_path: rollback_archive.display().to_string(),
        manifest_path: rollback_manifest.display().to_string(),
        size_bytes: Some(rollback_home.len() as u64),
        home_archive_sha256: Some(format!("{:x}", Sha256::digest(rollback_home))),
        image_id: Some(format!("sha256:{}", "2".repeat(64))),
        ..target_backup.clone()
    };
    std::fs::write(
        &rollback_manifest,
        serde_json::to_vec_pretty(&rollback_backup).expect("rollback manifest should encode"),
    )
    .expect("rollback manifest should write");
    runtime
        .begin_slice_backup_restore(crate::slice::SliceBackupRestoreTransactionRecord {
            id: "restore-interruption-fixture".to_string(),
            source_slice_id: slice.id.clone(),
            target_backup,
            rollback_backup,
            previous_saved_state: None,
            started_at_ms: 3,
        })
        .expect("preexisting restore intent should persist");
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
        "child must die at the injected startup recovery boundary: stdout={} stderr={}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr),
    );
    assert!(interruption_marker.exists(), "fault boundary must trigger");
    assert!(
        partial_runtime.exists(),
        "partial recovery runtime must exist at interruption"
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
    let unavailable_archive = root.path().join("unavailable-rollback.tar.zst");
    std::fs::rename(
        &transaction.rollback_backup.home_archive_path,
        &unavailable_archive,
    )
    .expect("fault should make only the rollback archive unavailable");
    let mut blocked = DaemonApp::bootstrap(config.clone())
        .expect("an unavailable rollback must not prevent the home kernel from starting");
    assert_eq!(
        blocked.slices().list_pending_backup_restores(),
        vec![transaction.clone()],
        "failed recovery must retain the transaction",
    );
    let blocked_slice = blocked
        .slices()
        .resolve(&slice.id)
        .expect("slice should remain visible");
    assert_eq!(blocked_slice.status, crate::slice::SliceStatus::Unhealthy);
    assert_eq!(
        blocked_slice.last_operation_status,
        Some(crate::slice::SliceOperationStatus::Failed),
    );
    assert!(blocked_slice.last_error.as_deref().is_some_and(
        |error| error.contains("rollback") && error.contains("restart the kernel to retry")
    ));
    assert!(blocked
        .slices()
        .try_begin_operation(&slice.id, "slice.start")
        .is_err());
    assert!(Path::new(&transaction.rollback_backup.manifest_path).exists());
    assert_eq!(
        std::fs::read(&unavailable_archive).expect("original archive remains intact"),
        b"prior-home-generation",
    );
    let (unrelated, _) = crate::app::KernelSessionService::new(&mut blocked)
        .create_session(CreateSessionRequest::new(
            "unrelated-workspace",
            "unrelated-workspace",
        ))
        .expect("unrelated Rooms must remain usable during slice recovery failure");
    assert!(blocked.sessions().get_session(unrelated.id()).is_ok());
    drop(blocked);
    std::fs::rename(
        &unavailable_archive,
        &transaction.rollback_backup.home_archive_path,
    )
    .expect("repair should make the same rollback archive available again");

    let docker_unavailable = root.path().join("docker-unavailable");
    std::fs::write(&docker_unavailable, b"").expect("Docker outage marker should write");
    let blocked = DaemonApp::bootstrap(config.clone())
        .expect("Docker outage during rollback must not prevent kernel startup");
    assert_eq!(
        blocked.slices().list_pending_backup_restores(),
        vec![transaction.clone()]
    );
    let blocked_slice = blocked
        .slices()
        .resolve(&slice.id)
        .expect("slice remains visible");
    assert_eq!(blocked_slice.status, crate::slice::SliceStatus::Unhealthy);
    assert!(blocked_slice
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("rollback") && error.contains("Docker")));
    assert!(blocked.sessions().get_session(unrelated.id()).is_ok());
    assert!(blocked
        .slices()
        .try_begin_operation(&slice.id, "slice.start")
        .is_err());
    assert!(Path::new(&transaction.rollback_backup.manifest_path).exists());
    assert_eq!(
        std::fs::read(&transaction.rollback_backup.home_archive_path)
            .expect("Docker outage must preserve the archive"),
        b"prior-home-generation",
    );
    drop(blocked);
    std::fs::remove_file(&docker_unavailable).expect("Docker outage marker should clear");
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
        Path::new(&transaction.rollback_backup.manifest_path).exists(),
        "published rollback state must retain its referenced manifest",
    );
    let provisioner_calls =
        std::fs::read_to_string(&provisioner_log).expect("provisioner calls should read");
    assert_eq!(
        provisioner_calls
            .lines()
            .filter(|line| line.starts_with("restore-state "))
            .count(),
        2,
        "only interrupted recovery and resumed recovery should restore: {provisioner_calls}",
    );
    let docker_calls = std::fs::read_to_string(&docker_log).expect("Docker calls should read");
    assert!(
        !docker_calls.lines().any(|line| {
            line == format!("image rm -f {}", transaction.rollback_backup.image_ref)
        }),
        "published rollback image must not be garbage collected"
    );

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
            "schema": "chariox.slice_restore_interruption_probe.v2",
            "childInterruptedDuringStartupRecovery": true,
            "postTargetCreationInterruptionStillRequiresProtectedLiveFixture": true,
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
#[tokio::test(flavor = "current_thread")]
async fn slice_pending_backup_restore_acknowledgement_survives_crash_and_broker_outage() {
    let _environment = crate::env_lock::lock();
    let root = RuntimeTransportTempDir::new("slice-restore-acknowledgement");
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
            name: "restore-acknowledgement".to_string(),
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
            source_slice_ref: None,
        })
        .await
        .expect("slice seed should persist");
    let backup = |id: &str| crate::slice::SliceBackupRecord {
        id: id.to_string(),
        name: id.to_string(),
        source_slice_id: slice.id.clone(),
        source_state_id: "restore-acknowledgement".to_string(),
        image_ref: format!("chariox-slice-backup:{id}"),
        home_archive_path: root
            .path()
            .join(id)
            .join("home.tar.zst")
            .display()
            .to_string(),
        manifest_path: root
            .path()
            .join(id)
            .join("manifest.json")
            .display()
            .to_string(),
        created_at_ms: 2,
        size_bytes: Some(1),
        home_archive_sha256: Some("a".repeat(64)),
        image_id: Some(format!("sha256:{}", "b".repeat(64))),
    };
    let transaction = |id: &str| crate::slice::SliceBackupRestoreTransactionRecord {
        id: id.to_string(),
        source_slice_id: slice.id.clone(),
        target_backup: backup(&format!("{id}-target")),
        rollback_backup: backup(&format!("{id}-rollback")),
        previous_saved_state: None,
        started_at_ms: 3,
    };
    let state = |id: &str| crate::slice::SliceSavedStateRecord {
        id: id.to_string(),
        slice_name: slice.name.clone(),
        source_slice_id: slice.id.clone(),
        backend: SliceBackendKind::LocalDocker,
        os: "linux".to_string(),
        image_ref: format!("chariox-slice-state:{id}"),
        home_archive_path: root
            .path()
            .join(format!("{id}.tar.zst"))
            .display()
            .to_string(),
        manifest_path: root.path().join(format!("{id}.json")).display().to_string(),
        created_at_ms: 4,
        updated_at_ms: 4,
        size_bytes: Some(1),
        last_operation: Some("backup.restore".to_string()),
        last_operation_status: Some(crate::slice::SliceOperationStatus::Completed),
        last_error: None,
    };
    let first = transaction("restore-first");
    let second = transaction("restore-second");
    let owed = crate::slice::SliceBackupRestoreAcknowledgementRecord {
        transaction_id: first.id.clone(),
        source_slice_id: slice.id.clone(),
        home_archive_path: first.target_backup.home_archive_path.clone(),
        retained_rollback_backup: None,
    };
    runtime
        .begin_slice_backup_restore(first.clone())
        .expect("first restore intent should persist");
    // Fault boundary: the kernel durably resolves the restore and dies before
    // acknowledging publication to the broker.
    runtime
        .resolve_slice_backup_restore(
            &first,
            state("restored-first"),
            crate::slice::SliceBackupRestoreResolution::Restored,
        )
        .expect("first restore should commit");
    let error = runtime
        .begin_slice_backup_restore(second.clone())
        .expect_err("the next restore must wait for the owed acknowledgement");
    assert!(error.to_string().contains(&first.id), "{error}");
    drop(runtime);
    drop(router);
    drop(app);

    let crashed = crate::durable_state::DurableKernelStateStore::open(config.durable_state_path())
        .expect("crashed durable state should remain readable");
    let committed = crashed
        .load_events_by_kind("slice.backup.restore.committed")
        .expect("commit events should read");
    assert_eq!(committed.len(), 1);
    assert_eq!(
        committed[0].payload["acknowledgement"],
        serde_json::to_value(&owed).expect("acknowledgement should encode"),
        "the owed acknowledgement commits atomically with the resolution",
    );
    assert!(crashed
        .load_events_by_kind("slice.backup.restore.acknowledged")
        .expect("acknowledgement events should read")
        .is_empty());
    drop(crashed);

    // Restart while the managed broker is unavailable: startup continues, the
    // committed restore stays committed, and the acknowledgement stays owed.
    let broker_outage = RuntimeTransportEnvGuard::set(
        "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED",
        std::ffi::OsStr::new("1"),
    );
    let blocked = crate::test_support::bootstrap_after_test_owner_exit(config.clone()).await;
    assert_eq!(
        blocked.slices().list_pending_restore_acknowledgements(),
        vec![owed.clone()]
    );
    assert_eq!(
        blocked
            .slices()
            .resolve(&slice.id)
            .expect("slice should remain visible")
            .saved_state_ref
            .as_deref(),
        Some("restored-first"),
        "a committed restore must never be rolled back",
    );
    assert!(blocked.slices().list_pending_backup_restores().is_empty());
    let error = blocked
        .slices()
        .begin_backup_restore_transactionally(second.clone(), |_| Ok(()))
        .expect_err("the next restore must wait for the owed acknowledgement");
    assert!(error.to_string().contains(&first.id), "{error}");
    drop(blocked);
    drop(broker_outage);

    // Restart with the broker repaired: reconciliation acknowledges, and the
    // next restore starts and commits.
    let app = Arc::new(Mutex::new(
        crate::test_support::bootstrap_after_test_owner_exit(config.clone()).await,
    ));
    let router = CommandRouter::with_interactive_capacity(
        Arc::clone(&app),
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    );
    let runtime = router.runtime_state();
    assert!(runtime
        .reconcile_slice_backup_restore_acknowledgements(None)
        .expect("reconciliation should run")
        .is_empty());
    runtime
        .begin_slice_backup_restore(second.clone())
        .expect("the next restore should start after acknowledgement");
    let restored = runtime
        .resolve_slice_backup_restore(
            &second,
            state("restored-second"),
            crate::slice::SliceBackupRestoreResolution::Restored,
        )
        .expect("the next restore should commit");
    assert_eq!(restored.saved_state_ref.as_deref(), Some("restored-second"));
    assert!(runtime
        .reconcile_slice_backup_restore_acknowledgements(Some(&slice.id))
        .expect("reconciliation should run")
        .is_empty());
    drop(runtime);
    drop(router);
    drop(app);
    let durable = crate::durable_state::DurableKernelStateStore::open(config.durable_state_path())
        .expect("durable state should remain readable");
    let acknowledged = durable
        .load_events_by_kind("slice.backup.restore.acknowledged")
        .expect("acknowledgement events should read");
    assert_eq!(
        acknowledged
            .iter()
            .map(|event| event.payload["transaction_id"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec![first.id.as_str(), second.id.as_str()],
    );
}

#[cfg(unix)]
#[test]
fn room_takeover_response_loss_and_reconnect_retain_human_input_authority() {
    crate::test_support::isolated_env_test!();
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
        IncomingConnection {
            runtime: &runtime,
            router: &router,
            connection_state: &Arc::new(Mutex::new(ConnectionState {
                subscription: None,
                watch_task: None,
            })),
            inbound_request_admission: &InboundRequestAdmission::new(
                process_inbound_request_limit(),
            ),
            connection_inbound_request_permits: &Arc::new(Semaphore::new(
                CONNECTION_INBOUND_REQUEST_LIMIT,
            )),
            outgoing_tx: &outgoing,
            close_tx: &close_tx,
            close_requested: &Arc::new(AtomicBool::new(false)),
            connection_class: KernelConnectionClass::Unauthenticated,
            peer: None,
            bound_grant: &Arc::default(),
        },
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
fn slice_capture_refusal(frame: &KernelOutgoingFrame) -> serde_json::Value {
    let KernelOutgoingFrame::Response {
        response, error, ..
    } = frame
    else {
        panic!("slice refusal should return a response")
    };
    assert!(
        response.is_none(),
        "unsupported capture must not publish saved state"
    );
    let error = error.as_ref().expect("unsupported capture must refuse");
    let value = serde_json::to_value(error).expect("refusal should encode");
    assert!(value.to_string().contains("storage layout"));
    value
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
    config.local_socket_path =
        DaemonConfig::default_local_socket_path(&format!("fixture:{}", root.display()));
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

fn decode_error_reply(payload: serde_json::Value) -> (String, KernelTransportError) {
    let payload = serde_json::to_vec(&payload).expect("payload should serialize");
    let error = serde_json::from_slice::<KernelIncomingFrame>(&payload)
        .expect_err("payload should fail typed decoding");
    match incoming_frame_decode_error(&payload, &error) {
        KernelOutgoingFrame::Response {
            request_id,
            response,
            error,
        } => {
            assert!(response.is_none());
            (
                request_id,
                error.expect("decode failure should carry an error"),
            )
        }
        other => panic!("expected response frame, got {other:?}"),
    }
}

#[test]
fn malformed_local_request_replies_with_its_request_id() {
    let (request_id, error) = decode_error_reply(serde_json::json!({
        "type": "request",
        "request_id": "req-missing-fields",
        "request": {
            "StartRoomEnvironment": {
                "session_id": "session-1",
                "viewport": {
                    "css_width": 1280,
                    "css_height": 800,
                    "device_scale_factor": 1
                }
            }
        }
    }));
    assert_eq!(request_id, "req-missing-fields");
    assert_eq!(error.code, "invalid_request");
    assert!(
        error.message.starts_with("invalid request: "),
        "{}",
        error.message
    );
    assert!(!error.retryable);

    let (request_id, error) = decode_error_reply(serde_json::json!({
        "type": "request",
        "request_id": "req-unknown-variant",
        "request": { "SetKernelConfigValue": { "key": "k", "value": "v" } }
    }));
    assert_eq!(request_id, "req-unknown-variant");
    assert_eq!(error.code, "invalid_request");
    assert!(error.message.contains(&format!("This kernel (protocol {}) does not support this request; update the Chariox client or kernel so both match.", crate::local::LOCAL_DAEMON_PROTOCOL_VERSION)));
}

#[test]
fn request_decode_refusal_preserves_websocket_correlation_for_unknown_field_shape() {
    let (request_id, error) = decode_error_reply(serde_json::json!({
        "type": "request", "request_id": "req-host-shape",
        "request": { "AcceptAppHostAction": { "session_id": "s", "operation_id": 17 } }
    }));
    assert_eq!(request_id, "req-host-shape");
    assert_eq!(error.code, "invalid_request");
    assert!(!error.retryable);
    assert!(error
        .message
        .contains("update the Chariox client or kernel so both match"));
}

#[test]
fn undecodable_frame_without_request_id_replies_as_invalid_frame() {
    let payload = b"not json";
    let error = serde_json::from_slice::<KernelIncomingFrame>(payload).expect_err("not json");
    match incoming_frame_decode_error(payload, &error) {
        KernelOutgoingFrame::Response {
            request_id, error, ..
        } => {
            assert_eq!(request_id, "unknown");
            assert_eq!(error.expect("error").code, "invalid_frame");
        }
        other => panic!("expected response frame, got {other:?}"),
    }
}

/// A schedule committed after the transport pump falls asleep must be processed
/// at its deadline, without any client traffic or a five-second idle sweep.
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn app_wake_deadline_interrupts_idle_transport_without_client_traffic() {
    use crate::durable_state::{
        app_state::AppStateOperation,
        app_wakes::{AppWakeOperation, AppWakeOutcome},
    };
    use crate::runtime::app_operation_budget::AppOperationBudget;
    use chariox_app_runtime::managed_state::{Wake, WakeChange};
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let mcp = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let app = DaemonApp::bootstrap(daemon_config_for_runtime_mcp_listener(&mcp)).unwrap();
    let store = app.durable_state_store();
    let catalog = crate::durable_state::app_state::fixture_event_catalog(&store);
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(run_kernel_websocket_server_on_listeners_with_auth(
        Arc::new(Mutex::new(app)),
        listener,
        mcp,
        KernelLocalAuth::Unconfigured,
        async {
            let _ = stopped.await;
        },
    ));
    // Let the initial transport pass finish before scheduling anything.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let due = crate::session::unix_epoch_ms() + 150;
    let registration = store.clone();
    tokio::task::spawn_blocking(move || {
        registration.execute_app_state(
            "alice",
            catalog,
            AppStateOperation::Schedule {
                wakes: vec![WakeChange::Set(Wake {
                    id: "deadline".into(),
                    due_at_ms: due,
                    revision: "r1".into(),
                })],
                wakes_count_as_use: false,
            },
            AppOperationBudget::fixture(TokioInstant::now() + Duration::from_secs(5), || false),
        )
    })
    .await
    .unwrap()
    .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let probe = store.clone();
    // No package was staged: the scheduler must record its bounded failed
    // start and defer the wake, rather than leave it due for the idle sweep.
    let result = timeout(Duration::from_millis(700), async {
        loop {
            let probe = probe.clone();
            let due = tokio::task::spawn_blocking(move || {
                probe.app_wakes(AppWakeOperation::Due {
                    now_ms: crate::session::unix_epoch_ms(),
                    limit: 8,
                })
            })
            .await
            .unwrap()
            .unwrap();
            if matches!(due, AppWakeOutcome::Due(items) if items.is_empty()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    stop.send(()).unwrap();
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        result.is_ok(),
        "due App wake waited for coarse transport reconciliation"
    );
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod wake_pressure;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod kernel_access_grants;

mod ka_validation;

mod provider_account_portability;
