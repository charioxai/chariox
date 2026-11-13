//! Answers with no paging option stay answerable above the 1 MiB reply ceiling.

use super::collaborator_gate_tests::relay_user;
use super::*;
use crate::local::{
    CaptureVisibleRegionRequest, LocalDaemonResponse, RoomTerminalOperation, RoomTerminalRequest,
    RoomTerminalResult, ScreenshotRegion, ScreenshotSurface, VisibleRegionCapture,
};
use base64::Engine as _;

/// A Room terminal and the client that asks for it over the relay.
struct Asker {
    router: CommandRouter,
    cache: Arc<CommandResultCache>,
    session_id: String,
    client_key: String,
    kernel_public_key: String,
    _worktree: crate::test_support::TestWorktree,
}

impl Asker {
    fn new() -> Self {
        let worktree = crate::test_support::TestWorktree::new("relay_unpaged_reply");
        let app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let session_id = session.id().to_string();
        let router =
            CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
        let kernel_public_key =
            relay_crypto::public_key_from_private_key_base64(&router.relay_private_key()).unwrap();
        Self {
            router,
            cache: Arc::new(CommandResultCache::default()),
            session_id,
            client_key: relay_crypto::generate_private_key_base64(),
            kernel_public_key,
            _worktree: worktree,
        }
    }

    /// The terminal operation as a client sends it, and the answer as the client reads it.
    async fn terminal(
        &self,
        operation: RoomTerminalOperation,
    ) -> Result<(RoomTerminalResult, usize), RelayError> {
        let request = LocalDaemonRequest::RoomTerminal(RoomTerminalRequest {
            session_id: self.session_id.clone(),
            acting_agent: None,
            host_agent: None,
            operation,
        });
        let body = serde_json::json!({ "request": request }).to_string();
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &self.client_key,
            &self.kernel_public_key,
            body.as_bytes(),
        )
        .unwrap();
        let outcome = handle_daemon_request(
            &self.router,
            &AtomicU64::new(1),
            Some(relay_user("local")),
            encrypted,
            &self.cache,
            &Default::default(),
            None,
        )
        .await;
        if let Some(error) = outcome.error {
            return Err(error);
        }
        let plaintext = relay_crypto::decrypt_payload_for_private_key(
            &self.client_key,
            &outcome.encrypted_response.expect("an answer or an error"),
        )
        .unwrap()
        .plaintext;
        // Snapshot and output replies carry their bytes in CXT1 framing, as the client opens them.
        let json = crate::transport::terminal_output_wire::open(&plaintext).unwrap();
        let LocalDaemonResponse::RoomTerminal { result } = serde_json::from_slice(&json).unwrap()
        else {
            panic!("a terminal answer");
        };
        Ok((result, plaintext.len()))
    }
}

// MP-08 / MP-09 / MP-10 / MP-11 (relayproof r4, F1): a 240-column Room terminal with 5,000
// populated scrollback lines answers a Snapshot with more than 1 MiB. The snapshot has no
// cursor or length, and the web client needs it to attach, so it must not be refused as too
// large to send.
#[test]
fn a_wide_terminal_snapshot_above_the_reply_ceiling_is_answered_over_the_relay() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let asker = Asker::new();
                    let (RoomTerminalResult::Opened { terminal }, _) = asker
                        .terminal(RoomTerminalOperation::Open {
                            cols: 240,
                            rows: 24,
                            title: None,
                            command: Some(vec![
                                "sh".into(),
                                "-c".into(),
                                "i=0; while [ $i -lt 6500 ]; do printf '%0239d\\n' $i; \
                                 i=$((i+1)); done; sleep 120"
                                    .into(),
                            ]),
                        })
                        .await
                        .expect("the owner opens the terminal")
                    else {
                        panic!("an opened terminal");
                    };
                    let mut answered = 0;
                    for _ in 0..200 {
                        let snapshot = asker
                            .terminal(RoomTerminalOperation::Snapshot {
                                terminal_id: terminal.terminal_id.clone(),
                            })
                            .await
                            .unwrap_or_else(|error| {
                                panic!(
                                    "the snapshot of a wide terminal was refused: {} ({})",
                                    error.code, error.message
                                )
                            });
                        if let (RoomTerminalResult::Snapshot { snapshot }, len) = snapshot {
                            if snapshot.scrollback_text.len() > 1_100_000 {
                                answered = len;
                                break;
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    assert!(
                        answered > 1024 * 1024,
                        "the terminal never filled its scrollback: {answered} bytes"
                    );
                    let _ = asker
                        .terminal(RoomTerminalOperation::Close {
                            terminal_id: terminal.terminal_id,
                        })
                        .await;
                });
        })
        .unwrap()
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

