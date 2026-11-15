//! A file chunk crosses the relay as bytes in both directions when the client frames its
//! plaintext, and a client that does not frame it is answered in JSON as before.

use super::collaborator_gate_tests::relay_user;
use super::*;
use crate::local::LocalDaemonResponse;
use crate::transport::framed_plaintext;
use base64::Engine as _;

struct Client {
    router: CommandRouter,
    cache: Arc<CommandResultCache>,
    key: String,
    kernel_public_key: String,
}

impl Client {
    fn new() -> Self {
        let app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let router =
            CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
        let kernel_public_key =
            relay_crypto::public_key_from_private_key_base64(&router.relay_private_key()).unwrap();
        Self {
            router,
            cache: Arc::new(CommandResultCache::default()),
            key: relay_crypto::generate_private_key_base64(),
            kernel_public_key,
        }
    }

    /// The raw plaintext of the answer to `request`, sent framed or as JSON.
    async fn ask(&self, request: LocalDaemonRequest, framed: bool) -> Vec<u8> {
        let json = serde_json::json!({ "request": request })
            .to_string()
            .into_bytes();
        let plaintext = if framed {
            framed_plaintext::frame(&json)
        } else {
            json
        };
        let encrypted =
            relay_crypto::encrypt_payload_for_peer(&self.key, &self.kernel_public_key, &plaintext)
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
        assert!(outcome.error.is_none(), "answered: {:?}", outcome.error);
        relay_crypto::decrypt_payload_for_private_key(
            &self.key,
            &outcome.encrypted_response.expect("an answer"),
        )
        .unwrap()
        .plaintext
        .to_vec()
    }
}

fn write(path: &std::path::Path, bytes: &[u8]) -> LocalDaemonRequest {
    LocalDaemonRequest::WriteKernelFile(crate::local::WriteKernelFileRequest {
        path: path.display().to_string(),
        offset: 0,
        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        total_size: bytes.len() as u64,
        sha256: None,
        overwrite: true,
    })
}

fn read(path: &std::path::Path, length: usize) -> LocalDaemonRequest {
    LocalDaemonRequest::ReadKernelFile(crate::local::ReadKernelFileRequest {
        path: path.display().to_string(),
        offset: 0,
        length: length as u64,
    })
}

#[test]
fn a_file_chunk_is_written_and_read_back_as_bytes_when_the_request_is_framed() {
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
                    let dir = std::env::temp_dir()
                        .join(format!("framed-plaintext-{:08x}", rand::random::<u32>()));
                    std::fs::create_dir_all(&dir).unwrap();
                    let bytes: Vec<u8> = (0..64 * 1024u32).map(|n| (n % 251) as u8).collect();
                    let client = Client::new();
                    let path = dir.join("file.bin");
                    client.ask(write(&path, &bytes), true).await;
                    assert!(
                        std::fs::read(&path).unwrap() == bytes,
                        "the chunk was written"
                    );
                    let framed = client.ask(read(&path, bytes.len()), true).await;
                    assert!(
                        framed.starts_with(b"CXF1"),
                        "a framed request is answered framed"
                    );
                    assert!(
                        framed.len() < bytes.len() + 1024,
                        "{} bytes for {} file bytes",
                        framed.len(),
                        bytes.len()
                    );
                    let (json, _) = framed_plaintext::open(&framed).unwrap();
                    let LocalDaemonResponse::KernelFileChunk { data_base64, .. } =
                        serde_json::from_slice(&json).unwrap()
                    else {
                        panic!("a file chunk")
                    };
                    assert!(
                        base64::engine::general_purpose::STANDARD
                            .decode(data_base64)
                            .unwrap()
                            == bytes
                    );
                    let plain = client.ask(read(&path, bytes.len()), false).await;
                    assert!(
                        plain.starts_with(b"{"),
                        "a request in JSON is answered in JSON"
                    );
                    assert!(plain.len() > bytes.len() * 4 / 3);
                    std::fs::remove_dir_all(&dir).unwrap();
                })
        })
        .unwrap()
        .join()
        .unwrap();
}

#[tokio::test]
async fn polled_terminal_replies_are_raw_for_plain_and_framed_requests() {
    for framed in [false, true] {
        let mut app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace",
                "worktree",
            ))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "native",
                crate::attachment::ClientCapabilityLevel::MessageTransport,
            ))
            .unwrap();
        let source = vec![255u8; 64 * 1024];
        app.terminal_stream_store().fan_out_output(
            session.id(),
            "run",
            None,
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            vec![attachment.id().into()],
            &source,
        );
        let router =
            CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
        let kernel_public_key =
            relay_crypto::public_key_from_private_key_base64(&router.relay_private_key()).unwrap();
        let client = Client {
            router,
            cache: Arc::new(CommandResultCache::default()),
            key: relay_crypto::generate_private_key_base64(),
            kernel_public_key,
        };
        let wire = client
            .ask(
                LocalDaemonRequest::PumpTerminalOutput(crate::local::PumpTerminalOutputRequest {
                    session_id: session.id().into(),
                    attachment_id: attachment.id().into(),
                }),
                framed,
            )
            .await;
        assert!(
            wire.starts_with(b"CXT1"),
            "a terminal poll still expands bytes as JSON"
        );
        assert!(
            wire.len() < source.len() + 2048,
            "a polled reply expands on the wire"
        );
        let json = crate::transport::terminal_output_wire::open(&wire).unwrap();
        let response: LocalDaemonResponse = serde_json::from_slice(&json).unwrap();
        let LocalDaemonResponse::TerminalOutput { records } = response else {
            panic!("terminal reply")
        };
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].bytes, source);
    }
}
