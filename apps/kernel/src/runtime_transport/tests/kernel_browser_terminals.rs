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
        kernel: None,
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
fn mdnotes_two_local_connections_keep_observation_private_and_share_the_takeover() {
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
                .completed_browser_caller("local:MD-N5-a-open")
                .await
                .unwrap(),
        );
        let caller_b = caller(
            runtime
                .command_result_cache
                .completed_browser_caller("local:MD-N5-b-state")
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
                .completed_browser_caller("local:MD-N5-a-state")
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
        let mut other_person = caller_b.clone();
        other_person.caller.user_id = Some("different-person".into());
        assert!(
            state
                .kernel_browser_display_request(
                    &other_person,
                    Display::Release {
                        tab_id: tab.clone(),
                        generation,
                    },
                )
                .await
                .is_err(),
            "MD-N5: another person cannot release A takeover"
        );
        state
            .kernel_browser_display_request(
                &caller_b,
                Display::Release {
                    tab_id: tab.clone(),
                    generation,
                },
            )
            .await
            .expect("MD-N5: the same person's B connection can return A takeover");
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
                    wire: None,
                    navigation_preview: None,
                    font_requirements: None,
                    font_absent: None,
                    viewport_first: None,
                    visited_links: None,
                    layout_width: None,
                    layout_height: None,
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
                known_sheets: vec![],
                known_fonts: vec![],
                wait_ms: None,
                resend: None,
                flush: None,
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
                    known_sheets: vec![],
                    known_fonts: vec![],
                    wait_ms: None,
                    resend: None,
                    flush: None,
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
                    known_sheets: vec![],
                    known_fonts: vec![],
                    wait_ms: None,
                    resend: None,
                    flush: None,
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
            } => assert!(
                error.message.contains("MP-11: stale mirror input epoch"),
                "MP-11: mirror refusal: {error:?}"
            ),
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
                    wire: None,
                    navigation_preview: None,
                    font_requirements: None,
                    font_absent: None,
                    viewport_first: None,
                    visited_links: None,
                    layout_width: None,
                    layout_height: None,
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
                    known_sheets: vec![],
                    known_fonts: vec![],
                    wait_ms: None,
                    resend: None,
                    flush: None,
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
        // The takeover belongs to the person: their other connection types on, through the same ledger.
        success(
            request(
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
            .await,
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
        let refreshed = success(request(&mut a.0, "MD-N5-a-refresh", Browser::State).await);
        // A person's display input is answered by the frame it causes: its reply
        // carries the generation, never the Tab state (every Tab's favicon).
        let document_id = refreshed["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|shown| shown["tab_id"] == tab.as_str())
            .unwrap()["document_id"]
            .as_str()
            .unwrap()
            .to_string();
        let typed = state
            .kernel_browser_display_request(
                &caller_a,
                Display::Input {
                    binding: crate::runtime::kernel_browser_host::KernelBrowserDocumentBinding {
                        tab_id: tab.clone(),
                        generation,
                        document_id,
                    },
                    input: KernelBrowserInput::Text {
                        text: "MD-N5-display".into(),
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(typed, serde_json::json!({ "generation": generation }));
        // A state read names each Tab's icon by hash; its PNG travels only to a
        // viewer that lists no such hash (protocol 1012), never on a plain read.
        let shown = |reply: serde_json::Value| {
            reply["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|shown| shown["tab_id"] == tab.as_str())
                .unwrap()
                .clone()
        };
        let mut plain = serde_json::Value::Null;
        for attempt in 0..40 {
            plain = shown(success(
                request(&mut a.0, &format!("MP-08-icon-{attempt}"), Browser::State).await,
            ));
            if !plain["favicon_hash"].is_null() || !plain["favicon_png_base64"].is_null() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let hash = plain["favicon_hash"].as_str().expect("MP-08: a named icon");
        assert!(plain.get("favicon_png_base64").is_none(), "MP-08: {plain}");
        let wait = |id: &'static str, known: Vec<&str>| {
            let command = serde_json::json!({"op":"state_wait","after":"","wait_ms":0,"known_favicons":known});
            (id, serde_json::from_value::<Browser>(command).expect("MP-08: protocol 1012 state wait"))
        };
        let (id, lacking) = wait("MP-08-icon-lacking", vec![]);
        let lacking = shown(success(request(&mut a.0, id, lacking).await));
        assert_eq!(lacking["favicon_hash"], hash);
        assert!(lacking["favicon_png_base64"].as_str().unwrap().starts_with("iVBORw0KGgo"));
        let (id, holding) = wait("MP-08-icon-holding", vec![hash]);
        let holding = shown(success(request(&mut a.0, id, holding).await));
        assert_eq!(holding["favicon_hash"], hash);
        assert!(holding.get("favicon_png_base64").is_none(), "MP-08: {holding}");
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
