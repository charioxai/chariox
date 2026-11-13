//! A Room terminal viewer subscribes once and the kernel pushes the output as the program
//! writes it: no read request per page, raw bytes, the summary only when it changed, and the
//! Room's terminals (the viewer's tabs) without a list request.

use super::support::*;
use crate::local::{RoomTerminalOperation, RoomTerminalRequest, RoomTerminalResult};
use base64::Engine as _;

fn owner_identity() -> chariox_relay::protocol::RelayCallerIdentity {
    chariox_relay::protocol::RelayCallerIdentity {
        realm_id: "test".into(),
        subject: "client-local".into(),
        subject_kind: chariox_relay::auth::RelaySubjectKind::Client,
        expires_at_ms: u64::MAX,
        token_id: None,
        user_id: Some(crate::session::DEFAULT_LOCAL_USER_ID.into()),
        public_key_thumbprint: None,
    }
}

async fn terminal(
    router: &CommandRouter,
    session_id: &str,
    operation: RoomTerminalOperation,
) -> RoomTerminalResult {
    let request = LocalDaemonRequest::RoomTerminal(RoomTerminalRequest {
        session_id: session_id.to_string(),
        acting_agent: None,
        host_agent: None,
        operation,
    });
    let command = KernelCommand::from_local_request("room-terminal", None, None, &request);
    match router.dispatch(command, request).await {
        Ok(LocalDaemonResponse::RoomTerminal { result }) => result,
        other => panic!("unexpected terminal response: {other:?}"),
    }
}

/// One pushed event: its id, its JSON header and the raw bytes after it.
async fn next_event(
    rx: &mut tokio::sync::mpsc::Receiver<RelayEnvelope>,
    key: &str,
) -> (String, u64, serde_json::Value, Vec<u8>) {
    let RelayEnvelope::DaemonEvent {
        subscription_id,
        event_id,
        encrypted_event,
    } = rx.recv().await.expect("the stream should push an event")
    else {
        panic!("expected an event")
    };
    let plaintext = relay_crypto::decrypt_payload_for_private_key(key, &encrypted_event)
        .unwrap()
        .plaintext;
    if !plaintext.starts_with(b"CXT1") {
        return (
            subscription_id,
            event_id,
            serde_json::from_slice(&plaintext).unwrap(),
            Vec::new(),
        );
    }
    let size = u32::from_be_bytes(plaintext[4..8].try_into().unwrap()) as usize;
    let header: serde_json::Value = serde_json::from_slice(&plaintext[8..8 + size]).unwrap();
    let data = plaintext[8 + size..].to_vec();
    assert_eq!(header["bytes"], serde_json::json!([0, data.len()]));
    (subscription_id, event_id, header, data)
}