// MP-08 / MP-09 / MP-10 / MP-11 (relayproof r4, F2): the capture producer accepts images of
// several MiB and returns the whole image in one answer; a valid 1 MiB PNG is already over the
// ceiling once encoded as JSON.
#[test]
fn a_captured_image_above_the_reply_ceiling_is_sent() {
    let surface = ScreenshotSurface::KernelBrowser {
        tab_id: "tab-1".into(),
        generation: 2,
    };
    let request = LocalDaemonRequest::CaptureVisibleRegion(CaptureVisibleRegionRequest {
        capture_id: "capture-1".into(),
        surface: surface.clone(),
        region: ScreenshotRegion {
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
            viewport_width: 1280,
            viewport_height: 800,
            frame_width: 1280,
            frame_height: 800,
        },
    });
    let png = vec![0x89u8; 1024 * 1024];
    let answer = serde_json::to_vec(&LocalDaemonResponse::VisibleRegionCaptured {
        capture: VisibleRegionCapture {
            capture_id: "capture-1".into(),
            surface,
            captured_at_ms: 1,
            width: 1280,
            height: 800,
            media_type: "image/png".into(),
            display_name: "capture.png".into(),
            sha256: "0".repeat(64),
            data_base64: base64::engine::general_purpose::STANDARD.encode(png),
        },
    })
    .unwrap();
    assert!(
        answer.len() > 1024 * 1024,
        "a 1 MiB image is over the ceiling as JSON"
    );
    let daemon = crate::config::DaemonConfig::for_tests();
    let client = crate::config::DaemonConfig::for_tests();
    let outcome = seal_reply(
        answer,
        None,
        SealContext {
            request_kind: relay_request_kind(&request),
            command_id: Some("capture-1"),
            received_at_ms: crate::session::unix_epoch_ms(),
            daemon_private_key: &daemon.relay_private_key,
            client_public_key: &client.relay_public_key,
            framed: false,
        },
    );
    assert!(
        outcome.error.is_none() && outcome.encrypted_response.is_some(),
        "the capture was refused: {:?}",
        outcome.error
    );
}

// The kernel browser's own screenshot command has no offset or size control and its host
// returns up to 4 MiB of image data, so a valid answer between 1 and 24 MiB must not be refused.
#[test]
fn a_kernel_browser_screenshot_above_the_reply_ceiling_is_sent() {
    let request = LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
        command: crate::local::KernelBrowserCommand::Screenshot {
            tab_id: "tab-1".into(),
            generation: 2,
        },
    });
    let image = vec![0x89u8; 3 * 1024 * 1024];
    let answer = serde_json::to_vec(&serde_json::json!({"KernelBrowser":{"result":{
        "generation": 2,
        "tab_id": "tab-1",
        "document_id": "document",
        "mime_type": "image/png",
        "data_base64": base64::engine::general_purpose::STANDARD.encode(image),
        "width": 2560,
        "height": 1600,
    }}}))
    .unwrap();
    assert!(answer.len() > 1024 * 1024);
    let daemon = crate::config::DaemonConfig::for_tests();
    let client = crate::config::DaemonConfig::for_tests();
    let outcome = seal_reply(
        answer,
        None,
        SealContext {
            request_kind: relay_request_kind(&request),
            command_id: Some("screenshot-1"),
            received_at_ms: crate::session::unix_epoch_ms(),
            daemon_private_key: &daemon.relay_private_key,
            client_public_key: &client.relay_public_key,
            framed: false,
        },
    );
    assert!(
        outcome.error.is_none() && outcome.encrypted_response.is_some(),
        "the screenshot was refused: {:?}",
        outcome.error
    );
}

// The owned desktop's screenshot is the same kind of answer as the kernel browser's: one image
// the caller cannot page, so a valid answer between 1 and 24 MiB must not be refused either.
#[test]
fn a_kernel_computer_screenshot_above_the_reply_ceiling_is_sent() {
    let request = LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
        command: crate::local::KernelBrowserCommand::Computer {
            command: crate::local::KernelComputerCommand::Screenshot {
                target: crate::local::KernelDesktopTarget {
                    surface_id: "desktop-1".into(),
                    generation: "2".into(),
                },
            },
        },
    });
    let image = vec![0x89u8; 3 * 1024 * 1024];
    let answer = serde_json::to_vec(&serde_json::json!({"KernelBrowser":{"result":{
        "mime_type": "image/png",
        "data_base64": base64::engine::general_purpose::STANDARD.encode(image),
        "width": 2560,
        "height": 1600,
    }}}))
    .unwrap();
    assert!(answer.len() > 1024 * 1024);
    let daemon = crate::config::DaemonConfig::for_tests();
    let client = crate::config::DaemonConfig::for_tests();
    let outcome = seal_reply(
        answer,
        None,
        SealContext {
            request_kind: relay_request_kind(&request),
            command_id: Some("screenshot-2"),
            received_at_ms: crate::session::unix_epoch_ms(),
            daemon_private_key: &daemon.relay_private_key,
            client_public_key: &client.relay_public_key,
            framed: false,
        },
    );
    assert!(
        outcome.error.is_none() && outcome.encrypted_response.is_some(),
        "the screenshot was refused: {:?}",
        outcome.error
    );
}
