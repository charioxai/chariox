//! Native Vault assertions over the real kernel/router and local encrypted relay.
use super::*;
use crate::local::{
    CaptureVisibleRegionRequest, NoteCommand, NoteResult, NoteWindow, NotesRequest,
    ScreenshotRegion, ScreenshotSurface,
};
use crate::transport::relay_client::VaultRelayDrill;
use base64::Engine;
use std::path::Path;

const FIXTURE_VALUE: &str = "synthetic-md5-protected-value";
fn stimulus(router: &CommandRouter, url: &str, operation: &str) {
    let profile = router
        .runtime_state
        .kernel_browser_profile_root(DEFAULT_LOCAL_USER_ID);
    let status = std::process::Command::new("node")
        .arg(std::env::var_os("CHARIOX_MDVAULT_DRIVER").unwrap())
        .arg(profile)
        .arg(url)
        .arg(operation)
        .status()
        .unwrap();
    assert!(status.success(), "credential-free native stimulus failed");
}
fn args(snapshot: &Value, tab: &str, generation: u64, id: &str, credential: &str) -> Value {
    let node = snapshot["snapshot"]["dom_nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["attributes"]["id"] == id)
        .expect("observed fixture password field");
    json!({"credential_id":credential,"tab_id":tab,"generation":generation,"document_id":snapshot["snapshot"]["document_id"],"node_ref":node["node_ref"]})
}
async fn approve(relay: &mut VaultRelayDrill, router: &CommandRouter, session: &str) {
    let snapshot = router
        .runtime_state
        .session_snapshot(session)
        .await
        .unwrap();
    let id = snapshot
        .active_interactions()
        .first()
        .expect("owner popup required")
        .id()
        .to_string();
    let response = relay
        .request(LocalDaemonRequest::RespondToInteraction(
            crate::local::RespondToInteractionRequest {
                session_id: session.into(),
                interaction_id: id,
                choice_id: "allow".into(),
                custom_reply: None,
                passkey: Some(crate::local::ApprovalPasskey::new(
                    "synthetic-md5-passphrase",
                )),
                passkey_remember_minutes: None,
            },
        ))
        .await;
    assert!(matches!(
        response,
        LocalDaemonResponse::InteractionResponded { .. }
    ));
}
fn pixels(data: &str, path: &Path, x: u32, y: u32, black: bool) {
    let data = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&data));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes).unwrap();
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        _ => panic!("native RGB(A) PNG required"),
    };
    assert!(x < info.width && y < info.height);
    let i = (y as usize * info.width as usize + x as usize) * channels;
    assert_eq!(
        bytes[i..i + 3] == [0, 0, 0],
        black,
        "native field pixel masking"
    );
    std::fs::write(path, data).unwrap();
}
async fn protected_surfaces(
    relay: &mut VaultRelayDrill,
    router: &CommandRouter,
    tab: &str,
    generation: u64,
    url: &str,
    root: &Path,
    dpr: u32,
) {
    let command = |command| LocalDaemonRequest::KernelBrowser(KernelBrowserRequest { command });
    let screenshot = relay
        .request(command(KernelBrowserCommand::Screenshot {
            tab_id: tab.into(),
            generation,
        }))
        .await;
    let LocalDaemonResponse::KernelBrowser { result } = screenshot else {
        panic!("real screenshot response required")
    };
    pixels(
        result["data_base64"].as_str().unwrap(),
        &root.join(format!("MD-Vault-dpr{dpr}-screenshot.png")),
        40 * dpr,
        200 * dpr,
        true,
    );
    pixels(
        result["data_base64"].as_str().unwrap(),
        &root.join(format!("MD-Vault-dpr{dpr}-echo.png")),
        12 * dpr,
        110 * dpr,
        true,
    );
    let capture = relay
        .request(LocalDaemonRequest::CaptureVisibleRegion(
            CaptureVisibleRegionRequest {
                capture_id: format!("vault-dpr{dpr}"),
                surface: ScreenshotSurface::KernelBrowser {
                    tab_id: tab.into(),
                    generation,
                },
                region: ScreenshotRegion {
                    x: 0,
                    y: 160,
                    width: 360,
                    height: 90,
                    viewport_width: 1280,
                    viewport_height: 800,
                    frame_width: 1280 * dpr,
                    frame_height: 800 * dpr,
                },
            },
        ))
        .await;
    let LocalDaemonResponse::VisibleRegionCaptured { capture } = capture else {
        panic!("real protected capture required")
    };
    pixels(
        &capture.data_base64,
        &root.join(format!("MD-Vault-dpr{dpr}-capture.png")),
        40 * dpr,
        40 * dpr,
        true,
    );
    stimulus(router, url, "select_echo");
    let note = relay
        .request(LocalDaemonRequest::Notes(NotesRequest {
            command: NoteCommand::CaptureSelection {
                window: NoteWindow::KernelBrowser {
                    tab_id: tab.into(),
                    generation,
                },
            },
        }))
        .await;
    assert!(
        matches!(
            note,
            LocalDaemonResponse::Notes {
                result: NoteResult::SelectionChanged { selection: None }
            }
        ),
        "protected echoed quote must yield no receipt"
    );
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn check(
    router: &Arc<CommandRouter>,
    config: &DaemonConfig,
    token: &str,
    session: &str,
    agent: &str,
    second: &str,
    tab: &str,
    generation: u64,
    root: &Path,
    url: &str,
) {
    let mut relay = VaultRelayDrill::start(Arc::clone(router), config).await;
    let mut cases = vec![
        "owner-popup-before-fill",
        "encrypted-relay-owner-approval",
        "same-document-replay-refused",
    ];
    stimulus(router, url, "filled");
    cases.push("native-insertion-once");
    let text = router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.kernel_browser",
            json!({"command":{"op":"snapshot","tab_id":tab,"generation":generation}}),
        )
        .await
        .unwrap();
    assert!(!text.payload.to_string().contains(FIXTURE_VALUE));
    assert!(text.payload.to_string().contains("[redacted]"));
    cases.push("agent-text-echo-suppressed");
    let display1 = human(
        router,
        KernelBrowserCommand::DisplaySubscribe {
            tab_id: tab.into(),
            generation,
            codecs: vec!["png".into()],
            bitrate: 1_000_000,
            device_scale_factor: 1,
        },
    )
    .await;
    let frame1 = human(
        router,
        KernelBrowserCommand::DisplayNext {
            subscription_id: display1["subscription_id"].as_str().unwrap().into(),
            generation,
            after_sequence: 0,
        },
    )
    .await;
    pixels(
        frame1["display_frame"]["data_base64"].as_str().unwrap(),
        &root.join("MD-Vault-dpr1-display.png"),
        40,
        200,
        true,
    );
    cases.push("dpr1-display-frame");
    protected_surfaces(&mut relay, router, tab, generation, url, root, 1).await;
    cases.extend([
        "dpr1-screenshot",
        "dpr1-region-capture",
        "dpr1-notes-echo-refused",
        "dpr1-echo-pixels-masked",
    ]);
    let snapshot = human(
        router,
        KernelBrowserCommand::Snapshot {
            tab_id: tab.into(),
            generation,
        },
    )
    .await;
    let child_args = args(&snapshot, tab, generation, "framepassword", "md5-login");
    assert!(router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.kernel_browser_paste_secret",
            child_args
        )
        .await
        .is_err());
    cases.push("cross-origin-host-allowlist-refused");
    let child_args = args(
        &snapshot,
        tab,
        generation,
        "framepassword",
        "md5-child-login",
    );
    let pending = router.dispatch_authenticated_runtime_tool_call(
        token,
        "chariox.kernel_browser_paste_secret",
        child_args,
    );
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut pending)
            .await
            .is_err()
    );
    approve(&mut relay, router, session).await;
    assert_eq!(pending.await.unwrap().payload, json!({"inserted":true}));
    stimulus(router, url, "child_filled");
    let frame = human(
        router,
        KernelBrowserCommand::Screenshot {
            tab_id: tab.into(),
            generation,
        },
    )
    .await;
    pixels(
        frame["data_base64"].as_str().unwrap(),
        &root.join("MD-Vault-oopif-masked.png"),
        62,
        442,
        true,
    );
    cases.push("allowed-oopif-fill-and-mask");
    // A new document revokes the old target binding, but preserves echo suppression.
    human(
        router,
        KernelBrowserCommand::Navigate {
            tab_id: tab.into(),
            generation,
            url: format!("{url}?new-document"),
        },
    )
    .await;
    let current_url = format!("{url}?new-document");
    let fresh = human(
        router,
        KernelBrowserCommand::Snapshot {
            tab_id: tab.into(),
            generation,
        },
    )
    .await;
    let fresh_args = args(&fresh, tab, generation, "password", "md5-login");
    let mut stale = fresh_args.clone();
    stale["generation"] = json!(generation + 10);
    assert!(router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.kernel_browser_paste_secret",
            stale
        )
        .await
        .is_err());
    cases.push("stale-generation-refused");
    let fresh_frame = human(
        router,
        KernelBrowserCommand::Screenshot {
            tab_id: tab.into(),
            generation,
        },
    )
    .await;
    pixels(
        fresh_frame["data_base64"].as_str().unwrap(),
        &root.join("MD-Vault-new-document.png"),
        40,
        200,
        false,
    );
    cases.push("navigation-revokes-old-field-mask");
    // Focus change during approval cancels the grant; the old token remains unusable.
    let pending = router.dispatch_authenticated_runtime_tool_call(
        token,
        "chariox.kernel_browser_paste_secret",
        fresh_args.clone(),
    );
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut pending)
            .await
            .is_err()
    );
    focus(router, session, second).await;
    assert!(tokio::time::timeout(Duration::from_secs(2), &mut pending)
        .await
        .unwrap()
        .is_err());
    assert!(router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.kernel_browser_paste_secret",
            fresh_args.clone()
        )
        .await
        .is_err());
    cases.push("focus-revocation-and-unfocused-agent-refused");
    focus(router, session, agent).await;
    router
        .dispatch_authenticated_runtime_tool_call(token, "chariox.load_kernel_browser", json!({}))
        .await
        .unwrap();
    let pending = router.dispatch_authenticated_runtime_tool_call(
        token,
        "chariox.kernel_browser_paste_secret",
        fresh_args,
    );
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut pending)
            .await
            .is_err()
    );
    human(
        router,
        KernelBrowserCommand::Navigate {
            tab_id: tab.into(),
            generation,
            url: format!("{url}?during-approval"),
        },
    )
    .await;
    approve(&mut relay, router, session).await;
    assert!(pending.await.is_err());
    stimulus(router, &format!("{url}?during-approval"), "empty");
    cases.push("navigation-during-approval-refused");
    // Dedicated tab permits native DPR 2 negotiation without changing another client's geometry.
    let opened = human(
        router,
        KernelBrowserCommand::Open {
            url: current_url.clone(),
        },
    )
    .await;
    let dpr_tab = opened["tab_id"].as_str().unwrap();
    let display = human(
        router,
        KernelBrowserCommand::DisplaySubscribe {
            tab_id: dpr_tab.into(),
            generation,
            codecs: vec!["png".into()],
            bitrate: 1_000_000,
            device_scale_factor: 2,
        },
    )
    .await;
    let fresh = human(
        router,
        KernelBrowserCommand::Snapshot {
            tab_id: dpr_tab.into(),
            generation,
        },
    )
    .await;
    let pending = router.dispatch_authenticated_runtime_tool_call(
        token,
        "chariox.kernel_browser_paste_secret",
        args(&fresh, dpr_tab, generation, "password", "md5-login"),
    );
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut pending)
            .await
            .is_err()
    );
    approve(&mut relay, router, session).await;
    assert_eq!(pending.await.unwrap().payload, json!({"inserted":true}));
    protected_surfaces(
        &mut relay,
        router,
        dpr_tab,
        generation,
        &current_url,
        root,
        2,
    )
    .await;
    cases.extend([
        "dpr2-screenshot",
        "dpr2-region-capture",
        "dpr2-notes-echo-refused",
        "dpr2-echo-pixels-masked",
    ]);
    let frame = human(
        router,
        KernelBrowserCommand::DisplayNext {
            subscription_id: display["subscription_id"].as_str().unwrap().into(),
            generation,
            after_sequence: 0,
        },
    )
    .await;
    pixels(
        frame["display_frame"]["data_base64"].as_str().unwrap(),
        &root.join("MD-Vault-dpr2-display.png"),
        80,
        400,
        true,
    );
    cases.push("dpr2-display-frame");
    human(
        router,
        KernelBrowserCommand::Close {
            tab_id: dpr_tab.into(),
            generation,
        },
    )
    .await;
    let fresh = human(
        router,
        KernelBrowserCommand::Snapshot {
            tab_id: tab.into(),
            generation,
        },
    )
    .await;
    let pending = router.dispatch_authenticated_runtime_tool_call(
        token,
        "chariox.kernel_browser_paste_secret",
        args(&fresh, tab, generation, "password", "md5-login"),
    );
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut pending)
            .await
            .is_err()
    );
    approve(&mut relay, router, session).await;
    assert_eq!(pending.await.unwrap().payload, json!({"inserted":true}));
    cases.push("new-document-requires-new-approval");
    relay.close().await;
    std::fs::write(root.join("MD-Vault-RECEIPT.json"),serde_json::to_vec_pretty(&json!({"passed":cases.len(),"failed":0,"cases":cases,"protocol":427,"relay":74,"provider":"dev-stub fixture only","real_chromium":true,"local_encrypted_relay":true})).unwrap()).unwrap();
}