#[tokio::test]
async fn room_terminal_output_is_pushed_to_one_subscription_without_read_requests() {
    let _guard = relay_client_test_guard().await;
    let worktree = crate::test_support::TestWorktree::new("room_terminal_stream");
    let config = DaemonConfig::for_tests();
    let app = DaemonApp::bootstrap(config.clone()).unwrap();
    let session_id = app
        .sessions_mut()
        .create_session(worktree.session_request())
        .unwrap()
        .id()
        .to_string();
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));
    let RoomTerminalResult::Opened { terminal: opened } = terminal(
        &router,
        &session_id,
        RoomTerminalOperation::Open {
            cols: 80,
            rows: 24,
            title: None,
            command: None,
        },
    )
    .await
    else {
        panic!("the Room terminal should open");
    };
    let key = relay_crypto::generate_private_key_base64();
    let public_key = relay_crypto::public_key_from_private_key_base64(&key).unwrap();
    approve_test_terminal(&config, &public_key);
    let (tx, mut answers, mut rx) = RelayOutgoingSender::channel(256);
    let tasks = RelaySubscriptionTasks::default();
    handle_relay_subscribe(
        &router,
        &tx,
        &tasks,
        &Arc::new(RelayEventRuntime::for_tests(64)),
        "subscribe-1".into(),
        "stream-1".into(),
        session_id.clone(),
        opened.terminal_id.clone(),
        Some(owner_identity()),
        public_key,
        Some("room_terminal".into()),
        Some(0),
        Some(1000),
    )
    .await
    .unwrap();
    let RelayEnvelope::DaemonResponse {
        relay_request_id,
        encrypted_response,
        error,
    } = answers.recv().await.unwrap()
    else {
        panic!("the subscription should be answered")
    };
    assert_eq!(relay_request_id, "subscribe-1");
    assert!(
        error.is_none() && encrypted_response.is_some(),
        "the Room terminal stream was refused: {error:?}"
    );

    terminal(
        &router,
        &session_id,
        RoomTerminalOperation::Input {
            terminal_id: opened.terminal_id.clone(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(
                "printf 'PUSHED_%s\\n' ONE TWO; seq 1 200000; sleep 2; printf 'PUSHED_THREE\\n'\r",
            ),
        },
    )
    .await;

    let received = tokio::time::timeout(Duration::from_secs(20), async {
        let (mut output, mut offset) = (Vec::new(), 0);
        let (mut pages, mut summaries, mut heartbeats, mut lists) = (0, 0, 0, Vec::new());
        while !String::from_utf8_lossy(&output).contains("PUSHED_THREE\r\n") {
            let (subscription, event_id, header, data) = next_event(&mut rx, &key).await;
            assert_eq!(subscription, "stream-1");
            match header["event"].as_str() {
                Some("room_terminal_output") => {
                    let next = header["next_offset"].as_u64().unwrap();
                    assert_eq!(next, offset + data.len() as u64, "output pages join up");
                    assert_eq!(event_id, next, "the event id is the offset to resume from");
                    offset = next;
                    output.extend(data);
                    pages += 1;
                    if !header["terminal"].is_null() {
                        summaries += 1;
                    }
                }
                Some("heartbeat") => heartbeats += 1,
                Some("room_terminals_listed") => lists.push(header["terminals"].clone()),
                other => panic!("unexpected stream event {other:?}: {header}"),
            }
        }
        (
            String::from_utf8_lossy(&output).into_owned(),
            pages,
            summaries,
            heartbeats,
            lists,
        )
    })
    .await;
    for task in tasks.lock().await.values() {
        task.handle.abort();
    }
    router.runtime_state().shutdown_cleanup().await.unwrap();
    let (output, pages, summaries, heartbeats, lists) =
        received.expect("the program's output should be pushed as it is written");
    assert!(output.contains("PUSHED_ONE\r\nPUSHED_TWO\r\n1\r\n"));
    assert!(output.contains("\r\n200000\r\n"), "every page arrives");
    assert!(
        heartbeats >= 1,
        "a quiet terminal still proves the stream lives"
    );
    assert_eq!(
        lists
            .first()
            .and_then(|tabs| tabs[0]["terminal_id"].as_str()),
        Some(opened.terminal_id.as_str()),
        "the stream starts with the viewer's tabs"
    );
    assert!(
        lists.len() < pages,
        "the tabs travel when they change, not with every page"
    );
    assert!(
        pages >= summaries + 4,
        "the summary travels when it changes, not with every page ({summaries} of {pages})"
    );
}

#[tokio::test]
async fn a_room_terminal_stream_without_a_start_offset_is_refused() {
    let _guard = relay_client_test_guard().await;
    let config = DaemonConfig::for_tests();
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let session_id = create_test_session(&mut app, "workspace", "worktree");
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));
    let key = relay_crypto::generate_private_key_base64();
    let public_key = relay_crypto::public_key_from_private_key_base64(&key).unwrap();
    approve_test_terminal(&config, &public_key);
    let (tx, mut answers, _events) = RelayOutgoingSender::channel(8);
    let tasks = RelaySubscriptionTasks::default();
    handle_relay_subscribe(
        &router,
        &tx,
        &tasks,
        &Arc::new(RelayEventRuntime::for_tests(8)),
        "subscribe-1".into(),
        "stream-1".into(),
        session_id,
        "term-missing".into(),
        Some(owner_identity()),
        public_key,
        Some("room_terminal".into()),
        None,
        None,
    )
    .await
    .unwrap();
    let RelayEnvelope::DaemonResponse { error, .. } =
        tokio::time::timeout(Duration::from_secs(5), answers.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("the subscription should be answered")
    };
    router.runtime_state().shutdown_cleanup().await.unwrap();
    let error = error.expect("a stream needs the offset it starts from");
    assert!(error.message.contains("offset to start from"), "{error:?}");
}
