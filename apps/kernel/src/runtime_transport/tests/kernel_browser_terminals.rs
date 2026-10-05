//! MD-N5 / MP-08/MP-10/MP-11: two actual local transport connections, the
//! production Rust authority and JS host adapter; synthetic CDP, no native acceptance.
use super::*;
use crate::local::{
    KernelBrowserCommand as Browser, KernelBrowserInput, KernelBrowserMirrorAction,
    KernelBrowserRequest,
};
use crate::runtime::state::KernelBrowserDisplayRequest as Display;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn request(socket: &mut Socket, id: &str, command: Browser) -> KernelOutgoingFrame {
    let frame = KernelIncomingFrame::Request {
        request_id: id.into(),
        command_id: Some(id.into()),
        causation_id: None,
        correlation_id: None,
        request: LocalDaemonRequest::KernelBrowser(KernelBrowserRequest { command }),
    };
    socket
        .send(Message::Text(serde_json::to_string(&frame).unwrap().into()))
        .await
        .unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                let frame: KernelOutgoingFrame = serde_json::from_str(&text).unwrap();
                if matches!(&frame, KernelOutgoingFrame::Response { request_id, .. } if request_id == id) { return frame; }
            }
        }
    }).await.expect("MD-N5: bounded local response")
}
fn success(frame: KernelOutgoingFrame) -> serde_json::Value {
    let KernelOutgoingFrame::Response {
        response, error, ..
    } = frame
    else {
        panic!("MD-N5: response required")
    };
    assert!(error.is_none(), "MD-N5: fixture request failed: {error:?}");
    let crate::local::LocalDaemonResponse::KernelBrowser { result } =
        serde_json::from_value(response.unwrap()).expect("MD-N5: typed browser response")
    else {
        panic!("MD-N5: browser result required")
    };
    result
}

#[test]
fn mdnotes_two_local_connections_keep_observation_and_takeover_private() {
    let root = RuntimeTransportTempDir::new("mdnotes-two-terminals");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("slice-linux-docker/docker/kernel-browser-terminal-fixture.mjs");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "runtime_transport::tests::kernel_browser_terminals::mdnotes_two_local_connections_child", "--ignored", "--nocapture"])
        .env("TMPDIR", root.path())
        .env("CHARIOX_HOME", root.path().join("state"))
        .env("CHARIOX_KERNEL_BROWSER_SCRIPT", fixture)
        .env("CHARIOX_BROWSER_CONTROLLER_NODE", "node")
        .env("CHARIOX_KERNEL_BROWSER_MIRROR", "1")
        .output().expect("MD-N5: isolated child starts");
    assert!(
        output.status.success(),
        "MD-N5: transport child failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "isolated child sets only its own browser fixture environment"]
async fn mdnotes_two_local_connections_child() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let listener = Arc::new(listener);
    let mcp = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(
            DaemonApp::bootstrap(daemon_config_for_runtime_mcp_listener(&mcp)).unwrap(),
        )),
        4,
    ));
    let runtime = Arc::new(KernelTransportRuntime::new(router.transport_health_store()));
    let auth = Arc::new(local_auth::LocalTokenAuth::new(
        local_auth::generate_kernel_local_auth_token(),
    ));
    let check_router = router.clone();
    let checked = tokio::spawn(Box::pin(async move {
        let (mut a, mut b) = {
            let connect = |router: Arc<CommandRouter>, runtime: Arc<KernelTransportRuntime>| {
                let listener = listener.clone();
                let auth = auth.clone();
                async move {
                    let mut handshake = format!("ws://{address}").into_client_request().unwrap();
                    handshake.headers_mut().insert(
                        AUTHORIZATION,
                        HeaderValue::from_str(&format!("Bearer {}", auth.token())).unwrap(),
                    );
                    let client = connect_async(handshake);
                    let server = tokio::spawn(async move {
                        let (stream, _) = listener.accept().await.unwrap();
                        handle_kernel_connection(
                            runtime,
                            router,
                            InboundRequestAdmission::new(process_inbound_request_limit()),
                            KernelLocalAuth::LocalToken(auth),
                            stream,
                        )
                        .await
                    });
                    (client.await.unwrap().0, server)
                }
            };
            (
                connect(check_router.clone(), runtime.clone()).await,
                connect(check_router.clone(), runtime.clone()).await,
            )
        };
        let state = check_router.runtime_state();
        let opened = success(
            request(
                &mut a.0,
                "MD-N5-a-open",
                Browser::Open {
                    url: "https://fixture.test/d1".into(),
                },
            )
            .await,
        );
        let tab = opened["tab_id"].as_str().unwrap().to_string();
        let generation = opened["generation"].as_u64().unwrap();
        let first_doc = opened["tabs"][0]["document_id"].clone();
        success(request(&mut a.0, "MD-N5-a-state", Browser::State).await);
        success(
            request(
                &mut b.0,
                "MD-N5-b-navigate",
                Browser::Navigate {
                    tab_id: tab.clone(),
                    generation,
                    url: "https://fixture.test/d2".into(),
                },
            )
            .await,
        );
        let next = success(request(&mut b.0, "MD-N5-b-state", Browser::State).await);
        assert_ne!(first_doc, next["tabs"][0]["document_id"]);
        let stale = request(
            &mut a.0,
            "MD-N5-a-stale",
            Browser::Input {
                tab_id: tab.clone(),
                generation,
                input: KernelBrowserInput::Text {
                    text: "MD-N5-stale-must-not-type".into(),
                },
            },
        )
        .await;
        // The host deliberately returns a fixed safe failure message. A fresh
        // A observation/input below proves this is the stale binding refusal.
        assert!(
            matches!(stale, KernelOutgoingFrame::Response { error: Some(_), .. }),
            "MD-N5: B must not overwrite A's document receipt"
        );

        let caller = |identity| {
            KernelCommand::from_local_request_with_caller(
                "MD-N5-display",
                KernelCommandSource::LocalCli,
                identity,
                None,
                None,
                &LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
                    command: Browser::State,
                }),
            )
        };
        let caller_a = caller(
            runtime
                .command_result_cache
                .completed_browser_caller("MD-N5-a-open")
                .await
                .unwrap(),
        );
        let caller_b = caller(
            runtime
                .command_result_cache
                .completed_browser_caller("MD-N5-b-state")
                .await
                .unwrap(),
        );
        assert_ne!(
            caller_a.caller, caller_b.caller,
            "MD-N5: actual transport owns distinct callers"
        );
        assert_eq!(
            caller_a.caller,
            runtime
                .command_result_cache
                .completed_browser_caller("MD-N5-a-state")
                .await
                .unwrap(),
            "MD-N5: caller stable within a connection"
        );
        let take = state
            .kernel_browser_display_request(
                &caller_a,
                Display::Takeover {
                    tab_id: tab.clone(),
                    generation,
                },
            )
            .await
            .unwrap();
        assert_eq!(take["state"], "granted");
        assert!(
            state
                .kernel_browser_display_request(
                    &caller_b,
                    Display::Release {
                        tab_id: tab.clone(),
                        generation
                    }
                )
                .await
                .is_err(),
            "MD-N5: B cannot release A takeover"
        );
        state
            .kernel_browser_display_request(
                &caller_a,
                Display::Release {
                    tab_id: tab.clone(),
                    generation,
                },
            )
            .await
            .unwrap();
        // MP-08/MP-10/MP-11: the production typed mirror adapter over the
        // same two authenticated sockets cannot borrow subscription authority.
        let subscribed = success(
            request(
                &mut a.0,
                "MP-mirror-a-subscribe",
                Browser::MirrorSubscribe {
                    tab_id: tab.clone(),
                    generation,
                    device_scale_factor: 1,
                },
            )
            .await,
        );
        let subscription = subscribed["subscription_id"].as_str().unwrap().to_string();
        let foreign = request(
            &mut b.0,
            "MP-mirror-b-foreign",
            Browser::MirrorNext {
                subscription_id: subscription.clone(),
                generation,
                after_sequence: 0,
                drift_nodes: vec![],
            },
        )
        .await;
        assert!(
            matches!(
                foreign,
                KernelOutgoingFrame::Response { error: Some(_), .. }
            ),
            "MP-11: B cannot read A's DOM stream"
        );
        let mirrored = success(
            request(
                &mut a.0,
                "MP-mirror-a-next",
                Browser::MirrorNext {
                    subscription_id: subscription.clone(),
                    generation,
                    after_sequence: 0,
                    drift_nodes: vec![],
                },
            )
            .await,
        );
        assert_eq!(mirrored["reset"], true);
        assert!(
            runtime
                .command_result_cache
                .completed_browser_caller("MP-mirror-a-next")
                .await
                .is_none(),
            "MP-11: no cached DOM packet"
        );
        // MP-08/MP-11: an in-flight newer packet must not retire input on
        // the previously issued view. Future/unissued epochs dispatch nothing
        // and preserve the exact Cloud retry marker through host/Rust wrappers.
        success(
            request(
                &mut a.0,
                "MP-mirror-a-inflight",
                Browser::MirrorNext {
                    subscription_id: subscription.clone(),
                    generation,
                    after_sequence: 1,
                    drift_nodes: vec![],
                },
            )
            .await,
        );
        let future = request(
            &mut a.0,
            "MP-mirror-a-future",
            Browser::MirrorInput {
                tab_id: tab.clone(),
                generation,
                document_id: mirrored["document_id"].as_str().unwrap().into(),
                subscription_id: subscription.clone(),
                sequence: 3,
                action: KernelBrowserMirrorAction::Key { key: "Tab".into() },
            },
        )
        .await;
        match future {
            KernelOutgoingFrame::Response {
                error: Some(error), ..
            } => assert!(error.message.contains("MP-11: stale mirror input epoch")),
            other => panic!("MP-11: future mirror epoch unexpectedly admitted: {other:?}"),
        }
        let b_subscribed = success(
            request(
                &mut b.0,
                "MP-mirror-b-subscribe",
                Browser::MirrorSubscribe {
                    tab_id: tab.clone(),
                    generation,
                    device_scale_factor: 1,
                },
            )
            .await,
        );
        let b_subscription = b_subscribed["subscription_id"]
            .as_str()
            .unwrap()
            .to_string();
        let b_packet = success(
            request(
                &mut b.0,
                "MP-mirror-b-next",
                Browser::MirrorNext {
                    subscription_id: b_subscription.clone(),
                    generation,
                    after_sequence: 0,
                    drift_nodes: vec![],
                },
            )
            .await,
        );
        success(
            request(
                &mut a.0,
                "MP-mirror-a-takeover",
                Browser::DisplayTakeover {
                    tab_id: tab.clone(),
                    generation,
                },
            )
            .await,
        );
        let blocked = request(
            &mut b.0,
            "MP-mirror-b-input",
            Browser::MirrorInput {
                tab_id: tab.clone(),
                generation,
                document_id: b_packet["document_id"].as_str().unwrap().into(),
                subscription_id: b_subscription.clone(),
                sequence: 1,
                action: KernelBrowserMirrorAction::Key { key: "Tab".into() },
            },
        )
        .await;
        assert!(
            matches!(
                blocked,
                KernelOutgoingFrame::Response { error: Some(_), .. }
            ),
            "MP-11: mirror input uses video takeover ledger"
        );
        success(
            request(
                &mut a.0,
                "MP-mirror-a-input",
                Browser::MirrorInput {
                    tab_id: tab.clone(),
                    generation,
                    document_id: mirrored["document_id"].as_str().unwrap().into(),
                    subscription_id: subscription.clone(),
                    sequence: 1,
                    action: KernelBrowserMirrorAction::Key { key: "Tab".into() },
                },
            )
            .await,
        );
        success(
            request(
                &mut a.0,
                "MP-mirror-a-release",
                Browser::DisplayRelease {
                    tab_id: tab.clone(),
                    generation,
                },
            )
            .await,
        );
        success(
            request(
                &mut a.0,
                "MP-mirror-a-close",
                Browser::MirrorClose {
                    subscription_id: subscription,
                    generation,
                },
            )
            .await,
        );
        success(
            request(
                &mut b.0,
                "MP-mirror-b-close",
                Browser::MirrorClose {
                    subscription_id: b_subscription,
                    generation,
                },
            )
            .await,
        );
        // A can act again after refreshing its own receipt; failed stale input
        // must not be repaired by B's state refresh or a cached response.
        success(request(&mut a.0, "MD-N5-a-refresh", Browser::State).await);
        success(
            request(
                &mut a.0,
                "MD-N5-a-fresh",
                Browser::Input {
                    tab_id: tab,
                    generation,
                    input: KernelBrowserInput::Text {
                        text: "MD-N5-fresh".into(),
                    },
                },
            )
            .await,
        );
        a.0.close(None).await.unwrap();
        b.0.close(None).await.unwrap();
        timeout(Duration::from_secs(3), a.1)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(3), b.1)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }))
    .await;
    router.shutdown_cleanup().await.unwrap();
    checked.expect("MD-N5: transport regression must pass");
}
